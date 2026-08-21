import asyncio
import logging
import json
import subprocess
import threading
from collections import deque
from pathlib import Path
from typing import Dict, List, Optional, Any

from agent.config import FARMER_EXE, FARMER_BIN_DIR

logger = logging.getLogger(__name__)

class ProcessManager:
    """
    Manages TwitchDropsBot as a single native C# .NET process running all
    assigned accounts in parallel via async task loops (Task.WhenAll).
    Implements the exact same interface as DockerManager for 100% compatibility.
    """
    def __init__(self):
        self.exe_path = FARMER_EXE
        self.bin_dir = FARMER_BIN_DIR
        self.active_jobs: Dict[str, Dict[str, Any]] = {}  # job_id -> {account, target, limits}
        self.process: Optional[subprocess.Popen] = None
        self.log_buffer = deque(maxlen=20000)
        self.log_reader_thread: Optional[threading.Thread] = None
        self._restart_task: Optional[asyncio.Task] = None
        self._lock = asyncio.Lock()

    def ensure_farmer_image(self):
        """Verify that TwitchDropsBot.Console.exe is present and executable, and kill orphan instances."""
        logger.info(f"Checking Native Farmer Binary at: {self.exe_path}...")
        if not self.exe_path.exists():
            raise FileNotFoundError(
                f"Farmer binary not found at {self.exe_path}! Please ensure farmer_bin is deployed."
            )
        # Kill any orphan TwitchDropsBot processes from previous runs
        self._terminate_process()
        try:
            import os
            if os.name == 'nt':
                subprocess.run(["taskkill", "/f", "/im", "TwitchDropsBot.Console.exe"], capture_output=True)
            else:
                subprocess.run(["pkill", "-9", "-f", "TwitchDropsBot.Console"], capture_output=True)
        except Exception:
            pass
        logger.info(f"Native Farmer Binary verified: {self.exe_path} (Process Mode Active)")

    async def spawn_container(self, job_id: str, account: dict, target: dict, limits: dict) -> str:
        """Register a job/account and trigger single-process startup/reload."""
        login = account.get('login', f'job_{job_id[:8]}')
        logger.info(f"[ProcessManager] Adding account {login} for job {job_id[:8]} ({target.get('game', 'Unknown')})")

        async with self._lock:
            self.active_jobs[job_id] = {
                'account': account,
                'target': target,
                'limits': limits,
                'job_id': job_id,
                'login': login,
                'game': target.get('game', 'Grand Theft Auto V')
            }

        # Debounce restart: if multiple jobs arrive in a batch (e.g. 85 at once),
        # restart the process only once after the batch settles (1.5 seconds delay).
        self._schedule_debounced_restart(delay=1.5)

        cid = f"proc_{job_id[:8]}"
        return cid

    async def stop_container(self, container_id: str, job_id: str = None) -> bool:
        """Remove a job/account and reload or stop the unified process."""
        target_job_id = None
        async with self._lock:
            if job_id and job_id in self.active_jobs:
                target_job_id = job_id
            elif container_id:
                clean_cid = container_id.replace("proc_", "")
                for jid in list(self.active_jobs.keys()):
                    if jid[:8] == clean_cid or jid == container_id:
                        target_job_id = jid
                        break

            if target_job_id:
                login = self.active_jobs[target_job_id]['login']
                logger.info(f"[ProcessManager] Stopping job {target_job_id[:8]} (account: {login})")
                del self.active_jobs[target_job_id]
                self._schedule_debounced_restart(delay=1.0)
                return True
            else:
                logger.debug(f"[ProcessManager] stop_container: job not found for cid={container_id}, jid={job_id}")
                return False

    async def stop_all(self):
        """Stop all running jobs and kill the process."""
        async with self._lock:
            self.active_jobs.clear()
        if self._restart_task and not self._restart_task.done():
            self._restart_task.cancel()
        self._terminate_process()
        logger.info("[ProcessManager] All accounts stopped and process terminated.")

    def _schedule_debounced_restart(self, delay: float = 1.5):
        if self._restart_task and not self._restart_task.done():
            self._restart_task.cancel()

        async def _debounced():
            try:
                await asyncio.sleep(delay)
                await self._apply_process_state()
            except asyncio.CancelledError:
                pass

        self._restart_task = asyncio.create_task(_debounced())

    async def _apply_process_state(self):
        """Write the unified config.json and start/restart the single process."""
        async with self._lock:
            jobs_snapshot = list(self.active_jobs.values())

        if not jobs_snapshot:
            self._terminate_process()
            logger.info("[ProcessManager] No active accounts remaining. Process stopped.")
            return

        # Build unified config.json
        favourite_games = list({j['game'] for j in jobs_snapshot if j.get('game')})
        twitch_users = []

        for j in jobs_snapshot:
            acc = j['account']
            login = acc.get('login')
            auth_token = acc.get('auth_token', '')
            client_secret = acc.get('client_secret', '')
            twitch_user_id = acc.get('twitch_user_id', '')
            game = j['game']
            priority_streamers = j.get('target', {}).get('priority_streamers', [])

            twitch_users.append({
                "Enabled": True,
                "AuthToken": auth_token,
                "ClientSecret": client_secret,
                "Id": twitch_user_id,
                "Login": login,
                "FavouriteGames": [game]
            })

        config_data = {
            "FavouriteGames": favourite_games,
            "TwitchSettings": {
                "TwitchUsers": twitch_users,
                "AvoidCampaign": [],
                "OnlyFavouriteGames": True,
                "ForceTryWithTags": True,
                "OnlyConnectedAccounts": False,
                "PriorityChannels": [],
                "WatchManager": "WatchRequest"
            },
            "LaunchOnStartup": False,
            "LogLevel": 0,
            "WebhookURL": "",
            "WaitingSeconds": 120,
            "AttemptToWatch": 9999,
            "WatchBrowserHeadless": True,
            "MinimizeInTray": False
        }

        config_dir = self.bin_dir / "Configuration"
        config_dir.mkdir(parents=True, exist_ok=True)
        config_file = config_dir / "config.json"

        try:
            with open(config_file, 'w', encoding='utf-8') as f:
                json.dump(config_data, f, indent=2)
            logger.info(f"[ProcessManager] Saved unified config with {len(twitch_users)} account(s) to {config_file}")
        except Exception as e:
            logger.error(f"[ProcessManager] Failed to write config.json: {e}")
            return

        # Restart process with the new unified config
        self._terminate_process()
        self._start_process()

    def _start_process(self):
        """Spawn TwitchDropsBot.Console.exe and start stdout reader thread."""
        try:
            import os
            env = os.environ.copy()
            env['INSIDE_DOCKER'] = 'true'

            logger.info(f"[ProcessManager] Launching native TwitchDropsBot process: {self.exe_path}")
            self.process = subprocess.Popen(
                [str(self.exe_path)],
                cwd=str(self.bin_dir),
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                encoding='utf-8',
                errors='replace',
                bufsize=1,
                env=env
            )
            logger.info(f"[ProcessManager] TwitchDropsBot started successfully (PID: {self.process.pid})")

            # Start background reader thread
            self.log_reader_thread = threading.Thread(
                target=self._read_process_output,
                args=(self.process,),
                daemon=True
            )
            self.log_reader_thread.start()
        except Exception as e:
            logger.error(f"[ProcessManager] Failed to start TwitchDropsBot process: {e}")

    def _read_process_output(self, proc: subprocess.Popen):
        try:
            for line in iter(proc.stdout.readline, ''):
                clean_line = line.rstrip()
                if clean_line:
                    self.log_buffer.append(clean_line)
            proc.stdout.close()
        except Exception as e:
            logger.debug(f"[ProcessManager] Output reader terminated: {e}")

    def _terminate_process(self):
        if self.process:
            logger.info(f"[ProcessManager] Terminating process (PID: {self.process.pid})...")
            try:
                self.process.terminate()
                try:
                    self.process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    self.process.wait(timeout=1)
            except Exception as e:
                logger.debug(f"[ProcessManager] Error terminating process: {e}")
            self.process = None

    async def list_running_containers(self) -> List[Dict[str, Any]]:
        """Return list of all active farming accounts in the standard format."""
        async with self._lock:
            jobs_snapshot = list(self.active_jobs.values())

        is_running = self.process is not None and self.process.poll() is None

        res = []
        for j in jobs_snapshot:
            job_id = j['job_id']
            login = j['login']
            game = j['game']
            acc_id = str(j['account'].get('id', ''))
            cid = f"proc_{job_id[:8]}"

            res.append({
                'container_id': cid,
                'name': f'tdc-farm-{job_id[:8]}',
                'status': 'running' if is_running else 'stopped',
                'login': login,
                'game': game,
                'labels': {
                    'tdc.login': login,
                    'tdc.job_id': str(job_id),
                    'tdc.game': game,
                    'tdc.account_id': acc_id
                }
            })
        return res

    async def get_running_container_count(self) -> int:
        is_running = self.process is not None and self.process.poll() is None
        return len(self.active_jobs) if is_running else 0

    async def get_container_status(self, container_id: str) -> str:
        """Return running or stopped."""
        if self.process is not None and self.process.poll() is None:
            return 'running'
        return 'stopped'

    async def cleanup_dead_containers(self):
        """Supervisor check: auto-recover if native process exited unexpectedly."""
        if self.process and self.process.poll() is not None:
            if self.active_jobs:
                logger.warning("[ProcessManager] Process exited unexpectedly. Triggering self-healing restart...")
                self._schedule_debounced_restart(delay=2.0)



    async def get_container_logs(
        self,
        container_id: str = None,
        job_id: str = None,
        login: str = None,
        tail: int = 0
    ) -> str:
        """
        Return logs for the specified account/job filtered from the unified buffer,
        or the full log if no specific filter matches.
        """
        # Determine target login
        target_login = login
        if not target_login and job_id:
            async with self._lock:
                if job_id in self.active_jobs:
                    target_login = self.active_jobs[job_id]['login']
                else:
                    for jid, info in self.active_jobs.items():
                        if jid[:8] == job_id[:8]:
                            target_login = info['login']
                            break

        lines = list(self.log_buffer)

        if target_login:
            tag1 = f"[TwitchUser - {target_login}]"
            tag2 = f'"{target_login}"'
            user_lines = [
                line for line in lines
                if tag1 in line or tag2 in line or "Starting bot for user" in line or "[ - ]" in line
            ]
            if user_lines:
                if tail > 0:
                    user_lines = user_lines[-tail:]
                return "\n".join(user_lines)

        # Default fallback
        if tail > 0:
            lines = lines[-tail:]
        return "\n".join(lines)

    async def run_auth_container(self, acc: dict, temp_dir: str):
        """Run standalone auth process for Twitch Device Code authorization."""
        login = acc.get('login')
        config_path = Path(temp_dir) / f"config-{login}.json"
        
        env = {
            "ADD_ACCOUNT": "true",
            "INSIDE_DOCKER": "false"
        }
        
        proc = subprocess.Popen(
            [str(self.exe_path), "--add-account"],
            cwd=temp_dir,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True
        )
        return proc

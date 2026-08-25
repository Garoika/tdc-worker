import asyncio
import logging
import json
import subprocess
import threading
from collections import deque
from pathlib import Path
from typing import Dict, List, Optional, Any

from agent.config import FARMER_EXE, FARMER_BIN_DIR
from agent.state_manager import state_manager

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

    @staticmethod
    def _find_dotnet_binary() -> Optional[str]:
        import shutil, os
        dotnet_cmd = shutil.which("dotnet")
        if dotnet_cmd:
            return dotnet_cmd
        candidate_paths = [
            "/usr/bin/dotnet",
            "/usr/local/bin/dotnet",
            "/snap/bin/dotnet",
            "/opt/dotnet/dotnet",
            os.path.expanduser("~/.dotnet/dotnet"),
            "/usr/share/dotnet/dotnet",
            "/var/lib/snapd/snap/bin/dotnet"
        ]
        for p in candidate_paths:
            if os.path.isfile(p) and os.access(p, os.X_OK):
                return p
        return None

    @classmethod
    def _auto_install_dotnet(cls):
        """Automatically install .NET runtime on Linux via pacman, apt, or Microsoft user-level script."""
        import shutil, subprocess, os
        logger.info("[ProcessManager] Attempting automatic .NET Runtime installation...")
        
        # 1. Try pacman (Arch Linux / Manjaro)
        if shutil.which("pacman"):
            try:
                cmd = ["sudo", "pacman", "-Sy", "--noconfirm", "dotnet-runtime"]
                if os.name != 'nt' and hasattr(os, 'geteuid') and os.geteuid() == 0:
                    cmd = ["pacman", "-Sy", "--noconfirm", "dotnet-runtime"]
                subprocess.run(cmd, check=True)
                logger.info("[ProcessManager] Successfully installed dotnet-runtime via pacman!")
                return
            except Exception as e:
                logger.debug(f"[ProcessManager] pacman dotnet install attempt: {e}")

        # 2. Try apt-get (Debian / Ubuntu)
        if shutil.which("apt-get"):
            try:
                cmd = ["sudo", "apt-get", "install", "-y", "dotnet-runtime-8.0"]
                if os.name != 'nt' and hasattr(os, 'geteuid') and os.geteuid() == 0:
                    cmd = ["apt-get", "install", "-y", "dotnet-runtime-8.0"]
                subprocess.run(cmd, check=True)
                logger.info("[ProcessManager] Successfully installed dotnet-runtime via apt-get!")
                return
            except Exception as e:
                logger.debug(f"[ProcessManager] apt-get dotnet install attempt: {e}")

        # 3. Universal user-level fallback via Microsoft official installer (no root password needed)
        try:
            home_dotnet = os.path.expanduser("~/.dotnet")
            install_cmd = f'curl -sSL https://dot.net/v1/dotnet-install.sh | bash -s -- --runtime dotnet --channel 8.0 --install-dir "{home_dotnet}"'
            subprocess.run(install_cmd, shell=True, check=True)
            os.environ["PATH"] = f"{home_dotnet}:{os.environ.get('PATH', '')}"
            os.environ["DOTNET_ROOT"] = home_dotnet
            logger.info(f"[ProcessManager] Successfully installed user-level .NET 8.0 into {home_dotnet}!")
        except Exception as e:
            logger.error(f"[ProcessManager] Failed to auto-install .NET runtime: {e}")

    def ensure_farmer_image(self):
        """Verify that TwitchDropsBot binary or dll is present, and kill orphan instances."""
        dll_path = self.bin_dir / 'TwitchDropsBot.Console.dll'
        exe_path = self.bin_dir / 'TwitchDropsBot.Console.exe'
        linux_bin = self.bin_dir / 'TwitchDropsBot.Console'

        import os
        if os.name != 'nt':
            dotnet_bin = self._find_dotnet_binary()
            if not dotnet_bin and not (linux_bin.exists() and not linux_bin.name.endswith('.exe')):
                logger.warning("[ProcessManager] .NET runtime (dotnet) not found! Starting automatic installation...")
                self._auto_install_dotnet()
                dotnet_bin = self._find_dotnet_binary()

            logger.info(f"Checking Native Farmer Binary on Linux (dotnet: {dotnet_bin or 'system PATH'})...")
            if not dll_path.exists() and not exe_path.exists() and not linux_bin.exists():
                raise FileNotFoundError(
                    f"Farmer binary/dll not found at {self.bin_dir}! Please ensure farmer_bin is deployed."
                )
        else:
            logger.info(f"Checking Native Farmer Binary at: {self.exe_path}...")
            if not self.exe_path.exists() and not dll_path.exists():
                raise FileNotFoundError(
                    f"Farmer binary not found at {self.exe_path} or {dll_path}! Please ensure farmer_bin is deployed."
                )

        # Kill any orphan TwitchDropsBot processes from previous runs
        self._terminate_process()
        try:
            if os.name == 'nt':
                subprocess.run(["taskkill", "/f", "/im", "TwitchDropsBot.Console.exe"], capture_output=True)
            else:
                subprocess.run(["pkill", "-9", "-f", "TwitchDropsBot.Console"], capture_output=True)
        except Exception:
            pass
        logger.info("Native Farmer Binary verified (Process Mode Active)")

    async def spawn_container(self, job_id: str, account: dict, target: dict, limits: dict) -> str:
        """Register a job/account and trigger single-process startup/reload."""
        login = account.get('login', f'job_{job_id[:8]}')
        logger.info(f"[ProcessManager] Adding account {login} for job {job_id[:8]} ({target.get('game', 'Unknown')})")

        async with self._lock:
            self.active_jobs[job_id] = {
                'job_id': job_id,
                'account': account,
                'target': target,
                'limits': limits,
                'login': login,
                'game': target.get('game', '')
            }
            state_manager.update_jobs(self.active_jobs)

        # Apply state with debounced restart
        self._schedule_debounced_restart(delay=1.5)
        return f"proc_{job_id[:8]}"

    async def stop_container(self, container_id: str = None, job_id: str = None) -> bool:
        """Unregister a job/account and trigger single-process reload."""
        async with self._lock:
            target_jid = None
            if job_id and job_id in self.active_jobs:
                target_jid = job_id
            elif container_id and container_id.startswith("proc_"):
                clean_jid = container_id.replace("proc_", "")
                for jid in self.active_jobs:
                    if jid.startswith(clean_jid) or jid == container_id:
                        target_jid = jid
                        break
            elif job_id:
                for jid in self.active_jobs:
                    if jid.startswith(job_id[:8]) or jid == job_id:
                        target_jid = jid
                        break

            if target_jid and target_jid in self.active_jobs:
                login = self.active_jobs[target_jid]['login']
                del self.active_jobs[target_jid]
                state_manager.update_jobs(self.active_jobs)
                logger.info(f"[ProcessManager] Removed account {login} (job {target_jid[:8]}). Remaining: {len(self.active_jobs)}")
                self._schedule_debounced_restart(delay=1.5)
                return True
            else:
                logger.debug(f"[ProcessManager] stop_container: job not found for cid={container_id}, jid={job_id}")
                return False

    async def stop_all(self):
        """Stop all running jobs and kill the process."""
        async with self._lock:
            self.active_jobs.clear()
            state_manager.update_jobs(self.active_jobs)
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
                "GpuMode": "None",
                "AppDirectory": "",
                "StreamQuality": "None",
                "ClaimDrops": True,
                "ClaimMoments": False,
                "ClaimBadges": True,
                "AutoReloadLiveStreamers": True,
                "AutoReloadInactiveStreamers": True,
                "PriorityStreamers": [],
                "BlacklistedStreamers": [],
                "StreamerSelectionStrategy": "LowestViewers",
                "LiveStreamerCacheExpiration": 15,
                "InactiveStreamerCacheExpiration": 60,
                "PeriodicChannelPointsClaim": False,
                "WatchPreferences": {
                    "Reruns": False,
                    "Drops": True,
                    "NonDrops": False
                }
            }
        }

        config_dir = self.bin_dir / 'Configuration'
        config_dir.mkdir(parents=True, exist_ok=True)
        config_path = config_dir / 'config.json'

        try:
            with open(config_path, 'w', encoding='utf-8') as f:
                json.dump(config_data, f, indent=2)
            logger.info(f"[ProcessManager] Saved unified config with {len(twitch_users)} account(s) to {config_path}")
        except Exception as e:
            logger.error(f"[ProcessManager] Failed to write config.json: {e}")
            return

        # Restart process with the new unified config
        self._terminate_process()
        self._start_process()

    def _get_launch_cmd(self) -> list:
        import os
        dll_path = self.bin_dir / 'TwitchDropsBot.Console.dll'
        exe_path = self.bin_dir / 'TwitchDropsBot.Console.exe'
        linux_bin = self.bin_dir / 'TwitchDropsBot.Console'

        # On Linux / macOS:
        if os.name != 'nt':
            if linux_bin.exists() and not linux_bin.name.endswith('.exe'):
                try:
                    os.chmod(linux_bin, 0o755)
                    return [str(linux_bin)]
                except Exception:
                    pass

            dotnet_path = self._find_dotnet_binary() or "dotnet"
            target_file = dll_path if dll_path.exists() else exe_path
            return [dotnet_path, str(target_file)]

        # On Windows (nt):
        if exe_path.exists():
            return [str(exe_path)]
        if dll_path.exists():
            dotnet_path = self._find_dotnet_binary() or "dotnet"
            return [dotnet_path, str(dll_path)]
        return [str(self.exe_path)]

    def _start_process(self):
        """Spawn native farmer process and start stdout reader thread."""
        try:
            import os
            env = os.environ.copy()
            env['INSIDE_DOCKER'] = 'true'

            cmd = self._get_launch_cmd()
            logger.info(f"[ProcessManager] Launching native TwitchDropsBot: {' '.join(cmd)}")
            self.process = subprocess.Popen(
                cmd,
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
            state_manager.update_system_info(farmer_pid=self.process.pid)

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
            state_manager.update_system_info(farmer_pid=None)

    async def list_running_containers(self) -> List[Dict[str, Any]]:
        """Return list of all active farming accounts in the standard format."""
        async with self._lock:
            jobs_snapshot = list(self.active_jobs.values())

        is_running = self.process is not None and self.process.poll() is None

        res = []
        for j in jobs_snapshot:
            job_id = j.get('job_id') or ''
            login = j.get('login', '')
            game = j.get('game', '')
            acc_id = str(j.get('account', {}).get('id', ''))
            cid = f"proc_{job_id[:8]}" if job_id else f"proc_{login}"

            res.append({
                'container_id': cid,
                'name': f'tdc-farm-{job_id[:8]}' if job_id else f'tdc-farm-{login}',
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
        Return logs strictly for the specified account/job filtered from the unified buffer.
        Never leaks logs from other accounts to prevent telemetry cross-talk between games.
        """
        target_login = login
        if not target_login and container_id and container_id.startswith("proc_"):
            clean_jid = container_id.replace("proc_", "")
            async with self._lock:
                for jid, info in self.active_jobs.items():
                    if jid.startswith(clean_jid) or jid == container_id:
                        target_login = info['login']
                        break

        if not target_login and job_id:
            async with self._lock:
                if job_id in self.active_jobs:
                    target_login = self.active_jobs[job_id]['login']
                else:
                    for jid, info in self.active_jobs.items():
                        if jid.startswith(job_id[:8]) or jid == job_id:
                            target_login = info['login']
                            break

        lines = list(self.log_buffer)

        if target_login:
            tag = f"[TwitchUser - {target_login}]"
            user_lines = [line for line in lines if tag in line]
            if tail > 0:
                user_lines = user_lines[-tail:]
            return "\n".join(user_lines)

        # Global logs fallback if no specific account requested (used by Full Worker Logs modal)
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

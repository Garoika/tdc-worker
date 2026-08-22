import os
import sys
import subprocess
import asyncio
import logging

logger = logging.getLogger("worker.autoupdate")

class AutoUpdater:
    def __init__(self, repo_dir: str = None, check_interval: int = 30):
        self.repo_dir = repo_dir or os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
        self.worker_dir = os.path.join(self.repo_dir, "worker")
        self.check_interval = check_interval
        self._running = False

    def restart_process(self):
        """Cleanly restart the worker agent process on Windows or Linux."""
        logger.info(f"🚀 [AutoUpdate] Restarting worker process in {self.worker_dir}...")
        
        if sys.platform == "win32":
            # On Windows: start_worker.bat loop will automatically restart the process in the same console.
            # Cleanly exit so no duplicate rogue processes are created.
            os._exit(0)
        else:
            # On Linux/POSIX: execv seamlessly replaces current process
            try:
                os.chdir(self.worker_dir)
                os.execv(sys.executable, [sys.executable, "-m", "agent.main"])
            except Exception as e:
                logger.error(f"[AutoUpdate] Failed to execv on Linux: {e}")
                os._exit(0)

    async def check_and_update(self) -> bool:
        """Checks GitHub for new commits, pulls changes, updates dependencies, and restarts worker."""
        git_dir = os.path.join(self.repo_dir, ".git")
        if not os.path.exists(git_dir):
            return False

        try:
            # 1. Fetch latest commits from GitHub
            proc = await asyncio.create_subprocess_exec(
                "git", "fetch", "origin", "main",
                cwd=self.repo_dir,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE
            )
            await proc.communicate()

            # 2. Compare local HEAD with remote origin/main
            proc_local = await asyncio.create_subprocess_exec(
                "git", "rev-parse", "HEAD",
                cwd=self.repo_dir,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE
            )
            stdout_local, _ = await proc_local.communicate()
            local_commit = stdout_local.decode().strip()

            proc_remote = await asyncio.create_subprocess_exec(
                "git", "rev-parse", "origin/main",
                cwd=self.repo_dir,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE
            )
            stdout_remote, _ = await proc_remote.communicate()
            remote_commit = stdout_remote.decode().strip()

            if local_commit and remote_commit and local_commit != remote_commit:
                logger.info(f"🔄 [AutoUpdate] New version detected on GitHub ({local_commit[:7]} -> {remote_commit[:7]})! Updating repository...")
                
                proc_pull = await asyncio.create_subprocess_exec(
                    "git", "reset", "--hard", "origin/main",
                    cwd=self.repo_dir,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE
                )
                stdout_p, stderr_p = await proc_pull.communicate()
                
                # Verify that HEAD actually changed to remote_commit
                proc_verify = await asyncio.create_subprocess_exec(
                    "git", "rev-parse", "HEAD",
                    cwd=self.repo_dir,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE
                )
                stdout_v, _ = await proc_verify.communicate()
                new_local_commit = stdout_v.decode().strip()

                if new_local_commit != remote_commit:
                    logger.warning(f"[AutoUpdate] Git reset failed to update HEAD to {remote_commit[:7]}. Output: {stderr_p.decode().strip()}. Skipping restart to avoid infinite loop.")
                    return False

                logger.info(f"[AutoUpdate] Successfully updated to {remote_commit[:7]}.")

                # 3. Update pip dependencies if requirements.txt exists
                req_file = os.path.join(self.worker_dir, "requirements.txt")
                if os.path.exists(req_file):
                    try:
                        proc_pip = await asyncio.create_subprocess_exec(
                            sys.executable, "-m", "pip", "install", "-q", "-r", req_file,
                            cwd=self.worker_dir,
                            stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL
                        )
                        await proc_pip.wait()
                    except Exception:
                        pass

                # 4. Restart process immediately
                self.restart_process()
                return True

        except Exception as e:
            logger.debug(f"[AutoUpdate] Check error: {e}")

        return False

    async def start_loop(self):
        """Background loop constantly checking for updates."""
        self._running = True
        logger.info(f"🔄 Auto-updater service active (checking GitHub every {self.check_interval}s)")
        while self._running:
            await asyncio.sleep(self.check_interval)
            await self.check_and_update()

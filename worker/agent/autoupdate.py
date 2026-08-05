import os
import sys
import subprocess
import asyncio
import logging

logger = logging.getLogger("worker.autoupdate")

class AutoUpdater:
    def __init__(self, repo_dir: str = None, check_interval: int = 60):
        self.repo_dir = repo_dir or os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
        self.check_interval = check_interval
        self._running = False

    async def check_and_update(self) -> bool:
        git_dir = os.path.join(self.repo_dir, ".git")
        if not os.path.exists(git_dir):
            return False

        try:
            # 1. Fetch latest changes from GitHub
            proc = await asyncio.create_subprocess_exec(
                "git", "fetch", "origin", "main",
                cwd=self.repo_dir,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE
            )
            await proc.communicate()

            # 2. Compare local HEAD with origin/main
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
                logger.info(f"🔄 [AutoUpdate] New version detected on GitHub ({local_commit[:7]} -> {remote_commit[:7]})! Pulling update...")
                
                proc_pull = await asyncio.create_subprocess_exec(
                    "git", "pull", "origin", "main",
                    cwd=self.repo_dir,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE
                )
                stdout_p, _ = await proc_pull.communicate()
                logger.info(f"[AutoUpdate] Git pull: {stdout_p.decode().strip()}")

                from agent.config import DOCKER_IMAGE
                proc_docker = await asyncio.create_subprocess_exec(
                    "docker", "pull", DOCKER_IMAGE,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL
                )
                await proc_docker.wait()

                logger.info("🚀 [AutoUpdate] Restarting worker process with new code...")
                os.execv(sys.executable, [sys.executable, "-m", "agent.main"])
                return True

        except Exception as e:
            logger.warning(f"Auto-update check error: {e}")

        return False

    async def start_loop(self):
        self._running = True
        logger.info(f"🔄 Auto-updater background service active (checking GitHub every {self.check_interval}s)")
        while self._running:
            await asyncio.sleep(self.check_interval)
            await self.check_and_update()

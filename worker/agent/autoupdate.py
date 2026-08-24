import os
import sys
import json
import shutil
import logging
import asyncio
import zipfile
import subprocess
import urllib.request
from pathlib import Path

logger = logging.getLogger("worker.autoupdate")

class AutoUpdater:
    def __init__(self, repo_dir: str = None, check_interval: int = 30):
        self.repo_dir = repo_dir or os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
        self.worker_dir = os.path.join(self.repo_dir, "worker")
        self.check_interval = check_interval
        self._running = False
        self.repo_name = "Garoika/tdc-worker"
        self.branch = "main"

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
        """Checks GitHub for new commits via Git (or via GitHub API fallback if git is missing), updates and restarts."""
        git_dir = os.path.join(self.repo_dir, ".git")
        has_git = os.path.exists(git_dir) and bool(shutil.which("git"))

        if has_git:
            return await self._update_via_git()
        else:
            return await self._update_via_api()

    async def _update_via_git(self) -> bool:
        try:
            # 1. Fetch latest commits from GitHub
            proc = await asyncio.create_subprocess_exec(
                "git", "fetch", "origin", self.branch,
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
                "git", "rev-parse", f"origin/{self.branch}",
                cwd=self.repo_dir,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE
            )
            stdout_remote, _ = await proc_remote.communicate()
            remote_commit = stdout_remote.decode().strip()

            if local_commit and remote_commit and local_commit != remote_commit:
                logger.info(f"🔄 [AutoUpdate] New version detected on GitHub ({local_commit[:7]} -> {remote_commit[:7]})! Updating repository via git...")
                
                proc_pull = await asyncio.create_subprocess_exec(
                    "git", "reset", "--hard", f"origin/{self.branch}",
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
                    logger.warning(f"[AutoUpdate] Git reset failed to update HEAD to {remote_commit[:7]}. Output: {stderr_p.decode().strip()}. Skipping restart.")
                    return False

                logger.info(f"[AutoUpdate] Successfully updated to {remote_commit[:7]}.")
                self._update_pip_deps()
                self.restart_process()
                return True

        except Exception as e:
            logger.debug(f"[AutoUpdate] Git check error: {e}")

        return False

    async def _update_via_api(self) -> bool:
        """Git-less fallback: checks GitHub API, downloads zip archive, unpacks and restarts."""
        try:
            sha_file = Path(self.repo_dir) / ".current_commit"
            local_sha = ""
            if sha_file.exists():
                try:
                    local_sha = sha_file.read_text(encoding="utf-8").strip()
                except Exception:
                    pass

            api_url = f"https://api.github.com/repos/{self.repo_name}/commits/{self.branch}"
            req = urllib.request.Request(
                api_url,
                headers={
                    "User-Agent": "TDC-Worker-AutoUpdater",
                    "Accept": "application/vnd.github.v3+json"
                }
            )

            loop = asyncio.get_running_loop()
            def _fetch_commit():
                with urllib.request.urlopen(req, timeout=10) as resp:
                    if resp.status == 200:
                        data = json.loads(resp.read().decode("utf-8"))
                        return data.get("sha", "")
                return ""

            remote_sha = await loop.run_in_executor(None, _fetch_commit)
            if not remote_sha:
                return False

            if not local_sha:
                # First time tracking without git: save current commit and continue
                sha_file.write_text(remote_sha, encoding="utf-8")
                return False

            if local_sha != remote_sha:
                logger.info(f"🔄 [AutoUpdate] New release detected on GitHub ({local_sha[:7]} -> {remote_sha[:7]})! Downloading update zip (Git-less mode)...")

                def _download_and_extract():
                    zip_url = f"https://github.com/{self.repo_name}/archive/refs/heads/{self.branch}.zip"
                    zip_path = Path(self.repo_dir) / "update_pkg.zip"
                    extract_dir = Path(self.repo_dir) / "update_temp_pkg"

                    urllib.request.urlretrieve(zip_url, zip_path)

                    if extract_dir.exists():
                        shutil.rmtree(extract_dir, ignore_errors=True)

                    with zipfile.ZipFile(zip_path, 'r') as zf:
                        zf.extractall(extract_dir)

                    # Subfolder inside zip is usually tdc-worker-main
                    subfolders = [d for d in extract_dir.iterdir() if d.is_dir()]
                    src_dir = subfolders[0] if subfolders else extract_dir

                    # Preserve configs, logs, venv
                    protected = {".worker_config.json", ".worker_state.json", ".venv", "logs", ".git"}
                    for item in src_dir.iterdir():
                        if item.name in protected:
                            continue
                        dest = Path(self.repo_dir) / item.name
                        if item.is_dir():
                            shutil.copytree(item, dest, dirs_exist_ok=True)
                        else:
                            shutil.copy2(item, dest)

                    # Cleanup
                    shutil.rmtree(extract_dir, ignore_errors=True)
                    if zip_path.exists():
                        zip_path.unlink(missing_ok=True)

                    sha_file.write_text(remote_sha, encoding="utf-8")

                await loop.run_in_executor(None, _download_and_extract)
                logger.info(f"[AutoUpdate] Git-less update complete to {remote_sha[:7]}! Restarting worker...")
                self._update_pip_deps()
                self.restart_process()
                return True

        except Exception as e:
            logger.debug(f"[AutoUpdate] API check error: {e}")

        return False

    def _update_pip_deps(self):
        req_file = os.path.join(self.worker_dir, "requirements.txt")
        if os.path.exists(req_file):
            try:
                subprocess.run(
                    [sys.executable, "-m", "pip", "install", "-q", "-r", req_file],
                    cwd=self.worker_dir,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    timeout=60
                )
            except Exception:
                pass

    async def start_loop(self):
        """Background loop constantly checking for updates."""
        self._running = True
        logger.info(f"🔄 Auto-updater service active (checking GitHub every {self.check_interval}s)")
        while self._running:
            await asyncio.sleep(self.check_interval)
            await self.check_and_update()

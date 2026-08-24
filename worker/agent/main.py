import os
import sys
import atexit
import asyncio
import logging
import shutil
import subprocess

from agent.config import MASTER_URL, WORKER_TOKEN, DOCKER_IMAGE, RUNNER_TYPE
from agent.metrics import SystemMetrics
from agent.docker_manager import DockerManager
from agent.process_manager import ProcessManager
from agent.ws_client import WebSocketClient
from agent.autoupdate import AutoUpdater
from agent.state_manager import state_manager

logging.basicConfig(level=logging.INFO, format='%(asctime)s [%(levelname)s] %(name)s: %(message)s')
logger = logging.getLogger('worker.main')

monitor_proc = None

def cleanup_monitor():
    global monitor_proc
    if monitor_proc and monitor_proc.poll() is None:
        try:
            monitor_proc.terminate()
        except Exception:
            pass

def get_linux_terminal_cmd(base_cmd: list) -> list | None:
    """Finds an available desktop terminal emulator on Linux or tmux session."""
    # 1. If inside an active tmux session
    if os.environ.get("TMUX") and shutil.which("tmux"):
        return ["tmux", "split-window", "-h"] + base_cmd

    # 2. If GUI desktop session is active (X11 / Wayland)
    has_display = bool(os.environ.get("DISPLAY") or os.environ.get("WAYLAND_DISPLAY"))
    if has_display:
        candidates = [
            os.environ.get("TERMINAL"),
            "kitty",
            "foot",
            "alacritty",
            "ghostty",
            "gnome-terminal",
            "konsole",
            "xfce4-terminal",
            "mate-terminal",
            "lxterminal",
            "xterm"
        ]
        
        for term in candidates:
            if not term:
                continue
            term_bin = shutil.which(term)
            if not term_bin:
                continue
            term_name = os.path.basename(term_bin).lower()

            if term_name in ("kitty", "foot"):
                return [term_bin, "--title", "TDC Live Monitor"] + base_cmd
            elif term_name in ("alacritty", "ghostty"):
                return [term_bin, "-T", "TDC Live Monitor", "-e"] + base_cmd
            elif term_name == "gnome-terminal":
                return [term_bin, "--title=TDC Live Monitor", "--"] + base_cmd
            elif term_name == "konsole":
                return [term_bin, "-e"] + base_cmd
            elif term_name in ("xfce4-terminal", "mate-terminal", "lxterminal"):
                return [term_bin, "-T", "TDC Live Monitor", "-e", " ".join(base_cmd)]
            elif term_name == "xterm":
                return [term_bin, "-title", "TDC Live Monitor", "-e"] + base_cmd
            else:
                return [term_bin, "-e"] + base_cmd

    return None

def launch_monitor():
    global monitor_proc
    try:
        current_pid = os.getpid()
        base_cmd = [sys.executable, "-m", "agent.monitor", "--parent-pid", str(current_pid)]
        env = os.environ.copy()
        env["PYTHONIOENCODING"] = "utf-8"
        env["PYTHONUTF8"] = "1"

        if os.name == 'nt':
            creationflags = subprocess.CREATE_NEW_CONSOLE
            monitor_proc = subprocess.Popen(base_cmd, creationflags=creationflags, env=env)
            atexit.register(cleanup_monitor)
            logger.info(f"Launched companion Live Monitor Dashboard in new window (PID: {monitor_proc.pid})")
        else:
            term_cmd = get_linux_terminal_cmd(base_cmd)
            if term_cmd:
                monitor_proc = subprocess.Popen(term_cmd, env=env)
                atexit.register(cleanup_monitor)
                logger.info(f"Launched companion Live Monitor in separate terminal ({term_cmd[0]}, PID: {monitor_proc.pid})")
            else:
                logger.info("Headless / no desktop terminal detected. Live Monitor skipped (logs will stream cleanly here). Run 'python -m agent.monitor' in another session if needed.")
    except Exception as e:
        logger.warning(f"Could not launch Live Monitor Console: {e}")

async def main():
    logger.info("Starting Twitch Drops Farm Worker Node")
    logger.info(f"Master URL: {MASTER_URL}")
    logger.info(f"Runner Mode: {RUNNER_TYPE.upper()}")
    if RUNNER_TYPE == 'docker':
        logger.info(f"Docker Image: {DOCKER_IMAGE}")

    # Launch companion Live Monitor dashboard in a second console window
    launch_monitor()
    
    autoupdater = AutoUpdater(check_interval=30)

    metrics = SystemMetrics()
    if RUNNER_TYPE == 'docker':
        runner = DockerManager()
    else:
        runner = ProcessManager()

    runner.ensure_farmer_image()
    ws_client = WebSocketClient(MASTER_URL, WORKER_TOKEN, runner, metrics)
    
    state_manager.update_system_info(
        master_url=MASTER_URL,
        runner_mode=RUNNER_TYPE.upper()
    )

    try:
        await asyncio.gather(
            ws_client.run(),
            autoupdater.start_loop()
        )
    except asyncio.CancelledError:
        logger.info("Shutting down worker...")
    finally:
        ws_client.stop()
        cleanup_monitor()

if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        logger.info("Worker node stopped by user.")
        cleanup_monitor()
        sys.exit(0)

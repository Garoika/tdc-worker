import os
import sys
import atexit
import asyncio
import logging
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

def launch_monitor():
    global monitor_proc
    try:
        current_pid = os.getpid()
        cmd = [sys.executable, "-m", "agent.monitor", "--parent-pid", str(current_pid)]
        creationflags = 0
        if os.name == 'nt':
            creationflags = subprocess.CREATE_NEW_CONSOLE
        monitor_proc = subprocess.Popen(cmd, creationflags=creationflags)
        atexit.register(cleanup_monitor)
        logger.info(f"Launched companion Live Monitor Dashboard (PID: {monitor_proc.pid})")
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

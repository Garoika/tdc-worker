import asyncio
import logging
import sys

from agent.config import MASTER_URL, WORKER_TOKEN, DOCKER_IMAGE
from agent.metrics import SystemMetrics
from agent.docker_manager import DockerManager
from agent.ws_client import WebSocketClient
from agent.autoupdate import AutoUpdater

logging.basicConfig(level=logging.INFO, format='%(asctime)s [%(levelname)s] %(name)s: %(message)s')
logger = logging.getLogger('worker.main')

async def main():
    logger.info("Starting Twitch Drops Farm Worker Node")
    logger.info(f"Master URL: {MASTER_URL}")
    logger.info(f"Docker Image: {DOCKER_IMAGE}")
    
    metrics = SystemMetrics()
    docker_manager = DockerManager()
    ws_client = WebSocketClient(MASTER_URL, WORKER_TOKEN, docker_manager, metrics)
    autoupdater = AutoUpdater(check_interval=60)
    
    try:
        await asyncio.gather(
            ws_client.run(),
            autoupdater.start_loop()
        )
    except asyncio.CancelledError:
        logger.info("Shutting down worker...")
    finally:
        ws_client.stop()

if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        logger.info("Worker node stopped by user.")
        sys.exit(0)

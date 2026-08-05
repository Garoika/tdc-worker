import os
import sys
import time
import logging
import requests
from twitch_client import TwitchClient

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format='[%(asctime)s] [%(levelname)s] %(message)s',
    datefmt='%Y-%m-%d %H:%M:%S'
)
logger = logging.getLogger(__name__)

def main():
    auth_token = os.getenv("AUTH_TOKEN", "")
    proxy_url = os.getenv("PROXY_URL", "")
    target_game = os.getenv("TARGET_GAME", "Rust")
    priority_streamers = os.getenv("PRIORITY_STREAMERS", "")

    logger.info("Starting Twitch Drops Farming Bot...")
    logger.info(f"Target Game: {target_game}")
    logger.info(f"Priority Streamers: {priority_streamers if priority_streamers else 'None'}")
    logger.info(f"Using Proxy: {'Yes' if proxy_url else 'No'}")

    if not auth_token:
        logger.warning("AUTH_TOKEN is not set in environment variables.")

    session = requests.Session()
    if proxy_url:
        session.proxies = {
            "http": proxy_url,
            "https": proxy_url
        }

    client = TwitchClient(auth_token=auth_token, proxy_url=proxy_url if proxy_url else None)

    current_minutes = 0
    required_minutes = 180
    consecutive_errors = 0
    max_errors = 5

    logger.info(f"Watching stream for {target_game}...")

    try:
        while True:
            try:
                # Perform watch tick simulation
                client.send_watch_event(channel_name=priority_streamers.split(",")[0] if priority_streamers else "default_streamer")

                # Reset consecutive error counter on successful operation
                consecutive_errors = 0

                current_minutes += 1
                logger.info(f"{current_minutes}/{required_minutes} minutes watched")

                if current_minutes >= required_minutes:
                    logger.info("Target watch duration reached. Claiming drop...")
                    client.claim_drop(drop_id="drop-target-001")
                    current_minutes = 0

                time.sleep(20)

            except requests.RequestException as exc:
                consecutive_errors += 1
                logger.error(f"Network error encountered ({consecutive_errors}/{max_errors}): {exc}")
                if consecutive_errors >= max_errors:
                    logger.critical("FATAL: Maximum consecutive network errors reached. Exiting.")
                    sys.exit(1)
                time.sleep(5)

    except KeyboardInterrupt:
        logger.info("Bot shutting down gracefully due to KeyboardInterrupt.")
        sys.exit(0)

if __name__ == "__main__":
    main()

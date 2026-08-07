import re
import logging
from agent.docker_manager import DockerManager

logger = logging.getLogger(__name__)

class LogStreamer:
    def __init__(self, docker_manager: DockerManager):
        self.docker_manager = docker_manager
        # Patterns for progress: 88/120, 88 / 120 min, 88 of 120, (73%), Progress: 88/120
        self.prog_re1 = re.compile(r'(\d+)\s*(?:/|of)\s*(\d+)', re.IGNORECASE)
        self.prog_pct_re = re.compile(r'(\d+(?:\.\d+)?)\s*%', re.IGNORECASE)
        
        # Patterns for drop name
        self.drop_re1 = re.compile(r'\[([^\]]+)\]\s*\d+\s*/\s*\d+', re.IGNORECASE)
        self.drop_re2 = re.compile(r'(?:Time based drops|Active drop|Mining drop|Current drop|Drop)\s*[:\-]\s*"?([^"\n\r]+)"?', re.IGNORECASE)
        self.drop_re3 = re.compile(r'Drop\s+"([^"]+)"', re.IGNORECASE)
        self.drop_re4 = re.compile(r'([A-Za-z0-9\s\-]+(?:Points|Drop|Reward|Skin|Chest|Pack|Key|Item|Crate|Spray|Emote))', re.IGNORECASE)
        
        # Patterns for streamer
        self.st_re1 = re.compile(r'SendSpadeEvents accepted for "([^"]+)"', re.IGNORECASE)
        self.st_re2 = re.compile(r'(?:watching|channel|streamer|broadcaster|live on|switched to)\s*[:\-]?\s*"?([a-zA-Z0-9_]{3,25})"?', re.IGNORECASE)
        self.st_re3 = re.compile(r'Channel\s+"([^"]+)"', re.IGNORECASE)

    def parse_logs(self, logs: str) -> dict:
        telemetry = {}
        if not logs:
            return telemetry
            
        for line in logs.splitlines():
            line_str = line.strip()
            if not line_str:
                continue

            # 1. Progress parsing
            prog_match = self.prog_re1.search(line_str)
            if prog_match:
                try:
                    c_min = int(prog_match.group(1))
                    t_min = int(prog_match.group(2))
                    if 0 <= c_min <= 10000 and 0 < t_min <= 10000:
                        telemetry['watched_minutes'] = c_min
                        telemetry['target_minutes'] = t_min
                        telemetry['current_minutes'] = c_min
                        telemetry['required_minutes'] = t_min
                        telemetry['percentage'] = min(100, int(round((c_min / t_min) * 100)))
                except Exception:
                    pass

            pct_match = self.prog_pct_re.search(line_str)
            if pct_match and 'percentage' not in telemetry:
                try:
                    pct_val = int(float(pct_match.group(1)))
                    if 0 <= pct_val <= 100:
                        telemetry['percentage'] = pct_val
                except Exception:
                    pass

            # 2. Drop name parsing
            d_match = self.drop_re1.search(line_str) or self.drop_re2.search(line_str) or self.drop_re3.search(line_str) or self.drop_re4.search(line_str)
            if d_match:
                d_name = d_match.group(1).strip(' "\'[]:-,')
                if d_name and len(d_name) >= 2 and not d_name.isdigit() and d_name.lower() not in ['info', 'warning', 'error', 'debug', 'minutes', 'min']:
                    telemetry['drop_name'] = d_name
                    telemetry['current_drop'] = d_name

            # 3. Streamer parsing
            st_match = self.st_re1.search(line_str) or self.st_re2.search(line_str) or self.st_re3.search(line_str)
            if st_match:
                streamer = st_match.group(1).strip(' "\'[]:-,')
                if streamer and len(streamer) >= 3 and streamer.lower() not in ['channel', 'true', 'false', 'streamer', 'twitch', 'offline', 'online', 'none', 'null', 'info', 'warning']:
                    telemetry['active_streamer'] = streamer

        return telemetry

    async def process_container_logs(self, container_id: str, tail: int) -> dict:
        logs = await self.docker_manager.get_container_logs(container_id, tail=tail)
        return self.parse_logs(logs)

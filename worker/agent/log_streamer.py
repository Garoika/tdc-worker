import re
import logging
from agent.docker_manager import DockerManager

logger = logging.getLogger(__name__)

class LogStreamer:
    def __init__(self, docker_manager: DockerManager):
        self.docker_manager = docker_manager
        # User login pattern: [TwitchUser - login]
        self.user_re = re.compile(r'\[TwitchUser\s*-\s*([a-zA-Z0-9_]+)\]', re.IGNORECASE)
        
        # Primary Progress patterns (e.g. "3/240 minutes watched", "45/120 min")
        self.prog_watched_re = re.compile(r'(\d+)\s*(?:/|of)\s*(\d+)\s*(?:minutes?|min)', re.IGNORECASE)
        self.prog_bracket_re = re.compile(r'\[(?:Drop Progress|Progress|Mining)\]\s*(\d+)\s*(?:/|of)\s*(\d+)', re.IGNORECASE)
        self.prog_re1 = re.compile(r'(\d+)\s*(?:/|of)\s*(\d+)', re.IGNORECASE)
        self.prog_pct_re = re.compile(r'(\d+(?:\.\d+)?)\s*%', re.IGNORECASE)
        
        # Specific C# bot drop patterns
        self.time_drop_re = re.compile(r'Time based drops\s*:\s*([^"\n\r]+)', re.IGNORECASE)
        self.campaign_drop_re = re.compile(r'Current drop campaign\s*:\s*([^,\(\n\r]+)', re.IGNORECASE)
        self.drop_re1 = re.compile(r'\[([^\]]+)\]\s*\d+\s*/\s*\d+', re.IGNORECASE)
        self.drop_re3 = re.compile(r'Drop\s+"([^"]+)"', re.IGNORECASE)
        self.drop_generic_re = re.compile(r'(?:Active drop|Mining drop|Claimed drop)\s*[:\-]\s*"?([^"\n\r]+)"?', re.IGNORECASE)
        
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

            # 0. Account login parsing
            u_match = self.user_re.search(line_str)
            if u_match:
                telemetry['account_login'] = u_match.group(1).strip()

            # 1. Progress parsing (prioritize lines explicitly mentioning minutes)
            p_match = self.prog_watched_re.search(line_str) or self.prog_bracket_re.search(line_str)
            if not p_match and not any(w in line_str.lower() for w in ['seconds', 'campaigns', 'games', 'http', 'socket', 'batch']):
                p_match = self.prog_re1.search(line_str)

            if p_match:
                try:
                    c_min = int(p_match.group(1))
                    t_min = int(p_match.group(2))
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

            # 2. Drop name parsing (priority: specific item name > campaign name)
            d_match = (
                self.time_drop_re.search(line_str) or 
                self.drop_re3.search(line_str) or 
                self.drop_generic_re.search(line_str) or 
                self.campaign_drop_re.search(line_str) or 
                self.drop_re1.search(line_str)
            )
            if d_match:
                d_name = d_match.group(1).strip(' "\'[]:-,')
                if (
                    d_name and 
                    len(d_name) >= 2 and 
                    not d_name.isdigit() and 
                    d_name.lower() not in ['info', 'warning', 'error', 'debug', 'minutes', 'min', 'current drop', 'starting...', 'watching 100 seconds to e...']
                ):
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

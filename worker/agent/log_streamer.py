import re
import logging
from agent.docker_manager import DockerManager

logger = logging.getLogger(__name__)

class LogStreamer:
    def __init__(self, docker_manager: DockerManager):
        self.docker_manager = docker_manager
        # User login pattern: [TwitchUser - login]
        self.user_re = re.compile(r'\[TwitchUser\s*-\s*([a-zA-Z0-9_]+)\]', re.IGNORECASE)
        
        # 1. Strict Progress patterns (MUST contain minutes/min or bracketed progress)
        self.prog_watched_re = re.compile(r'(\d+)\s*/\s*(\d+)\s*(?:minutes?|min)\s*watched', re.IGNORECASE)
        self.prog_min_re = re.compile(r'(\d+)\s*(?:/|of)\s*(\d+)\s*(?:minutes?|min)', re.IGNORECASE)
        self.prog_bracket_re = re.compile(r'\[(?:Drop Progress|Progress|Mining)\]\s*(\d+)\s*(?:/|of)\s*(\d+)', re.IGNORECASE)
        self.prog_pct_re = re.compile(r'(\d+(?:\.\d+)?)\s*%', re.IGNORECASE)
        
        # 2. Specific C# bot drop patterns
        self.time_drop_re = re.compile(r'Time based drops\s*:\s*(.+)$', re.IGNORECASE)
        self.drop_generic_re = re.compile(r'(?:Active drop|Mining drop|Claimed drop|Current drop)\s*[:\-]\s*(.+)$', re.IGNORECASE)
        self.campaign_drop_re = re.compile(r'Current drop campaign\s*:\s*([^,\(\n\r]+)', re.IGNORECASE)
        self.drop_re_quotes = re.compile(r'Drop\s+"([^"]+)"', re.IGNORECASE)
        
        # 3. Streamer patterns (STRICT: only actual stream watch / events)
        self.st_spade_re = re.compile(r'SendSpadeEvents accepted for "([^"]+)"', re.IGNORECASE)
        self.st_watching_pipe_re = re.compile(r'watching\s+([a-zA-Z0-9_]{3,25})\s*\|', re.IGNORECASE)
        self.st_priority_re = re.compile(r'Selected priority channel "([^"]+)"', re.IGNORECASE)
        self.st_watching_quotes_re = re.compile(r'watching\s+"([^"]+)"', re.IGNORECASE)

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

            # 1. Progress parsing (ONLY match lines with 'minutes' or '[Drop Progress]')
            # Explicitly ignore group pagination lines like "(1/68)" or "(4/57)"
            if not any(ign in line_str.lower() for ign in ['group of channels', 'trying next group', 'finished campaigns', 'watching 100 seconds']):
                p_match = self.prog_watched_re.search(line_str) or self.prog_min_re.search(line_str) or self.prog_bracket_re.search(line_str)
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

            # 2. Drop item name parsing
            d_match = (
                self.time_drop_re.search(line_str) or 
                self.drop_generic_re.search(line_str) or 
                self.drop_re_quotes.search(line_str)
            )
            if d_match:
                raw_d = d_match.group(1).strip()
                # Clean quotes, brackets and extra whitespace
                d_name = re.sub(r'["\'「」\[\]]', '', raw_d).strip(' :-,')
                if (
                    d_name and 
                    len(d_name) >= 2 and 
                    not d_name.isdigit() and 
                    d_name.lower() not in ['info', 'warning', 'error', 'debug', 'minutes', 'min', 'null', 'none']
                ):
                    telemetry['drop_name'] = d_name
                    telemetry['current_drop'] = d_name
            elif 'drop_name' not in telemetry:
                # Fallback to campaign name if specific item not yet encountered
                c_match = self.campaign_drop_re.search(line_str)
                if c_match:
                    c_name = c_match.group(1).strip(' "\'[]:-,')
                    if c_name and len(c_name) >= 2 and c_name.lower() not in ['info', 'warning', 'error']:
                        telemetry['drop_name'] = c_name
                        telemetry['current_drop'] = c_name

            # 3. Streamer parsing (Strict patterns only)
            st_match = (
                self.st_spade_re.search(line_str) or 
                self.st_watching_pipe_re.search(line_str) or 
                self.st_priority_re.search(line_str) or
                self.st_watching_quotes_re.search(line_str)
            )
            if st_match:
                streamer = st_match.group(1).strip(' "\'[]:-,')
                if streamer and len(streamer) >= 3 and streamer.lower() not in ['channel', 'true', 'false', 'streamer', 'twitch', 'offline', 'online', 'none', 'null', 'info', 'warning', 'found', 'broadcaster']:
                    telemetry['active_streamer'] = streamer
                    telemetry['is_actively_watching'] = True

            if 'sendspadeevents accepted' in line_str.lower() or 'watching broadcaster' in line_str.lower():
                telemetry['is_actively_watching'] = True

        return telemetry

    async def process_container_logs(self, container_id: str, tail: int) -> dict:
        logs = await self.docker_manager.get_container_logs(container_id, tail=tail)
        return self.parse_logs(logs)

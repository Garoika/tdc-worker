import re
import logging
from agent.docker_manager import DockerManager

logger = logging.getLogger(__name__)

class LogStreamer:
    def __init__(self, docker_manager: DockerManager):
        self.docker_manager = docker_manager
        # User login pattern: [TwitchUser - login]
        self.user_re = re.compile(r'\[TwitchUser\s*-\s*([a-zA-Z0-9_]+)\]', re.IGNORECASE)
        
        # 0. Campaign Progress pattern from C# bot (overall campaign progress + drop breakdown)
        self.campaign_progress_re = re.compile(
            r'\[Campaign Progress\]\s*([^|]+)\|\s*(\d+)\s*/\s*(\d+)\s*(?:minutes?|min)\s*(?:\((\d+)%\))?\s*(?:\|\s*Drop\s*(\d+)\s*/\s*(\d+)\s*"([^"]*)")?(?::\s*(\d+)\s*/\s*(\d+))?',
            re.IGNORECASE
        )
        
        # 1. Strict Progress patterns (MUST contain minutes/min or bracketed progress)
        self.prog_watched_re = re.compile(r'(\d+)\s*/\s*(\d+)\s*(?:minutes?|min)\s*watched', re.IGNORECASE)
        self.prog_min_re = re.compile(r'(\d+)\s*(?:/|of)\s*(\d+)\s*(?:minutes?|min)', re.IGNORECASE)
        self.prog_bracket_re = re.compile(r'\[(?:Drop Progress|Progress|Mining)\]\s*(\d+)\s*(?:/|of)\s*(\d+)', re.IGNORECASE)
        self.prog_pct_re = re.compile(r'(\d+(?:\.\d+)?)\s*%', re.IGNORECASE)
        
        # 2. Specific C# bot drop and campaign patterns
        self.time_drop_re = re.compile(r'Time based drops\s*:\s*(.+)$', re.IGNORECASE)
        self.drop_generic_re = re.compile(r'(?:Active drop|Mining drop|Claimed drop|Current drop)\s*[:\-]\s*(.+)$', re.IGNORECASE)
        self.campaign_drop_re = re.compile(r'Current drop campaign\s*:\s*([^,\(\n\r]+)', re.IGNORECASE)
        self.checking_campaign_re = re.compile(r'Checking\s+"[^"]+"\s*\("([^"]+)"\)', re.IGNORECASE)
        self.campaign_completed_re = re.compile(r'Campaign\s+"([^"]+)"\s*already completed', re.IGNORECASE)
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
            
        lines = logs.splitlines()
        
        # Find account login first
        for line in reversed(lines):
            u_match = self.user_re.search(line)
            if u_match:
                telemetry['account_login'] = u_match.group(1).strip()
                break

        spade_count = 0
        new_campaign_detected = None
        campaign_completed_detected = False

        for line_str in lines:
            line_str = line_str.strip()
            if not line_str:
                continue

            # Check if previous campaign completed
            if self.campaign_completed_re.search(line_str) or 'removing' in line_str.lower() and 'finished campaigns' in line_str.lower():
                campaign_completed_detected = True

            # Check campaign checking lines: Checking "NARAKA: BLADEPOINT" ("JS1 PARTNER? 7.30")...
            chk_match = self.checking_campaign_re.search(line_str)
            if chk_match:
                new_campaign_detected = chk_match.group(1).strip(' "\'[]:-,')

            # 1. Explicit Progress parsing (Campaign total prioritized, then fallback to single drop)
            if not any(ign in line_str.lower() for ign in ['group of channels', 'trying next group', 'finished campaigns', 'watching 100 seconds', 'watching 1999']):
                cp_match = self.campaign_progress_re.search(line_str)
                if cp_match:
                    try:
                        c_name = cp_match.group(1).strip()
                        c_watched = int(cp_match.group(2))
                        c_total = int(cp_match.group(3))
                        c_pct = int(cp_match.group(4)) if cp_match.group(4) else (int(round((c_watched / c_total) * 100)) if c_total > 0 else 0)

                        # Primary progress is strictly the overall campaign progress
                        telemetry['watched_minutes'] = c_watched
                        telemetry['target_minutes'] = c_total
                        telemetry['current_minutes'] = c_watched
                        telemetry['required_minutes'] = c_total
                        telemetry['percentage'] = min(100, max(0, c_pct))

                        telemetry['campaign_name'] = c_name
                        telemetry['campaign_watched'] = c_watched
                        telemetry['campaign_total'] = c_total
                        telemetry['campaign_percentage'] = min(100, max(0, c_pct))

                        if cp_match.group(5):
                            telemetry['drop_index'] = int(cp_match.group(5))
                        if cp_match.group(6):
                            telemetry['drop_total'] = int(cp_match.group(6))
                        if cp_match.group(7):
                            d_name = cp_match.group(7).strip()
                            if d_name:
                                telemetry['drop_name'] = d_name
                                telemetry['current_drop'] = d_name
                        if cp_match.group(8):
                            telemetry['drop_watched'] = int(cp_match.group(8))
                        if cp_match.group(9):
                            telemetry['drop_required'] = int(cp_match.group(9))
                    except Exception:
                        pass
                else:
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
                c_match = self.campaign_drop_re.search(line_str)
                if c_match:
                    c_name = c_match.group(1).strip(' "\'[]:-,')
                    if c_name and len(c_name) >= 2 and c_name.lower() not in ['info', 'warning', 'error']:
                        telemetry['drop_name'] = c_name
                        telemetry['current_drop'] = c_name

            # 3. Streamer parsing
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

            if 'sendspadeevents accepted' in line_str.lower():
                spade_count += 1
                telemetry['is_actively_watching'] = True

        # If campaign changed to a new one
        if new_campaign_detected and 'drop_name' not in telemetry:
            telemetry['drop_name'] = new_campaign_detected
            telemetry['current_drop'] = new_campaign_detected

        return telemetry

    async def process_container_logs(self, container_id: str, tail: int, job_id: str = None, login: str = None) -> dict:
        logs = await self.docker_manager.get_container_logs(container_id=container_id, job_id=job_id, login=login, tail=tail)
        return self.parse_logs(logs)

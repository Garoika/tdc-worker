import re
import logging
from typing import Any

logger = logging.getLogger(__name__)

class LogStreamer:
    def __init__(self, process_manager: Any):
        self.process_manager = process_manager
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
        self.campaign_completed_re = re.compile(
            r'(?:\[Status\]\s*)?Campaign\s+["\']?([^"\']+)["\']?\s*(?:is\s+100%\s+completed|already\s+completed)', 
            re.IGNORECASE
        )
        self.stream_inactive_re = re.compile(
            r'(?:No active broadcaster|No broadcaster found|No broadcaster online|No broadcaster or campaign left|Stream went offline|Waiting \d+ seconds before|No campaign found)',
            re.IGNORECASE
        )
        self.drop_re_quotes = re.compile(r'Drop\s+"([^"]+)"', re.IGNORECASE)
        
        # 2b. Claim patterns from C# bot (multi-campaign claim detection)
        # e.g.: [Claim] Claiming drop 'DropName' (ID: DropInstanceID) for 'CampaignName'...
        self.claim_intent_re = re.compile(
            r'\[Claim\]\s*Claiming drop\s*[\'"]([^\'"]+)[\'"]\s*(?:\(ID:\s*([^\'\)]+)\))?\s*for\s*[\'"]([^\'"]+)[\'"]',
            re.IGNORECASE
        )
        # e.g.: [Claim] Successfully claimed drop 'DropName'!
        self.claim_success_re = re.compile(
            r'\[Claim\]\s*Successfully claimed drop\s*[\'"]([^\'"]+)[\'"]',
            re.IGNORECASE
        )
        # e.g.: [Claim] Claim of 'DropName' for 'CampaignName' is unconfirmed
        self.claim_unconfirmed_re = re.compile(
            r'\[Claim\]\s*Claim of\s*[\'"]([^\'"]+)[\'"]\s*for\s*[\'"]([^\'"]+)[\'"]\s*is unconfirmed',
            re.IGNORECASE
        )
        self.drop_campaign_cache = {}

        # 3. Streamer patterns (STRICT: only actual stream watch / events)
        self.st_spade_re = re.compile(r'SendSpadeEvents accepted for "([^"]+)"', re.IGNORECASE)
        self.st_watching_pipe_re = re.compile(r'watching\s+([a-zA-Z0-9_]{3,25})\s*\|', re.IGNORECASE)
        self.st_priority_re = re.compile(r'Selected priority channel\s+["\']?([^"\'\.\s]+)', re.IGNORECASE)
        self.st_switched_re = re.compile(r'Broadcaster switched from \S+ to ([a-zA-Z0-9_]{3,25})', re.IGNORECASE)
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
        completed_campaigns = set()
        claims_map = {}
        last_active_watch_line = -1
        last_inactive_line = -1

        for line_idx, line_str in enumerate(lines):
            line_str = line_str.strip()
            if not line_str:
                continue

            # Check if specific campaign is logged as completed
            comp_match = self.campaign_completed_re.search(line_str)
            if comp_match:
                c_comp_name = comp_match.group(1).strip(' "\'[]:-,')
                if c_comp_name:
                    completed_campaigns.add(c_comp_name.lower())

            # Check if stream/broadcaster is inactive or waiting
            if self.stream_inactive_re.search(line_str):
                last_inactive_line = line_idx

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
                self.st_switched_re.search(line_str) or
                self.st_watching_quotes_re.search(line_str)
            )
            if st_match:
                streamer = st_match.group(1).strip(' "\'[]:-,')
                if streamer and len(streamer) >= 3 and streamer.lower() not in ['channel', 'true', 'false', 'streamer', 'twitch', 'offline', 'online', 'none', 'null', 'info', 'warning', 'found', 'broadcaster']:
                    telemetry['active_streamer'] = streamer
                    last_active_watch_line = line_idx

            if 'sendspadeevents accepted' in line_str.lower():
                spade_count += 1
                last_active_watch_line = line_idx

            # 4. Multi-campaign Claim detection
            ci_match = self.claim_intent_re.search(line_str)
            if ci_match:
                d_name = ci_match.group(1).strip()
                d_id = ci_match.group(2).strip() if ci_match.group(2) else None
                c_name = ci_match.group(3).strip()
                key = d_name.lower()
                if c_name:
                    self.drop_campaign_cache[key] = c_name
                if key not in claims_map:
                    claims_map[key] = {
                        "drop_name": d_name,
                        "drop_id": d_id,
                        "campaign_name": c_name,
                        "confirmed": False
                    }
                else:
                    if c_name:
                        claims_map[key]["campaign_name"] = c_name
                    if d_id:
                        claims_map[key]["drop_id"] = d_id

            cs_match = self.claim_success_re.search(line_str)
            if cs_match:
                d_name = cs_match.group(1).strip()
                key = d_name.lower()
                if key in claims_map:
                    claims_map[key]["confirmed"] = True
                else:
                    cached_camp = self.drop_campaign_cache.get(key)
                    claims_map[key] = {
                        "drop_name": d_name,
                        "drop_id": None,
                        "campaign_name": cached_camp,
                        "confirmed": True
                    }

            cu_match = self.claim_unconfirmed_re.search(line_str)
            if cu_match:
                d_name = cu_match.group(1).strip()
                c_name = cu_match.group(2).strip()
                key = d_name.lower()
                if c_name:
                    self.drop_campaign_cache[key] = c_name
                if key not in claims_map:
                    claims_map[key] = {
                        "drop_name": d_name,
                        "drop_id": None,
                        "campaign_name": c_name,
                        "confirmed": True
                    }
                else:
                    if c_name:
                        claims_map[key]["campaign_name"] = c_name

        # If campaign changed to a new one
        if new_campaign_detected and 'campaign_name' not in telemetry:
            telemetry['campaign_name'] = new_campaign_detected

        # Multi-campaign claimed drops list
        claimed_drops_list = []
        for item in claims_map.values():
            if item.get("drop_name"):
                c_name = (
                    item.get("campaign_name") or
                    self.drop_campaign_cache.get(item["drop_name"].lower()) or
                    telemetry.get("campaign_name") or
                    new_campaign_detected
                )
                claimed_drops_list.append({
                    "drop_name": item["drop_name"],
                    "drop_id": item.get("drop_id"),
                    "campaign_name": c_name
                })
        if claimed_drops_list:
            telemetry['claimed_drops'] = claimed_drops_list

        # 5. Resolve active stream watching vs waiting / completed
        if last_active_watch_line > last_inactive_line and last_active_watch_line != -1:
            telemetry['is_actively_watching'] = True
        else:
            telemetry['is_actively_watching'] = False
            telemetry['active_streamer'] = None

        # 6. Check completed status
        pct = telemetry.get('percentage', 0)
        watched_m = telemetry.get('watched_minutes', 0)
        req_m = telemetry.get('target_minutes', 0)
        is_100_percent = (pct >= 100) or (req_m > 0 and watched_m >= req_m)

        current_camp = (telemetry.get('campaign_name') or new_campaign_detected or '').lower()
        is_current_campaign_completed = bool(current_camp and current_camp in completed_campaigns)

        # An account with 0 progress and target_minutes > 0 has just started and cannot be completed
        has_zero_progress = (req_m > 0 and watched_m == 0 and pct == 0)

        if not has_zero_progress and (is_100_percent or is_current_campaign_completed) and not telemetry.get('is_actively_watching'):
            telemetry['is_completed'] = True
            telemetry['active_streamer'] = None
        else:
            telemetry['is_completed'] = False

        return telemetry

    async def process_container_logs(self, container_id: str, tail: int, job_id: str = None, login: str = None) -> dict:
        logs = await self.process_manager.get_container_logs(container_id=container_id, job_id=job_id, login=login, tail=tail)
        return self.parse_logs(logs)

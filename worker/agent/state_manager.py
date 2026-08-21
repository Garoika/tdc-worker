import os
import json
import time
import logging
from pathlib import Path
from typing import Dict, Any, List

logger = logging.getLogger(__name__)

STATE_FILE = Path(__file__).resolve().parent.parent / ".worker_state.json"

class StateManager:
    """
    Maintains real-time worker state in-memory and dumps it to a local JSON file
    for the companion Live Monitor Console to read with zero server requests.
    """
    _instance = None

    def __new__(cls, *args, **kwargs):
        if not cls._instance:
            cls._instance = super(StateManager, cls).__new__(cls)
            cls._instance._initialized = False
        return cls._instance

    def __init__(self):
        if self._initialized:
            return
        self._initialized = True
        self.start_time = time.time()
        self.node_name = "PC"
        self.runner_mode = "PROCESS"
        self.master_url = ""
        self.master_connected = False
        self.parent_pid = os.getpid()
        self.farmer_pid = None
        self.cpu_percent = 0.0
        self.ram_used_mb = 0.0
        self.ram_total_mb = 0.0
        self.active_jobs: Dict[str, Dict[str, Any]] = {}
        self.account_telemetry: Dict[str, Dict[str, Any]] = {}
        self.recent_events: List[Dict[str, Any]] = []
        self._last_save = 0

    def update_system_info(self, node_name: str = None, master_url: str = None, master_connected: bool = None, runner_mode: str = None, farmer_pid: int = None):
        if node_name:
            self.node_name = node_name
        if master_url:
            self.master_url = master_url
        if master_connected is not None:
            self.master_connected = master_connected
        if runner_mode:
            self.runner_mode = runner_mode
        if farmer_pid is not None:
            self.farmer_pid = farmer_pid
        self.save_state()

    def update_metrics(self, cpu: float, ram_used: float, ram_total: float):
        self.cpu_percent = cpu
        self.ram_used_mb = ram_used
        self.ram_total_mb = ram_total
        self.save_state()

    def update_jobs(self, jobs: Dict[str, Dict[str, Any]]):
        self.active_jobs = jobs
        self.save_state()

    def record_telemetry(self, login: str, game: str, telemetry: dict):
        if not login:
            return
        
        now_str = time.strftime("%H:%M:%S")
        watched = telemetry.get('watched_minutes') or telemetry.get('current_minutes') or 0
        required = telemetry.get('target_minutes') or telemetry.get('required_minutes') or 60
        pct = telemetry.get('percentage') or (int(round((watched / required) * 100)) if required > 0 else 0)
        streamer = telemetry.get('active_streamer') or ''
        drop_name = telemetry.get('drop_name') or ''

        self.account_telemetry[login] = {
            'login': login,
            'game': game,
            'watched': watched,
            'required': required,
            'percent': pct,
            'streamer': streamer,
            'drop_name': drop_name,
            'updated_at': now_str
        }

        # Add to recent events ticker if progress or streamer reported
        if watched > 0 or streamer:
            evt_text = f"[{now_str}] {login} -> {game}: {watched}/{required}m ({pct}%)"
            if streamer:
                evt_text += f" | {streamer}"
            
            # Avoid duplicate consecutive events for same account & minute
            if not self.recent_events or self.recent_events[0].get('text') != evt_text:
                self.recent_events.insert(0, {
                    'time': now_str,
                    'text': evt_text,
                    'login': login,
                    'game': game
                })
                if len(self.recent_events) > 15:
                    self.recent_events.pop()

        self.save_state()

    def save_state(self):
        # Throttle file writes to at most once every 300ms
        now = time.time()
        if now - self._last_save < 0.3:
            return
        self._last_save = now

        # Compute game groupings
        game_stats = {}
        for job_id, job in self.active_jobs.items():
            login = job.get('login', '')
            game = job.get('target', {}).get('game', 'Unknown')
            if game not in game_stats:
                game_stats[game] = {
                    'count': 0,
                    'streamers': set(),
                    'total_percent': 0,
                    'accounts': []
                }
            game_stats[game]['count'] += 1
            
            t = self.account_telemetry.get(login, {})
            pct = t.get('percent', 0)
            st = t.get('streamer', '')
            if st:
                game_stats[game]['streamers'].add(st)
            game_stats[game]['total_percent'] += pct
            game_stats[game]['accounts'].append({
                'login': login,
                'watched': t.get('watched', 0),
                'required': t.get('required', 0),
                'percent': pct,
                'streamer': st
            })

        # Format game stats for json
        formatted_games = {}
        for g_name, g_data in game_stats.items():
            cnt = g_data['count']
            avg_p = round(g_data['total_percent'] / cnt) if cnt > 0 else 0
            formatted_games[g_name] = {
                'count': cnt,
                'streamers': list(g_data['streamers'])[:5],
                'avg_percent': avg_p
            }

        state_data = {
            'updated_at': now,
            'start_time': self.start_time,
            'uptime_seconds': int(now - self.start_time),
            'node_name': self.node_name,
            'runner_mode': self.runner_mode,
            'master_url': self.master_url,
            'master_connected': self.master_connected,
            'parent_pid': self.parent_pid,
            'farmer_pid': self.farmer_pid,
            'cpu_percent': self.cpu_percent,
            'ram_used_mb': self.ram_used_mb,
            'ram_total_mb': self.ram_total_mb,
            'total_accounts': len(self.active_jobs),
            'games': formatted_games,
            'recent_events': [e['text'] for e in self.recent_events[:8]]
        }

        try:
            temp_file = STATE_FILE.with_suffix('.tmp')
            with open(temp_file, 'w', encoding='utf-8') as f:
                json.dump(state_data, f, indent=2)
            temp_file.replace(STATE_FILE)
        except Exception:
            pass

state_manager = StateManager()

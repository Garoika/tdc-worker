import os
import sys
import json
from pathlib import Path

possible_config_paths = [
    Path('.worker_config.json'),
    Path('../.worker_config.json'),
    Path(__file__).parent.parent.parent / '.worker_config.json'
]

config_file = None
for p in possible_config_paths:
    if p.exists():
        config_file = p
        break

if not config_file:
    config_file = Path('../.worker_config.json')

file_config = {}
if config_file.exists():
    try:
        with open(config_file, 'r', encoding='utf-8') as f:
            file_config = json.load(f)
    except Exception as e:
        print(f"[Warning] Failed to read {config_file}: {e}", flush=True)

def prompt_user(text: str) -> str:
    sys.stdout.write(text)
    sys.stdout.flush()
    return sys.stdin.readline().strip()

if not file_config.get('worker_token') and not os.environ.get('WORKER_TOKEN'):
    print("\n==========================================", flush=True)
    print("      Worker First-Time Configuration     ", flush=True)
    print("==========================================", flush=True)
    
    url = prompt_user("Enter Master Server WS URL [e.g. ws://185.104.248.62/ws/workers]: ")
    if not url:
        url = "ws://185.104.248.62/ws/workers"
    
        
    print("\nPlease register a worker in Dashboard -> 'Workers' tab -> 'Register Worker'.", flush=True)
    token = prompt_user("Enter Worker Token (wt_...): ")
    while not token:
        print("Error: Worker token cannot be empty!", flush=True)
        token = prompt_user("Enter Worker Token (wt_...): ")
        
    ip = prompt_user("Enter Worker Public/LAN IP (optional, press Enter for auto): ")
    
    file_config = {
        "master_url": url,
        "worker_token": token,
        "worker_public_ip": ip
    }
    
    try:
        with open(config_file, 'w', encoding='utf-8') as f:
            json.dump(file_config, f, indent=2)
        print(f"[OK] Configuration saved to {config_file.resolve()}\n", flush=True)
    except Exception as e:
        print(f"[Warning] Could not save config file: {e}", flush=True)

MASTER_URL = os.environ.get('MASTER_URL') or file_config.get('master_url') or 'ws://192.168.1.100:5173/ws/workers'
WORKER_TOKEN = os.environ.get('WORKER_TOKEN') or file_config.get('worker_token') or ''
WORKER_PUBLIC_IP = os.environ.get('WORKER_PUBLIC_IP') or file_config.get('worker_public_ip') or ''
RUNNER_TYPE = 'process'
MAX_PROCESSES = int(os.environ.get('MAX_PROCESSES') or os.environ.get('MAX_CONTAINERS', '100'))
MAX_CONTAINERS = MAX_PROCESSES
HEARTBEAT_INTERVAL = int(os.environ.get('HEARTBEAT_INTERVAL', '3'))
WORKER_NAME = os.environ.get('WORKER_NAME') or file_config.get('worker_name') or 'Worker'
LOG_TAIL_LINES = int(os.environ.get('LOG_TAIL_LINES', '50'))

# Farmer binary paths for Process Mode
FARMER_BIN_DIR = Path(__file__).parent.parent.parent / 'farmer_bin'
FARMER_EXE = FARMER_BIN_DIR / 'TwitchDropsBot.Console.exe'
if not FARMER_EXE.exists():
    # Fallback to dll
    FARMER_EXE = FARMER_BIN_DIR / 'TwitchDropsBot.Console.dll'


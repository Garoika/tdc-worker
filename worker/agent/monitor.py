import os
import sys
import time
import json
import argparse
import signal
from pathlib import Path

try:
    import psutil
except ImportError:
    psutil = None

STATE_FILE = Path(__file__).resolve().parent.parent / ".worker_state.json"

# ANSI color codes
RESET = "\033[0m"
BOLD = "\033[1m"
DIM = "\033[2m"
CYAN = "\033[36m"
GREEN = "\033[32m"
YELLOW = "\033[33m"
BLUE = "\033[34m"
MAGENTA = "\033[35m"
RED = "\033[31m"
WHITE = "\033[37m"

def enable_windows_ansi():
    """Enable VT100 / ANSI escape sequences in Windows Console."""
    if os.name == 'nt':
        import ctypes
        kernel32 = ctypes.windll.kernel32
        h_stdout = kernel32.GetStdHandle(-11)
        mode = ctypes.c_ulong()
        kernel32.GetConsoleMode(h_stdout, ctypes.byref(mode))
        mode.value |= 0x0004 | 0x0008  # ENABLE_VIRTUAL_TERMINAL_PROCESSING
        kernel32.SetConsoleMode(h_stdout, mode)
        # Set console window title
        ctypes.windll.kernel32.SetConsoleTitleW("TDC Cluster — Live Worker Monitor")

def render_cpu_bar(percent: float, width: int = 10) -> str:
    filled = int(round((percent / 100) * width))
    filled = max(0, min(width, filled))
    bar = "█" * filled + "░" * (width - filled)
    if percent > 80:
        return f"{RED}[{bar}] {percent:.1f}%{RESET}"
    elif percent > 50:
        return f"{YELLOW}[{bar}] {percent:.1f}%{RESET}"
    else:
        return f"{GREEN}[{bar}] {percent:.1f}%{RESET}"

def format_uptime(seconds: int) -> str:
    days = seconds // 86400
    rem = seconds % 86400
    hours = rem // 3600
    mins = (rem % 3600) // 60
    secs = rem % 60
    
    parts = []
    if days > 0:
        parts.append(f"{days}d")
    if hours > 0 or days > 0:
        parts.append(f"{hours}h")
    parts.append(f"{mins}m")
    parts.append(f"{secs}s")
    return " ".join(parts)

def clear_screen():
    # Move cursor to top-left & clear
    sys.stdout.write("\033[H\033[J")
    sys.stdout.flush()

def main():
    parser = argparse.ArgumentParser(description="TDC Worker Live Monitor")
    parser.add_argument("--parent-pid", type=int, default=None, help="Worker parent PID to watch")
    args = parser.parse_args()

    enable_windows_ansi()

    def handle_sig(sig, frame):
        sys.exit(0)

    signal.signal(signal.SIGINT, handle_sig)
    signal.signal(signal.SIGTERM, handle_sig)

    parent_pid = args.parent_pid

    # Hide cursor
    sys.stdout.write("\033[?25l")
    sys.stdout.flush()

    try:
        while True:
            # Watchdog: Exit immediately if parent worker process died
            if parent_pid and psutil:
                try:
                    if not psutil.pid_exists(parent_pid):
                        break
                    p = psutil.Process(parent_pid)
                    if p.status() == psutil.STATUS_ZOMBIE or p.status() == psutil.STATUS_DEAD:
                        break
                except (psutil.NoSuchProcess, psutil.AccessDenied):
                    break

            # Read state
            state = {}
            if STATE_FILE.exists():
                try:
                    with open(STATE_FILE, 'r', encoding='utf-8') as f:
                        state = json.load(f)
                except Exception:
                    pass

            # Fallback parent_pid from state if not passed in CLI
            if not parent_pid and state.get('parent_pid'):
                parent_pid = state.get('parent_pid')

            # Render frame
            node_name = state.get('node_name', 'PC')
            runner_mode = state.get('runner_mode', 'PROCESS')
            master_connected = state.get('master_connected', False)
            uptime_str = format_uptime(state.get('uptime_seconds', 0))
            cpu_pct = state.get('cpu_percent', 0.0)
            ram_used = state.get('ram_used_mb', 0.0)
            ram_tot = state.get('ram_total_mb', 0.0)
            total_acc = state.get('total_accounts', 0)
            farmer_pid = state.get('farmer_pid') or 'Running'
            games = state.get('games', {})
            recent_events = state.get('recent_events', [])

            status_badge = f"{GREEN}ONLINE 🟢{RESET}" if master_connected else f"{RED}CONNECTING... 🔴{RESET}"
            
            clear_screen()
            
            lines = [
                f"{CYAN}╔════════════════════════════════════════════════════════════════════════════════╗{RESET}",
                f"{CYAN}║{BOLD}{WHITE}                   ⚡ TDC WORKER — LIVE MONITOR DASHBOARD ⚡                    {RESET}{CYAN}║{RESET}",
                f"{CYAN}╚════════════════════════════════════════════════════════════════════════════════╝{RESET}",
                f" {BOLD}Node:{RESET} {WHITE}{node_name}{RESET} ({status_badge})   {BOLD}Mode:{RESET} {YELLOW}{runner_mode}{RESET}   {BOLD}PID:{RESET} {DIM}{farmer_pid}{RESET}   {BOLD}Uptime:{RESET} {WHITE}{uptime_str}{RESET}",
                f" {BOLD}CPU:{RESET}  {render_cpu_bar(cpu_pct)}    {BOLD}RAM:{RESET} {WHITE}{ram_used:.0f} MB{RESET} / {DIM}{ram_tot/1024:.1f} GB{RESET}   {BOLD}Time:{RESET} {DIM}{time.strftime('%H:%M:%S')}{RESET}",
                f"{CYAN}────────────────────────────────────────────────────────────────────────────────{RESET}",
                f" {BOLD}👥 ACTIVE FARMING ACCOUNTS:{RESET} {GREEN}{BOLD}{total_acc}{RESET} {DIM}accounts assigned to this node{RESET}",
                f"{CYAN}────────────────────────────────────────────────────────────────────────────────{RESET}",
                f" {BOLD}🎮 GAMES BREAKDOWN:{RESET}"
            ]

            if games:
                lines.append(f" ┌──────────────────────────┬────────────┬────────────────────────┬──────────────────┐")
                lines.append(f" │ {BOLD}Game Name{RESET}                │ {BOLD}Accounts{RESET}   │ {BOLD}Live Streamers{RESET}         │ {BOLD}Time Left{RESET}        │")
                lines.append(f" ├──────────────────────────┼────────────┼────────────────────────┼──────────────────┤")
                for g_name, g_info in games.items():
                    cnt = g_info.get('count', 0)
                    st_list = ", ".join(g_info.get('streamers', [])) or "Seeking streamer..."
                    if len(st_list) > 22:
                        st_list = st_list[:19] + "..."
                    
                    min_w = g_info.get('min_watched', 0)
                    req_w = g_info.get('required_minutes', 60)
                    time_left_str = g_info.get('time_left', 'Done')
                    
                    # Format Time Left cell: e.g. "1h 45m (15/120m)"
                    if time_left_str == "Done":
                        left_cell = f"{GREEN}Done ({req_w}/{req_w}m){RESET}"
                    else:
                        left_cell = f"{CYAN}{time_left_str}{RESET} {DIM}({min_w}/{req_w}m){RESET}"

                    # Pad game name nicely
                    display_g = g_name if len(g_name) <= 24 else g_name[:21] + "..."
                    raw_left_len = len(f"{time_left_str} ({min_w}/{req_w}m)")
                    padding = " " * max(0, 16 - raw_left_len)

                    lines.append(f" │ {WHITE}{display_g:<24}{RESET} │ {YELLOW}{cnt:>4} acc{RESET}   │ {DIM}{st_list:<22}{RESET} │ {left_cell}{padding} │")
                lines.append(f" └──────────────────────────┴────────────┴────────────────────────┴──────────────────┘")
            else:
                lines.append(f"   {DIM}No accounts currently assigned. Waiting for master server tasks...{RESET}")

            lines.append("")
            lines.append(f" {BOLD}📜 RECENT ACTIVITY LOG (Latest {min(len(recent_events), 20)} events):{RESET}")
            if recent_events:
                for evt in recent_events[:20]:
                    lines.append(f"  {CYAN}•{RESET} {DIM}{evt}{RESET}")
            else:
                lines.append(f"  {DIM}Waiting for telemetry stream from farmer process...{RESET}")

            lines.append(f"{CYAN}────────────────────────────────────────────────────────────────────────────────{RESET}")
            lines.append(f" {DIM}Auto-refreshes live (1s) • Closes automatically when main worker exits{RESET}")

            sys.stdout.write("\n".join(lines) + "\n")
            sys.stdout.flush()

            time.sleep(1.0)
    finally:
        # Show cursor
        sys.stdout.write("\033[?25h")
        sys.stdout.flush()

if __name__ == "__main__":
    main()

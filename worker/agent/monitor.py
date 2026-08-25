import os
import re
import sys
import time
import json
import shutil
import signal
import argparse
from pathlib import Path

# Force UTF-8 encoding on Windows to prevent charmap UnicodeEncodeError with emojis
if sys.platform == 'win32':
    try:
        sys.stdout.reconfigure(encoding='utf-8', errors='replace')
        sys.stderr.reconfigure(encoding='utf-8', errors='replace')
    except Exception:
        pass

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

import unicodedata

ANSI_REGEX = re.compile(r'\x1b\[[0-9;]*[a-zA-Z]')

def char_width(c: str) -> int:
    """Returns terminal column width of a single character (2 for wide emojis)."""
    # Quick check for zero-width / control characters
    if unicodedata.category(c) in ('Mn', 'Me', 'Cc', 'Cf'):
        return 0
    # East Asian Wide (W) and Fullwidth (F) emojis/chars take 2 cells
    if unicodedata.east_asian_width(c) in ('W', 'F'):
        return 2
    return 1

def visible_len(s: str) -> int:
    """Calculates visible character length of a string ignoring ANSI color codes and handling wide emojis."""
    clean = ANSI_REGEX.sub('', s)
    return sum(char_width(c) for c in clean)

def pad_visible(s: str, width: int, align: str = '<') -> str:
    """Pads string to width based on its visible length."""
    vlen = visible_len(s)
    if vlen >= width:
        return s
    pad = " " * (width - vlen)
    if align == '>':
        return pad + s
    elif align == '^':
        half = (width - vlen) // 2
        return (" " * half) + s + (" " * (width - vlen - half))
    return s + pad

def truncate_visible(s: str, max_width: int) -> str:
    """Truncates string to max_width taking into account wide emojis and colors."""
    if visible_len(s) <= max_width:
        return s
    target_w = max(1, max_width - 3) if max_width >= 4 else max_width
    res = []
    curr_w = 0
    for c in s:
        w = char_width(c)
        if curr_w + w > target_w:
            break
        res.append(c)
        curr_w += w
    return "".join(res) + ("..." if max_width >= 4 else "")

def enable_windows_ansi():
    """Enable VT100 / ANSI escape sequences in Windows Console."""
    if os.name == 'nt':
        try:
            import ctypes
            kernel32 = ctypes.windll.kernel32
            h_stdout = kernel32.GetStdHandle(-11)
            mode = ctypes.c_ulong()
            kernel32.GetConsoleMode(h_stdout, ctypes.byref(mode))
            mode.value |= 0x0004 | 0x0008  # ENABLE_VIRTUAL_TERMINAL_PROCESSING
            kernel32.SetConsoleMode(h_stdout, mode)
            ctypes.windll.kernel32.SetConsoleTitleW("TDC Cluster — Live Worker Monitor")
        except Exception:
            pass

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

            # Read terminal dimensions dynamically on each frame
            term_size = shutil.get_terminal_size(fallback=(80, 24))
            cols = max(55, min(240, term_size.columns))
            rows = max(18, term_size.lines)

            # Read state
            state = {}
            if STATE_FILE.exists():
                try:
                    with open(STATE_FILE, 'r', encoding='utf-8') as f:
                        state = json.load(f)
                except Exception:
                    pass

            if not parent_pid and state.get('parent_pid'):
                parent_pid = state.get('parent_pid')

            node_name = state.get('node_name', 'Worker')
            master_connected = state.get('master_connected', False)
            uptime_str = format_uptime(state.get('uptime_seconds', 0))
            
            if psutil:
                try:
                    cpu_pct = psutil.cpu_percent(interval=None)
                    vmem = psutil.virtual_memory()
                    ram_used = vmem.used / (1024 * 1024)
                    ram_tot = vmem.total / (1024 * 1024)
                except Exception:
                    cpu_pct = state.get('cpu_percent', 0.0)
                    ram_used = state.get('ram_used_mb', 0.0)
                    ram_tot = state.get('ram_total_mb', 0.0)
            else:
                cpu_pct = state.get('cpu_percent', 0.0)
                ram_used = state.get('ram_used_mb', 0.0)
                ram_tot = state.get('ram_total_mb', 0.0)

            total_acc = state.get('total_accounts', 0)
            farmer_pid = state.get('farmer_pid') or 'Running'
            games = state.get('games', {})
            recent_events = state.get('recent_events', [])

            status_badge = f"{GREEN}[ONLINE ●]{RESET}" if master_connected else f"{RED}[CONNECTING ●]{RESET}"
            ram_str = f"{WHITE}{ram_used/1024:.1f} GB{RESET} / {DIM}{ram_tot/1024:.1f} GB{RESET}" if ram_tot > 0 else f"{WHITE}{ram_used:.0f} MB{RESET}"
            
            total_farming = sum(g.get('active_count', 0) for g in games.values())
            total_waiting = sum(g.get('waiting_count', 0) for g in games.values())
            
            # Dynamic Header Construction with safety margin
            box_w = cols
            inner_w = max(20, box_w - 4)
            title = "TDC WORKER — LIVE MONITOR DASHBOARD"
            if visible_len(title) > inner_w:
                title = "TDC MONITOR"
            
            t_pad_left = max(0, (inner_w - visible_len(title)) // 2)
            t_pad_right = max(0, inner_w - visible_len(title) - t_pad_left)
            
            lines = [
                f"{CYAN}╔{'═' * (inner_w + 2)}╗{RESET}",
                f"{CYAN}║ {' ' * t_pad_left}{BOLD}{WHITE}{title}{RESET}{CYAN}{' ' * t_pad_right} ║{RESET}",
                f"{CYAN}╚{'═' * (inner_w + 2)}╝{RESET}",
                f" {BOLD}Node:{RESET} {WHITE}{node_name}{RESET} ({status_badge})   {BOLD}PID:{RESET} {DIM}{farmer_pid}{RESET}   {BOLD}Uptime:{RESET} {WHITE}{uptime_str}{RESET}   {BOLD}Time:{RESET} {DIM}{time.strftime('%H:%M:%S')}{RESET}",
                f" {BOLD}CPU:{RESET}  {render_cpu_bar(cpu_pct, width=max(6, min(16, cols // 10)))}   {BOLD}RAM:{RESET} {ram_str}",
                f"{CYAN}{'─' * (inner_w + 4)}{RESET}",
                f" {CYAN}●{RESET} {BOLD}ACTIVE ACCOUNTS:{RESET} {GREEN}{BOLD}{total_farming}{RESET} {GREEN}Farming{RESET}  |  {YELLOW}{BOLD}{total_waiting}{RESET} {YELLOW}Seeking Streamer{RESET}  {DIM}(Total: {total_acc}){RESET}",
                f"{CYAN}{'─' * (inner_w + 4)}{RESET}",
                f" {CYAN}●{RESET} {BOLD}GAMES BREAKDOWN:{RESET}"
            ]

            # Dynamic Responsive Table Layout
            if games:
                table_target_w = inner_w + 4
                avail_table_w = max(36, table_target_w - 13)
                
                w_acc = 9
                rem_w = avail_table_w - w_acc
                w_game = max(12, int(rem_w * 0.38))
                w_streamers = max(12, int(rem_w * 0.34))
                w_time = max(10, rem_w - w_game - w_streamers)

                lines.append(f"{CYAN}┌─{'─'*w_game}─┬─{'─'*w_acc}─┬─{'─'*w_streamers}─┬─{'─'*w_time}─┐{RESET}")
                lines.append(f"{CYAN}│{RESET} {BOLD}{'Game Name':<{w_game}}{RESET} {CYAN}│{RESET} {BOLD}{'Accounts':<{w_acc}}{RESET} {CYAN}│{RESET} {BOLD}{'Live Streamers':<{w_streamers}}{RESET} {CYAN}│{RESET} {BOLD}{'Progress / Time':<{w_time}}{RESET} {CYAN}│{RESET}")
                lines.append(f"{CYAN}├─{'─'*w_game}─┼─{'─'*w_acc}─┼─{'─'*w_streamers}─┼─{'─'*w_time}─┤{RESET}")

                for g_name, g_info in games.items():
                    cnt = g_info.get('count', 0)
                    is_active = g_info.get('is_active', False)
                    st_list = ", ".join(g_info.get('streamers', []))
                    if not st_list:
                        st_list = "Seeking streamer..."
                    st_list = truncate_visible(st_list, w_streamers)

                    min_w = g_info.get('min_watched', 0)
                    req_w = g_info.get('required_minutes', 60)
                    time_left_str = g_info.get('time_left', 'Searching...')
                    
                    if time_left_str == "Done":
                        left_cell = f"{GREEN}Done ({req_w}/{req_w}m){RESET}"
                    elif not is_active or time_left_str in ["Starts Soon", "Searching..."]:
                        left_cell = f"{YELLOW}Searching...{RESET}"
                    else:
                        left_cell = f"{CYAN}{time_left_str}{RESET} {DIM}({min_w}/{req_w}m){RESET}"

                    display_g = truncate_visible(g_name, w_game)
                    acc_cell = f"{YELLOW}{cnt:>2} acc{RESET}"

                    g_pad = pad_visible(f"{WHITE}{display_g}{RESET}", w_game)
                    a_pad = pad_visible(acc_cell, w_acc)
                    st_color = WHITE if is_active else YELLOW
                    s_pad = pad_visible(f"{st_color}{st_list}{RESET}", w_streamers)
                    t_pad = pad_visible(left_cell, w_time)

                    lines.append(f"{CYAN}│{RESET} {g_pad} {CYAN}│{RESET} {a_pad} {CYAN}│{RESET} {s_pad} {CYAN}│{RESET} {t_pad} {CYAN}│{RESET}")

                lines.append(f"{CYAN}└─{'─'*w_game}─┴─{'─'*w_acc}─┴─{'─'*w_streamers}─┴─{'─'*w_time}─┘{RESET}")
            else:
                lines.append(f"   {DIM}No accounts currently assigned. Waiting for master server tasks...{RESET}")

            lines.append("")
            lines.append(f" {CYAN}●{RESET} {BOLD}RECENT ACTIVITY:{RESET}")
            
            # Calculate remaining vertical rows for events dynamically
            fixed_lines_count = len(lines) + 3  # footer lines
            max_events = max(2, min(25, rows - fixed_lines_count))

            if recent_events:
                for evt in recent_events[:max_events]:
                    truncated_evt = truncate_visible(evt, (inner_w + 4) - 4)
                    lines.append(f"  {CYAN}▸{RESET} {DIM}{truncated_evt}{RESET}")
            else:
                lines.append(f"  {DIM}Waiting for telemetry stream from farmer process...{RESET}")

            lines.append(f"{CYAN}{'─' * (inner_w + 4)}{RESET}")
            footer = truncate_visible(f"Live (1s) • Auto-scaled ({term_size.columns}x{rows}) • Closes with worker", inner_w + 4)
            lines.append(f" {DIM}{footer}{RESET}")

            clear_screen()
            sys.stdout.write("\n".join(lines) + "\n")
            sys.stdout.flush()

            time.sleep(1.0)
    finally:
        # Show cursor
        sys.stdout.write("\033[?25h")
        sys.stdout.flush()

if __name__ == "__main__":
    main()

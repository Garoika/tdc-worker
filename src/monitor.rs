use std::fs;
use std::io::{self, Write};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use crate::state_writer::WorkerStateFile;

// ANSI Colors matching legacy Python monitor
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const WHITE: &str = "\x1b[37m";

#[cfg(windows)]
pub fn enable_windows_ansi() {
    extern "system" {
        fn GetStdHandle(nStdHandle: i32) -> *mut std::ffi::c_void;
        fn GetConsoleMode(hConsoleHandle: *mut std::ffi::c_void, lpMode: *mut u32) -> i32;
        fn SetConsoleMode(hConsoleHandle: *mut std::ffi::c_void, dwMode: u32) -> i32;
        fn SetConsoleTitleW(lpConsoleTitle: *const u16) -> i32;
    }
    unsafe {
        let handle = GetStdHandle(-11);
        if !handle.is_null() && handle != (-1isize as *mut std::ffi::c_void) {
            let mut mode: u32 = 0;
            if GetConsoleMode(handle, &mut mode) != 0 {
                // 0x0001 = ENABLE_PROCESSED_OUTPUT
                // 0x0002 = ENABLE_WRAP_AT_EOL_OUTPUT
                // 0x0004 = ENABLE_VIRTUAL_TERMINAL_PROCESSING
                // 0x0008 = DISABLE_NEWLINE_AUTO_RETURN
                SetConsoleMode(handle, mode | 0x0001 | 0x0002 | 0x0004 | 0x0008);
            }
        }
        let title: Vec<u16> = "TDC Cluster \u{2014} Live Worker Monitor\0".encode_utf16().collect();
        SetConsoleTitleW(title.as_ptr());
    }
}

#[cfg(not(windows))]
pub fn enable_windows_ansi() {}

#[cfg(windows)]
fn is_process_alive(pid: u32) -> bool {
    extern "system" {
        fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> *mut std::ffi::c_void;
        fn GetExitCodeProcess(hProcess: *mut std::ffi::c_void, lpExitCode: *mut u32) -> i32;
        fn CloseHandle(hObject: *mut std::ffi::c_void) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut exit_code: u32 = 0;
        let res = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        res != 0 && exit_code == STILL_ACTIVE
    }
}

#[cfg(not(windows))]
fn is_process_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{}", pid)).exists()
}

#[cfg(windows)]
fn get_terminal_size() -> (usize, usize) {
    extern "system" {
        fn GetStdHandle(nStdHandle: i32) -> *mut std::ffi::c_void;
        fn GetConsoleScreenBufferInfo(h: *mut std::ffi::c_void, info: *mut CONSOLE_SCREEN_BUFFER_INFO) -> i32;
    }
    #[repr(C)]
    #[allow(dead_code)]
    #[derive(Clone, Copy)]
    struct COORD { x: i16, y: i16 }
    #[repr(C)]
    #[allow(dead_code)]
    #[derive(Clone, Copy)]
    struct SMALL_RECT { left: i16, top: i16, right: i16, bottom: i16 }
    #[repr(C)]
    #[allow(non_snake_case, dead_code)]
    struct CONSOLE_SCREEN_BUFFER_INFO {
        dwSize: COORD,
        dwCursorPosition: COORD,
        wAttributes: u16,
        srWindow: SMALL_RECT,
        dwMaximumWindowSize: COORD,
    }

    unsafe {
        let handle = GetStdHandle(-11);
        let mut info = std::mem::zeroed::<CONSOLE_SCREEN_BUFFER_INFO>();
        if GetConsoleScreenBufferInfo(handle, &mut info) != 0 {
            let win_w = (info.srWindow.right - info.srWindow.left + 1) as usize;
            let win_h = (info.srWindow.bottom - info.srWindow.top + 1) as usize;
            return (win_w.clamp(55, 240), win_h.max(18));
        }
    }
    (100, 30)
}

#[cfg(not(windows))]
fn get_terminal_size() -> (usize, usize) {
    (100, 30)
}

fn char_width(c: char) -> usize {
    let u = c as u32;
    // Fast path: ASCII printable is 1 cell
    if (0x20..=0x7E).contains(&u) {
        return 1;
    }
    // Zero-width combining marks
    if (0x0300..=0x036F).contains(&u) || (0x1AB0..=0x1AFF).contains(&u) || (0x1DC0..=0x1DFF).contains(&u) || (0x20D0..=0x20FF).contains(&u) || (0xFE20..=0xFE2F).contains(&u) {
        return 0;
    }
    // Wide emojis and East Asian Wide characters take 2 cells
    if (0x1F300..=0x1FAFF).contains(&u) || (0x2E80..=0x9FFF).contains(&u) || (0xAC00..=0xD7AF).contains(&u) || (0xF900..=0xFAFF).contains(&u) {
        2
    } else {
        1
    }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn visible_len(s: &str) -> usize {
    strip_ansi(s).chars().map(char_width).sum()
}

fn pad_visible(s: &str, width: usize, align: char) -> String {
    let vlen = visible_len(s);
    if vlen >= width {
        return s.to_string();
    }
    let pad = " ".repeat(width - vlen);
    if align == '>' {
        format!("{}{}", pad, s)
    } else if align == '^' {
        let half = (width - vlen) / 2;
        let right = width - vlen - half;
        format!("{}{}{}", " ".repeat(half), s, " ".repeat(right))
    } else {
        format!("{}{}", s, pad)
    }
}

fn truncate_visible(s: &str, max_width: usize) -> String {
    let vlen = visible_len(s);
    if vlen <= max_width {
        return s.to_string();
    }
    let target_w = if max_width >= 4 { max_width.saturating_sub(3) } else { max_width };

    let mut clean_w = 0;
    let mut out = String::new();
    let mut in_escape = false;

    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
            out.push(c);
        } else if in_escape {
            out.push(c);
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else {
            let cw = char_width(c);
            if clean_w + cw > target_w {
                break;
            }
            out.push(c);
            clean_w += cw;
        }
    }
    if in_escape {
        out.push('m');
    }
    out.push_str(RESET);
    if max_width >= 4 {
        out.push_str("...");
    }
    out
}

fn render_cpu_bar(percent: f64, width: usize) -> String {
    let filled = ((percent / 100.0) * (width as f64)).round() as usize;
    let filled = filled.min(width);
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(width.saturating_sub(filled)));
    if percent > 80.0 {
        format!("{RED}[{bar}] {:.1}%{RESET}", percent)
    } else if percent > 50.0 {
        format!("{YELLOW}[{bar}] {:.1}%{RESET}", percent)
    } else {
        format!("{GREEN}[{bar}] {:.1}%{RESET}", percent)
    }
}

fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86400;
    let rem = seconds % 86400;
    let hours = rem / 3600;
    let mins = (rem % 3600) / 60;
    let secs = rem % 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{}d", days));
    }
    if hours > 0 || days > 0 {
        parts.push(format!("{}h", hours));
    }
    parts.push(format!("{}m", mins));
    parts.push(format!("{}s", secs));
    parts.join(" ")
}

pub async fn run_monitor_loop(parent_pid: Option<u32>) -> anyhow::Result<()> {
    enable_windows_ansi();

    // Hide cursor
    print!("\x1b[?25l");
    let _ = io::stdout().flush();

    loop {
        // Watchdog: check parent process
        if let Some(pid) = parent_pid {
            if !is_process_alive(pid) {
                break;
            }
        }

        let (cols_raw, rows_raw) = get_terminal_size();
        let cols = cols_raw.clamp(55, 240);
        let rows = rows_raw.max(18);

        // Read .worker_state.json
        let state_opt = if let Ok(content) = fs::read_to_string(".worker_state.json") {
            serde_json::from_str::<WorkerStateFile>(&content).ok()
        } else {
            None
        };

        let mut lines = Vec::new();

        // Exact box geometry matching legacy Python monitor
        let box_w = cols;
        let inner_w = box_w.saturating_sub(4).max(20);

        let title = "TDC WORKER \u{2014} LIVE MONITOR DASHBOARD";
        let title_disp = if visible_len(title) > inner_w { "TDC MONITOR" } else { title };
        let t_vlen = visible_len(title_disp);
        let t_pad_left = (inner_w.saturating_sub(t_vlen)) / 2;
        let t_pad_right = inner_w.saturating_sub(t_vlen + t_pad_left);

        lines.push(format!("{CYAN}╔{}╗{RESET}", "═".repeat(inner_w + 2)));
        lines.push(format!(
            "{CYAN}║ {}{BOLD}{WHITE}{}{RESET}{CYAN}{} ║{RESET}",
            " ".repeat(t_pad_left),
            title_disp,
            " ".repeat(t_pad_right)
        ));
        lines.push(format!("{CYAN}╚{}╝{RESET}", "═".repeat(inner_w + 2)));

        if let Some(st) = state_opt {
            let status_badge = if st.master_connected {
                format!("{GREEN}[ONLINE ●]{RESET}")
            } else {
                format!("{RED}[CONNECTING ●]{RESET}")
            };

            let uptime_str = format_uptime(st.uptime_seconds);
            let now_hms = {
                let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                format!("{:02}:{:02}:{:02}", (secs / 3600) % 24, (secs / 60) % 60, secs % 60)
            };

            let ram_str = if st.ram_total_mb > 0.0 {
                format!("{WHITE}{:.1} GB{RESET} / {DIM}{:.1} GB{RESET}", st.ram_used_mb / 1024.0, st.ram_total_mb / 1024.0)
            } else {
                format!("{WHITE}{:.0} MB{RESET}", st.ram_used_mb)
            };

            let cpu_bar = render_cpu_bar(st.cpu_percent.max(1.5), (cols / 10).clamp(6, 16));

            lines.push(format!(
                " {BOLD}Node:{RESET} {WHITE}{}{RESET} ({status_badge})   {BOLD}PID:{RESET} {DIM}Running{RESET}   {BOLD}Uptime:{RESET} {WHITE}{}{RESET}   {BOLD}Time:{RESET} {DIM}{}{RESET}",
                st.node_name, uptime_str, now_hms
            ));
            lines.push(format!(" {BOLD}CPU:{RESET}  {cpu_bar}   {BOLD}RAM:{RESET} {ram_str}"));
            lines.push(format!("{CYAN}{}{RESET}", "─".repeat(inner_w + 4)));

            let mut total_farming = 0;
            let mut total_waiting = 0;
            for g in st.games.values() {
                total_farming += g.active_count;
                total_waiting += g.waiting_count;
            }

            lines.push(format!(
                " {CYAN}●{RESET} {BOLD}ACTIVE ACCOUNTS:{RESET} {GREEN}{BOLD}{total_farming}{RESET} {GREEN}Farming{RESET}  |  {YELLOW}{BOLD}{total_waiting}{RESET} {YELLOW}Seeking Streamer{RESET}  {DIM}(Total: {}){RESET}",
                st.total_accounts
            ));
            lines.push(format!("{CYAN}{}{RESET}", "─".repeat(inner_w + 4)));
            lines.push(format!(" {CYAN}●{RESET} {BOLD}GAMES BREAKDOWN:{RESET}"));

            if !st.games.is_empty() {
                let table_target_w = inner_w + 4;
                let avail_table_w = table_target_w.saturating_sub(13).max(36);

                let w_acc = 9;
                let rem_w = avail_table_w.saturating_sub(w_acc);
                let w_game = ((rem_w * 38) / 100).max(12);
                let w_streamers = ((rem_w * 34) / 100).max(12);
                let w_time = rem_w.saturating_sub(w_game + w_streamers).max(10);

                lines.push(format!(
                    "{CYAN}┌─{0}─┬─{1}─┬─{2}─┬─{3}─┐{RESET}",
                    "─".repeat(w_game), "─".repeat(w_acc), "─".repeat(w_streamers), "─".repeat(w_time)
                ));
                lines.push(format!(
                    "{CYAN}│{RESET} {BOLD}{:<w_game$}{RESET} {CYAN}│{RESET} {BOLD}{:<w_acc$}{RESET} {CYAN}│{RESET} {BOLD}{:<w_streamers$}{RESET} {CYAN}│{RESET} {BOLD}{:<w_time$}{RESET} {CYAN}│{RESET}",
                    "Game Name", "Accounts", "Live Streamers", "Progress / Time"
                ));
                lines.push(format!(
                    "{CYAN}├─{0}─┼─{1}─┼─{2}─┼─{3}─┤{RESET}",
                    "─".repeat(w_game), "─".repeat(w_acc), "─".repeat(w_streamers), "─".repeat(w_time)
                ));

                for (g_name, g_info) in &st.games {
                    let cnt = g_info.count;
                    let is_active = g_info.is_active;
                    let st_list = if !g_info.streamers.is_empty() {
                        g_info.streamers.join(", ")
                    } else if is_active {
                        "Active Farming".to_string()
                    } else {
                        "Seeking streamer...".to_string()
                    };
                    let st_list_trunc = truncate_visible(&st_list, w_streamers);

                    let min_w = g_info.min_watched;
                    let req_w = g_info.required_minutes;
                    let time_left_str = &g_info.time_left;

                    let left_cell = if time_left_str == "Done" {
                        format!("{GREEN}Done ({req_w}/{req_w}m){RESET}")
                    } else if !is_active || time_left_str == "Starts Soon" || time_left_str == "Searching..." {
                        format!("{YELLOW}Searching...{RESET}")
                    } else {
                        format!("{CYAN}{time_left_str}{RESET} {DIM}({min_w}/{req_w}m){RESET}")
                    };

                    let display_g = truncate_visible(g_name, w_game);
                    let acc_cell = format!("{YELLOW}{cnt:>2} acc{RESET}");

                    let g_pad = pad_visible(&format!("{WHITE}{display_g}{RESET}"), w_game, '<');
                    let a_pad = pad_visible(&acc_cell, w_acc, '<');
                    let st_color = if is_active { WHITE } else { YELLOW };
                    let s_pad = pad_visible(&format!("{st_color}{st_list_trunc}{RESET}"), w_streamers, '<');
                    let t_pad = pad_visible(&left_cell, w_time, '<');

                    lines.push(format!(
                        "{CYAN}│{RESET} {g_pad} {CYAN}│{RESET} {a_pad} {CYAN}│{RESET} {s_pad} {CYAN}│{RESET} {t_pad} {CYAN}│{RESET}"
                    ));
                }

                lines.push(format!(
                    "{CYAN}└─{0}─┴─{1}─┴─{2}─┴─{3}─┘{RESET}",
                    "─".repeat(w_game), "─".repeat(w_acc), "─".repeat(w_streamers), "─".repeat(w_time)
                ));
            } else {
                lines.push(format!("   {DIM}No accounts currently assigned. Waiting for master server tasks...{RESET}"));
            }

            lines.push(String::new());
            lines.push(format!(" {CYAN}●{RESET} {BOLD}RECENT ACTIVITY:{RESET}"));

            let fixed_lines_count = lines.len() + 3;
            let max_events = rows.saturating_sub(fixed_lines_count).clamp(2, 25);

            if !st.recent_events.is_empty() {
                let max_e = (inner_w + 4).saturating_sub(4);
                for evt in st.recent_events.iter().take(max_events) {
                    let truncated_evt = truncate_visible(evt, max_e);
                    lines.push(format!("  {CYAN}▸{RESET} {DIM}{truncated_evt}{RESET}"));
                }
            } else {
                lines.push(format!("  {DIM}Waiting for telemetry stream from farmer process...{RESET}"));
            }

            lines.push(format!("{CYAN}{}{RESET}", "─".repeat(inner_w + 4)));
            let footer = truncate_visible(&format!("Live (1s) • Auto-scaled ({}x{}) • Closes with worker", cols, rows), inner_w + 4);
            lines.push(format!(" {DIM}{footer}{RESET}"));
        } else {
            lines.push(String::new());
            lines.push(format!(" {YELLOW}Waiting for worker process to initialize and write state...{RESET}"));
            lines.push(format!(" {DIM}If worker is running, telemetry will appear here momentarily.{RESET}"));
        }

        // Move cursor to top-left & clear
        let mut stdout = io::stdout();
        let payload = lines.join("\r\n");
        let _ = write!(stdout, "\x1b[H\x1b[J{}\r\n", payload);
        let _ = stdout.flush();

        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    // Show cursor on exit
    print!("\x1b[?25h");
    let _ = io::stdout().flush();

    Ok(())
}

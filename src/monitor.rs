use std::fs;
use std::io::{self, Write};
use std::time::Duration;
use crate::state_writer::WorkerStateFile;

// ANSI Colors
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
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
        let mut mode: u32 = 0;
        if GetConsoleMode(handle, &mut mode) != 0 {
            SetConsoleMode(handle, mode | 0x0004 | 0x0008);
        }
        let title: Vec<u16> = "TDC Cluster — Live Worker Monitor\0".encode_utf16().collect();
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
    Path::new(&format!("/proc/{}", pid)).exists()
}

#[cfg(windows)]
fn get_terminal_size() -> (usize, usize) {
    extern "system" {
        fn GetStdHandle(nStdHandle: i32) -> *mut std::ffi::c_void;
        fn GetConsoleScreenBufferInfo(h: *mut std::ffi::c_void, info: *mut CONSOLE_SCREEN_BUFFER_INFO) -> i32;
    }
    #[repr(C)]
    #[allow(dead_code)]
    struct COORD { x: i16, y: i16 }
    #[repr(C)]
    #[allow(dead_code)]
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
            let w = (info.srWindow.right - info.srWindow.left + 1) as usize;
            let h = (info.srWindow.bottom - info.srWindow.top + 1) as usize;
            return (w.max(60), h.max(15));
        }
    }
    (80, 25)
}

#[cfg(not(windows))]
fn get_terminal_size() -> (usize, usize) {
    (80, 25)
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else if max_len > 3 {
        let tr: String = s.chars().take(max_len - 3).collect();
        format!("{}...", tr)
    } else {
        s.chars().take(max_len).collect()
    }
}

fn format_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let rem = secs % 86400;
    let hours = rem / 3600;
    let mins = (rem % 3600) / 60;
    let s = rem % 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{}d", days));
    }
    if hours > 0 || days > 0 {
        parts.push(format!("{}h", hours));
    }
    parts.push(format!("{}m", mins));
    parts.push(format!("{}s", s));
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

        let (cols, rows) = get_terminal_size();
        let term_width = cols.min(160);
        let table_target_w = term_width.saturating_sub(4);
        let inner_w = table_target_w.saturating_sub(4);

        // Read .worker_state.json
        let state_opt = if let Ok(content) = fs::read_to_string(".worker_state.json") {
            serde_json::from_str::<WorkerStateFile>(&content).ok()
        } else {
            None
        };

        let mut lines = Vec::new();

        lines.push(format!("{CYAN}╔{}╗{RESET}", "═".repeat(inner_w + 2)));
        let title_text = "  TWITCH DROPS CLUSTER — NATIVE WORKER MONITOR  ";
        let title_pad = (inner_w + 2).saturating_sub(title_text.len());
        lines.push(format!("{CYAN}║{RESET}{BOLD}{WHITE}{}{}{RESET}{CYAN}║{RESET}", title_text, " ".repeat(title_pad)));
        lines.push(format!("{CYAN}╚{}╝{RESET}", "═".repeat(inner_w + 2)));

        if let Some(st) = state_opt {
            let conn_badge = if st.master_connected {
                format!("{GREEN}● CONNECTED{RESET}")
            } else {
                format!("{YELLOW}○ RECONNECTING...{RESET}")
            };

            let uptime_str = format_uptime(st.uptime_seconds);

            lines.push(format!(
                " {BOLD}Node:{RESET} {WHITE}{}{RESET} ({conn_badge})   {BOLD}PID:{RESET} {DIM}{}{RESET}   {BOLD}Uptime:{RESET} {WHITE}{}{RESET}",
                st.node_name, st.parent_pid, uptime_str
            ));
            lines.push(format!(
                " {BOLD}Master Server:{RESET} {CYAN}{}{RESET}",
                st.master_url
            ));
            lines.push(format!(
                " {BOLD}System RAM:{RESET} {WHITE}{:.1} MB{RESET} / {:.1} MB   {BOLD}Limit:{RESET} {WHITE}{} acc{RESET}",
                st.ram_used_mb, st.ram_total_mb, st.max_containers
            ));
            lines.push(format!("{CYAN}{}{RESET}", "─".repeat(inner_w + 2)));

            let mut total_farming = 0;
            let mut total_waiting = 0;
            for g in st.games.values() {
                total_farming += g.active_count;
                total_waiting += g.waiting_count;
            }

            lines.push(format!(
                " {CYAN}●{RESET} {BOLD}ACTIVE ACCOUNTS:{RESET} {GREEN}{BOLD}{} Farming{RESET}  |  {YELLOW}{BOLD}{} Waiting{RESET}  {DIM}(Total: {}){RESET}",
                total_farming, total_waiting, st.total_accounts
            ));
            lines.push(format!("{CYAN}{}{RESET}", "─".repeat(inner_w + 2)));
            lines.push(format!(" {CYAN}●{RESET} {BOLD}GAMES BREAKDOWN:{RESET}"));

            if !st.games.is_empty() {
                let avail_table_w = (inner_w + 2).saturating_sub(13);
                let w_acc = 9;
                let rem_w = avail_table_w.saturating_sub(w_acc);
                let w_game = (rem_w * 36 / 100).max(12);
                let w_streamer = (rem_w * 30 / 100).max(10);
                let w_prog = rem_w.saturating_sub(w_game + w_streamer);

                lines.push(format!(
                    "{CYAN}┌─{0}─┬─{1}─┬─{2}─┬─{3}─┐{RESET}",
                    "─".repeat(w_game), "─".repeat(w_acc), "─".repeat(w_streamer), "─".repeat(w_prog)
                ));
                lines.push(format!(
                    "{CYAN}│{RESET} {BOLD}{:<w_game$}{RESET} {CYAN}│{RESET} {BOLD}{:<w_acc$}{RESET} {CYAN}│{RESET} {BOLD}{:<w_streamer$}{RESET} {CYAN}│{RESET} {BOLD}{:<w_prog$}{RESET} {CYAN}│{RESET}",
                    "Game Name", "Accounts", "Live Streamer", "Progress / Time"
                ));
                lines.push(format!(
                    "{CYAN}├─{0}─┼─{1}─┼─{2}─┼─{3}─┤{RESET}",
                    "─".repeat(w_game), "─".repeat(w_acc), "─".repeat(w_streamer), "─".repeat(w_prog)
                ));

                for (g_name, g_info) in &st.games {
                    let disp_game = truncate_str(g_name, w_game);
                    let acc_str = format!("{} acc", g_info.count);
                    let st_name = g_info.streamers.first().map(|s| s.as_str()).unwrap_or(if g_info.is_active { "Active" } else { "Seeking..." });
                    let disp_st = truncate_str(st_name, w_streamer);

                    let prog_str = if g_info.left_minutes <= 0 {
                        "Done (100%)".to_string()
                    } else if !g_info.is_active {
                        "Searching...".to_string()
                    } else {
                        format!("{}/{}m ({} left)", g_info.min_watched, g_info.required_minutes, g_info.time_left)
                    };
                    let disp_prog = truncate_str(&prog_str, w_prog);

                    lines.push(format!(
                        "{CYAN}│{RESET} {:<w_game$} {CYAN}│{RESET} {:>w_acc$} {CYAN}│{RESET} {:<w_streamer$} {CYAN}│{RESET} {:<w_prog$} {CYAN}│{RESET}",
                        disp_game, acc_str, disp_st, disp_prog
                    ));
                }

                lines.push(format!(
                    "{CYAN}└─{0}─┴─{1}─┴─{2}─┴─{3}─┘{RESET}",
                    "─".repeat(w_game), "─".repeat(w_acc), "─".repeat(w_streamer), "─".repeat(w_prog)
                ));
            } else {
                lines.push(format!("   {DIM}No accounts assigned yet. Waiting for jobs from Master Server...{RESET}"));
            }

            lines.push(String::new());
            lines.push(format!(" {CYAN}●{RESET} {BOLD}RECENT ACTIVITY:{RESET}"));

            // Dynamic event limit based on terminal window height so screen NEVER scrolls
            let fixed_lines_count = lines.len() + 3; // separator + footer
            let max_events = rows.saturating_sub(fixed_lines_count).max(1).min(15);

            if !st.recent_events.is_empty() {
                let max_e = (inner_w + 2).saturating_sub(6);
                for evt in st.recent_events.iter().take(max_events) {
                    let disp_e = truncate_str(evt, max_e);
                    lines.push(format!("  {CYAN}▸{RESET} {DIM}{disp_e}{RESET}"));
                }
            } else {
                lines.push(format!("  {DIM}Waiting for telemetry events from active workers...{RESET}"));
            }

            lines.push(format!("{CYAN}{}{RESET}", "─".repeat(inner_w + 2)));
            let footer = format!("Live (1s) • Auto-scaled ({}x{}) • Closes with worker process", cols, rows);
            let disp_footer = truncate_str(&footer, inner_w + 2);
            lines.push(format!(" {DIM}{disp_footer}{RESET}"));
        } else {
            lines.push(format!(" {YELLOW}Waiting for worker process to initialize and write state...{RESET}"));
            lines.push(format!(" {DIM}If worker is running, telemetry will appear here momentarily.{RESET}"));
        }

        // Clear screen and scrollback, move cursor to 0,0
        print!("\x1b[2J\x1b[3J\x1b[H");
        let output = lines.join("\r\n");
        print!("{}\r\n", output);
        let _ = io::stdout().flush();

        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    // Show cursor on exit
    print!("\x1b[?25h");
    let _ = io::stdout().flush();

    Ok(())
}

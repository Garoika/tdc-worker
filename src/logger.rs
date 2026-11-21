use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};

const MAX_LOG_LINES: usize = 5000;

static LOG_BUFFER: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
static PARTIAL_LINE: OnceLock<Mutex<Vec<u8>>> = OnceLock::new();

fn get_buffer() -> &'static Mutex<VecDeque<String>> {
    LOG_BUFFER.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_LOG_LINES)))
}

fn get_partial() -> &'static Mutex<Vec<u8>> {
    PARTIAL_LINE.get_or_init(|| Mutex::new(Vec::new()))
}

#[derive(Clone)]
pub struct DualWriter;

impl Write for DualWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = io::stdout().write_all(buf);

        if let (Ok(mut partial), Ok(mut logs)) = (get_partial().lock(), get_buffer().lock()) {
            for &b in buf {
                if b == b'\n' {
                    let line = String::from_utf8_lossy(&partial).to_string();
                    partial.clear();
                    if logs.len() >= MAX_LOG_LINES {
                        logs.pop_front();
                    }
                    logs.push_back(line);
                } else if b != b'\r' {
                    partial.push(b);
                }
            }
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = io::stdout().flush();
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for DualWriter {
    type Writer = DualWriter;
    fn make_writer(&'a self) -> Self::Writer {
        DualWriter
    }
}

pub fn get_all_logs(max_lines: usize) -> String {
    if let Ok(logs) = get_buffer().lock() {
        let start = logs.len().saturating_sub(max_lines);
        let slice: Vec<&str> = logs.iter().skip(start).map(|s| s.as_str()).collect();
        slice.join("\n")
    } else {
        String::new()
    }
}

pub fn get_logs_for_account(login: &str, max_lines: usize) -> String {
    let tag = format!("[{}]", login.to_lowercase());
    let login_lower = login.to_lowercase();

    if let Ok(logs) = get_buffer().lock() {
        let matching: Vec<&str> = logs
            .iter()
            .filter(|line| {
                let l = line.to_lowercase();
                l.contains(&tag) || l.contains(&login_lower)
            })
            .map(|s| s.as_str())
            .collect();

        if matching.is_empty() {
            return format!("[INFO] [{}] Worker started. Initializing account session...", login);
        }

        let start = matching.len().saturating_sub(max_lines);
        matching[start..].join("\n")
    } else {
        format!("[INFO] [{}] Log buffer lock error", login)
    }
}

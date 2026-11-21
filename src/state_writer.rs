use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStateFile {
    pub updated_at: u64,
    pub start_time: u64,
    pub uptime_seconds: u64,
    pub node_name: String,
    pub master_url: String,
    pub master_connected: bool,
    pub parent_pid: u32,
    pub ram_used_mb: f64,
    pub ram_total_mb: f64,
    pub total_accounts: usize,
    pub max_containers: usize,
    pub games: HashMap<String, GameStateInfo>,
    pub recent_events: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameStateInfo {
    pub count: usize,
    pub active_count: usize,
    pub waiting_count: usize,
    pub is_active: bool,
    pub streamers: Vec<String>,
    pub min_watched: i32,
    pub required_minutes: i32,
    pub left_minutes: i32,
    pub time_left: String,
    pub drop_index: i32,
    pub drop_total: i32,
}

#[derive(Debug, Clone)]
pub struct AccountTelemetryState {
    pub login: String,
    pub game: String,
    pub streamer: String,
    pub watched_mins: i32,
    pub required_mins: i32,
    pub percent: i32,
    pub drop_index: i32,
    pub drop_total: i32,
    pub is_actively_watching: bool,
}

pub struct StateTracker {
    pub start_time: u64,
    pub node_name: String,
    pub master_url: String,
    pub master_connected: bool,
    pub max_containers: usize,
    pub parent_pid: u32,
    pub accounts: HashMap<String, AccountTelemetryState>,
    pub recent_events: Vec<String>,
    last_saved: u64,
}

static TRACKER: OnceLock<Mutex<StateTracker>> = OnceLock::new();

fn get_tracker() -> &'static Mutex<StateTracker> {
    TRACKER.get_or_init(|| {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        Mutex::new(StateTracker {
            start_time: now,
            node_name: "Worker Node".to_string(),
            master_url: "".to_string(),
            master_connected: false,
            max_containers: 100,
            parent_pid: std::process::id(),
            accounts: HashMap::new(),
            recent_events: Vec::new(),
            last_saved: 0,
        })
    })
}

pub fn format_duration_en(minutes: i32) -> String {
    if minutes <= 0 {
        return "Done".to_string();
    }
    let days = minutes / 1440;
    let rem = minutes % 1440;
    let hours = rem / 60;
    let mins = rem % 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{}d", days));
    }
    if hours > 0 {
        parts.push(format!("{}h", hours));
    }
    if mins > 0 || parts.is_empty() {
        parts.push(format!("{}m", mins));
    }
    parts.join(" ")
}

pub fn update_connection_info(url: &str, connected: bool) {
    if let Ok(mut t) = get_tracker().lock() {
        t.master_url = url.to_string();
        t.master_connected = connected;
    }
}

pub fn update_node_registration(name: &str, limit: usize) {
    if let Ok(mut t) = get_tracker().lock() {
        if !name.is_empty() {
            t.node_name = name.to_string();
        }
        t.max_containers = limit;
    }
}

pub fn register_job(login: &str, game: &str) {
    if let Ok(mut t) = get_tracker().lock() {
        t.accounts.entry(login.to_string()).or_insert_with(|| AccountTelemetryState {
            login: login.to_string(),
            game: game.to_string(),
            streamer: String::new(),
            watched_mins: 0,
            required_mins: 60,
            percent: 0,
            drop_index: 1,
            drop_total: 1,
            is_actively_watching: false,
        });
    }
}

pub fn remove_job(login: &str) {
    if let Ok(mut t) = get_tracker().lock() {
        t.accounts.remove(login);
    }
}

pub fn record_telemetry(
    login: &str,
    game: &str,
    camp_watched: i32,
    camp_total: i32,
    pct: i32,
    drop_idx: i32,
    drop_tot: i32,
    streamer: Option<&str>,
    is_watching: bool,
) {
    if let Ok(mut t) = get_tracker().lock() {
        let entry = t.accounts.entry(login.to_string()).or_insert_with(|| AccountTelemetryState {
            login: login.to_string(),
            game: game.to_string(),
            streamer: String::new(),
            watched_mins: 0,
            required_mins: 60,
            percent: 0,
            drop_index: 1,
            drop_total: 1,
            is_actively_watching: false,
        });

        entry.game = game.to_string();
        entry.watched_mins = camp_watched;
        if camp_total > 0 {
            entry.required_mins = camp_total;
        }
        entry.percent = pct;
        entry.drop_index = drop_idx;
        entry.drop_total = drop_tot;
        if let Some(s) = streamer {
            if !s.is_empty() {
                entry.streamer = s.to_string();
            }
        }
        entry.is_actively_watching = is_watching;

        let now_hms = {
            let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
            let h = (secs / 3600) % 24;
            let m = (secs / 60) % 60;
            let s = secs % 60;
            format!("{:02}:{:02}:{:02}", h, m, s)
        };

        let st_str = if !entry.streamer.is_empty() {
            format!(" | {}", entry.streamer)
        } else {
            String::new()
        };

        let evt_text = format!(
            "[{}] {} -> {}: {}/{}m (Drop {}/{}){}",
            now_hms, login, game, camp_watched, entry.required_mins, drop_idx, drop_tot, st_str
        );

        if t.recent_events.first() != Some(&evt_text) {
            t.recent_events.insert(0, evt_text);
            if t.recent_events.len() > 30 {
                t.recent_events.pop();
            }
        }
    }
}

pub fn save_state_to_disk(ram_used: f64, ram_total: f64) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();

    let state_file = if let Ok(mut t) = get_tracker().lock() {
        // Compute game aggregates
        let mut games_map: HashMap<String, GameStateInfo> = HashMap::new();

        for acc in t.accounts.values() {
            let g = games_map.entry(acc.game.clone()).or_insert_with(|| GameStateInfo {
                count: 0,
                active_count: 0,
                waiting_count: 0,
                is_active: false,
                streamers: Vec::new(),
                min_watched: i32::MAX,
                required_minutes: 60,
                left_minutes: 60,
                time_left: "Searching...".to_string(),
                drop_index: 1,
                drop_total: 1,
            });

            g.count += 1;
            if acc.is_actively_watching && !acc.streamer.is_empty() {
                g.active_count += 1;
                g.is_active = true;
                if !g.streamers.contains(&acc.streamer) && g.streamers.len() < 5 {
                    g.streamers.push(acc.streamer.clone());
                }
            } else {
                g.waiting_count += 1;
            }

            if acc.watched_mins < g.min_watched {
                g.min_watched = acc.watched_mins;
            }
            if acc.required_mins > 0 {
                g.required_minutes = acc.required_mins;
            }
            g.drop_index = acc.drop_index;
            g.drop_total = acc.drop_total;
        }

        for g in games_map.values_mut() {
            if g.min_watched == i32::MAX {
                g.min_watched = 0;
            }
            g.left_minutes = (g.required_minutes - g.min_watched).max(0);
            if g.is_active {
                g.time_left = format_duration_en(g.left_minutes);
            } else {
                g.time_left = "Searching...".to_string();
            }
        }

        let uptime = now.saturating_sub(t.start_time);
        t.last_saved = now;

        WorkerStateFile {
            updated_at: now,
            start_time: t.start_time,
            uptime_seconds: uptime,
            node_name: t.node_name.clone(),
            master_url: t.master_url.clone(),
            master_connected: t.master_connected,
            parent_pid: t.parent_pid,
            ram_used_mb: ram_used,
            ram_total_mb: ram_total,
            total_accounts: t.accounts.len(),
            max_containers: t.max_containers,
            games: games_map,
            recent_events: t.recent_events.clone(),
        }
    } else {
        return;
    };

    if let Ok(json_bytes) = serde_json::to_vec_pretty(&state_file) {
        let tmp = Path::new(".worker_state.json.tmp");
        let target = Path::new(".worker_state.json");
        if fs::write(tmp, &json_bytes).is_ok() {
            let _ = fs::rename(tmp, target);
        }
    }
}

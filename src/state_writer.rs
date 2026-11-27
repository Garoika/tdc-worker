use std::collections::BTreeMap;
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
    #[serde(default)]
    pub cpu_percent: f64,
    pub ram_used_mb: f64,
    pub ram_total_mb: f64,
    pub total_accounts: usize,
    pub max_containers: usize,
    pub games: BTreeMap<String, GameStateInfo>,
    pub recent_events: Vec<String>,
}

impl WorkerStateFile {
    pub fn ordered_games(&self) -> Vec<(&String, &GameStateInfo)> {
        let mut items: Vec<(&String, &GameStateInfo)> = self.games.iter().collect();
        items.sort_by(|(name_a, a), (name_b, b)| {
            a.order.cmp(&b.order).then_with(|| name_a.cmp(name_b))
        });
        items
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameStateInfo {
    #[serde(default)]
    pub order: u64,
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
    pub has_error: bool,
}

pub struct StateTracker {
    pub start_time: u64,
    pub node_name: String,
    pub master_url: String,
    pub master_connected: bool,
    pub max_containers: usize,
    pub parent_pid: u32,
    pub accounts: BTreeMap<String, AccountTelemetryState>,
    pub game_order: Vec<String>,
    pub recent_events: Vec<String>,
    last_saved: u64,
}

impl StateTracker {
    fn new(start_time: u64, parent_pid: u32) -> Self {
        Self {
            start_time,
            node_name: "Worker Node".to_string(),
            master_url: String::new(),
            master_connected: false,
            max_containers: 100,
            parent_pid,
            accounts: BTreeMap::new(),
            game_order: Vec::new(),
            recent_events: Vec::new(),
            last_saved: 0,
        }
    }

    fn ensure_game_registered(&mut self, game: &str) {
        if !game.is_empty() && !self.game_order.iter().any(|g| g == game) {
            self.game_order.push(game.to_string());
        }
    }

    fn register(&mut self, job_id: &str, login: &str, game: &str) {
        self.ensure_game_registered(game);
        self.accounts.entry(job_id.to_string()).or_insert_with(|| AccountTelemetryState {
            login: login.to_string(),
            game: game.to_string(),
            streamer: String::new(),
            watched_mins: 0,
            required_mins: 60,
            percent: 0,
            drop_index: 1,
            drop_total: 1,
            is_actively_watching: false,
            has_error: false,
        });
    }

    fn remove(&mut self, job_id: &str) {
        if let Some(removed) = self.accounts.remove(job_id) {
            if !self.accounts.values().any(|a| a.game == removed.game) {
                self.game_order.retain(|g| g != &removed.game);
            }
        }
    }

    fn build_state_file(&mut self, now: u64, ram_used: f64, ram_total: f64) -> WorkerStateFile {
        let mut games_map: BTreeMap<String, GameStateInfo> = BTreeMap::new();
        let mut game_errors: BTreeMap<String, usize> = BTreeMap::new();

        for acc in self.accounts.values() {
            let g = games_map.entry(acc.game.clone()).or_insert_with(|| GameStateInfo {
                order: 0,
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
            if acc.has_error {
                *game_errors.entry(acc.game.clone()).or_insert(0) += 1;
            }
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

        for (game_name, g) in games_map.iter_mut() {
            if g.min_watched == i32::MAX {
                g.min_watched = 0;
            }
            g.left_minutes = (g.required_minutes - g.min_watched).max(0);
            let base_time = if g.is_active {
                format_duration_en(g.left_minutes)
            } else {
                "Searching...".to_string()
            };
            let err_cnt = game_errors.get(game_name).copied().unwrap_or(0);
            g.time_left = if err_cnt > 0 {
                format!("{} ({} err)", base_time, err_cnt)
            } else {
                base_time
            };
        }

        self.game_order.retain(|g| games_map.contains_key(g));
        for game_name in games_map.keys() {
            if !self.game_order.iter().any(|g| g == game_name) {
                self.game_order.push(game_name.clone());
            }
        }
        for (idx, game_name) in self.game_order.iter().enumerate() {
            if let Some(g) = games_map.get_mut(game_name) {
                g.order = idx as u64;
            }
        }

        let uptime = now.saturating_sub(self.start_time);
        self.last_saved = now;

        WorkerStateFile {
            updated_at: now,
            start_time: self.start_time,
            uptime_seconds: uptime,
            node_name: self.node_name.clone(),
            master_url: self.master_url.clone(),
            master_connected: self.master_connected,
            parent_pid: self.parent_pid,
            cpu_percent: 1.5,
            ram_used_mb: ram_used,
            ram_total_mb: ram_total,
            total_accounts: self.accounts.len(),
            max_containers: self.max_containers,
            games: games_map,
            recent_events: self.recent_events.clone(),
        }
    }
}

static TRACKER: OnceLock<Mutex<StateTracker>> = OnceLock::new();

fn get_tracker() -> &'static Mutex<StateTracker> {
    TRACKER.get_or_init(|| {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        Mutex::new(StateTracker::new(now, std::process::id()))
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
    // Immediately flush to disk so companion monitor/dashboard catches disconnect in real-time
    save_state_to_disk(0.0, 0.0);
}

pub fn update_node_registration(name: &str, limit: usize) {
    if let Ok(mut t) = get_tracker().lock() {
        if !name.is_empty() {
            t.node_name = name.to_string();
        }
        t.max_containers = limit;
    }
}

pub fn register_job(job_id: &str, login: &str, game: &str) {
    if let Ok(mut t) = get_tracker().lock() {
        t.register(job_id, login, game);
    }
}

pub fn remove_job(job_id: &str) {
    if let Ok(mut t) = get_tracker().lock() {
        t.remove(job_id);
    }
}

pub fn record_error(job_id: &str, err_msg: &str) {
    if let Ok(mut t) = get_tracker().lock() {
        let (login, game) = if let Some(acc) = t.accounts.get_mut(job_id) {
            acc.has_error = true;
            acc.is_actively_watching = false;
            (acc.login.clone(), acc.game.clone())
        } else {
            (job_id[..8.min(job_id.len())].to_string(), "Unknown".to_string())
        };

        let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let now_hms = format!("{:02}:{:02}:{:02}", (secs / 3600) % 24, (secs / 60) % 60, secs % 60);
        let short_err: String = err_msg.chars().take(70).collect();
        let evt = format!("[{}] [ERR] {} ({}): {}", now_hms, login, game, short_err);
        t.recent_events.insert(0, evt);
        if t.recent_events.len() > 30 {
            t.recent_events.pop();
        }
    }
}

pub fn clear_error(job_id: &str) {
    if let Ok(mut t) = get_tracker().lock() {
        if let Some(acc) = t.accounts.get_mut(job_id) {
            acc.has_error = false;
        }
    }
}

pub fn record_telemetry(
    job_id: &str,
    login: &str,
    game: &str,
    camp_watched: i32,
    camp_total: i32,
    pct: i32,
    drop_idx: i32,
    drop_tot: i32,
    streamer: Option<&str>,
    is_watching: bool,
    drop_name: Option<&str>,
) {
    if let Ok(mut t) = get_tracker().lock() {
        t.ensure_game_registered(game);
        let entry = t.accounts.entry(job_id.to_string()).or_insert_with(|| AccountTelemetryState {
            login: login.to_string(),
            game: game.to_string(),
            streamer: String::new(),
            watched_mins: 0,
            required_mins: 60,
            percent: 0,
            drop_index: 1,
            drop_total: 1,
            is_actively_watching: false,
            has_error: false,
        });

        entry.login = login.to_string();
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
        entry.has_error = false;

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

        let drop_label = if let Some(dn) = drop_name {
            let lower = dn.trim().to_lowercase();
            if !lower.is_empty() && !lower.starts_with("drop ") && lower != "drop" {
                if drop_tot > 1 {
                    format!("{} ({}/{})", dn, drop_idx, drop_tot)
                } else {
                    dn.to_string()
                }
            } else {
                format!("Drop {}/{}", drop_idx, drop_tot)
            }
        } else {
            format!("Drop {}/{}", drop_idx, drop_tot)
        };

        let evt_text = format!(
            "[{}] {} -> {}: {}/{}m ({}){}",
            now_hms, login, game, camp_watched, entry.required_mins, drop_label, st_str
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
        t.build_state_file(now, ram_used, ram_total)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn game_names(state: &WorkerStateFile) -> Vec<&str> {
        state.ordered_games().into_iter().map(|(name, _)| name.as_str()).collect()
    }

    #[test]
    fn preserves_game_arrival_order_across_rebuilds() {
        let mut tracker = StateTracker::new(100, 1);
        tracker.register("j1", "u1", "Zenless Zone Zero");
        tracker.register("j2", "u2", "Escape from Tarkov");
        tracker.register("j3", "u3", "Apex Legends");
        tracker.register("j4", "u4", "Escape from Tarkov");

        let state1 = tracker.build_state_file(110, 100.0, 1000.0);
        assert_eq!(
            game_names(&state1),
            vec!["Zenless Zone Zero", "Escape from Tarkov", "Apex Legends"]
        );

        let json = serde_json::to_string(&state1).unwrap();
        let parsed: WorkerStateFile = serde_json::from_str(&json).unwrap();
        assert_eq!(
            game_names(&parsed),
            vec!["Zenless Zone Zero", "Escape from Tarkov", "Apex Legends"]
        );
    }

    #[test]
    fn keeps_game_position_until_last_account_leaves_then_appends_on_rearrival() {
        let mut tracker = StateTracker::new(100, 1);
        tracker.register("j1", "u1", "Game B");
        tracker.register("j2", "u2", "Game A");
        tracker.register("j3", "u3", "Game B");

        tracker.remove("j1");
        let s1 = tracker.build_state_file(105, 0.0, 0.0);
        assert_eq!(game_names(&s1), vec!["Game B", "Game A"]);

        tracker.remove("j3");
        let s2 = tracker.build_state_file(106, 0.0, 0.0);
        assert_eq!(game_names(&s2), vec!["Game A"]);

        tracker.register("j4", "u4", "Game B");
        let s3 = tracker.build_state_file(107, 0.0, 0.0);
        assert_eq!(game_names(&s3), vec!["Game A", "Game B"]);
    }

    #[test]
    fn deserializes_legacy_state_without_order_deterministically() {
        let legacy_json = r#"{
            "updated_at": 100,
            "start_time": 90,
            "uptime_seconds": 10,
            "node_name": "n",
            "master_url": "ws://localhost",
            "master_connected": true,
            "parent_pid": 1,
            "ram_used_mb": 10.0,
            "ram_total_mb": 100.0,
            "total_accounts": 2,
            "max_containers": 10,
            "games": {
                "Standoff 2": {
                    "count": 1, "active_count": 1, "waiting_count": 0,
                    "is_active": true, "streamers": [], "min_watched": 0,
                    "required_minutes": 60, "left_minutes": 60,
                    "time_left": "1h", "drop_index": 1, "drop_total": 1
                },
                "Escape from Tarkov": {
                    "count": 1, "active_count": 1, "waiting_count": 0,
                    "is_active": true, "streamers": [], "min_watched": 0,
                    "required_minutes": 60, "left_minutes": 60,
                    "time_left": "1h", "drop_index": 1, "drop_total": 1
                }
            },
            "recent_events": []
        }"#;
        let parsed: WorkerStateFile = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(game_names(&parsed), vec!["Escape from Tarkov", "Standoff 2"]);
    }
}

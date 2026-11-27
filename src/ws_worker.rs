use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn, error, debug};

use crate::models::{
    AccountCredentials, IpcEvent, ServerMessage, WorkerConfigFile,
};
use crate::twitch_gql::TwitchGqlClient;
use crate::spade::SpadeClient;
use crate::farm_worker::run_account_worker;

#[cfg(windows)]
fn get_system_ram_mb() -> (f64, f64) {
    use std::mem::MaybeUninit;
    #[repr(C)]
    struct MEMORYSTATUSEX {
        dw_length: u32,
        dw_memory_load: u32,
        ull_total_phys: u64,
        ull_avail_phys: u64,
        ull_total_page_file: u64,
        ull_avail_page_file: u64,
        ull_total_virtual: u64,
        ull_avail_virtual: u64,
        ull_avail_extended_virtual: u64,
    }
    extern "system" {
        fn GlobalMemoryStatusEx(lp_buffer: *mut MEMORYSTATUSEX) -> i32;
    }
    unsafe {
        let mut status = MaybeUninit::<MEMORYSTATUSEX>::uninit();
        (*status.as_mut_ptr()).dw_length = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(status.as_mut_ptr()) != 0 {
            let status = status.assume_init();
            let total = status.ull_total_phys as f64 / (1024.0 * 1024.0);
            let used = (status.ull_total_phys - status.ull_avail_phys) as f64 / (1024.0 * 1024.0);
            return (used, total);
        }
    }
    (0.0, 0.0)
}

#[cfg(not(windows))]
fn get_system_ram_mb() -> (f64, f64) {
    if let Ok(content) = fs::read_to_string("/proc/meminfo") {
        let mut total = 0.0;
        let mut avail = 0.0;
        for line in content.lines() {
            if line.starts_with("MemTotal:") {
                if let Some(val) = line.split_whitespace().nth(1) {
                    total = val.parse::<f64>().unwrap_or(0.0) / 1024.0;
                }
            } else if line.starts_with("MemAvailable:") {
                if let Some(val) = line.split_whitespace().nth(1) {
                    avail = val.parse::<f64>().unwrap_or(0.0) / 1024.0;
                }
            }
        }
        return (total - avail, total);
    }
    (0.0, 0.0)
}

struct ActiveJob {
    account_id: String,
    login: String,
    cancel_token: CancellationToken,
}

pub fn load_worker_config() -> WorkerConfigFile {
    let candidates = [
        Path::new(".worker_config.json"),
        Path::new("../.worker_config.json"),
    ];

    for p in &candidates {
        if p.exists() {
            if let Ok(content) = fs::read_to_string(p) {
                if let Ok(cfg) = serde_json::from_str::<WorkerConfigFile>(&content) {
                    info!("Loaded worker configuration from '{}'", p.display());
                    return cfg;
                }
            }
        }
    }

    let master_url = std::env::var("MASTER_URL").unwrap_or_else(|_| "ws://127.0.0.1:8000/ws/workers".to_string());
    let worker_token = std::env::var("WORKER_TOKEN").unwrap_or_default();
    let worker_public_ip = std::env::var("WORKER_PUBLIC_IP").ok();

    WorkerConfigFile {
        master_url,
        worker_token,
        worker_public_ip,
    }
}

#[cfg(windows)]
pub fn try_launch_monitor_window() {
    if std::env::var("NO_MONITOR").is_ok() || Path::new("tdc-dashboard.exe").exists() {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        use std::os::windows::process::CommandExt;
        let my_pid = std::process::id();
        let _ = std::process::Command::new(exe)
            .arg("--monitor")
            .arg("--parent-pid")
            .arg(my_pid.to_string())
            .creation_flags(0x00000010) // CREATE_NEW_CONSOLE
            .spawn();
    }
}

#[cfg(not(windows))]
pub fn try_launch_monitor_window() {
    if std::env::var("NO_MONITOR").is_ok() {
        return;
    }

    let has_display = std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok();
    if !has_display {
        return;
    }

    if let Ok(exe) = std::env::current_exe() {
        let my_pid = std::process::id();
        let exe_str = exe.to_string_lossy().to_string();
        let pid_str = my_pid.to_string();

        let candidates: &[(&str, &[&str])] = &[
            ("foot", &["--title", "TDC Live Monitor", &exe_str, "--monitor", "--parent-pid", &pid_str]),
            ("kitty", &["--title", "TDC Live Monitor", &exe_str, "--monitor", "--parent-pid", &pid_str]),
            ("alacritty", &["-T", "TDC Live Monitor", "-e", &exe_str, "--monitor", "--parent-pid", &pid_str]),
            ("ghostty", &["-e", &exe_str, "--monitor", "--parent-pid", &pid_str]),
            ("gnome-terminal", &["--title=TDC Live Monitor", "--", &exe_str, "--monitor", "--parent-pid", &pid_str]),
            ("konsole", &["-e", &exe_str, "--monitor", "--parent-pid", &pid_str]),
            ("xfce4-terminal", &["-T", "TDC Live Monitor", "-e", &exe_str, "--monitor", "--parent-pid", &pid_str]),
            ("xterm", &["-title", "TDC Live Monitor", "-e", &exe_str, "--monitor", "--parent-pid", &pid_str]),
        ];

        for (term, args) in candidates {
            if is_cmd_available(term) {
                if let Ok(child) = std::process::Command::new(term)
                    .args(*args)
                    .spawn()
                {
                    tracing::info!("🖥️  Launched companion TUI Live Monitor in separate window ({}, PID: {})", term, child.id());
                    return;
                }
            }
        }
    }
}

#[cfg(not(windows))]
fn is_cmd_available(cmd: &str) -> bool {
    if let Ok(path) = std::env::var("PATH") {
        for p in std::env::split_paths(&path) {
            let full = p.join(cmd);
            if full.is_file() {
                return true;
            }
        }
    }
    false
}

pub async fn run_ws_worker() -> Result<()> {
    let cfg = load_worker_config();

    info!("=== TDC Pure Rust Native Worker v0.2.0 ===");
    info!("Master Server URL : {}", cfg.master_url);
    info!("Worker Token      : {}", if cfg.worker_token.is_empty() { "(empty)" } else { "********" });

    try_launch_monitor_window();
    crate::state_writer::update_connection_info(&cfg.master_url, false);

    let gql_client = TwitchGqlClient::new();
    let spade_client = SpadeClient::new();

    let (event_tx, mut event_rx) = mpsc::channel::<IpcEvent>(2000);
    let active_jobs: Arc<Mutex<HashMap<String, ActiveJob>>> = Arc::new(Mutex::new(HashMap::new()));

    let mut retry_delay = 1u64;

    loop {
        info!("Connecting to TDC Master Server at {}...", cfg.master_url);

        match connect_and_serve(&cfg, &gql_client, &spade_client, event_tx.clone(), &mut event_rx, Arc::clone(&active_jobs)).await {
            Ok(()) => {
                info!("WebSocket connection closed gracefully. Reconnecting in 2s...");
                retry_delay = 2;
            }
            Err(err) => {
                warn!("Connection error: {}. Retrying in {}s...", err, retry_delay);
            }
        }

        crate::state_writer::update_connection_info(&cfg.master_url, false);

        tokio::time::sleep(Duration::from_secs(retry_delay)).await;
        retry_delay = (retry_delay * 2).min(30);
    }
}

async fn connect_and_serve(
    cfg: &WorkerConfigFile,
    gql_client: &TwitchGqlClient,
    spade_client: &SpadeClient,
    event_tx: mpsc::Sender<IpcEvent>,
    event_rx: &mut mpsc::Receiver<IpcEvent>,
    active_jobs: Arc<Mutex<HashMap<String, ActiveJob>>>,
) -> Result<()> {
    let mut request = cfg.master_url.as_str().into_client_request()
        .context("Invalid master URL")?;

    if !cfg.worker_token.is_empty() {
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", cfg.worker_token).parse()
                .context("Failed to build Authorization header")?,
        );
    }

    let (ws_stream, response) = connect_async(request).await
        .context("WebSocket handshake failed")?;

    info!("Connected to TDC Master! HTTP status: {}", response.status());
    crate::state_writer::update_connection_info(&cfg.master_url, true);

    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    let mut max_containers: usize = 100;

    // 1. Send initial SYNC_STATE reporting currently running containers
    let containers: Vec<serde_json::Value> = {
        let lock = active_jobs.lock().await;
        lock.iter().map(|(jid, job)| {
            json!({
                "name": format!("tdc-farm-{}", &jid[..8.min(jid.len())]),
                "container_id": format!("proc_{}", &jid[..8.min(jid.len())]),
                "job_id": jid,
                "login": job.login,
                "account_id": job.account_id,
                "status": "running"
            })
        }).collect()
    };
    let sync_msg = json!({
        "type": "SYNC_STATE",
        "containers": containers
    });
    ws_tx.send(Message::Text(sync_msg.to_string().into())).await?;

    let mut heartbeat_ticker = tokio::time::interval(Duration::from_secs(5));

    loop {
        tokio::select! {
            // Heartbeat loop every 5s
            _ = heartbeat_ticker.tick() => {
                let now_epoch = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                let (ram_used, ram_total) = get_system_ram_mb();
                crate::state_writer::save_state_to_disk(ram_used, ram_total);

                let active_count = active_jobs.lock().await.len();
                let hb = json!({
                    "type": "HEARTBEAT",
                    "timestamp": now_epoch,
                    "current_containers": active_count,
                    "metrics": {
                        "cpu_usage_percent": 1.0,
                        "ram_used_mb": ram_used,
                        "ram_total_mb": ram_total,
                    }
                });

                if let Err(e) = ws_tx.send(Message::Text(hb.to_string().into())).await {
                    warn!("Failed to send heartbeat: {}", e);
                    break;
                }
            }

            // Outgoing telemetry & status events from active accounts
            Some(evt) = event_rx.recv() => {
                if let IpcEvent::TelemetryUpdate { job_id, account_login, game, telemetry, .. } = &evt {
                    crate::state_writer::record_telemetry(
                        job_id,
                        account_login,
                        game,
                        telemetry.campaign_watched,
                        telemetry.campaign_total,
                        telemetry.campaign_percentage,
                        telemetry.drop_index,
                        telemetry.drop_total,
                        telemetry.active_streamer.as_deref(),
                        telemetry.is_actively_watching,
                        Some(telemetry.drop_name.as_str()),
                    );
                    let (ram_u, ram_t) = get_system_ram_mb();
                    crate::state_writer::save_state_to_disk(ram_u, ram_t);
                } else if let IpcEvent::ContainerEvent { job_id, status, event, error, .. } = &evt {
                    if status == "stopped" || status == "failed" {
                        active_jobs.lock().await.remove(job_id);
                        crate::state_writer::remove_job(job_id);
                    } else if event == "error" {
                        let msg = error.as_deref().unwrap_or("Unknown error");
                        crate::state_writer::record_error(job_id, msg);
                        let (ram_u, ram_t) = get_system_ram_mb();
                        crate::state_writer::save_state_to_disk(ram_u, ram_t);
                    } else if event == "spawned" {
                        crate::state_writer::clear_error(job_id);
                    }
                }

                let payload_str = match serde_json::to_string(&evt) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Failed to serialize event: {}", e);
                        continue;
                    }
                };

                if let Err(e) = ws_tx.send(Message::Text(payload_str.into())).await {
                    warn!("Failed to send telemetry event to master: {}", e);
                    break;
                }
            }

            // Incoming messages from Master Server
            Some(msg_res) = ws_rx.next() => {
                let msg = match msg_res {
                    Ok(Message::Text(txt)) => txt,
                    Ok(Message::Ping(data)) => {
                        let _ = ws_tx.send(Message::Pong(data)).await;
                        continue;
                    }
                    Ok(Message::Close(_)) => {
                        info!("Master server requested connection close.");
                        break;
                    }
                    Ok(_) => continue,
                    Err(e) => {
                        warn!("WebSocket receive error: {}", e);
                        break;
                    }
                };

                let server_msg: ServerMessage = match serde_json::from_str(&msg) {
                    Ok(m) => m,
                    Err(e) => {
                        warn!("Failed to parse server message: {} | Error: {}", msg, e);
                        continue;
                    }
                };

                match server_msg {
                    ServerMessage::Init { name, max_containers: limit } => {
                        if let Some(l) = limit {
                            max_containers = l;
                        }
                        let n_str = name.clone().unwrap_or_default();
                        crate::state_writer::update_node_registration(&n_str, max_containers);
                        info!("Master confirmed node registration: '{}' (limit: {})", name.unwrap_or_default(), max_containers);
                    }
                    ServerMessage::HeartbeatAck => {
                        debug!("HEARTBEAT_ACK received");
                    }
                    ServerMessage::SpawnContainer { job_id, account, target } => {
                        let active_count = active_jobs.lock().await.len();
                        if active_count >= max_containers {
                            warn!("Capacity limit reached ({}/{}). Rejecting job {}", active_count, max_containers, job_id);
                            let cid = format!("proc_{}", &job_id[..8.min(job_id.len())]);
                            let reject_evt = json!({
                                "type": "CONTAINER_EVENT",
                                "job_id": job_id,
                                "container_id": cid,
                                "event": "capacity_rejected",
                                "status": "queued"
                            });
                            let _ = ws_tx.send(Message::Text(reject_evt.to_string().into())).await;
                            continue;
                        }

                        let cid = format!("proc_{}", &job_id[..8.min(job_id.len())]);
                        let login = if !account.login.is_empty() {
                            account.login.clone()
                        } else {
                            format!("job_{}", &job_id[..8.min(job_id.len())])
                        };

                        info!("[NativeWorker] Spawning account '{}' for job {} (Game: '{}')", login, &job_id[..8.min(job_id.len())], target.game);

                        {
                            let mut lock = active_jobs.lock().await;
                            if let Some(old) = lock.remove(&job_id) {
                                crate::state_writer::remove_job(&job_id);
                                old.cancel_token.cancel();
                            }
                        }

                        let child_token = CancellationToken::new();

                        let priority_streamers: Vec<String> = target
                            .priority_streamers
                            .into_iter()
                            .filter_map(|v| {
                                if let Some(s) = v.as_str() {
                                    Some(s.trim().to_lowercase().replace('@', ""))
                                } else if let Some(obj) = v.as_object() {
                                    obj.get("login").and_then(|l| l.as_str()).map(|s| s.trim().to_lowercase().replace('@', ""))
                                } else {
                                    None
                                }
                            })
                            .filter(|s| !s.is_empty())
                            .collect();

                        let credentials = AccountCredentials {
                            login: login.clone(),
                            password: None,
                            auth_token: account.auth_token,
                            user_id: account.twitch_user_id,
                        };

                        let jid_clone = job_id.clone();
                        let acc_id_clone = account.id.clone();
                        let game_clone = target.game.clone();
                        let gql_clone = gql_client.clone();
                        let spade_clone = spade_client.clone();
                        let event_tx_clone = event_tx.clone();
                        let token_clone = child_token.clone();

                        tokio::spawn(async move {
                            run_account_worker(
                                jid_clone,
                                acc_id_clone,
                                credentials,
                                game_clone,
                                priority_streamers,
                                gql_clone,
                                spade_clone,
                                None,
                                Some(event_tx_clone),
                                token_clone,
                            ).await;
                        });

                        crate::state_writer::register_job(&job_id, &login, &target.game);

                        {
                            let mut lock = active_jobs.lock().await;
                            lock.insert(job_id.clone(), ActiveJob {
                                account_id: account.id,
                                login,
                                cancel_token: child_token,
                            });
                        }

                        let spawned_evt = json!({
                            "type": "CONTAINER_EVENT",
                            "job_id": job_id,
                            "container_id": cid,
                            "event": "spawned",
                            "status": "running"
                        });
                        let _ = ws_tx.send(Message::Text(spawned_evt.to_string().into())).await;
                    }
                    ServerMessage::StopContainer { job_id, container_id, account_id, login } => {
                        let mut target_jid = job_id;
                        let mut lock = active_jobs.lock().await;

                        if target_jid.is_none() && container_id.is_some() {
                            let cid_val = container_id.as_ref().unwrap();
                            let clean_cid = cid_val.replace("proc_", "").replace("tdc-farm-", "");
                            for jid in lock.keys() {
                                if jid.starts_with(&clean_cid) || jid == cid_val {
                                    target_jid = Some(jid.clone());
                                    break;
                                }
                            }
                        }

                        if target_jid.is_none() && account_id.is_some() {
                            let acc_val = account_id.as_ref().unwrap();
                            for (jid, info) in lock.iter() {
                                if &info.account_id == acc_val {
                                    target_jid = Some(jid.clone());
                                    break;
                                }
                            }
                        }

                        if target_jid.is_none() && login.is_some() {
                            let log_val = login.as_ref().unwrap().to_lowercase();
                            for (jid, info) in lock.iter() {
                                if info.login.to_lowercase() == log_val {
                                    target_jid = Some(jid.clone());
                                    break;
                                }
                            }
                        }

                        if let Some(jid) = target_jid {
                            if let Some(job) = lock.remove(&jid) {
                                crate::state_writer::remove_job(&jid);
                                job.cancel_token.cancel();
                                info!("[NativeWorker] Stopped account '{}' (job {})", job.login, &jid[..8.min(jid.len())]);
                                let cid = format!("proc_{}", &jid[..8.min(jid.len())]);
                                let stopped_evt = json!({
                                    "type": "CONTAINER_EVENT",
                                    "job_id": jid,
                                    "container_id": cid,
                                    "event": "stopped",
                                    "status": "stopped"
                                });
                                let _ = ws_tx.send(Message::Text(stopped_evt.to_string().into())).await;
                            }
                        }
                    }
                    ServerMessage::StopAllContainers => {
                        let mut lock = active_jobs.lock().await;
                        info!("[NativeWorker] Stopping all {} active container(s)...", lock.len());
                        for (jid, job) in lock.drain() {
                            crate::state_writer::remove_job(&jid);
                            job.cancel_token.cancel();
                            let cid = format!("proc_{}", &jid[..8.min(jid.len())]);
                            let stopped_evt = json!({
                                "type": "CONTAINER_EVENT",
                                "job_id": jid,
                                "container_id": cid,
                                "event": "stopped",
                                "status": "stopped"
                            });
                            let _ = ws_tx.send(Message::Text(stopped_evt.to_string().into())).await;
                        }
                    }
                    ServerMessage::ApplyLimit { limit } => {
                        max_containers = limit;
                        crate::state_writer::update_node_registration("", limit);
                        info!("[NativeWorker] Max capacity updated to {}", limit);
                    }
                    ServerMessage::UpdateMaxContainers { max_containers: limit } => {
                        max_containers = limit;
                        crate::state_writer::update_node_registration("", limit);
                        info!("[NativeWorker] Max capacity updated to {}", limit);
                    }
                    ServerMessage::GetContainerLogs { request_id, job_id: _, container_id: _, login } => {
                        let logs = match login {
                            Some(ref l) if !l.trim().is_empty() => {
                                crate::logger::get_logs_for_account(l.trim(), 200)
                            }
                            _ => {
                                crate::logger::get_all_logs(500)
                            }
                        };

                        let resp = json!({
                            "type": "CONTAINER_LOGS_RESPONSE",
                            "request_id": request_id,
                            "logs": logs
                        });
                        let _ = ws_tx.send(Message::Text(resp.to_string().into())).await;
                    }
                    ServerMessage::Unknown => {}
                }
            }
        }
    }

    // Keep active tasks running across temporary WebSocket reconnects.
    // They will report via SYNC_STATE upon reconnect, and the master server
    // will send STOP_CONTAINER if any were reassigned to another worker.
    crate::state_writer::update_connection_info(&cfg.master_url, false);

    Ok(())
}

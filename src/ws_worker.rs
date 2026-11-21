use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::mpsc;
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
    game: String,
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
    if std::env::var("NO_MONITOR").is_ok() {
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
pub fn try_launch_monitor_window() {}

pub async fn run_ws_worker() -> Result<()> {
    let cfg = load_worker_config();

    info!("=== TDC Pure Rust Native Worker v0.2.0 ===");
    info!("Master Server URL : {}", cfg.master_url);
    info!("Worker Token      : {}", if cfg.worker_token.is_empty() { "(empty)" } else { "********" });

    try_launch_monitor_window();
    crate::state_writer::update_connection_info(&cfg.master_url, false);

    let gql_client = TwitchGqlClient::new();
    let spade_client = SpadeClient::new();

    let mut retry_delay = 1u64;

    loop {
        info!("Connecting to TDC Master Server at {}...", cfg.master_url);

        match connect_and_serve(&cfg, &gql_client, &spade_client).await {
            Ok(()) => {
                info!("WebSocket connection closed gracefully. Reconnecting in 2s...");
                retry_delay = 2;
            }
            Err(err) => {
                warn!("Connection error: {}. Retrying in {}s...", err, retry_delay);
            }
        }

        tokio::time::sleep(Duration::from_secs(retry_delay)).await;
        retry_delay = (retry_delay * 2).min(30);
    }
}

async fn connect_and_serve(
    cfg: &WorkerConfigFile,
    gql_client: &TwitchGqlClient,
    spade_client: &SpadeClient,
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

    let (event_tx, mut event_rx) = mpsc::channel::<IpcEvent>(2000);
    let mut active_jobs: HashMap<String, ActiveJob> = HashMap::new();
    let mut max_containers: usize = 100;

    // 1. Send initial SYNC_STATE
    let sync_msg = json!({
        "type": "SYNC_STATE",
        "containers": []
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

                let hb = json!({
                    "type": "HEARTBEAT",
                    "timestamp": now_epoch,
                    "current_containers": active_jobs.len(),
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
                if let IpcEvent::TelemetryUpdate { account_login, game, telemetry, .. } = &evt {
                    crate::state_writer::record_telemetry(
                        account_login,
                        game,
                        telemetry.campaign_watched,
                        telemetry.campaign_total,
                        telemetry.campaign_percentage,
                        telemetry.drop_index,
                        telemetry.drop_total,
                        telemetry.active_streamer.as_deref(),
                        telemetry.is_actively_watching,
                    );
                    let (ram_u, ram_t) = get_system_ram_mb();
                    crate::state_writer::save_state_to_disk(ram_u, ram_t);
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
                        if active_jobs.len() >= max_containers {
                            warn!("Capacity limit reached ({}/{}). Rejecting job {}", active_jobs.len(), max_containers, job_id);
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

                        if let Some(old) = active_jobs.remove(&job_id) {
                            old.cancel_token.cancel();
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

                        crate::state_writer::register_job(&login, &target.game);

                        active_jobs.insert(job_id.clone(), ActiveJob {
                            account_id: account.id,
                            login,
                            game: target.game,
                            cancel_token: child_token,
                        });

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

                        if target_jid.is_none() && container_id.is_some() {
                            let cid_val = container_id.as_ref().unwrap();
                            let clean_cid = cid_val.replace("proc_", "").replace("tdc-farm-", "");
                            for jid in active_jobs.keys() {
                                if jid.starts_with(&clean_cid) || jid == cid_val {
                                    target_jid = Some(jid.clone());
                                    break;
                                }
                            }
                        }

                        if target_jid.is_none() && account_id.is_some() {
                            let acc_val = account_id.as_ref().unwrap();
                            for (jid, info) in &active_jobs {
                                if &info.account_id == acc_val {
                                    target_jid = Some(jid.clone());
                                    break;
                                }
                            }
                        }

                        if target_jid.is_none() && login.is_some() {
                            let log_val = login.as_ref().unwrap().to_lowercase();
                            for (jid, info) in &active_jobs {
                                if info.login.to_lowercase() == log_val {
                                    target_jid = Some(jid.clone());
                                    break;
                                }
                            }
                        }

                        if let Some(jid) = target_jid {
                            if let Some(job) = active_jobs.remove(&jid) {
                                crate::state_writer::remove_job(&job.login);
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
                        info!("[NativeWorker] Stopping all {} active container(s)...", active_jobs.len());
                        for (jid, job) in active_jobs.drain() {
                            crate::state_writer::remove_job(&job.login);
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

    // Cancel any active tasks when disconnected
    for (_, job) in active_jobs.drain() {
        job.cancel_token.cancel();
    }
    crate::state_writer::update_connection_info(&cfg.master_url, false);

    Ok(())
}

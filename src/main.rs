mod models;
mod twitch_gql;
mod hls_watcher;
mod spade;
mod farm_worker;
mod ws_worker;
pub mod logger;
pub mod state_writer;
pub mod monitor;

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use tracing_subscriber::{fmt, EnvFilter};

use crate::models::{AccountCredentials, AccountRuntimeStatus};
use crate::twitch_gql::TwitchGqlClient;
use crate::spade::SpadeClient;
use crate::farm_worker::run_account_worker;

#[cfg(windows)]
fn get_process_memory_mb() -> f64 {
    use std::mem::MaybeUninit;
    #[repr(C)]
    #[allow(non_snake_case)]
    struct PROCESS_MEMORY_COUNTERS {
        cb: u32,
        PageFaultCount: u32,
        PeakWorkingSetSize: usize,
        WorkingSetSize: usize,
        QuotaPeakPagedPoolUsage: usize,
        QuotaPagedPoolUsage: usize,
        QuotaPeakNonPagedPoolUsage: usize,
        QuotaNonPagedPoolUsage: usize,
        PagefileUsage: usize,
        PeakPagefileUsage: usize,
    }
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, ppsmps: *mut PROCESS_MEMORY_COUNTERS, cb: u32) -> i32;
    }
    unsafe {
        let mut pmc = MaybeUninit::<PROCESS_MEMORY_COUNTERS>::uninit();
        let handle = GetCurrentProcess();
        if K32GetProcessMemoryInfo(handle, pmc.as_mut_ptr(), std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32) != 0 {
            let pmc = pmc.assume_init();
            return pmc.WorkingSetSize as f64 / (1024.0 * 1024.0);
        }
    }
    0.0
}

#[cfg(not(windows))]
fn get_process_memory_mb() -> f64 {
    if let Ok(statm) = fs::read_to_string("/proc/self/statm") {
        if let Some(rss_pages) = statm.split_whitespace().nth(1) {
            if let Ok(pages) = rss_pages.parse::<u64>() {
                return (pages * 4096) as f64 / (1024.0 * 1024.0);
            }
        }
    }
    0.0
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // 0. If explicit monitor mode requested -> run live companion dashboard monitor!
    if args.iter().any(|a| a == "--monitor") {
        let parent_pid = args
            .windows(2)
            .find(|w| w[0] == "--parent-pid")
            .and_then(|w| w[1].parse::<u32>().ok());

        return monitor::run_monitor_loop(parent_pid).await;
    }

    // 1. If explicit IPC mode requested
    if args.iter().any(|a| a == "--ipc") {
        return run_ipc_engine().await;
    }

    // 2. If explicit standalone mode requested
    if args.iter().any(|a| a == "--standalone") {
        return run_standalone_engine().await;
    }

    // 3. If explicit worker mode requested OR .worker_config.json exists -> Native Rust Worker!
    if args.iter().any(|a| a == "--worker")
        || Path::new(".worker_config.json").exists()
        || Path::new("../.worker_config.json").exists()
    {
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        fmt()
            .with_writer(crate::logger::DualWriter)
            .with_env_filter(filter)
            .with_target(false)
            .init();
        return ws_worker::run_ws_worker().await;
    }

    // Default: Standalone mode (accounts.txt)
    run_standalone_engine().await
}

async fn run_ipc_engine() -> anyhow::Result<()> {
    // Structured logs to stderr so stdout is strictly clean JSON lines
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .with_target(false)
        .init();

    tracing::info!("=== TDC Pure Rust Twitch Drops Farm Engine (IPC Mode) ===");

    let gql_client = TwitchGqlClient::new();
    let spade_client = SpadeClient::new();
    let (event_tx, mut event_rx) = mpsc::channel::<crate::models::IpcEvent>(2000);

    // Stdout event emitter loop
    tokio::spawn(async move {
        use std::io::Write;
        while let Some(evt) = event_rx.recv().await {
            if let Ok(line) = serde_json::to_string(&evt) {
                let stdout = std::io::stdout();
                let mut handle = stdout.lock();
                let _ = writeln!(handle, "{}", line);
                let _ = handle.flush();
            }
        }
    });

    let mut active_accounts: HashMap<String, CancellationToken> = HashMap::new();

    use tokio::io::AsyncBufReadExt;
    let stdin = tokio::io::stdin();
    let reader = tokio::io::BufReader::new(stdin);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let cmd: crate::models::IpcCommand = match serde_json::from_str(trimmed) {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!("Failed to parse IPC command: {}. Line: {}", err, trimmed);
                continue;
            }
        };

        match cmd {
            crate::models::IpcCommand::StartAccount {
                job_id,
                account_id,
                login,
                auth_token,
                game,
                priority_streamers,
            } => {
                if let Some(old_token) = active_accounts.remove(&job_id) {
                    old_token.cancel();
                }

                let token = CancellationToken::new();
                active_accounts.insert(job_id.clone(), token.clone());

                let acc = AccountCredentials {
                    login: login.clone(),
                    password: None,
                    auth_token,
                    user_id: None,
                };

                let gql_clone = gql_client.clone();
                let spade_clone = spade_client.clone();
                let event_tx_clone = event_tx.clone();
                let jid_clone = job_id.clone();
                let acc_id_clone = account_id.clone();

                tokio::spawn(async move {
                    run_account_worker(
                        jid_clone,
                        acc_id_clone,
                        acc,
                        game,
                        priority_streamers,
                        gql_clone,
                        spade_clone,
                        None,
                        Some(event_tx_clone),
                        token,
                    )
                    .await;
                });
            }
            crate::models::IpcCommand::StopAccount { job_id } => {
                if let Some(token) = active_accounts.remove(&job_id) {
                    token.cancel();
                }
                let cid = format!("proc_{}", &job_id[..8.min(job_id.len())]);
                let _ = event_tx
                    .send(crate::models::IpcEvent::ContainerEvent {
                        job_id,
                        container_id: cid,
                        event: "stopped".to_string(),
                        status: "stopped".to_string(),
                        error: None,
                    })
                    .await;
            }
            crate::models::IpcCommand::StopAll => {
                for (_, token) in active_accounts.drain() {
                    token.cancel();
                }
            }
        }
    }

    Ok(())
}

async fn run_standalone_engine() -> anyhow::Result<()> {
    // Initialize structured logging
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    info!("=== TDC Pure Rust Twitch Drops Farm Engine v0.1.0 ===");
    info!("Runtime: Native Rust (tokio async multi-thread)");

    let accounts_path = Path::new("accounts.txt");
    if !accounts_path.exists() {
        let template = "# Accounts format: login:password:auth_token or just auth_token\n# Example:\n# my_login:my_pass:abcd1234efgh5678ijkl9012\n";
        fs::write(accounts_path, template)?;
        warn!("'accounts.txt' was not found. Created empty template.");
        warn!("Please add your Twitch accounts to 'accounts.txt' and rerun.");
        return Ok(());
    }

    let content = fs::read_to_string(accounts_path)?;
    let mut accounts = Vec::new();

    for line in content.lines() {
        if let Some(acc) = AccountCredentials::parse(line) {
            accounts.push(acc);
        }
    }

    if accounts.is_empty() {
        warn!("No valid accounts found in 'accounts.txt'. Please add accounts.");
        return Ok(());
    }

    info!("Loaded {} account(s) from accounts.txt", accounts.len());

    let target_game = if Path::new("target_game.txt").exists() {
        fs::read_to_string("target_game.txt")?.trim().to_lowercase()
    } else {
        "rust".to_string()
    };
    info!("Target game for drops: '{}' (change in target_game.txt)", target_game);

    let priority_streamers: Vec<String> = if Path::new("priority_streamers.txt").exists() {
        fs::read_to_string("priority_streamers.txt")
            .unwrap_or_default()
            .split(&[',', '\n', '\r'][..])
            .map(|s| s.trim().to_lowercase().replace('@', ""))
            .filter(|s| !s.is_empty() && !s.starts_with('#'))
            .collect()
    } else {
        Vec::new()
    };
    if !priority_streamers.is_empty() {
        info!("Loaded {} priority streamer(s) from priority_streamers.txt: {:?}", priority_streamers.len(), priority_streamers);
    }

    let cancel_token = CancellationToken::new();
    let (status_tx, mut status_rx) = mpsc::channel::<AccountRuntimeStatus>(500);

    let gql_client = TwitchGqlClient::new();
    let total_loaded = accounts.len();

    // Spawn an isolated worker task for every account
    for (idx, acc) in accounts.into_iter().enumerate() {
        let token_clone = cancel_token.child_token();
        let gql_clone = gql_client.clone();
        let spade_clone = SpadeClient::new();
        let tx_clone = status_tx.clone();
        let game_clone = target_game.clone();
        let prio_clone = priority_streamers.clone();
        let jid = format!("standalone_{}", idx);

        tokio::spawn(async move {
            run_account_worker(
                jid,
                "".to_string(),
                acc,
                game_clone,
                prio_clone,
                gql_clone,
                spade_clone,
                Some(tx_clone),
                None,
                token_clone,
            ).await;
        });
    }

    // Background monitor task: collects statuses & prints live summary every 15s
    let monitor_cancel = cancel_token.clone();
    tokio::spawn(async move {
        let mut status_map: HashMap<String, AccountRuntimeStatus> = HashMap::new();
        let mut ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(15),
            Duration::from_secs(15),
        );

        loop {
            tokio::select! {
                _ = monitor_cancel.cancelled() => break,
                Some(status) = status_rx.recv() => {
                    status_map.insert(status.login.clone(), status);
                }
                _ = ticker.tick() => {
                    let ram_mb = get_process_memory_mb();
                    let active_farming = status_map.values().filter(|s| s.state == "FARMING").count();
                    let invalid = status_map.values().filter(|s| s.state == "INVALID_TOKEN").count();

                    info!(
                        "[MONITOR] Active: {} / {} | Invalid: {} | Process RAM: {:.2} MB",
                        active_farming, total_loaded, invalid, ram_mb
                    );

                    if status_map.len() <= 10 {
                        for (login, s) in &status_map {
                            if s.state == "FARMING" {
                                info!(
                                    "  -> [{}] Channel: {} | Drop: {} ({:.1}%, {}/{}m) | HLS Segs: {} | Spades: {}",
                                    login, s.current_channel, s.drop_name, s.percent, s.progress_minutes, s.required_minutes, s.hls_segments_checked, s.spade_count
                                );
                            }
                        }
                    }
                }
            }
        }
    });

    tokio::signal::ctrl_c().await?;
    info!("Shutting down pure Rust Twitch drops farm engine...");
    cancel_token.cancel();
    tokio::time::sleep(Duration::from_millis(500)).await;
    info!("All workers stopped cleanly. Bye!");

    Ok(())
}

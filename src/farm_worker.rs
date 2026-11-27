use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn, error, debug};

use crate::models::{AccountCredentials, AccountRuntimeStatus, StreamerChannel, IpcEvent, TelemetryPayload};
use crate::twitch_gql::TwitchGqlClient;
use crate::hls_watcher::HlsWatcher;
use crate::spade::SpadeClient;

pub async fn run_account_worker(
    job_id: String,
    account_id: String,
    mut account: AccountCredentials,
    target_game: String,
    priority_streamers: Vec<String>,
    gql: TwitchGqlClient,
    spade: SpadeClient,
    status_tx: Option<mpsc::Sender<AccountRuntimeStatus>>,
    event_tx: Option<mpsc::Sender<IpcEvent>>,
    cancel_token: CancellationToken,
) {
    let acc_tag = if !account.login.is_empty() {
        account.login.clone()
    } else {
        format!("token_{}...", &account.auth_token[..account.auth_token.len().min(8)])
    };

    let jitter_ms = job_id.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64)) % 5000;
    if jitter_ms > 0 {
        tokio::select! {
            _ = cancel_token.cancelled() => return,
            _ = tokio::time::sleep(Duration::from_millis(jitter_ms)) => {}
        }
    }

    info!("[{}] Authenticating account via Twitch GQL...", acc_tag);

    let mut had_auth_error = false;
    let (user_id, login) = loop {
        if cancel_token.is_cancelled() {
            return;
        }
        match gql.validate_token(&account.auth_token).await {
            Ok(res) => {
                if had_auth_error {
                    if let Some(tx) = &event_tx {
                        let cid = format!("proc_{}", &job_id[..8.min(job_id.len())]);
                        let _ = tx.send(IpcEvent::ContainerEvent {
                            job_id: job_id.clone(),
                            container_id: cid,
                            event: "spawned".to_string(),
                            status: "running".to_string(),
                            error: None,
                        }).await;
                    }
                }
                break res;
            }
            Err(err) => {
                if cancel_token.is_cancelled() {
                    return;
                }
                had_auth_error = true;
                let err_text = format!("Token validation failed: {} (retrying in 30s)", err);
                error!("[{}] {}", acc_tag, err_text);
                if let Some(tx) = &status_tx {
                    let _ = tx.send(AccountRuntimeStatus {
                        login: acc_tag.clone(),
                        state: "ERROR_RETRY".to_string(),
                        current_game: target_game.clone(),
                        current_channel: "-".to_string(),
                        drop_name: "-".to_string(),
                        progress_minutes: 0,
                        required_minutes: 0,
                        percent: 0.0,
                        spade_count: 0,
                        hls_segments_checked: 0,
                    }).await;
                }
                if let Some(tx) = &event_tx {
                    let cid = format!("proc_{}", &job_id[..8.min(job_id.len())]);
                    let _ = tx.send(IpcEvent::ContainerEvent {
                        job_id: job_id.clone(),
                        container_id: cid,
                        event: "error".to_string(),
                        status: "running".to_string(),
                        error: Some(err_text),
                    }).await;
                }
                tokio::select! {
                    _ = cancel_token.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_secs(30)) => {}
                }
            }
        }
    };

    account.user_id = Some(user_id.clone());
    account.login = login.clone();
    let acc_tag = login.clone();

    info!("[{}] Authenticated successfully! User ID: {}", acc_tag, user_id);

    let mut hls = HlsWatcher::new();
    let mut total_spades: u32 = 0;
    let mut total_hls_segments: u32 = 0;
    let mut current_streamer: Option<StreamerChannel> = None;
    let mut streamer_blacklist: HashMap<String, std::time::Instant> = HashMap::new();

    while !cancel_token.is_cancelled() {
        // Expire blacklisted streamers older than 15 minutes
        streamer_blacklist.retain(|_, added| added.elapsed() < Duration::from_secs(900));

        // Find a live streamer for target game if not watching
        if current_streamer.is_none() {
            let mut chosen = None;

            if !priority_streamers.is_empty() {
                // If priority streamers are specified, ONLY check them. Never fall back to game directory.
                for p_login in &priority_streamers {
                    let p_lower = p_login.trim().to_lowercase();
                    if streamer_blacklist.contains_key(&p_lower) {
                        continue;
                    }
                    if let Ok(Some(channel)) = gql.fetch_streamer_by_login(&account.auth_token, p_login).await {
                        if gql.verify_channel_drops(&account.auth_token, &channel.id, &target_game).await {
                            info!("[{}] Selected priority drops streamer: {} (Channel ID: {})", acc_tag, channel.login, channel.id);
                            chosen = Some(channel);
                            break;
                        }
                    }
                }

                if chosen.is_none() {
                    warn!("[{}] None of priority streamers {:?} are available/live with drops for '{}'. Retrying in 60s...", acc_tag, priority_streamers, target_game);
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    continue;
                }
            } else {
                // ONLY when priority_streamers is EMPTY: search directory / leaderboard
                info!("[{}] No priority streamers configured. Searching game directory for live streamers with drops in '{}'...", acc_tag, target_game);
                match gql.fetch_streamers_for_game(&account.auth_token, &target_game).await {
                    Ok(streamers) if !streamers.is_empty() => {
                        for s in &streamers {
                            let s_lower = s.login.trim().to_lowercase();
                            if streamer_blacklist.contains_key(&s_lower) {
                                continue;
                            }
                            if gql.verify_channel_drops(&account.auth_token, &s.id, &target_game).await {
                                chosen = Some(s.clone());
                                break;
                            }
                        }

                        if chosen.is_none() {
                            warn!("[{}] Found {} live streamer(s), but none are available or have active drops for '{}'. Retrying in 60s...", acc_tag, streamers.len(), target_game);
                            tokio::time::sleep(Duration::from_secs(60)).await;
                            continue;
                        }

                        let c = chosen.as_ref().unwrap();
                        info!("[{}] Selected verified drops streamer: {} (Channel ID: {})", acc_tag, c.login, c.id);
                    }
                    Ok(_) => {
                        info!("[{}] No live streamers found for game '{}'. Retrying in 60s...", acc_tag, target_game);
                        tokio::time::sleep(Duration::from_secs(60)).await;
                        continue;
                    }
                    Err(err) => {
                        warn!("[{}] Error searching streamers for '{}': {}. Retrying in 60s...", acc_tag, target_game, err);
                        tokio::time::sleep(Duration::from_secs(60)).await;
                        continue;
                    }
                }
            }

            let chosen = chosen.unwrap();

                    // Resolve playback token
                    match gql.fetch_playback_token(&account.auth_token, &chosen.login).await {
                        Ok(pt) => {
                            match hls.resolve_media_playlist(&chosen.login, &pt.signature, &pt.value).await {
                                Ok(_) => {
                                    info!("[{}] HLS media playlist resolved for {}", acc_tag, chosen.login);
                                    let chosen_channel = chosen.login.clone();
                                    current_streamer = Some(chosen);

                                    let mut init_camp_name = format!("{} Drop", target_game);
                                    let mut init_drop_name = "Connecting...".to_string();
                                    let mut init_drop_watched = 0;
                                    let mut init_drop_required = 60;
                                    let mut init_camp_watched = 0;
                                    let mut init_camp_total = 60;
                                    let mut init_pct = 0.0;
                                    let mut init_drop_idx = 1;
                                    let mut init_drop_tot = 1;
                                    let mut init_completed = false;
                                    let mut init_claimed_drops = None;

                                    if let Ok(Some(camp_info)) = gql.fetch_inventory_campaign_progress(&account.auth_token, &target_game).await {
                                        init_camp_name = camp_info.campaign_name;
                                        init_drop_name = camp_info.drop_name;
                                        init_drop_watched = camp_info.drop_watched;
                                        init_drop_required = camp_info.drop_required;
                                        init_camp_watched = camp_info.campaign_watched;
                                        init_camp_total = camp_info.campaign_total;
                                        init_pct = camp_info.campaign_percentage as f32;
                                        init_drop_idx = camp_info.drop_index;
                                        init_drop_tot = camp_info.drop_total;
                                        init_completed = camp_info.is_completed;
                                        if !camp_info.earned_drops.is_empty() {
                                            init_claimed_drops = Some(camp_info.earned_drops);
                                        }
                                        info!(
                                            "[{}] Initial Progress: {} (Drop {}/{}) | {}/{} mins (Campaign: {}/{} mins, {:.1}%)",
                                            acc_tag, init_drop_name, init_drop_idx, init_drop_tot, init_drop_watched, init_drop_required, init_camp_watched, init_camp_total, init_pct
                                        );
                                    } else if let Ok(Some(session)) = gql.fetch_current_session(&account.auth_token, &chosen_channel).await {
                                        init_drop_watched = session.current_minutes_watched;
                                        init_drop_required = session.required_minutes_watched;
                                        init_camp_watched = init_drop_watched;
                                        init_camp_total = init_drop_required;
                                        if init_drop_required > 0 {
                                            init_pct = (init_drop_watched as f32 / init_drop_required as f32) * 100.0;
                                        }
                                        init_drop_name = format!("{} Drop", session.game_name.as_deref().unwrap_or(&target_game));
                                        init_camp_name = init_drop_name.clone();
                                        info!(
                                            "[{}] Initial Session Progress: {} | {}/{} mins ({:.1}%)",
                                            acc_tag, init_drop_name, init_drop_watched, init_drop_required, init_pct
                                        );
                                    }

                                    // Send immediate FARMING status with real starting drop progress
                                    if let Some(tx) = &status_tx {
                                        let _ = tx.send(AccountRuntimeStatus {
                                            login: acc_tag.clone(),
                                            state: "FARMING".to_string(),
                                            current_game: target_game.clone(),
                                            current_channel: chosen_channel.clone(),
                                            drop_name: init_drop_name.clone(),
                                            progress_minutes: init_camp_watched,
                                            required_minutes: init_camp_total,
                                            percent: init_pct,
                                            spade_count: 0,
                                            hls_segments_checked: 0,
                                        }).await;
                                    }

                                    if let Some(tx) = &event_tx {
                                        let cid = format!("proc_{}", &job_id[..8.min(job_id.len())]);
                                        let _ = tx.send(IpcEvent::ContainerEvent {
                                            job_id: job_id.clone(),
                                            container_id: cid.clone(),
                                            event: "spawned".to_string(),
                                            status: "running".to_string(),
                                            error: None,
                                        }).await;
                                        let _ = tx.send(IpcEvent::TelemetryUpdate {
                                            job_id: job_id.clone(),
                                            account_id: account_id.clone(),
                                            account_login: acc_tag.clone(),
                                            game: target_game.clone(),
                                            container_id: cid,
                                            telemetry: TelemetryPayload {
                                                campaign_name: init_camp_name,
                                                current_drop: init_drop_name.clone(),
                                                drop_name: init_drop_name,
                                                current_minutes: init_drop_watched,
                                                required_minutes: init_drop_required,
                                                drop_watched: init_drop_watched,
                                                drop_required: init_drop_required,
                                                campaign_watched: init_camp_watched,
                                                campaign_total: init_camp_total,
                                                campaign_percentage: init_pct as i32,
                                                drop_index: init_drop_idx,
                                                drop_total: init_drop_tot,
                                                active_streamer: Some(chosen_channel),
                                                is_actively_watching: true,
                                                is_completed: init_completed,
                                                claimed_drops: init_claimed_drops,
                                            },
                                        }).await;
                                    }
                                }
                                Err(err) => {
                                    warn!("[{}] Failed to resolve HLS playlist for {}: {}. Rotating streamer...", acc_tag, chosen.login, err);
                                    streamer_blacklist.insert(chosen.login.to_lowercase(), std::time::Instant::now());
                                    tokio::time::sleep(Duration::from_secs(2)).await;
                                    continue;
                                }
                            }
                        }
                        Err(err) => {
                            warn!("[{}] Playback token rejected for {}: {}. Rotating streamer...", acc_tag, chosen.login, err);
                            streamer_blacklist.insert(chosen.login.to_lowercase(), std::time::Instant::now());
                            tokio::time::sleep(Duration::from_secs(2)).await;
                            continue;
                        }
                    }
        }

        let streamer = current_streamer.as_ref().unwrap().clone();

        // Single watch cycle: 50 seconds
        // Every 10 seconds: HLS segment poll
        let mut hls_err_count = 0;
        for _ in 0..5 {
            if cancel_token.is_cancelled() {
                break;
            }

            match hls.poll_and_head_segments().await {
                Ok(count) => {
                    total_hls_segments += count;
                    hls_err_count = 0;
                }
                Err(err) => {
                    debug!("[{}] HLS poll error: {}", acc_tag, err);
                    hls_err_count += 1;
                    if hls_err_count >= 3 {
                        warn!("[{}] Streamer '{}' HLS stream ended or unreachable. Rotating...", acc_tag, streamer.login);
                        streamer_blacklist.insert(streamer.login.to_lowercase(), std::time::Instant::now());
                        current_streamer = None;
                        break;
                    }
                }
            }

            tokio::time::sleep(Duration::from_secs(10)).await;
        }

        if current_streamer.is_none() {
            continue;
        }

        if cancel_token.is_cancelled() {
            break;
        }

        // Send Spade minute-watched heartbeat
        let bcast_id = streamer.broadcast_id.as_deref().unwrap_or("0");
        match spade.send_minute_watched(&account.auth_token, &user_id, &streamer.id, bcast_id).await {
            Ok(true) => {
                total_spades += 1;
                debug!("[{}] Spade minute-watched sent (#{})", acc_tag, total_spades);
            }
            Ok(false) => {
                debug!("[{}] Spade rejected or failed", acc_tag);
            }
            Err(err) => {
                debug!("[{}] Spade network error: {}", acc_tag, err);
            }
        }

        // Check drop progress via GQL InventoryQuery (with fallback to current_session)
        let mut drop_title = "-".to_string();
        let mut camp_name = format!("{} Drop", target_game);
        let mut cur_drop_watched = 0;
        let mut cur_drop_required = 60;
        let mut cur_camp_watched = 0;
        let mut cur_camp_total = 60;
        let mut cur_drop_idx = 1;
        let mut cur_drop_tot = 1;
        let mut camp_pct = 0.0;
        let mut is_done = false;
        let mut cur_claimed_drops = None;

        if let Ok(Some(camp_info)) = gql.fetch_inventory_campaign_progress(&account.auth_token, &target_game).await {
            camp_name = camp_info.campaign_name;
            drop_title = camp_info.drop_name;
            cur_drop_watched = camp_info.drop_watched;
            cur_drop_required = camp_info.drop_required;
            cur_camp_watched = camp_info.campaign_watched;
            cur_camp_total = camp_info.campaign_total;
            cur_drop_idx = camp_info.drop_index;
            cur_drop_tot = camp_info.drop_total;
            camp_pct = camp_info.campaign_percentage as f32;
            is_done = camp_info.is_completed;
            if !camp_info.earned_drops.is_empty() {
                cur_claimed_drops = Some(camp_info.earned_drops);
            }

            info!(
                "[{}] Progress: {} (Drop {}/{}) | {}/{} mins (Campaign: {}/{} mins, {:.1}%) | HLS segs: {} | Spades: {}",
                acc_tag, drop_title, cur_drop_idx, cur_drop_tot, cur_drop_watched, cur_drop_required, cur_camp_watched, cur_camp_total, camp_pct, total_hls_segments, total_spades
            );

            // RULE: NEVER CLAIM DROPS!
            if is_done {
                info!(
                    "[{}] Campaign '{}' reached 100% ({}/{} mins)! Leaving UNCLAIMED for buyer.",
                    acc_tag, camp_name, cur_camp_watched, cur_camp_total
                );
                current_streamer = None;
            }
        } else {
            match gql.fetch_current_session(&account.auth_token, &streamer.login).await {
                Ok(Some(session)) => {
                    cur_drop_watched = session.current_minutes_watched;
                    cur_drop_required = session.required_minutes_watched;
                    cur_camp_watched = cur_drop_watched;
                    cur_camp_total = cur_drop_required;

                    if cur_drop_required > 0 {
                        camp_pct = (cur_drop_watched as f32 / cur_drop_required as f32) * 100.0;
                    }

                    drop_title = format!("{} Drop", session.game_name.as_deref().unwrap_or(&target_game));
                    camp_name = drop_title.clone();
                    is_done = cur_drop_watched >= cur_drop_required && cur_drop_required > 0;

                    info!(
                        "[{}] Progress: {} | {}/{} mins ({:.1}%) | HLS segs: {} | Spades: {}",
                        acc_tag, drop_title, cur_drop_watched, cur_drop_required, camp_pct, total_hls_segments, total_spades
                    );

                    if is_done {
                        info!(
                            "[{}] Drop '{}' reached 100% ({} mins)! Leaving UNCLAIMED for buyer.",
                            acc_tag, drop_title, cur_drop_watched
                        );
                        current_streamer = None;
                    }
                }
                Ok(None) => {
                    debug!("[{}] No current drop session returned for {}", acc_tag, streamer.login);
                }
                Err(err) => {
                    debug!("[{}] Error fetching drop progress: {}", acc_tag, err);
                }
            }
        }

        // Verify streamer is still live and hasn't changed game/category
        match gql.fetch_streamer_by_login(&account.auth_token, &streamer.login).await {
            Ok(Some(live_info)) => {
                let cur_game = live_info.game_name.unwrap_or_default().to_lowercase();
                let tgt_game = target_game.trim().to_lowercase();
                if !cur_game.contains(&tgt_game) && !tgt_game.contains(&cur_game) {
                    warn!(
                        "[{}] Streamer '{}' switched game to '{}' (target: '{}'). Leaving channel...",
                        acc_tag, streamer.login, cur_game, target_game
                    );
                    current_streamer = None;
                }
            }
            Ok(None) => {
                warn!("[{}] Streamer '{}' went OFFLINE. Leaving channel...", acc_tag, streamer.login);
                current_streamer = None;
            }
            Err(err) => {
                debug!("[{}] Failed to verify live status for {}: {}", acc_tag, streamer.login, err);
            }
        }

        // Send status snapshot for UI
        if let Some(tx) = &status_tx {
            let _ = tx.send(AccountRuntimeStatus {
                login: acc_tag.clone(),
                state: if is_done { "COMPLETED".to_string() } else { "FARMING".to_string() },
                current_game: target_game.clone(),
                current_channel: streamer.login.clone(),
                drop_name: drop_title.clone(),
                progress_minutes: cur_camp_watched,
                required_minutes: cur_camp_total,
                percent: camp_pct,
                spade_count: total_spades,
                hls_segments_checked: total_hls_segments,
            }).await;
        }

        // Send real-time telemetry to IPC stream
        if let Some(tx) = &event_tx {
            let cid = format!("proc_{}", &job_id[..8.min(job_id.len())]);
            let _ = tx.send(IpcEvent::TelemetryUpdate {
                job_id: job_id.clone(),
                account_id: account_id.clone(),
                account_login: acc_tag.clone(),
                game: target_game.clone(),
                container_id: cid.clone(),
                telemetry: TelemetryPayload {
                    campaign_name: camp_name,
                    current_drop: drop_title.clone(),
                    drop_name: drop_title,
                    current_minutes: cur_drop_watched,
                    required_minutes: cur_drop_required,
                    drop_watched: cur_drop_watched,
                    drop_required: cur_drop_required,
                    campaign_watched: cur_camp_watched,
                    campaign_total: cur_camp_total,
                    campaign_percentage: camp_pct as i32,
                    drop_index: cur_drop_idx,
                    drop_total: cur_drop_tot,
                    active_streamer: Some(streamer.login.clone()),
                    is_actively_watching: true,
                    is_completed: is_done,
                    claimed_drops: cur_claimed_drops,
                },
            }).await;

            if is_done {
                let _ = tx.send(IpcEvent::ContainerEvent {
                    job_id: job_id.clone(),
                    container_id: cid,
                    event: "completed".to_string(),
                    status: "completed".to_string(),
                    error: None,
                }).await;
            }
        }
    }

    info!("[{}] Account worker stopped cleanly.", acc_tag);
}

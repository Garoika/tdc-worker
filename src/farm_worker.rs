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

    info!("[{}] Authenticating account via Twitch GQL...", acc_tag);

    let (user_id, login) = match gql.validate_token(&account.auth_token).await {
        Ok(res) => res,
        Err(err) => {
            error!("[{}] Token validation failed: {}", acc_tag, err);
            if let Some(tx) = &status_tx {
                let _ = tx.send(AccountRuntimeStatus {
                    login: acc_tag.clone(),
                    state: "INVALID_TOKEN".to_string(),
                    current_game: "-".to_string(),
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
                    event: "failed".to_string(),
                    status: "failed".to_string(),
                    error: Some(format!("Token validation failed: {}", err)),
                }).await;
            }
            return;
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

    while !cancel_token.is_cancelled() {
        // Find a live streamer for target game if not watching
        if current_streamer.is_none() {
            let mut chosen = None;

            if !priority_streamers.is_empty() {
                // If priority streamers are specified, ONLY check them. Never fall back to game directory.
                for p_login in &priority_streamers {
                    if let Ok(Some(channel)) = gql.fetch_streamer_by_login(&account.auth_token, p_login).await {
                        if gql.verify_channel_drops(&account.auth_token, &channel.id, &target_game).await {
                            info!("[{}] Selected priority drops streamer: {} (Channel ID: {})", acc_tag, channel.login, channel.id);
                            chosen = Some(channel);
                            break;
                        }
                    }
                }

                if chosen.is_none() {
                    warn!("[{}] None of priority streamers {:?} are live with drops for '{}'. Retrying in 30s...", acc_tag, priority_streamers, target_game);
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    continue;
                }
            } else {
                // ONLY when priority_streamers is EMPTY: search directory / leaderboard
                info!("[{}] No priority streamers configured. Searching game directory for live streamers with drops in '{}'...", acc_tag, target_game);
                match gql.fetch_streamers_for_game(&account.auth_token, &target_game).await {
                    Ok(streamers) if !streamers.is_empty() => {
                        for s in &streamers {
                            if gql.verify_channel_drops(&account.auth_token, &s.id, &target_game).await {
                                chosen = Some(s.clone());
                                break;
                            }
                        }

                        if chosen.is_none() {
                            warn!("[{}] Found {} live streamer(s) with 'Drops Enabled' badge, but none have active WATCH campaigns for '{}'. Retrying in 30s...", acc_tag, streamers.len(), target_game);
                            tokio::time::sleep(Duration::from_secs(30)).await;
                            continue;
                        }

                        let c = chosen.as_ref().unwrap();
                        info!("[{}] Selected verified drops streamer: {} (Channel ID: {})", acc_tag, c.login, c.id);
                    }
                    Ok(_) => {
                        info!("[{}] No live streamers found for game '{}'. Retrying in 30s...", acc_tag, target_game);
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        continue;
                    }
                    Err(err) => {
                        warn!("[{}] Error searching streamers for '{}': {}. Retrying in 20s...", acc_tag, target_game, err);
                        tokio::time::sleep(Duration::from_secs(20)).await;
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

                                    let mut init_mins = 0;
                                    let mut init_req = 0;
                                    let mut init_pct = 0.0;
                                    let mut init_title = "Connecting...".to_string();

                                    if let Ok(Some(session)) = gql.fetch_current_session(&account.auth_token, &chosen_channel).await {
                                        init_mins = session.current_minutes_watched;
                                        init_req = session.required_minutes_watched;
                                        if init_req > 0 {
                                            init_pct = (init_mins as f32 / init_req as f32) * 100.0;
                                        }
                                        init_title = format!("{} Drop", session.game_name.as_deref().unwrap_or(&target_game));
                                        info!(
                                            "[{}] Initial Progress: {} | {}/{} mins ({:.1}%)",
                                            acc_tag, init_title, init_mins, init_req, init_pct
                                        );
                                    }

                                    // Send immediate FARMING status with real starting drop progress
                                    if let Some(tx) = &status_tx {
                                        let _ = tx.send(AccountRuntimeStatus {
                                            login: acc_tag.clone(),
                                            state: "FARMING".to_string(),
                                            current_game: target_game.clone(),
                                            current_channel: chosen_channel.clone(),
                                            drop_name: init_title.clone(),
                                            progress_minutes: init_mins,
                                            required_minutes: init_req,
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
                                                campaign_name: init_title.clone(),
                                                current_drop: init_title.clone(),
                                                drop_name: init_title.clone(),
                                                current_minutes: init_mins,
                                                required_minutes: init_req,
                                                drop_watched: init_mins,
                                                drop_required: init_req,
                                                campaign_watched: init_mins,
                                                campaign_total: init_req,
                                                campaign_percentage: init_pct as i32,
                                                drop_index: 1,
                                                drop_total: 1,
                                                active_streamer: Some(chosen_channel),
                                                is_actively_watching: true,
                                                is_completed: false,
                                            },
                                        }).await;
                                    }
                                }
                                Err(err) => {
                                    warn!("[{}] Failed to resolve HLS playlist for {}: {}", acc_tag, chosen.login, err);
                                    tokio::time::sleep(Duration::from_secs(10)).await;
                                    continue;
                                }
                            }
                        }
                        Err(err) => {
                            warn!("[{}] Failed to fetch playback token for {}: {}", acc_tag, chosen.login, err);
                            tokio::time::sleep(Duration::from_secs(10)).await;
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
                        warn!("[{}] Streamer '{}' HLS stream ended or unreachable. Leaving channel...", acc_tag, streamer.login);
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

        // Check drop progress via GQL
        let mut drop_title = "-".to_string();
        let mut cur_mins = 0;
        let mut req_mins = 0;
        let mut pct = 0.0;

        match gql.fetch_current_session(&account.auth_token, &streamer.login).await {
            Ok(Some(session)) => {
                cur_mins = session.current_minutes_watched;
                req_mins = session.required_minutes_watched;

                if req_mins > 0 {
                    pct = (cur_mins as f32 / req_mins as f32) * 100.0;
                }

                drop_title = format!("{} Drop", session.game_name.as_deref().unwrap_or(&target_game));

                info!(
                    "[{}] Progress: {} | {}/{} mins ({:.1}%) | HLS segs: {} | Spades: {}",
                    acc_tag, drop_title, cur_mins, req_mins, pct, total_hls_segments, total_spades
                );

                // RULE: NEVER CLAIM DROPS!
                if cur_mins >= req_mins && req_mins > 0 {
                    info!(
                        "[{}] Drop '{}' reached 100% ({} mins)! Leaving UNCLAIMED for buyer.",
                        acc_tag, drop_title, cur_mins
                    );
                    // Rotate to next campaign or pause
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

        let is_done = cur_mins >= req_mins && req_mins > 0;

        // Send status snapshot for UI
        if let Some(tx) = &status_tx {
            let _ = tx.send(AccountRuntimeStatus {
                login: acc_tag.clone(),
                state: if is_done { "COMPLETED".to_string() } else { "FARMING".to_string() },
                current_game: target_game.clone(),
                current_channel: streamer.login.clone(),
                drop_name: drop_title.clone(),
                progress_minutes: cur_mins,
                required_minutes: req_mins,
                percent: pct,
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
                    campaign_name: drop_title.clone(),
                    current_drop: drop_title.clone(),
                    drop_name: drop_title,
                    current_minutes: cur_mins,
                    required_minutes: req_mins,
                    drop_watched: cur_mins,
                    drop_required: req_mins,
                    campaign_watched: cur_mins,
                    campaign_total: req_mins,
                    campaign_percentage: pct as i32,
                    drop_index: 1,
                    drop_total: 1,
                    active_streamer: Some(streamer.login.clone()),
                    is_actively_watching: true,
                    is_completed: is_done,
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

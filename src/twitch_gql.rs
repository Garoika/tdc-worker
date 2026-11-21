use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use anyhow::{Context, Result, anyhow};
use reqwest::header::{HeaderMap, HeaderValue, HeaderName};
use serde_json::{json, Value};
use crate::models::{CampaignProgressInfo, DropCurrentSession, PlaybackToken, StreamerChannel};

pub const TWITCH_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
pub const GQL_URL: &str = "https://gql.twitch.tv/gql";

#[derive(Clone)]
pub struct TwitchGqlClient {
    http: reqwest::Client,
    drops_cache: Arc<RwLock<HashMap<String, (Instant, bool)>>>,
    streamer_cache: Arc<RwLock<HashMap<String, (Instant, Option<StreamerChannel>)>>>,
}

impl TwitchGqlClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .pool_max_idle_per_host(50)
            .build()
            .expect("Failed to initialize HTTP client");
        Self {
            http,
            drops_cache: Arc::new(RwLock::new(HashMap::new())),
            streamer_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn headers(auth_token: &str) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(HeaderName::from_static("client-id"), HeaderValue::from_static(TWITCH_CLIENT_ID));
        if !auth_token.is_empty() {
            let oauth = format!("OAuth {}", auth_token);
            if let Ok(val) = HeaderValue::from_str(&oauth) {
                map.insert(HeaderName::from_static("authorization"), val);
            }
        }
        map.insert(
            HeaderName::from_static("user-agent"),
            HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.3"),
        );
        map
    }

    pub async fn validate_token(&self, auth_token: &str) -> Result<(String, String)> {
        let body = json!({
            "query": "query { currentUser { id login } }"
        });

        let resp = self.http.post(GQL_URL)
            .headers(Self::headers(auth_token))
            .json(&body)
            .send()
            .await
            .context("Network error during validate_token request")?;

        if !resp.status().is_success() {
            return Err(anyhow!("Twitch GQL HTTP error: {}", resp.status()));
        }

        let json_val: Value = resp.json().await.context("Failed to parse validate_token JSON")?;
        let user = json_val.pointer("/data/currentUser")
            .ok_or_else(|| anyhow!("Token invalid: currentUser is null"))?;

        let id = user.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let login = user.get("login").and_then(|v| v.as_str()).unwrap_or_default().to_string();

        if id.is_empty() || login.is_empty() {
            return Err(anyhow!("Invalid token: user ID or login missing"));
        }

        Ok((id, login))
    }

    pub async fn fetch_streamer_by_login(&self, auth_token: &str, login: &str) -> Result<Option<StreamerChannel>> {
        let clean_login = login.trim().to_lowercase().replace('@', "");

        // Check shared cache (TTL 30s)
        {
            let cache = self.streamer_cache.read().await;
            if let Some((inserted, res)) = cache.get(&clean_login) {
                if inserted.elapsed() < std::time::Duration::from_secs(30) {
                    return Ok(res.clone());
                }
            }
        }

        let body = json!({
            "query": format!(r#"query {{ user(login: "{}") {{ id login displayName stream {{ id game {{ name }} viewersCount }} }} }}"#, clean_login)
        });

        let resp = self.http.post(GQL_URL)
            .headers(Self::headers(auth_token))
            .json(&body)
            .send()
            .await
            .context("Error requesting streamer by login")?;

        let json_val: Value = resp.json().await.context("Invalid streamer by login response")?;
        let user = match json_val.pointer("/data/user") {
            Some(u) if !u.is_null() => u,
            _ => {
                let mut cache = self.streamer_cache.write().await;
                cache.insert(clean_login, (Instant::now(), None));
                return Ok(None);
            }
        };

        let stream = match user.get("stream") {
            Some(s) if !s.is_null() => s,
            _ => {
                let mut cache = self.streamer_cache.write().await;
                cache.insert(clean_login, (Instant::now(), None));
                return Ok(None); // Streamer is offline
            }
        };

        let id = user.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let login = user.get("login").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let display_name = user.get("displayName").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let stream_id = stream.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
        let game_name = stream.pointer("/game/name").and_then(|v| v.as_str()).map(|s| s.to_string());

        let res = Some(StreamerChannel {
            id,
            login,
            display_name,
            broadcast_id: stream_id,
            game_name,
        });

        let mut cache = self.streamer_cache.write().await;
        cache.insert(clean_login, (Instant::now(), res.clone()));

        Ok(res)
    }

    pub async fn fetch_playback_token(&self, auth_token: &str, channel_login: &str) -> Result<PlaybackToken> {
        let body = json!({
            "operationName": "PlaybackAccessToken",
            "variables": {
                "isLive": true,
                "login": channel_login,
                "isVod": false,
                "vodID": "",
                "platform": "web",
                "playerType": "site"
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "ed230aa1e33e07eebb8928504583da78a5173989fadfb1ac94be06a04f3cdbe9"
                }
            }
        });

        let resp = self.http.post(GQL_URL)
            .headers(Self::headers(auth_token))
            .json(&body)
            .send()
            .await
            .context("Error requesting PlaybackAccessToken")?;

        let json_val: Value = resp.json().await.context("Invalid PlaybackAccessToken response")?;
        let token_obj = json_val.pointer("/data/streamPlaybackAccessToken")
            .ok_or_else(|| anyhow!("streamPlaybackAccessToken missing"))?;

        let value = token_obj.get("value").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let signature = token_obj.get("signature").and_then(|v| v.as_str()).unwrap_or_default().to_string();

        if value.is_empty() || signature.is_empty() {
            return Err(anyhow!("Playback token or signature is empty"));
        }

        Ok(PlaybackToken { value, signature })
    }

    pub async fn fetch_current_session(&self, auth_token: &str, channel_login: &str) -> Result<Option<DropCurrentSession>> {
        let body = json!({
            "operationName": "DropCurrentSessionContext",
            "variables": {
                "channelLogin": channel_login
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "4d06b702d25d652afb9ef835d2a550031f1cf762b193523a92166f40ea3d142b"
                }
            }
        });

        let resp = self.http.post(GQL_URL)
            .headers(Self::headers(auth_token))
            .json(&body)
            .send()
            .await
            .context("Error requesting DropCurrentSessionContext")?;

        let json_val: Value = resp.json().await.context("Invalid DropCurrentSessionContext response")?;
        let session_val = match json_val.pointer("/data/currentUser/dropCurrentSession") {
            Some(v) if !v.is_null() => v,
            _ => return Ok(None),
        };

        let current_minutes = session_val.get("currentMinutesWatched").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let required_minutes = session_val.get("requiredMinutesWatched").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let drop_id = session_val.get("dropID").and_then(|v| v.as_str()).map(|s| s.to_string());
        let channel_login = session_val.pointer("/channel/name").and_then(|v| v.as_str()).map(|s| s.to_string());
        let game_name = session_val.pointer("/game/displayName").and_then(|v| v.as_str()).map(|s| s.to_string());

        Ok(Some(DropCurrentSession {
            drop_id,
            current_minutes_watched: current_minutes,
            required_minutes_watched: required_minutes,
            channel_login,
            game_name,
        }))
    }

    fn is_generic_drop_name(s: &str) -> bool {
        let s_clean = s.trim().to_lowercase();
        if s_clean.is_empty()
            || s_clean == "drop"
            || s_clean == "reward"
            || s_clean == "item"
            || s_clean == "reward item"
            || s_clean == "test"
        {
            return true;
        }
        let parts: Vec<&str> = s_clean.split_whitespace().collect();
        if parts.len() == 2 {
            let prefix = parts[0];
            let suffix = parts[1].trim_start_matches('#');
            if (prefix == "drop" || prefix == "reward" || prefix == "item" || prefix == "tier")
                && suffix.chars().all(|c| c.is_ascii_digit())
            {
                return true;
            }
        }
        s_clean.chars().all(|c| c.is_ascii_digit())
    }

    fn extract_drop_item_name(d: &Value) -> String {
        let mut benefits = Vec::new();
        if let Some(edges) = d.get("benefitEdges").and_then(|v| v.as_array()) {
            for edge in edges {
                if let Some(name) = edge.pointer("/benefit/name").and_then(|v| v.as_str()) {
                    let clean = name.trim();
                    if !clean.is_empty() && !benefits.contains(&clean.to_string()) {
                        benefits.push(clean.to_string());
                    }
                }
            }
        }

        if !benefits.is_empty() {
            return benefits.join(" + ");
        }

        let raw_name = d.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
        if !raw_name.is_empty() && !Self::is_generic_drop_name(raw_name) {
            return raw_name.to_string();
        }

        if !raw_name.is_empty() {
            return raw_name.to_string();
        }

        "Reward Item".to_string()
    }

    pub async fn fetch_inventory_campaign_progress(
        &self,
        auth_token: &str,
        target_game: &str,
    ) -> Result<Option<CampaignProgressInfo>> {
        let body = json!({
            "query": r#"query InventoryQuery {
              currentUser {
                inventory {
                  dropCampaignsInProgress {
                    id
                    name
                    status
                    game {
                      id
                      name
                    }
                    timeBasedDrops {
                      id
                      name
                      requiredMinutesWatched
                      benefitEdges {
                        benefit {
                          id
                          name
                        }
                      }
                      self {
                        isClaimed
                        currentMinutesWatched
                      }
                    }
                  }
                }
              }
            }"#
        });

        let resp = self.http.post(GQL_URL)
            .headers(Self::headers(auth_token))
            .json(&body)
            .send()
            .await
            .context("Error requesting InventoryQuery")?;

        let json_val: Value = resp.json().await.context("Invalid InventoryQuery response")?;
        Ok(Self::parse_inventory_campaign_info(&json_val, target_game))
    }

    pub fn parse_inventory_campaign_info(json_val: &Value, target_game: &str) -> Option<CampaignProgressInfo> {
        let campaigns = match json_val.pointer("/data/currentUser/inventory/dropCampaignsInProgress") {
            Some(Value::Array(arr)) => arr,
            _ => return None,
        };

        let target_clean = target_game.trim().to_lowercase();
        let mut best_campaign = None;
        let mut best_score = 0;

        for c in campaigns {
            let g_name = c.pointer("/game/name").and_then(|v| v.as_str()).unwrap_or_default().to_lowercase();
            let c_name = c.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_lowercase();

            let matches = (!g_name.is_empty() && (g_name.contains(&target_clean) || target_clean.contains(&g_name)))
                || (!c_name.is_empty() && (c_name.contains(&target_clean) || target_clean.contains(&c_name)));

            if matches {
                let status = c.get("status").and_then(|v| v.as_str()).unwrap_or_default();
                let is_active = status.eq_ignore_ascii_case("ACTIVE");
                let is_expired = status.eq_ignore_ascii_case("EXPIRED");

                let has_uncompleted_drop = c.get("timeBasedDrops")
                    .and_then(|v| v.as_array())
                    .map(|drops| {
                        drops.iter().any(|d| {
                            let req = d.get("requiredMinutesWatched").and_then(|v| v.as_i64()).unwrap_or(0);
                            let cur = d.pointer("/self/currentMinutesWatched").and_then(|v| v.as_i64()).unwrap_or(0);
                            let is_claimed = d.pointer("/self/isClaimed").and_then(|v| v.as_bool()).unwrap_or(false);
                            !is_claimed && cur < req
                        })
                    })
                    .unwrap_or(false);

                let score = if is_active && has_uncompleted_drop {
                    4
                } else if is_active {
                    3
                } else if !is_expired {
                    2
                } else {
                    1
                };

                if score > best_score {
                    best_score = score;
                    best_campaign = Some(c);
                    if score == 4 {
                        break;
                    }
                }
            }
        }

        let c = match best_campaign {
            Some(camp) => camp,
            None => return None,
        };

        let campaign_name = c.get("name").and_then(|v| v.as_str()).unwrap_or("Drops Campaign").to_string();
        let drops = match c.get("timeBasedDrops").and_then(|v| v.as_array()) {
            Some(d) if !d.is_empty() => d,
            _ => return None,
        };

        let is_cumulative = drops.len() > 1 && {
            let req0 = drops[0].get("requiredMinutesWatched").and_then(|v| v.as_i64()).unwrap_or(0);
            let req1 = drops[1].get("requiredMinutesWatched").and_then(|v| v.as_i64()).unwrap_or(0);
            req1 > req0
        };

        if is_cumulative {
            let last_req = drops.last()
                .and_then(|d| d.get("requiredMinutesWatched"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            let campaign_total = last_req.max(1);

            let mut max_watched = 0;
            for d in drops {
                let w = d.pointer("/self/currentMinutesWatched").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                if w > max_watched {
                    max_watched = w;
                }
            }
            let campaign_watched = max_watched.min(campaign_total);
            let campaign_percentage = ((campaign_watched as f64 / campaign_total as f64) * 100.0).round() as i32;

            let mut active_drop_name = Self::extract_drop_item_name(&drops[0]);
            let mut drop_index = 1;
            let first_req = drops[0].get("requiredMinutesWatched").and_then(|v| v.as_i64()).unwrap_or(60) as i32;
            let mut drop_required = first_req;
            let mut drop_watched = campaign_watched.min(drop_required);

            let mut all_done = true;
            for (i, d) in drops.iter().enumerate() {
                let req = d.get("requiredMinutesWatched").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let cur = d.pointer("/self/currentMinutesWatched").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let is_claimed = d.pointer("/self/isClaimed").and_then(|v| v.as_bool()).unwrap_or(false);

                let prev_req = if i == 0 {
                    0
                } else {
                    drops[i - 1].get("requiredMinutesWatched").and_then(|v| v.as_i64()).unwrap_or(0) as i32
                };
                let cur_segment_req = (req - prev_req).max(1);

                if !is_claimed && cur < req {
                    all_done = false;
                    active_drop_name = Self::extract_drop_item_name(d);
                    drop_index = (i + 1) as i32;
                    drop_required = cur_segment_req;
                    drop_watched = (campaign_watched - prev_req).max(0).min(drop_required);
                    break;
                } else if i == drops.len() - 1 {
                    active_drop_name = Self::extract_drop_item_name(d);
                    drop_index = drops.len() as i32;
                    drop_required = cur_segment_req;
                    drop_watched = cur_segment_req;
                    if !is_claimed && cur < req {
                        all_done = false;
                    }
                }
            }

            Some(CampaignProgressInfo {
                campaign_name,
                campaign_watched,
                campaign_total,
                campaign_percentage: campaign_percentage.min(100),
                drop_name: active_drop_name,
                drop_watched,
                drop_required,
                drop_index,
                drop_total: drops.len() as i32,
                is_completed: all_done || campaign_watched >= campaign_total,
            })
        } else {
            let mut campaign_total = 0;
            let mut campaign_watched = 0;
            let mut all_done = true;
            let mut active_idx = 1;
            let mut active_name = Self::extract_drop_item_name(&drops[0]);
            let mut active_req = drops[0].get("requiredMinutesWatched").and_then(|v| v.as_i64()).unwrap_or(60) as i32;
            let mut active_watched = drops[0].pointer("/self/currentMinutesWatched").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let mut found_active = false;

            for (i, d) in drops.iter().enumerate() {
                let req = d.get("requiredMinutesWatched").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let cur = d.pointer("/self/currentMinutesWatched").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let is_claimed = d.pointer("/self/isClaimed").and_then(|v| v.as_bool()).unwrap_or(false);

                campaign_total += req;
                campaign_watched += cur.min(req);

                if !found_active && !is_claimed && cur < req {
                    found_active = true;
                    active_idx = (i + 1) as i32;
                    active_name = Self::extract_drop_item_name(d);
                    active_req = req;
                    active_watched = cur;
                }

                if !is_claimed && cur < req {
                    all_done = false;
                }
            }

            campaign_total = campaign_total.max(1);
            let campaign_percentage = ((campaign_watched as f64 / campaign_total as f64) * 100.0).round() as i32;

            Some(CampaignProgressInfo {
                campaign_name,
                campaign_watched,
                campaign_total,
                campaign_percentage: campaign_percentage.min(100),
                drop_name: active_name,
                drop_watched: active_watched,
                drop_required: active_req,
                drop_index: active_idx,
                drop_total: drops.len() as i32,
                is_completed: all_done || campaign_watched >= campaign_total,
            })
        }
    }

    pub async fn verify_channel_drops(&self, auth_token: &str, channel_id: &str, target_game: &str) -> bool {
        // Fast path: check shared cache (valid for 10 minutes)
        {
            let cache = self.drops_cache.read().await;
            if let Some((inserted, has_drops)) = cache.get(channel_id) {
                if inserted.elapsed() < std::time::Duration::from_secs(600) {
                    return *has_drops;
                }
            }
        }

        let body = json!({
            "operationName": "ChannelDropsCampaigns",
            "variables": {
                "channelID": channel_id
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "bc073ff0bb13797898f83e93dc8009838545dec95ff7fe92243c8fa3a740f7e7"
                }
            }
        });

        let mut has_drops = false;
        if let Ok(resp) = self.http.post(GQL_URL)
            .headers(Self::headers(auth_token))
            .json(&body)
            .send()
            .await
        {
            if let Ok(json_val) = resp.json::<Value>().await {
                if let Some(campaigns) = json_val.pointer("/data/channelDropCampaigns").and_then(|v| v.as_array()) {
                    let target_lower = target_game.trim().to_lowercase();
                    for c in campaigns {
                        let game_name = c.pointer("/game/name").and_then(|v| v.as_str()).unwrap_or_default().to_lowercase();
                        let mut game_matches = !game_name.is_empty() && (game_name.contains(&target_lower) || target_lower.contains(&game_name));

                        if let Some(reward_groups) = c.get("rewardGroups").and_then(|v| v.as_array()) {
                            for rg in reward_groups {
                                let req_type = rg.pointer("/progressCriteria/requirementType").and_then(|v| v.as_str()).unwrap_or_default();
                                if !req_type.eq_ignore_ascii_case("WATCH") {
                                    continue;
                                }

                                if !game_matches {
                                    if let Some(cats) = rg.pointer("/progressCriteria/categories").and_then(|v| v.as_array()) {
                                        for cat in cats {
                                            let cat_name = cat.get("displayName").and_then(|v| v.as_str()).unwrap_or_default().to_lowercase();
                                            if !cat_name.is_empty() && (cat_name.contains(&target_lower) || target_lower.contains(&cat_name)) {
                                                game_matches = true;
                                                break;
                                            }
                                        }
                                    }
                                }

                                if game_matches {
                                    has_drops = true;
                                    break;
                                }
                            }
                        }

                        if has_drops {
                            break;
                        }
                    }
                }
            }
        }

        // Cache the result
        let mut cache = self.drops_cache.write().await;
        cache.insert(channel_id.to_string(), (Instant::now(), has_drops));

        has_drops
    }

    pub async fn fetch_streamers_for_game(&self, auth_token: &str, game_slug: &str) -> Result<Vec<StreamerChannel>> {
        let clean_slug = game_slug.trim().to_lowercase().replace(' ', "-").replace(':', "");
        let body = json!({
            "operationName": "DirectoryPage_Game",
            "variables": {
                "imageWidth": 50,
                "includeCostreaming": false,
                "slug": clean_slug,
                "options": {
                    "broadcasterLanguages": [],
                    "includeRestricted": ["SUB_ONLY_LIVE"],
                    "sort": "RELEVANCE",
                    "recommendationsContext": { "platform": "web" },
                    "requestID": "JIRA-VXP-2397",
                    "freeformTags": null,
                    "tags": [],
                    "systemFilters": ["DROPS_ENABLED"]
                },
                "sortTypeIsRecency": false,
                "limit": 10
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "76cb069d835b8a02914c08dc42c421d0dafda8af5b113a3f19141824b901402f"
                }
            }
        });

        let resp = self.http.post(GQL_URL)
            .headers(Self::headers(auth_token))
            .json(&body)
            .send()
            .await
            .context("Error requesting DirectoryPage_Game")?;

        let json_val: Value = resp.json().await.context("Invalid DirectoryPage_Game response")?;
        let mut streamers = Vec::new();

        if let Some(edges) = json_val.pointer("/data/game/streams/edges").and_then(|v| v.as_array()) {
            for edge in edges {
                if let Some(node) = edge.get("node") {
                    let broadcaster = node.get("broadcaster");
                    let id = broadcaster.and_then(|b| b.get("id")).and_then(|v| v.as_str()).unwrap_or_default().to_string();
                    let login = broadcaster.and_then(|b| b.get("login")).and_then(|v| v.as_str()).unwrap_or_default().to_string();
                    let display_name = broadcaster.and_then(|b| b.get("displayName")).and_then(|v| v.as_str()).unwrap_or_default().to_string();
                    let stream_id = node.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());

                    if !login.is_empty() {
                        streamers.push(StreamerChannel {
                            id,
                            login,
                            display_name,
                            broadcast_id: stream_id,
                            game_name: Some(game_slug.to_string()),
                        });
                    }
                }
            }
        }

        Ok(streamers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_campaign(name: &str, status: &str, drops: Vec<(i64, i64, bool)>) -> Value {
        let time_based_drops: Vec<Value> = drops.into_iter().enumerate().map(|(i, (req, cur, claimed))| {
            json!({
                "id": format!("drop_{}", i),
                "name": format!("Item {}", i + 1),
                "requiredMinutesWatched": req,
                "benefitEdges": [{
                    "benefit": {
                        "id": format!("b_{}", i),
                        "name": format!("Item {}", i + 1)
                    }
                }],
                "self": {
                    "currentMinutesWatched": cur,
                    "isClaimed": claimed
                }
            })
        }).collect();

        json!({
            "id": format!("camp_{}", name),
            "name": name,
            "status": status,
            "game": {
                "id": "12345",
                "name": "Escape from Tarkov"
            },
            "timeBasedDrops": time_based_drops
        })
    }

    #[test]
    fn test_three_campaigns_progression() {
        // Campaign 1: Finished (60/60 min, completed)
        let camp1_done = make_campaign("Campaign 1", "ACTIVE", vec![(60, 60, false)]);
        // Campaign 2: In progress (30/60 min)
        let camp2_prog = make_campaign("Campaign 2", "ACTIVE", vec![(60, 30, false)]);
        // Campaign 3: Not started (0/60 min)
        let camp3_wait = make_campaign("Campaign 3", "ACTIVE", vec![(60, 0, false)]);

        let payload_stage1 = json!({
            "data": {
                "currentUser": {
                    "inventory": {
                        "dropCampaignsInProgress": [
                            camp1_done.clone(),
                            camp2_prog.clone(),
                            camp3_wait.clone()
                        ]
                    }
                }
            }
        });

        // Stage 1: Campaign 1 is finished, Campaign 2 is active
        let res1 = TwitchGqlClient::parse_inventory_campaign_info(&payload_stage1, "Escape from Tarkov").unwrap();
        assert_eq!(res1.campaign_name, "Campaign 2");
        assert_eq!(res1.campaign_watched, 30);
        assert_eq!(res1.campaign_total, 60);
        assert_eq!(res1.is_completed, false);

        // Stage 2: Campaign 1 is finished, Campaign 2 is now ALSO finished (60/60), Campaign 3 is in progress
        let camp2_done = make_campaign("Campaign 2", "ACTIVE", vec![(60, 60, false)]);
        let payload_stage2 = json!({
            "data": {
                "currentUser": {
                    "inventory": {
                        "dropCampaignsInProgress": [
                            camp1_done.clone(),
                            camp2_done.clone(),
                            camp3_wait.clone()
                        ]
                    }
                }
            }
        });

        let res2 = TwitchGqlClient::parse_inventory_campaign_info(&payload_stage2, "Escape from Tarkov").unwrap();
        assert_eq!(res2.campaign_name, "Campaign 3");
        assert_eq!(res2.campaign_watched, 0);
        assert_eq!(res2.campaign_total, 60);
        assert_eq!(res2.is_completed, false);

        // Stage 3: ALL 3 campaigns are finished (60/60)
        let camp3_done = make_campaign("Campaign 3", "ACTIVE", vec![(60, 60, false)]);
        let payload_stage3 = json!({
            "data": {
                "currentUser": {
                    "inventory": {
                        "dropCampaignsInProgress": [
                            camp1_done,
                            camp2_done,
                            camp3_done
                        ]
                    }
                }
            }
        });

        let res3 = TwitchGqlClient::parse_inventory_campaign_info(&payload_stage3, "Escape from Tarkov").unwrap();
        assert_eq!(res3.is_completed, true);
    }
}

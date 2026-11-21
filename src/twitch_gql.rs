use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use anyhow::{Context, Result, anyhow};
use reqwest::header::{HeaderMap, HeaderValue, HeaderName};
use serde_json::{json, Value};
use crate::models::{DropCurrentSession, PlaybackToken, StreamerChannel};

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

use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct AccountCredentials {
    pub login: String,
    pub password: Option<String>,
    pub auth_token: String,
    pub user_id: Option<String>,
}

impl AccountCredentials {
    pub fn parse(line: &str) -> Option<Self> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return None;
        }

        let parts: Vec<&str> = trimmed.split(':').collect();
        let (login, password, mut token) = match parts.len() {
            3 => (parts[0].trim().to_string(), Some(parts[1].trim().to_string()), parts[2].trim().to_string()),
            2 => (parts[0].trim().to_string(), None, parts[1].trim().to_string()),
            1 => (String::new(), None, parts[0].trim().to_string()),
            _ => return None,
        };

        if token.to_lowercase().starts_with("oauth:") {
            token = token[6..].trim().to_string();
        }

        if token.is_empty() {
            return None;
        }

        Some(Self {
            login,
            password,
            auth_token: token,
            user_id: None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackToken {
    pub value: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropCurrentSession {
    pub drop_id: Option<String>,
    pub current_minutes_watched: i32,
    pub required_minutes_watched: i32,
    pub channel_login: Option<String>,
    pub game_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignProgressInfo {
    pub campaign_name: String,
    pub campaign_watched: i32,
    pub campaign_total: i32,
    pub campaign_percentage: i32,
    pub drop_name: String,
    pub drop_watched: i32,
    pub drop_required: i32,
    pub drop_index: i32,
    pub drop_total: i32,
    pub is_completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamerChannel {
    pub id: String,
    pub login: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub broadcast_id: Option<String>,
    pub game_name: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct AccountRuntimeStatus {
    pub login: String,
    pub state: String,
    pub current_game: String,
    pub current_channel: String,
    pub drop_name: String,
    pub progress_minutes: i32,
    pub required_minutes: i32,
    pub percent: f32,
    pub spade_count: u32,
    pub hls_segments_checked: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "cmd")]
pub enum IpcCommand {
    #[serde(rename = "start_account")]
    StartAccount {
        job_id: String,
        #[serde(default)]
        account_id: String,
        login: String,
        auth_token: String,
        game: String,
        #[serde(default)]
        priority_streamers: Vec<String>,
    },
    #[serde(rename = "stop_account")]
    StopAccount {
        job_id: String,
    },
    #[serde(rename = "stop_all")]
    StopAll,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum IpcEvent {
    #[serde(rename = "CONTAINER_EVENT")]
    ContainerEvent {
        job_id: String,
        container_id: String,
        event: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    #[serde(rename = "TELEMETRY_UPDATE")]
    TelemetryUpdate {
        job_id: String,
        account_id: String,
        account_login: String,
        game: String,
        container_id: String,
        telemetry: TelemetryPayload,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct TelemetryPayload {
    pub campaign_name: String,
    pub current_drop: String,
    pub drop_name: String,
    pub current_minutes: i32,
    pub required_minutes: i32,
    pub drop_watched: i32,
    pub drop_required: i32,
    pub campaign_watched: i32,
    pub campaign_total: i32,
    pub campaign_percentage: i32,
    pub drop_index: i32,
    pub drop_total: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_streamer: Option<String>,
    pub is_actively_watching: bool,
    pub is_completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerConfigFile {
    #[serde(default = "default_master_url")]
    pub master_url: String,
    #[serde(default)]
    pub worker_token: String,
    #[serde(default)]
    pub worker_public_ip: Option<String>,
}

fn default_master_url() -> String {
    "ws://127.0.0.1:8000/ws/workers".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    #[serde(rename = "INIT")]
    Init {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        max_containers: Option<usize>,
    },
    #[serde(rename = "HEARTBEAT_ACK")]
    HeartbeatAck,
    #[serde(rename = "SPAWN_CONTAINER")]
    SpawnContainer {
        job_id: String,
        account: SpawnAccountData,
        target: SpawnTargetData,
    },
    #[serde(rename = "STOP_CONTAINER")]
    StopContainer {
        #[serde(default)]
        job_id: Option<String>,
        #[serde(default)]
        container_id: Option<String>,
        #[serde(default)]
        account_id: Option<String>,
        #[serde(default)]
        login: Option<String>,
    },
    #[serde(rename = "STOP_ALL_CONTAINERS")]
    StopAllContainers,
    #[serde(rename = "APPLY_LIMIT")]
    ApplyLimit {
        limit: usize,
    },
    #[serde(rename = "UPDATE_MAX_CONTAINERS")]
    UpdateMaxContainers {
        max_containers: usize,
    },
    #[serde(rename = "GET_CONTAINER_LOGS")]
    GetContainerLogs {
        request_id: String,
        #[allow(dead_code)]
        #[serde(default)]
        job_id: Option<String>,
        #[allow(dead_code)]
        #[serde(default)]
        container_id: Option<String>,
        #[serde(default)]
        login: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpawnAccountData {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub login: String,
    pub auth_token: String,
    #[serde(default)]
    pub twitch_user_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpawnTargetData {
    pub game: String,
    #[serde(default)]
    pub priority_streamers: Vec<serde_json::Value>,
}

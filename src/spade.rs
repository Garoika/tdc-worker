use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::{json, Value};
use std::io::Write;

#[derive(Clone)]
pub struct SpadeClient {
    http: reqwest::Client,
}

impl SpadeClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap();
        Self { http }
    }

    pub async fn send_minute_watched(
        &self,
        auth_token: &str,
        user_id: &str,
        channel_id: &str,
        broadcast_id: &str,
    ) -> Result<bool> {
        let parsed_user_id = user_id.parse::<i64>().unwrap_or(0);
        let payload = json!([{
            "event": "minute-watched",
            "properties": {
                "channel_id": channel_id,
                "broadcast_id": broadcast_id,
                "player": "site",
                "user_id": parsed_user_id,
                "live": true,
                "logged_in": true,
                "hidden": false,
                "muted": false,
                "minutes_logged": 1
            }
        }]);

        let json_bytes = payload.to_string().into_bytes();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&json_bytes)?;
        let compressed = encoder.finish()?;
        let b64_payload = BASE64.encode(&compressed);

        let body = json!({
            "query": "mutation SendEvents($input: SendSpadeEventsInput!) { sendSpadeEvents(input: $input) { statusCode } }",
            "variables": {
                "input": {
                    "data": b64_payload
                }
            }
        });

        let mut req = self.http.post("https://gql.twitch.tv/gql")
            .header("Client-ID", crate::twitch_gql::TWITCH_CLIENT_ID)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.3");

        if !auth_token.is_empty() {
            req = req.header("Authorization", format!("OAuth {}", auth_token));
        }

        let resp = req.json(&body).send().await.context("Failed to send Spade GQL mutation")?;
        if !resp.status().is_success() {
            return Ok(false);
        }

        let json_val: Value = resp.json().await.context("Failed to parse Spade GQL response")?;
        let status_code = json_val.pointer("/data/sendSpadeEvents/statusCode").and_then(|v| v.as_i64()).unwrap_or(0);

        Ok(status_code == 204 || status_code == 200)
    }
}

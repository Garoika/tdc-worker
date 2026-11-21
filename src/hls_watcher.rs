use anyhow::{Context, Result, anyhow};
use std::collections::HashSet;
use std::time::Duration;
use tracing::debug;

pub struct HlsWatcher {
    http: reqwest::Client,
    seen_segments: HashSet<String>,
    media_playlist_url: Option<String>,
}

impl HlsWatcher {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        Self {
            http,
            seen_segments: HashSet::new(),
            media_playlist_url: None,
        }
    }

    pub async fn resolve_media_playlist(&mut self, channel_login: &str, sig: &str, token: &str) -> Result<String> {
        let master_url = format!(
            "https://usher.ttvnw.net/api/channel/hls/{}.m3u8?sig={}&token={}&allow_source=true&allow_audio_only=true",
            urlencoding::encode(channel_login),
            urlencoding::encode(sig),
            urlencoding::encode(token)
        );

        let resp = self.http.get(&master_url)
            .send()
            .await
            .context("Failed to request usher master playlist")?;

        if !resp.status().is_success() {
            return Err(anyhow!("Usher returned status {}", resp.status()));
        }

        let body = resp.text().await.context("Failed to read master playlist body")?;
        let mut variants = Vec::new();
        let mut pending = false;

        for line in body.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with("#EXT-X-STREAM-INF:") {
                pending = true;
                continue;
            }
            if pending && !trimmed.starts_with('#') {
                variants.push(trimmed.to_string());
                pending = false;
            }
        }

        let media_url = variants.last()
            .ok_or_else(|| anyhow!("No media stream variants found in master playlist"))?
            .clone();

        self.media_playlist_url = Some(media_url.clone());
        Ok(media_url)
    }

    /// Polls the media playlist and sends HEAD requests for any newly appeared segments.
    /// Returns the number of new segments checked.
    pub async fn poll_and_head_segments(&mut self) -> Result<u32> {
        let media_url = match &self.media_playlist_url {
            Some(url) => url.clone(),
            None => return Err(anyhow!("Media playlist URL not resolved yet")),
        };

        let resp = self.http.get(&media_url)
            .send()
            .await
            .context("Failed to fetch media playlist")?;

        if !resp.status().is_success() {
            return Err(anyhow!("Media playlist returned status {}", resp.status()));
        }

        let base_url = reqwest::Url::parse(&media_url).context("Failed to parse base media url")?;
        let body = resp.text().await.context("Failed to read media playlist")?;

        let mut segments = Vec::new();
        let mut pending = false;

        for line in body.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with("#EXTINF:") {
                pending = true;
                continue;
            }
            if pending && !trimmed.starts_with('#') {
                if let Ok(abs_url) = base_url.join(trimmed) {
                    segments.push(abs_url.to_string());
                }
                pending = false;
            }
        }

        let mut newly_checked = 0;
        for segment in segments {
            if self.seen_segments.contains(&segment) {
                continue;
            }

            // Send HEAD request (0 audio/video traffic, only headers)
            match self.http.head(&segment).send().await {
                Ok(head_resp) if head_resp.status().is_success() => {
                    self.seen_segments.insert(segment);
                    newly_checked += 1;
                    if self.seen_segments.len() > 500 {
                        self.seen_segments.clear(); // Keep bounded memory
                    }
                }
                Ok(head_resp) => {
                    debug!("Segment HEAD returned non-success: {}", head_resp.status());
                }
                Err(err) => {
                    debug!("Segment HEAD request failed: {}", err);
                }
            }
        }

        Ok(newly_checked)
    }
}

use std::fs;
use std::path::Path;
use std::time::Duration;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};

#[derive(Debug, Deserialize)]
struct UpdateManifest {
    #[allow(dead_code)]
    platform: String,
    files: Vec<UpdateFileEntry>,
}

#[derive(Debug, Deserialize)]
struct UpdateFileEntry {
    name: String,
    sha256: String,
    #[allow(dead_code)]
    size: u64,
}

pub async fn start_autoupdate_loop(check_interval: Duration) {
    info!(
        "🔄 Server auto-updater active (checking TDC Master every {}s)",
        check_interval.as_secs()
    );

    // Initial check after 5 seconds
    tokio::time::sleep(Duration::from_secs(5)).await;

    loop {
        if let Err(e) = check_and_update_from_server().await {
            debug!("[AutoUpdate] Server check error: {}", e);
        }
        tokio::time::sleep(check_interval).await;
    }
}

pub async fn check_and_update_from_server() -> anyhow::Result<bool> {
    let cfg = crate::ws_worker::load_worker_config();
    if cfg.master_url.is_empty() || cfg.worker_token.is_empty() {
        return Ok(false);
    }

    // Convert ws://127.0.0.1:8000/ws/workers -> http://127.0.0.1:8000
    let base_url = cfg.master_url
        .replace("ws://", "http://")
        .replace("wss://", "https://")
        .split("/ws/")
        .next()
        .unwrap_or(&cfg.master_url)
        .trim_end_matches('/')
        .to_string();

    #[cfg(windows)]
    let platform = "windows";
    #[cfg(not(windows))]
    let platform = "linux";

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;

    let manifest_url = format!("{}/api/workers/update/manifest?platform={}", base_url, platform);
    let resp = client
        .get(&manifest_url)
        .header("X-Worker-Token", &cfg.worker_token)
        .send()
        .await?;

    if !resp.status().is_success() {
        return Ok(false);
    }

    let manifest: UpdateManifest = resp.json().await?;
    let mut updated_any = false;

    for entry in manifest.files {
        let file_name = Path::new(&entry.name).file_name()
            .and_then(|f| f.to_str())
            .unwrap_or(&entry.name);

        if file_name.is_empty() {
            continue;
        }

        let local_path = Path::new(file_name);
        let matches = if local_path.exists() {
            compute_file_sha256(local_path).map(|h| h.eq_ignore_ascii_case(&entry.sha256)).unwrap_or(false)
        } else {
            false
        };

        if !matches {
            info!(
                "📥 [AutoUpdate] New version of '{}' detected on server (hash: {}). Downloading...",
                file_name, &entry.sha256[..8.min(entry.sha256.len())]
            );

            let download_url = format!(
                "{}/api/workers/update/file?platform={}&filename={}",
                base_url, platform, file_name
            );

            let file_resp = client
                .get(&download_url)
                .header("X-Worker-Token", &cfg.worker_token)
                .send()
                .await?;

            if file_resp.status().is_success() {
                let bytes = file_resp.bytes().await?;
                let temp_name = if let Some(stem) = local_path.file_stem().and_then(|s| s.to_str()) {
                    if let Some(ext) = local_path.extension().and_then(|e| e.to_str()) {
                        format!("{}.new.{}", stem, ext)
                    } else {
                        format!("{}.new", stem)
                    }
                } else {
                    format!("{}.new", file_name)
                };

                fs::write(&temp_name, bytes)?;
                info!("✅ [AutoUpdate] Saved update as '{}'", temp_name);
                updated_any = true;
            } else {
                warn!("[AutoUpdate] Failed to download '{}': HTTP {}", file_name, file_resp.status());
            }
        }
    }

    if updated_any {
        info!("🚀 [AutoUpdate] All components updated successfully from server. Restarting process...");
        std::process::exit(0);
    }

    Ok(false)
}

fn compute_file_sha256(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        use std::io::Read;
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buffer[..n]),
            Err(_) => return None,
        }
    }
    Some(format!("{:x}", hasher.finalize()))
}

use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{debug, info, warn};

pub async fn start_autoupdate_loop(check_interval: Duration) {
    info!(
        "🔄 Auto-updater service active (checking GitHub every {}s)",
        check_interval.as_secs()
    );

    // Initial check after 10 seconds
    tokio::time::sleep(Duration::from_secs(10)).await;

    loop {
        if let Err(e) = check_and_update().await {
            debug!("[AutoUpdate] Check error: {}", e);
        }
        tokio::time::sleep(check_interval).await;
    }
}

pub async fn check_and_update() -> anyhow::Result<bool> {
    if Path::new(".git").exists() {
        update_via_git().await
    } else {
        update_via_api().await
    }
}

async fn detect_git_branch() -> String {
    if let Ok(output) = tokio::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .await
    {
        if output.status.success() {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !branch.is_empty() && branch != "HEAD" {
                return branch;
            }
        }
    }
    "RUSTerization".to_string()
}

async fn update_via_git() -> anyhow::Result<bool> {
    let branch = detect_git_branch().await;

    // 1. Fetch latest commits from remote origin
    let fetch_status = tokio::process::Command::new("git")
        .args(["fetch", "origin", &branch])
        .output()
        .await?;

    if !fetch_status.status.success() {
        return Ok(false);
    }

    // 2. Compare local HEAD with remote tracking ref
    let local_out = tokio::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .await?;
    let local_sha = String::from_utf8_lossy(&local_out.stdout).trim().to_string();

    let remote_out = tokio::process::Command::new("git")
        .args(["rev-parse", &format!("origin/{}", branch)])
        .output()
        .await?;
    let remote_sha = String::from_utf8_lossy(&remote_out.stdout).trim().to_string();

    if local_sha.is_empty() || remote_sha.is_empty() || local_sha == remote_sha {
        return Ok(false);
    }

    info!(
        "🔄 [AutoUpdate] New version detected on GitHub ({} -> {})! Updating repository via git...",
        &local_sha[..7.min(local_sha.len())],
        &remote_sha[..7.min(remote_sha.len())]
    );

    // 3. Reset local repository to match remote commit
    let reset_status = tokio::process::Command::new("git")
        .args(["reset", "--hard", &format!("origin/{}", branch)])
        .output()
        .await?;

    if !reset_status.status.success() {
        warn!("[AutoUpdate] git reset failed: {}", String::from_utf8_lossy(&reset_status.stderr));
        return Ok(false);
    }

    // 4. If Cargo is available, compile fresh release binary locally
    let has_cargo = tokio::process::Command::new("cargo")
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);

    if has_cargo {
        info!("🔨 [AutoUpdate] Compiling updated release binary via cargo...");
        let build_status = tokio::process::Command::new("cargo")
            .args(["build", "--release"])
            .output()
            .await?;

        if build_status.status.success() {
            info!("✅ [AutoUpdate] Cargo release build succeeded. Applying updated binary...");
            apply_new_binary();
        } else {
            warn!("[AutoUpdate] Cargo build failed: {}", String::from_utf8_lossy(&build_status.stderr));
        }
    }

    info!("🚀 [AutoUpdate] Successfully updated to {}. Restarting worker process...", &remote_sha[..7.min(remote_sha.len())]);
    restart_process();
    Ok(true)
}

fn apply_new_binary() {
    #[cfg(windows)]
    {
        let target_bin = Path::new("target/release/tdc-worker.exe");
        let current_bin = Path::new("tdc-worker.exe");
        let old_bin = Path::new("tdc-worker.old.exe");

        if target_bin.exists() {
            // On Windows: running .exe can be renamed, but not overwritten directly
            let _ = std::fs::remove_file(old_bin);
            let _ = std::fs::rename(current_bin, old_bin);
            let _ = std::fs::copy(target_bin, current_bin);
        }
    }

    #[cfg(unix)]
    {
        let target_bin = Path::new("target/release/tdc-worker");
        let current_bin = Path::new("tdc-worker");
        if target_bin.exists() {
            let _ = std::fs::copy(target_bin, current_bin);
        }
    }
}

async fn update_via_api() -> anyhow::Result<bool> {
    let sha_path = PathBuf::from(".current_commit");
    let local_sha = if sha_path.exists() {
        std::fs::read_to_string(&sha_path).unwrap_or_default().trim().to_string()
    } else {
        String::new()
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let url = "https://api.github.com/repos/Garoika/tdc-worker/commits/RUSTerization";
    let resp = client
        .get(url)
        .header("User-Agent", "TDC-Worker-AutoUpdater")
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await?;

    if !resp.status().is_success() {
        return Ok(false);
    }

    let json_val: serde_json::Value = resp.json().await?;
    let remote_sha = json_val.get("sha").and_then(|v| v.as_str()).unwrap_or_default().to_string();

    if remote_sha.is_empty() {
        return Ok(false);
    }

    if local_sha.is_empty() {
        let _ = std::fs::write(&sha_path, &remote_sha);
        return Ok(false);
    }

    if local_sha != remote_sha {
        info!(
            "🔄 [AutoUpdate] New release detected on GitHub API ({} -> {})! Restarting to apply update...",
            &local_sha[..7.min(local_sha.len())],
            &remote_sha[..7.min(remote_sha.len())]
        );
        let _ = std::fs::write(&sha_path, &remote_sha);
        restart_process();
        return Ok(true);
    }

    Ok(false)
}

fn restart_process() {
    // Process exits cleanly with code 0.
    // The supervising loop in start_worker.bat / start_worker.sh automatically restarts it.
    std::process::exit(0);
}

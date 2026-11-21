#!/usr/bin/env bash
set -e

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPT_DIR"

echo "========================================================"
echo "   TDC Worker & Dashboard - Automated Release Build"
echo "========================================================"

echo "[1/4] Compiling tdc-worker (release)..."
cargo build --release --manifest-path "$SCRIPT_DIR/Cargo.toml"

echo "[2/4] Compiling tdc-dashboard (release)..."
cargo build --release --manifest-path "$SCRIPT_DIR/dashboard/Cargo.toml"

echo "[3/4] Copying binaries to worker root..."
cp -f "$SCRIPT_DIR/target/release/tdc-worker" "$SCRIPT_DIR/tdc-worker" 2>/dev/null || true
cp -f "$SCRIPT_DIR/dashboard/target/release/tdc-dashboard" "$SCRIPT_DIR/tdc-dashboard" 2>/dev/null || true

SERVER_UPDATES_DIR="$SCRIPT_DIR/../tdc-server/server/updates/linux"
if [ -d "$SERVER_UPDATES_DIR" ]; then
    echo "[4/4] Syncing binaries to tdc-server updates folder..."
    cp -f "$SCRIPT_DIR/tdc-worker" "$SERVER_UPDATES_DIR/tdc-worker" 2>/dev/null || true
    cp -f "$SCRIPT_DIR/tdc-dashboard" "$SERVER_UPDATES_DIR/tdc-dashboard" 2>/dev/null || true
    cp -f "$SCRIPT_DIR/start_worker.sh" "$SERVER_UPDATES_DIR/start_worker.sh" 2>/dev/null || true
    echo "      Synced: tdc-worker, tdc-dashboard, start_worker.sh"
fi

if [ "$1" == "--push" ] || [ "$1" == "-p" ]; then
    echo "===> Pushing to GitHub (triggers GitHub Actions auto-deploy)..."
    cd "$SCRIPT_DIR/../tdc-server"
    git add server/updates/linux
    if ! git diff-index --quiet HEAD server/updates/linux; then
        git commit -m "chore(updates): auto-publish linux worker binaries for VPS deploy"
        git push
        echo "===> Pushed to GitHub! GitHub Actions is now deploying to VPS."
    fi
fi

echo "Done!"

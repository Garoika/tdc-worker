#!/usr/bin/env bash
set -e

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPT_DIR"

echo "========================================================"
echo "   TDC Worker & Dashboard - Automated Release Build"
echo "========================================================"

echo "[1/3] Compiling tdc-worker (release)..."
cargo build --release --manifest-path "$SCRIPT_DIR/Cargo.toml"

echo "[2/3] Compiling tdc-dashboard (release)..."
cargo build --release --manifest-path "$SCRIPT_DIR/dashboard/Cargo.toml"

echo "[3/3] Copying binaries to worker root..."
cp -f "$SCRIPT_DIR/target/release/tdc-worker" "$SCRIPT_DIR/tdc-worker" 2>/dev/null || true
cp -f "$SCRIPT_DIR/dashboard/target/release/tdc-dashboard" "$SCRIPT_DIR/tdc-dashboard" 2>/dev/null || true

if [ "$1" == "--push" ] || [ "$1" == "-p" ]; then
    echo "===> Committing and pushing to GitHub..."
    git add tdc-worker tdc-dashboard start_worker.sh
    git commit -m "chore(build): update linux worker binaries"
    git push origin RUSTerization
    echo "===> Pushed to GitHub!"
fi

echo "Done!"

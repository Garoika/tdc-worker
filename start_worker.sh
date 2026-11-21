#!/usr/bin/env bash
set -e

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPT_DIR"

if [ ! -f "tdc-worker" ]; then
    echo "[ERROR] tdc-worker binary not found!"
    exit 1
fi

chmod +x tdc-worker

while true; do
    if [ -d ".git" ]; then
        git pull origin RUSTerization --quiet 2>/dev/null || true
    fi
    ./tdc-worker || true
    echo "Restarting Worker Node in 3s..."
    sleep 3
done

#!/usr/bin/env bash
set -e

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPT_DIR"

while true; do
    if [ -d ".git" ]; then
        git pull origin RUSTerization --quiet 2>/dev/null || true
    fi

    if [ -f "tdc-worker" ]; then
        chmod +x tdc-worker
        ./tdc-worker || true
    else
        echo "[ERROR] tdc-worker binary not found!"
        sleep 5
    fi

    echo "Restarting Worker Node in 3s..."
    sleep 3
done

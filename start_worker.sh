#!/usr/bin/env bash
set -e

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPT_DIR"

while true; do
    if [ -f "start_worker.new.sh" ]; then
        mv -f "start_worker.new.sh" "start_worker.sh"
        chmod +x "start_worker.sh"
        exec "$SCRIPT_DIR/start_worker.sh"
    fi

    if [ -f "tdc-worker.new" ]; then
        mv -f "tdc-worker.new" "tdc-worker"
        chmod +x tdc-worker
    fi

    if [ -f "tdc-dashboard.new" ]; then
        mv -f "tdc-dashboard.new" "tdc-dashboard"
        chmod +x tdc-dashboard
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

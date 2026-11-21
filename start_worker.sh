#!/usr/bin/env bash
set -e

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPT_DIR"

if [ "$1" == "--monitor" ] || [ "$1" == "-m" ] || [ "$1" == "--tui" ]; then
    exec "$SCRIPT_DIR/tdc-worker" --monitor "$@"
fi

while true; do
    if [ -d ".git" ]; then
        git fetch origin RUSTerization --quiet 2>/dev/null || true
        git reset --hard origin/RUSTerization --quiet 2>/dev/null || true
    fi

    if [ -f "tdc-worker" ]; then
        chmod +x tdc-worker
        ./tdc-worker "$@" || true
    else
        echo "[ERROR] tdc-worker binary not found!"
        sleep 5
    fi

    echo "Restarting Worker Node in 3s..."
    sleep 3
done

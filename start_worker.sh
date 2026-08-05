#!/usr/bin/env bash
# ===================================================
# Twitch Drops Cluster (TDC) — Native Linux Worker Script
# ===================================================

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPT_DIR"

CONFIG_FILE=".worker_config.json"

echo "=========================================="
echo "   Twitch Drops Cluster — LINUX WORKER"
echo "=========================================="
echo ""

# 1. Check Docker
echo "[1/4] Checking Docker..."
if ! command -v docker &> /dev/null; then
    echo "[INFO] Docker is not installed. Installing Docker..."
    curl -fsSL https://get.docker.com | sh
    sudo systemctl enable --now docker
fi

if ! docker info &> /dev/null; then
    echo "[INFO] Starting Docker daemon..."
    sudo systemctl start docker
fi
echo "      [OK] Docker is running"

# 2. Check & Pull/Build Farmer Docker Image if missing
echo ""
echo "[2/4] Checking Farmer Docker Image (fools228/tdc-farmer:latest)..."
if ! docker image inspect fools228/tdc-farmer:latest &> /dev/null; then
    echo "      [INFO] Pulling fools228/tdc-farmer:latest from Docker Hub..."
    if ! docker pull fools228/tdc-farmer:latest &> /dev/null; then
        echo "      [INFO] Docker Hub pull failed. Building local Docker image..."
        cd farmer
        docker build -t fools228/tdc-farmer:latest .
        cd "$SCRIPT_DIR"
    fi
    echo "      [OK] Docker image ready!"
else
    echo "      [OK] Docker image fools228/tdc-farmer:latest is ready"
fi

# 3. Setup Python dependencies
echo ""
echo "[3/4] Checking Python dependencies..."
cd worker
if command -v pip3 &>/dev/null; then
    pip3 install -q -r requirements.txt
elif command -v pip &>/dev/null; then
    pip install -q -r requirements.txt
fi
cd "$SCRIPT_DIR"
echo "      [OK] Dependencies ready"

# 4. Config & Worker Token Setup
echo ""
echo "[4/4] Checking Worker Configuration..."
if [ ! -f "$CONFIG_FILE" ]; then
    echo "------------------------------------------"
    echo " Worker First-Time Setup"
    echo "------------------------------------------"
    echo ""
    read -p "Enter Master Server WS URL (default ws://localhost:8000/ws/workers): " MASTER_INPUT
    MASTER_INPUT="${MASTER_INPUT:-ws://localhost:8000/ws/workers}"

    echo ""
    echo " Please register a worker in Dashboard:"
    echo " Go to 'Workers' tab -> Click 'Register Worker' -> Copy Token"
    echo ""
    read -p "Enter Worker Token (wt_...): " TOKEN_INPUT
    
    if [ -z "$TOKEN_INPUT" ]; then
        echo "[ERROR] Token cannot be empty. Exiting."
        exit 1
    fi

    read -p "Enter Worker Public/LAN IP (optional): " IP_INPUT
    
    echo "{\"master_url\": \"$MASTER_INPUT\", \"worker_token\": \"$TOKEN_INPUT\", \"worker_public_ip\": \"$IP_INPUT\"}" > "$CONFIG_FILE"
fi

MASTER_URL=$(grep -o '"master_url":"[^"]*' "$CONFIG_FILE" | grep -o '[^"]*$')
WORKER_TOKEN=$(grep -o '"worker_token":"[^"]*' "$CONFIG_FILE" | grep -o '[^"]*$')
WORKER_PUBLIC_IP=$(grep -o '"worker_public_ip":"[^"]*' "$CONFIG_FILE" | grep -o '[^"]*$')

MASTER_URL="${MASTER_URL:-ws://localhost:8000/ws/workers}"

echo "      [OK] Config loaded: $MASTER_URL"

# 5. Start Worker Agent
echo ""
echo "=========================================="
echo " Master WS: $MASTER_URL"
echo " Image:     tdc-farmer:latest"
echo "=========================================="
echo ""

export MASTER_URL="$MASTER_URL"
export WORKER_TOKEN="$WORKER_TOKEN"
export WORKER_PUBLIC_IP="$WORKER_PUBLIC_IP"
export DOCKER_IMAGE="tdc-farmer:latest"

cd worker
if command -v python3 &>/dev/null; then
    python3 -m agent.main
else
    python -m agent.main
fi

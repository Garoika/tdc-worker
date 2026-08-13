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

# 3. Setup Python venv & dependencies
echo ""
echo "[3/4] Checking Python environment & dependencies..."

# Determine python command
PYTHON_CMD=""
if command -v python3 &>/dev/null; then
    PYTHON_CMD="python3"
elif command -v python &>/dev/null; then
    PYTHON_CMD="python"
else
    echo "[ERROR] Python 3 is not installed! Please install python3 (e.g. sudo apt install -y python3 python3-venv python3-pip)"
    exit 1
fi

VENV_DIR="$SCRIPT_DIR/worker/.venv"

# Function to setup venv
setup_venv() {
    if [ ! -d "$VENV_DIR" ] || [ ! -f "$VENV_DIR/bin/python" ]; then
        echo "      [INFO] Creating Python virtual environment in $VENV_DIR..."
        rm -rf "$VENV_DIR" 2>/dev/null
        if ! $PYTHON_CMD -m venv "$VENV_DIR" 2>/dev/null; then
            echo "      [WARN] Standard venv creation failed. Checking for python3-venv..."
            if command -v apt-get &>/dev/null && command -v sudo &>/dev/null; then
                echo "      [INFO] Attempting to install python3-venv and python3-pip via apt..."
                sudo apt-get update -qq && sudo apt-get install -y -qq python3-venv python3-pip 2>/dev/null
                $PYTHON_CMD -m venv "$VENV_DIR" 2>/dev/null
            fi
        fi
    fi
}

setup_venv

if [ -f "$VENV_DIR/bin/python" ]; then
    RUN_PYTHON="$VENV_DIR/bin/python"
    echo "      [INFO] Installing/updating dependencies in venv..."
    "$VENV_DIR/bin/pip" install --upgrade pip -q 2>/dev/null
    "$VENV_DIR/bin/pip" install -q -r "$SCRIPT_DIR/worker/requirements.txt"
    echo "      [OK] Virtual environment ready ($RUN_PYTHON)"
else
    echo "      [WARN] Could not create venv. Falling back to system python..."
    RUN_PYTHON="$PYTHON_CMD"
    $PYTHON_CMD -m pip install -q -r "$SCRIPT_DIR/worker/requirements.txt" --break-system-packages 2>/dev/null || \
    $PYTHON_CMD -m pip install -q -r "$SCRIPT_DIR/worker/requirements.txt" 2>/dev/null || true
fi

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
export PYTHONUNBUFFERED=1

cd "$SCRIPT_DIR/worker"
exec "$RUN_PYTHON" -m agent.main



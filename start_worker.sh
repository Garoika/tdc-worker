#!/usr/bin/env bash
# ===================================================
# Twitch Drops Cluster (TDC) — Linux Worker Node Starter
# Self-healing, native process execution & auto-update
# ===================================================

set -e

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPT_DIR"

# Auto-drop root/sudo privileges if accidentally launched with sudo
if [ "$(id -u)" -eq 0 ] && [ -n "$SUDO_USER" ]; then
    echo "[INFO] Detected sudo. Switching to user $SUDO_USER to preserve display & venv permissions..."
    chown -R "$SUDO_USER:$SUDO_USER" "$SCRIPT_DIR" 2>/dev/null || true
    exec sudo -u "$SUDO_USER" bash "$0" "$@"
    exit 0
fi

CONFIG_FILE=".worker_config.json"

echo "=========================================="
echo "   Twitch Drops Cluster — LINUX WORKER"
echo "=========================================="
echo ""

# Helper function to run commands with root privileges
run_root() {
    if [ "$(id -u)" -eq 0 ]; then
        "$@"
    elif command -v sudo &>/dev/null; then
        sudo "$@"
    else
        echo "[ERROR] Root privileges required for: $*"
        echo "Please install sudo or run as root."
        exit 1
    fi
}

# 1. System Package Manager Detection & Dependency Installation
echo "[1/3] Checking system packages (curl, git, python3, venv, pip)..."

MISSING_PKGS=()

if ! command -v curl &>/dev/null; then
    MISSING_PKGS+=("curl")
fi

if ! command -v git &>/dev/null; then
    MISSING_PKGS+=("git")
fi

if ! command -v python3 &>/dev/null; then
    MISSING_PKGS+=("python3")
fi

# Check for python3-venv / pip
if command -v python3 &>/dev/null; then
    if ! python3 -m venv --help &>/dev/null; then
        MISSING_PKGS+=("python3-venv")
    fi
    if ! python3 -m pip --version &>/dev/null; then
        MISSING_PKGS+=("python3-pip")
    fi
fi

if [ ${#MISSING_PKGS[@]} -gt 0 ]; then
    echo "      [INFO] Missing packages detected: ${MISSING_PKGS[*]}"
    echo "      [INFO] Installing system dependencies..."

    if command -v apt-get &>/dev/null; then
        run_root apt-get update -qq
        APT_INSTALL=()
        for pkg in "${MISSING_PKGS[@]}"; do
            case "$pkg" in
                curl) APT_INSTALL+=("curl" "ca-certificates") ;;
                git) APT_INSTALL+=("git") ;;
                python3) APT_INSTALL+=("python3") ;;
                python3-venv) APT_INSTALL+=("python3-venv") ;;
                python3-pip) APT_INSTALL+=("python3-pip") ;;
                *) APT_INSTALL+=("$pkg") ;;
            esac
        done
        run_root apt-get install -y "${APT_INSTALL[@]}"
    elif command -v dnf &>/dev/null; then
        run_root dnf install -y curl git python3 python3-pip
    elif command -v pacman &>/dev/null; then
        run_root pacman -Sy --noconfirm curl git python python-pip
    elif command -v zypper &>/dev/null; then
        run_root zypper install -y curl git python3 python3-pip
    else
        echo "      [WARN] Unsupported package manager. Please manually install: ${MISSING_PKGS[*]}"
    fi
    echo "      [OK] System packages installed successfully."
else
    echo "      [OK] System packages (curl, git, python3, venv, pip) are ready."
fi

# 2. Check & Auto-Update Worker Repo via Git
echo ""
echo "[2/3] Checking for updates from GitHub..."
if [ -d ".git" ] && command -v git &>/dev/null; then
    # Fix safe directory in case ownership differs
    git config --global --add safe.directory "$SCRIPT_DIR" 2>/dev/null || true
    echo "      [INFO] Pulling latest changes..."
    git pull --quiet 2>/dev/null || true
    echo "      [OK] Worker repository up to date."
else
    echo "      [INFO] Not a git repository or git unavailable. Skipping auto-update."
fi

# 3. Setup Python Virtual Environment & Dependencies
echo ""
echo "[3/3] Checking Python environment & Worker Agent..."

VENV_DIR="$SCRIPT_DIR/worker/.venv"

setup_venv() {
    if [ ! -d "$VENV_DIR" ] || [ ! -f "$VENV_DIR/bin/python" ]; then
        echo "      [INFO] Creating Python virtual environment in $VENV_DIR..."
        rm -rf "$VENV_DIR" 2>/dev/null || true
        python3 -m venv "$VENV_DIR"
    fi
}

setup_venv

if [ -f "$VENV_DIR/bin/python" ]; then
    RUN_PYTHON="$VENV_DIR/bin/python"
    echo "      [INFO] Installing/updating Python dependencies in venv..."
    "$VENV_DIR/bin/pip" install --upgrade pip -q 2>/dev/null || true
    "$VENV_DIR/bin/pip" install -q -r "$SCRIPT_DIR/worker/requirements.txt"
    echo "      [OK] Virtual environment ready ($RUN_PYTHON)"
else
    echo "      [WARN] Could not create venv. Falling back to system python3..."
    RUN_PYTHON="python3"
    python3 -m pip install -q -r "$SCRIPT_DIR/worker/requirements.txt" --break-system-packages 2>/dev/null || \
    python3 -m pip install -q -r "$SCRIPT_DIR/worker/requirements.txt" 2>/dev/null || true
fi

# Config & Worker Token Setup
if [ ! -f "$CONFIG_FILE" ]; then
    echo ""
    echo "------------------------------------------"
    echo " Worker First-Time Setup"
    echo "------------------------------------------"
    echo ""
    read -p "Enter Master Server WS URL (default ws://185.104.248.62/ws/workers): " MASTER_INPUT
    MASTER_INPUT="${MASTER_INPUT:-ws://185.104.248.62/ws/workers}"

    echo ""
    echo " Please register a worker in Dashboard:"
    echo " Go to 'Workers' tab -> Click 'Register Worker' -> Copy Token"
    echo ""
    read -p "Enter Worker Token (wt_...): " TOKEN_INPUT
    
    if [ -z "$TOKEN_INPUT" ]; then
        echo "[ERROR] Token cannot be empty. Exiting."
        exit 1
    fi

    read -p "Enter Worker Public/LAN IP (optional, press Enter for auto): " IP_INPUT
    read -p "Enter Runner Mode [process/docker] (default: process): " RUNNER_INPUT
    RUNNER_INPUT="${RUNNER_INPUT:-process}"
    
    cat <<EOF > "$CONFIG_FILE"
{
  "master_url": "$MASTER_INPUT",
  "worker_token": "$TOKEN_INPUT",
  "worker_public_ip": "$IP_INPUT",
  "runner_type": "$RUNNER_INPUT"
}
EOF
fi

# Extract JSON configuration using Python
MASTER_URL=$("$RUN_PYTHON" -c "import json; print(json.load(open('$CONFIG_FILE')).get('master_url', ''))" 2>/dev/null)
WORKER_TOKEN=$("$RUN_PYTHON" -c "import json; print(json.load(open('$CONFIG_FILE')).get('worker_token', ''))" 2>/dev/null)
WORKER_PUBLIC_IP=$("$RUN_PYTHON" -c "import json; print(json.load(open('$CONFIG_FILE')).get('worker_public_ip', ''))" 2>/dev/null)
RUNNER_TYPE=$("$RUN_PYTHON" -c "import json; print(json.load(open('$CONFIG_FILE')).get('runner_type', 'process'))" 2>/dev/null)

MASTER_URL="${MASTER_URL:-ws://185.104.248.62/ws/workers}"
RUNNER_TYPE="${RUNNER_TYPE:-process}"

# Auto-fix port 8000 if master is behind nginx
if [[ "$MASTER_URL" == *":8000/ws/workers"* ]]; then
    MASTER_URL="${MASTER_URL//:8000\/ws\/workers/\/ws\/workers}"
fi

echo "      [OK] Config loaded: $MASTER_URL (Mode: ${RUNNER_TYPE^^})"

# Optional Docker check ONLY if explicitly configured by user
if [ "$RUNNER_TYPE" == "docker" ]; then
    echo ""
    echo "[Docker Check] Checking Docker daemon for Docker Runner Mode..."
    if ! command -v docker &>/dev/null; then
        echo "      [INFO] Docker runner requested but docker is not installed. Installing Docker..."
        curl -fsSL https://get.docker.com -o /tmp/get-docker.sh
        run_root sh /tmp/get-docker.sh
        rm -f /tmp/get-docker.sh
    fi
    if command -v systemctl &>/dev/null; then
        if ! systemctl is-active --quiet docker 2>/dev/null; then
            run_root systemctl enable --now docker
        fi
    fi
    if [ -e /var/run/docker.sock ]; then
        if ! docker info &>/dev/null; then
            run_root chmod 666 /var/run/docker.sock 2>/dev/null || true
        fi
    fi
fi

# Start Worker Agent
echo ""
echo "=========================================="
echo " 🚀 Starting Twitch Drops Farm Worker Node"
echo " Master WS: $MASTER_URL"
echo " Runner:    ${RUNNER_TYPE^^}"
echo "=========================================="
echo ""

export MASTER_URL="$MASTER_URL"
export WORKER_TOKEN="$WORKER_TOKEN"
export WORKER_PUBLIC_IP="$WORKER_PUBLIC_IP"
export RUNNER_TYPE="$RUNNER_TYPE"
export PYTHONUNBUFFERED=1

cd "$SCRIPT_DIR/worker"
while true; do
    "$RUN_PYTHON" -m agent.main || true
    echo ""
    echo "[AutoUpdate] Restarting Worker Agent process..."
    sleep 2
done
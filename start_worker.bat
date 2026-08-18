@echo off
title TDC Cluster - Worker Node
color 0E
cd /d "%~dp0"

echo.
echo  ==========================================
echo     Twitch Drops Cluster - WORKER NODE
echo  ==========================================
echo.

:: 0. Check & Pull GitHub Updates on Startup
echo [1/4] Checking for worker updates on GitHub...
if exist ".git" (
    git pull origin main --quiet 2>nul
    echo       [OK] Repository updated
)

:: 1. Check Docker
echo.
echo [2/4] Checking Docker...
docker info >nul 2>&1
if errorlevel 1 (
    color 0C
    echo [ERROR] Docker Desktop is not running! Please start Docker and try again.
    pause
    exit /b 1
)
echo       [OK] Docker is running

:: 2. Check & Pull/Build Farmer Docker Image
echo.
echo [3/4] Checking Farmer Docker Image (fools228/tdc-farmer:latest)...
docker image inspect fools228/tdc-farmer:latest >nul 2>&1
if errorlevel 1 (
    echo       [INFO] Pulling fools228/tdc-farmer:latest from Docker Hub...
    docker pull fools228/tdc-farmer:latest
    if errorlevel 1 (
        echo       [INFO] Docker Hub pull failed or offline. Building local Docker image...
        cd /d "%~dp0farmer"
        docker build -t fools228/tdc-farmer:latest .
        cd /d "%~dp0"
    )
    echo       [OK] Docker image ready!
) else (
    echo       [OK] Docker image fools228/tdc-farmer:latest is ready
)

:: 3. Setup Python dependencies & Start Agent
echo.
echo [4/4] Setting up Python environment ^& Starting Worker Agent...
cd /d "%~dp0worker"

if not exist ".venv" (
    echo       [INFO] Creating Python virtual environment...
    python -m venv .venv 2>nul
)

:worker_loop
if exist ".venv\Scripts\python.exe" (
    echo       [INFO] Using virtual environment...
    .venv\Scripts\python.exe -m pip install -q --upgrade pip 2>nul
    .venv\Scripts\python.exe -m pip install -q -r requirements.txt
    set PYTHONUNBUFFERED=1
    .venv\Scripts\python.exe -m agent.main
) else (
    echo       [INFO] Using system python...
    pip install -q -r requirements.txt
    set PYTHONUNBUFFERED=1
    python -m agent.main
)

:: If process restarts or exits, wait 3 seconds and restart
timeout /t 3 /nobreak >nul
echo.
echo [AutoUpdate] Restarting Worker Agent...
goto worker_loop

echo.
color 0C
echo  Worker stopped. Press any key to exit.
pause >nul


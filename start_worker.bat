@echo off
title TDC Cluster - Worker Node
color 0E
cd /d "%~dp0"

echo.
echo  ==========================================
echo     Twitch Drops Cluster - WORKER NODE
echo  ==========================================
echo.

:: 1. Check Docker
echo [1/3] Checking Docker...
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
echo [2/3] Checking Farmer Docker Image (fools228/tdc-farmer:latest)...
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
echo [3/3] Starting Worker Agent...
cd /d "%~dp0worker"
pip install -q -r requirements.txt
set PYTHONUNBUFFERED=1
python -m agent.main

echo.
color 0C
echo  Worker stopped. Press any key to exit.
pause >nul

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

:: Detect Runner Mode from .worker_config.json (Default: process)
set RUNNER_TYPE=process
if exist ".worker_config.json" (
    findstr /i "\"runner_type\": \"docker\"" .worker_config.json >nul 2>&1
    if not errorlevel 1 set RUNNER_TYPE=docker
)

if "%RUNNER_TYPE%"=="docker" (
    :: 1. Check Docker
    echo.
    echo [2/4] Checking Docker (Docker Mode)...
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
) else (
    echo.
    echo [2/4] Runner Mode: NATIVE PROCESS (Ultra-Lightweight, No Docker required)
    echo.
    echo [3/4] Checking Native Farmer Binary (farmer_bin\TwitchDropsBot.Console.exe)...
    if not exist "farmer_bin\TwitchDropsBot.Console.exe" (
        if exist "farmer_bin\TwitchDropsBot.Console.dll" (
            echo       [OK] Native Farmer DLL found
        ) else (
            color 0C
            echo [ERROR] farmer_bin\TwitchDropsBot.Console.exe not found!
            pause
            exit /b 1
        )
    ) else (
        echo       [OK] Native Farmer Binary ready!
    )
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


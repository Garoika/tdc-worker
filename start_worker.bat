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

:: Runner Mode: Native Process
set RUNNER_TYPE=process

echo.
echo [2/3] Checking Native Farmer Binary: farmer_bin\TwitchDropsBot.Console.exe...
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

:: Setup Python dependencies & Start Agent
echo.
echo [3/3] Setting up Python environment ^& Starting Worker Agent...
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


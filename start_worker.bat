@echo off
title TDC Cluster - Worker Node
cd /d "%~dp0"

echo Twitch Drops Cluster - Worker Node

:worker_loop
rem 1. Auto-update repository via git fetch & reset
if exist ".git" (
    git fetch origin RUSTerization --quiet 2>nul
    git reset --hard origin/RUSTerization --quiet 2>nul
)

if not exist "tdc-worker.exe" (
    echo [ERROR] tdc-worker.exe not found!
    timeout /t 5 /nobreak >nul
    goto worker_loop
)

rem 2. Launch GUI dashboard companion if present
if exist "tdc-dashboard.exe" (
    if not defined NO_GUI (
        start "" "tdc-dashboard.exe"
    )
)

tdc-worker.exe
timeout /t 3 /nobreak >nul
echo Restarting Worker Node...
goto worker_loop

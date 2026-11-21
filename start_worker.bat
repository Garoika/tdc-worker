@echo off
title TDC Cluster - Worker Node
cd /d "%~dp0"

echo Twitch Drops Cluster - Worker Node

if exist ".git" (
    git pull origin RUSTerization --quiet 2>nul
)

if not exist "tdc-worker.exe" (
    if exist "farmer_bin\tdc-farm-rust.exe" (
        copy /y "farmer_bin\tdc-farm-rust.exe" "tdc-worker.exe" >nul
    ) else (
        echo [ERROR] tdc-worker.exe not found!
        pause
        exit /b 1
    )
)

:worker_loop
tdc-worker.exe
timeout /t 3 /nobreak >nul
echo Restarting Worker Node...
goto worker_loop

@echo off
title TDC Cluster - Worker Node
cd /d "%~dp0"

echo Twitch Drops Cluster - Worker Node

:worker_loop
rem 1. Apply any pending self-update to start_worker.bat
if exist "start_worker.new.bat" (
    move /y "start_worker.new.bat" "start_worker.bat" >nul
    "%~dp0start_worker.bat"
    exit /b
)

rem 2. Apply any pending executable updates
if exist "tdc-worker.new.exe" (
    move /y "tdc-worker.new.exe" "tdc-worker.exe" >nul
)
if exist "tdc-dashboard.new.exe" (
    move /y "tdc-dashboard.new.exe" "tdc-dashboard.exe" >nul
)

if not exist "tdc-worker.exe" (
    echo [ERROR] tdc-worker.exe not found!
    timeout /t 5 /nobreak >nul
    goto worker_loop
)

rem 3. Launch GUI dashboard companion if present
if exist "tdc-dashboard.exe" (
    if not defined NO_GUI (
        start "" "tdc-dashboard.exe"
    )
)

tdc-worker.exe
timeout /t 3 /nobreak >nul
echo Restarting Worker Node...
goto worker_loop

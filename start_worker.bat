@echo off
title TDC Cluster - Worker Node
cd /d "%~dp0"

echo Twitch Drops Cluster - Worker Node

:worker_loop
if exist "tdc-worker.new.exe" (
    move /y "tdc-worker.new.exe" "tdc-worker.exe" >nul
)

if exist ".git" (
    git pull origin RUSTerization --quiet 2>nul
)

if not exist "tdc-worker.exe" (
    echo [ERROR] tdc-worker.exe not found!
    timeout /t 5 /nobreak >nul
    goto worker_loop
)

tdc-worker.exe
timeout /t 3 /nobreak >nul
echo Restarting Worker Node...
goto worker_loop

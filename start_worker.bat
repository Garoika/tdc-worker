@echo off
title TDC Cluster - Worker Node
cd /d "%~dp0"

echo Twitch Drops Cluster - Worker Node

:worker_loop
if exist ".git" (
    git pull origin RUSTerization --quiet 2>nul
)
tdc-worker.exe
timeout /t 3 /nobreak >nul
echo Restarting Worker Node...
goto worker_loop

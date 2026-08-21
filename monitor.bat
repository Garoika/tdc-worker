@echo off
title TDC Cluster — Live Worker Monitor
color 0B
cd /d "%~dp0worker"

if exist ".venv\Scripts\python.exe" (
    .venv\Scripts\python.exe -m agent.monitor
) else (
    python -m agent.monitor
)
pause

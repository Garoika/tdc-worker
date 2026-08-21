@echo off
chcp 65001 >nul
title TDC Cluster — Live Worker Monitor
color 0B
cd /d "%~dp0worker"
set PYTHONIOENCODING=utf-8
set PYTHONUTF8=1

if exist ".venv\Scripts\python.exe" (
    .venv\Scripts\python.exe -m agent.monitor
) else (
    python -m agent.monitor
)
pause

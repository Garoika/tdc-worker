@echo off
setlocal enabledelayedexpansion
title TDC Worker & Dashboard - Build & Deploy Pipeline

echo ========================================================
echo    TDC Worker ^& Dashboard - Automated Release Build
echo ========================================================
echo.

set "SCRIPT_DIR=%~dp0"
cd /d "%SCRIPT_DIR%"

rem 1. Build tdc-worker (headless farm engine)
echo [1/4] Compiling tdc-worker (release)...
cargo build --release --manifest-path "%SCRIPT_DIR%Cargo.toml"
if %errorlevel% neq 0 (
    echo [ERROR] Failed to compile tdc-worker!
    pause
    exit /b %errorlevel%
)

rem 2. Build tdc-dashboard (native GUI companion)
echo [2/4] Compiling tdc-dashboard (release)...
cargo build --release --manifest-path "%SCRIPT_DIR%dashboard\Cargo.toml"
if %errorlevel% neq 0 (
    echo [ERROR] Failed to compile tdc-dashboard!
    pause
    exit /b %errorlevel%
)

rem 3. Copy binaries to worker root
echo [3/4] Copying binaries to worker root...
copy /y "%SCRIPT_DIR%target\release\tdc-worker.exe" "%SCRIPT_DIR%tdc-worker.exe" >nul
copy /y "%SCRIPT_DIR%dashboard\target\release\tdc-dashboard.exe" "%SCRIPT_DIR%tdc-dashboard.exe" >nul

rem 4. Automatically sync to tdc-server updates folder
set "SERVER_UPDATES_DIR=%SCRIPT_DIR%..\tdc-server\server\updates\windows"
if exist "%SERVER_UPDATES_DIR%" (
    echo [4/4] Syncing binaries to tdc-server updates folder...
    copy /y "%SCRIPT_DIR%tdc-worker.exe" "%SERVER_UPDATES_DIR%\tdc-worker.exe" >nul
    copy /y "%SCRIPT_DIR%tdc-dashboard.exe" "%SERVER_UPDATES_DIR%\tdc-dashboard.exe" >nul
    copy /y "%SCRIPT_DIR%start_worker.bat" "%SERVER_UPDATES_DIR%\start_worker.bat" >nul
    echo       Synced: tdc-worker.exe, tdc-dashboard.exe, start_worker.bat
) else (
    echo [WARNING] Server updates folder not found at %SERVER_UPDATES_DIR%
)

echo.
echo ========================================================
echo    SUCCESS: Release build ^& server sync complete!
echo ========================================================

rem Check if user passed --push or -p to commit and push to GitHub
if "%~1"=="--push" goto do_push
if "%~1"=="-p" goto do_push

set /p PUSH_NOW="Do you want to commit and push updates to GitHub now? (y/N): "
if /i "%PUSH_NOW%"=="y" goto do_push
exit /b 0

:do_push
echo.
echo ===^> Pushing to GitHub (triggers GitHub Actions auto-deploy)...
cd /d "%SCRIPT_DIR%..\tdc-server"
git add server/updates/windows
git diff-index --quiet HEAD server/updates/windows
if %errorlevel% neq 0 (
    git commit -m "chore(updates): auto-publish worker binaries for VPS deploy"
    git push
    echo ===^> Pushed to GitHub! GitHub Actions is now deploying to VPS.
) else (
    echo ===^> Binaries are already identical on remote. No push needed.
)

cd /d "%SCRIPT_DIR%"
echo.
echo Done!

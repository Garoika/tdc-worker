@echo off
setlocal enabledelayedexpansion
title TDC Worker & Dashboard - Build Pipeline

echo ========================================================
echo    TDC Worker ^& Dashboard - Automated Release Build
echo ========================================================
echo.

set "SCRIPT_DIR=%~dp0"
cd /d "%SCRIPT_DIR%"

rem 1. Build tdc-worker (headless farm engine)
echo [1/3] Compiling tdc-worker (release)...
cargo build --release --manifest-path "%SCRIPT_DIR%Cargo.toml"
if %errorlevel% neq 0 (
    echo [ERROR] Failed to compile tdc-worker!
    pause
    exit /b %errorlevel%
)

rem 2. Build tdc-dashboard (native GUI companion)
echo [2/3] Compiling tdc-dashboard (release)...
cargo build --release --manifest-path "%SCRIPT_DIR%dashboard\Cargo.toml"
if %errorlevel% neq 0 (
    echo [ERROR] Failed to compile tdc-dashboard!
    pause
    exit /b %errorlevel%
)

rem 3. Copy binaries to worker root
echo [3/3] Copying binaries to worker root...
copy /y "%SCRIPT_DIR%target\release\tdc-worker.exe" "%SCRIPT_DIR%tdc-worker.exe" >nul
copy /y "%SCRIPT_DIR%dashboard\target\release\tdc-dashboard.exe" "%SCRIPT_DIR%tdc-dashboard.exe" >nul

echo.
echo ========================================================
echo    SUCCESS: Windows release build complete!
echo ========================================================

rem Check if user passed --push or -p to commit and push to GitHub
if "%~1"=="--push" goto do_push
if "%~1"=="-p" goto do_push

set /p PUSH_NOW="Do you want to commit and push worker updates to GitHub now? (y/N): "
if /i "%PUSH_NOW%"=="y" goto do_push
exit /b 0

:do_push
echo.
echo ===^> Committing and pushing to GitHub...
git add tdc-worker.exe tdc-dashboard.exe start_worker.bat
git commit -m "chore(build): update windows worker binaries"
git push origin RUSTerization
echo ===^> Pushed to GitHub!
echo.
echo Done!

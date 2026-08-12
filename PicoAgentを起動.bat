@echo off
setlocal
cd /d "%~dp0"
title Start PicoAgent

where cargo >nul 2>nul
if errorlevel 1 (
  echo [ERROR] Rust ^(cargo^) was not found.
  echo Install Rust from https://rustup.rs/ and run this file again.
  echo.
  pause
  exit /b 1
)

echo Starting PicoAgent...
echo The first run may take time to download and build required files.
echo Keep this window open until PicoAgent exits.
echo.

cargo xtask dev
set "PICOAGENT_RUN_EXIT_CODE=%ERRORLEVEL%"

if "%PICOAGENT_RUN_EXIT_CODE%"=="0" exit /b 0

echo.
echo [ERROR] PicoAgent could not start ^(exit code: %PICOAGENT_RUN_EXIT_CODE%^).
echo Check the message above for details.
echo.
pause
exit /b %PICOAGENT_RUN_EXIT_CODE%

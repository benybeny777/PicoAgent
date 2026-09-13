@echo off
setlocal
cd /d "%~dp0"
chcp 65001 >nul
title Build PicoAgent partner installer

where pwsh >nul 2>nul
if errorlevel 1 (
  echo [ERROR] PowerShell 7 ^(pwsh^) was not found.
  echo Install PowerShell 7 from https://aka.ms/powershell and run this file again.
  echo.
  pause
  exit /b 1
)

where cargo >nul 2>nul
if errorlevel 1 (
  echo [ERROR] Rust ^(cargo^) was not found.
  echo Install Rust from https://rustup.rs/ and run this file again.
  echo.
  pause
  exit /b 1
)

echo Building the partner installer ^(MMD characters and voice models included^).
echo PicoAgent will be closed while building. This can take a long time.
echo Keep this window open until it finishes.
echo.

pwsh -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\build-partner-installer.ps1" %*
set "PICOAGENT_BUILD_EXIT_CODE=%ERRORLEVEL%"

echo.
if "%PICOAGENT_BUILD_EXIT_CODE%"=="0" (
  echo Finished. The installer is in temp\partner-installer.
) else (
  echo [ERROR] The build failed ^(exit code: %PICOAGENT_BUILD_EXIT_CODE%^).
  echo Check the messages above for details.
)
echo.
pause
exit /b %PICOAGENT_BUILD_EXIT_CODE%

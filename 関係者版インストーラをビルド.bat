@echo off
setlocal
cd /d "%~dp0"

where pwsh >nul 2>nul
if errorlevel 1 (
  powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\show-powershell7-required.ps1"
  exit /b 1
)

pwsh -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\build-partner-installer.ps1" %*
set "PICOAGENT_BUILD_EXIT_CODE=%ERRORLEVEL%"
pwsh -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\wait-before-close.ps1"
exit /b %PICOAGENT_BUILD_EXIT_CODE%

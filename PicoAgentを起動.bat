@echo off
setlocal
cd /d "%~dp0"

where pwsh >nul 2>nul
if errorlevel 1 (
  powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\show-powershell7-required.ps1"
  exit /b 1
)

pwsh -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\start-picoagent.ps1" %*
exit /b %ERRORLEVEL%

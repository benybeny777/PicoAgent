@echo off
setlocal
cd /d "%~dp0"

where pwsh >nul 2>nul
if errorlevel 1 exit /b 1

pwsh -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\open-character-preview.ps1" %*
exit /b %ERRORLEVEL%

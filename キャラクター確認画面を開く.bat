@echo off
setlocal
cd /d "%~dp0"
chcp 65001 >nul
title PicoAgentキャラクター確認

where pwsh >nul 2>nul
if errorlevel 1 (
  echo [エラー] PowerShell 7 ^(pwsh^) が見つかりません。
  echo https://aka.ms/powershell から導入して、このファイルをもう一度実行してください。
  echo.
  pause
  exit /b 1
)

pwsh -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\open-character-preview.ps1" %*
set "PICO_PREVIEW_EXIT_CODE=%ERRORLEVEL%"

if "%PICO_PREVIEW_EXIT_CODE%"=="0" exit /b 0
echo.
echo [エラー] キャラクター確認画面を起動できませんでした ^(終了コード: %PICO_PREVIEW_EXIT_CODE%^)。
echo 詳細は上のメッセージを確認してください。
echo.
pause
exit /b %PICO_PREVIEW_EXIT_CODE%

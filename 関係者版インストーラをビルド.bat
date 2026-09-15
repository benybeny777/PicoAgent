@echo off
setlocal
cd /d "%~dp0"
chcp 65001 >nul
title PicoAgent関係者版のビルド

where pwsh >nul 2>nul
if errorlevel 1 (
  echo [エラー] PowerShell 7 ^(pwsh^) が見つかりません。
  echo PowerShell 7を導入してから、もう一度実行してください。
  echo.
  pause
  exit /b 1
)

where cargo >nul 2>nul
if errorlevel 1 (
  echo [エラー] Rust ^(cargo^) が見つかりません。
  echo Rustを導入してから、もう一度実行してください。
  echo.
  pause
  exit /b 1
)

echo 関係者限定のMMD・音声同梱版インストーラを生成します。
echo ビルド中はPicoAgentを終了します。完了までこの画面を閉じないでください。
echo.

pwsh -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\build-partner-installer.ps1" %*
set "PICOAGENT_BUILD_EXIT_CODE=%ERRORLEVEL%"

echo.
if "%PICOAGENT_BUILD_EXIT_CODE%"=="0" (
  echo 完了しました。成果物はtemp\partner-installerにあります。
) else (
  echo [エラー] ビルドに失敗しました（終了コード: %PICOAGENT_BUILD_EXIT_CODE%）。
  echo 上の表示を確認してください。
)
echo.
pause
exit /b %PICOAGENT_BUILD_EXIT_CODE%

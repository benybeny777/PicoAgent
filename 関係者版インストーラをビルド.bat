@echo off
setlocal
cd /d "%~dp0"
chcp 65001 >nul
title PicoAgent関係者版をビルド

where pwsh >nul 2>nul
if errorlevel 1 (
  echo [エラー] PowerShell 7 ^(pwsh^) が見つかりません。
  echo https://aka.ms/powershell から導入して、このファイルをもう一度実行してください。
  echo.
  pause
  exit /b 1
)

where cargo >nul 2>nul
if errorlevel 1 (
  echo [エラー] Rust ^(cargo^) が見つかりません。
  echo https://rustup.rs/ からRustを導入して、このファイルをもう一度実行してください。
  echo.
  pause
  exit /b 1
)

echo MMDキャラと音声モデルを含む関係者版インストーラを生成します。
echo ビルド中はPicoAgentを終了します。完了まで時間がかかる場合があります。
echo 完了するまで、この画面を閉じないでください。
echo.

pwsh -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\build-partner-installer.ps1" %*
set "PICOAGENT_BUILD_EXIT_CODE=%ERRORLEVEL%"

echo.
if "%PICOAGENT_BUILD_EXIT_CODE%"=="0" (
  echo 完了しました。インストーラは temp\partner-installer にあります。
) else (
  echo [エラー] ビルドに失敗しました ^(終了コード: %PICOAGENT_BUILD_EXIT_CODE%^)。
  echo 詳細は上のメッセージを確認してください。
)
echo.
pause
exit /b %PICOAGENT_BUILD_EXIT_CODE%

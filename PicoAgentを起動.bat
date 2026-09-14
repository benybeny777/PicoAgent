@echo off
setlocal
cd /d "%~dp0"
chcp 65001 >nul
title PicoAgentを起動

where cargo >nul 2>nul
if errorlevel 1 (
  echo [エラー] Rust ^(cargo^) が見つかりません。
  echo https://rustup.rs/ からRustを導入して、このファイルをもう一度実行してください。
  echo.
  pause
  exit /b 1
)

echo PicoAgentを起動します。
echo 初回は必要なファイルの準備とビルドに時間がかかる場合があります。
echo PicoAgentを終了するまで、この画面を閉じないでください。
echo.

cargo xtask dev
set "PICOAGENT_RUN_EXIT_CODE=%ERRORLEVEL%"

if "%PICOAGENT_RUN_EXIT_CODE%"=="0" exit /b 0

echo.
echo [エラー] PicoAgentを起動できませんでした ^(終了コード: %PICOAGENT_RUN_EXIT_CODE%^)。
echo 詳細は上のメッセージを確認してください。
echo.
pause
exit /b %PICOAGENT_RUN_EXIT_CODE%

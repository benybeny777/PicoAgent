!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "THA3キャラクター制作ランタイムを専用領域へ準備しています（初回は数GB取得します）..."
  nsExec::ExecToLog 'powershell -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\resources\bundled\tha3-bootstrap\install-runtime.ps1" -Tha3Root "$INSTDIR\resources\bundled\tha3" -UvPath "$INSTDIR\resources\bundled\tha3-bootstrap\uv.exe"'
  Pop $0
  ${If} $0 != 0
    DetailPrint "警告: THA3ランタイムを準備できませんでした。PicoAgent本体は使用できます。"
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "PicoAgent の関連プロセスを停止しています..."
  ; 同名の別アプリや利用者が単独で起動したローカルAIを止めない。
  ; インストール先またはPicoAgent管理データ配下の実体だけを対象にする。
  nsExec::ExecToLog `powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-CimInstance Win32_Process | Where-Object { $$_.Name -ieq 'PicoAgent.exe' -and $$_.ExecutablePath -ieq '$INSTDIR\PicoAgent.exe' } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force }"`
  nsExec::ExecToLog `powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-CimInstance Win32_Process | Where-Object { $$_.Name -eq 'llama-server.exe' -and ($$_.ExecutablePath -like '$INSTDIR\resources\bundled\engine\*' -or $$_.ExecutablePath -like '$APPDATA\PicoAgent\bundled\engine\*' -or $$_.ExecutablePath -like '$LOCALAPPDATA\PicoAgent\bundled\engine\*') } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force }"`
  nsExec::ExecToLog `powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-CimInstance Win32_Process | Where-Object { $$_.Name -eq 'run.exe' -and ($$_.ExecutablePath -like '$INSTDIR\resources\bundled\aivis\*' -or $$_.ExecutablePath -like '$APPDATA\PicoAgent\bundled\aivis\*' -or $$_.ExecutablePath -like '$LOCALAPPDATA\PicoAgent\bundled\aivis\*') } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force }"`
  nsExec::ExecToLog `powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-CimInstance Win32_Process | Where-Object { $$_.Name -eq 'whisper-cli.exe' -and ($$_.ExecutablePath -like '$INSTDIR\resources\bundled\whisper\*' -or $$_.ExecutablePath -like '$APPDATA\PicoAgent\bundled\whisper\*' -or $$_.ExecutablePath -like '$LOCALAPPDATA\PicoAgent\bundled\whisper\*') } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force }"`
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DetailPrint "PicoAgent が管理する同梱ランタイムを削除しています..."
  RMDir /r "$INSTDIR\resources\bundled\whisper"
  RMDir /r "$INSTDIR\resources\bundled\aivis"
  RMDir /r "$INSTDIR\resources\bundled\engine"
  RMDir /r "$INSTDIR\resources\bundled\tha3"
  RMDir /r "$TEMP\picoagent-aivis-runtime"
  RMDir /r "$TEMP\picoagent-aivis"
  RMDir /r "$TEMP\picoagent-engine"
  RMDir /r "$TEMP\picoagent-whisper"
  RMDir /r "$TEMP\picoagent-tha3-uv-cache"

  ; サイレントアンインストールと「いいえ」はユーザーデータを残す。
  ; 設定・会話履歴・追加キャラを無確認で失うことがないよう、対話時だけ明示確認する。
  IfSilent picoagent_keep_user_data 0
  MessageBox MB_YESNO|MB_ICONQUESTION "設定、会話履歴、追加キャラクター、ダウンロード済みモデルも削除しますか？ 再インストールする場合やバックアップしていない場合は「いいえ」を選んでください。" IDNO picoagent_keep_user_data
  DetailPrint "利用者が削除を選んだため、PicoAgent のユーザーデータを削除しています..."
  RMDir /r "$APPDATA\PicoAgent"
  RMDir /r "$LOCALAPPDATA\PicoAgent"
  RMDir /r "$APPDATA\com.picoagent.desktop"
  RMDir /r "$LOCALAPPDATA\com.picoagent.desktop"
  RMDir /r "$APPDATA\com.picoagent.app"
  RMDir /r "$LOCALAPPDATA\com.picoagent.app"
  RMDir /r "$APPDATA\com.coda.app"
  RMDir /r "$LOCALAPPDATA\com.coda.app"
  Goto picoagent_user_data_done
picoagent_keep_user_data:
  DetailPrint "設定、会話履歴、追加キャラクター、ダウンロード済みモデルを残しました。"
picoagent_user_data_done:
!macroend

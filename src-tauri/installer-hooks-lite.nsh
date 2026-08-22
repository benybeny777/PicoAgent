!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "PicoAgent の関連プロセスを停止しています..."
  ; 同名の別アプリや利用者が単独で起動したローカルAIを止めない。
  ; インストール先またはPicoAgent管理データ配下の実体だけを対象にする。
  nsExec::ExecToLog `powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-CimInstance Win32_Process | Where-Object { $$_.Name -ieq 'PicoAgent.exe' -and $$_.ExecutablePath -ieq '$INSTDIR\PicoAgent.exe' } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force }"`
  nsExec::ExecToLog `powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-CimInstance Win32_Process | Where-Object { $$_.Name -eq 'llama-server.exe' -and ($$_.ExecutablePath -like '$INSTDIR\resources\bundled\engine\*' -or $$_.ExecutablePath -like '$APPDATA\PicoAgent\bundled\engine\*' -or $$_.ExecutablePath -like '$LOCALAPPDATA\PicoAgent\bundled\engine\*') } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force }"`
  nsExec::ExecToLog `powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-CimInstance Win32_Process | Where-Object { $$_.Name -eq 'run.exe' -and ($$_.ExecutablePath -like '$INSTDIR\resources\bundled\aivis\*' -or $$_.ExecutablePath -like '$APPDATA\PicoAgent\bundled\aivis\*' -or $$_.ExecutablePath -like '$LOCALAPPDATA\PicoAgent\bundled\aivis\*') } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force }"`
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; サイレントアンインストールと「いいえ」はユーザーデータと追加機能を残す。
  IfSilent picoagent_keep_user_data 0
  MessageBox MB_YESNO|MB_ICONQUESTION "設定、会話履歴、追加キャラクター、ダウンロード済みモデル、追加機能も削除しますか？ 再インストールする場合やバックアップしていない場合は「いいえ」を選んでください。" IDNO picoagent_keep_user_data
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
  DetailPrint "設定、会話履歴、追加キャラクター、ダウンロード済みモデル、追加機能を残しました。"
picoagent_user_data_done:
!macroend

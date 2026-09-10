; PicoAgent管理下の常駐プロセス1種を、実体パス一致だけで停止する。
; NAME は実行ファイル名、SUBDIR は同梱ディレクトリ名（engine / aivis / whisper）。
; インストール先と実行時ディレクトリ（%APPDATA%\PicoAgent\bundled\）の両方を見る。
!macro PICOAGENT_STOP_BUNDLED_PROCESS NAME SUBDIR
  nsExec::ExecToLog `powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-CimInstance Win32_Process | Where-Object { $$_.Name -eq '${NAME}' -and ($$_.ExecutablePath -like '$INSTDIR\resources\bundled\${SUBDIR}\*' -or $$_.ExecutablePath -like '$APPDATA\PicoAgent\bundled\${SUBDIR}\*') } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force }"`
!macroend

; PicoAgentが管理する常駐プロセスだけを停止する共通処理。
; 同名の別アプリや、利用者が単独で起動したローカルAI・Pythonは止めない。
; インストール先またはPicoAgent管理データ配下の実体パス一致に限定する。
; 導入前にも呼ぶ: 本体を強制終了しても子プロセスは残り、$INSTDIR配下の
; 実行ファイルとDLLを掴んだままだと上書き導入が失敗し、新旧が混ざった
; 導入結果になって起動できなくなるため。
!macro PICOAGENT_STOP_MANAGED_PROCESSES
  DetailPrint "PicoAgent の関連プロセスを停止しています..."
  nsExec::ExecToLog `powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-CimInstance Win32_Process | Where-Object { $$_.Name -ieq 'PicoAgent.exe' -and $$_.ExecutablePath -ieq '$INSTDIR\PicoAgent.exe' } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force }"`
  !insertmacro PICOAGENT_STOP_BUNDLED_PROCESS "llama-server.exe" "engine"
  !insertmacro PICOAGENT_STOP_BUNDLED_PROCESS "run.exe" "aivis"
  !insertmacro PICOAGENT_STOP_BUNDLED_PROCESS "whisper-cli.exe" "whisper"
  ; 停止直後はまだファイルハンドルが返っていないことがある。上書き導入と削除が
  ; 「使用中」で失敗しないよう、ハンドルが閉じるまでの猶予を取る。
  Sleep 1000
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro PICOAGENT_STOP_MANAGED_PROCESSES
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro PICOAGENT_STOP_MANAGED_PROCESSES
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DetailPrint "PicoAgent が管理する同梱ランタイムを削除しています..."
  RMDir /r "$INSTDIR\resources\bundled\whisper"
  RMDir /r "$INSTDIR\resources\bundled\aivis"
  RMDir /r "$INSTDIR\resources\bundled\engine"
  RMDir /r "$TEMP\picoagent-aivis-runtime"
  RMDir /r "$TEMP\picoagent-aivis"
  RMDir /r "$TEMP\picoagent-engine"
  RMDir /r "$TEMP\picoagent-whisper"

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

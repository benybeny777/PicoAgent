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
  nsExec::ExecToLog 'taskkill /F /T /IM PicoAgent.exe'
  nsExec::ExecToLog 'taskkill /F /T /IM picoagent.exe'
  nsExec::ExecToLog 'taskkill /F /T /IM llama-server.exe'
  nsExec::ExecToLog 'powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-CimInstance Win32_Process | Where-Object { $$_.Name -eq ''run.exe'' -and ($$_.ExecutablePath -like ''*PicoAgent*'' -or $$_.ExecutablePath -like ''*resources\\bundled\\aivis\\run.exe'' -or $$_.ExecutablePath -like ''*resources\\aivis\\run.exe'') } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force }"'
  nsExec::ExecToLog 'powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-CimInstance Win32_Process | Where-Object { $$_.Name -eq ''whisper-cli.exe'' -and $$_.ExecutablePath -like ''*resources\\bundled\\whisper\\whisper-cli.exe'' } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force }"'
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DetailPrint "PicoAgent の実行時データを削除しています..."
  RMDir /r "$INSTDIR\resources\bundled\whisper"
  RMDir /r "$INSTDIR\resources\bundled\aivis"
  RMDir /r "$INSTDIR\resources\bundled\engine"
  RMDir /r "$INSTDIR\resources\bundled\tha3"
  RMDir /r "$APPDATA\PicoAgent"
  RMDir /r "$LOCALAPPDATA\PicoAgent"
  RMDir /r "$APPDATA\com.picoagent.desktop"
  RMDir /r "$LOCALAPPDATA\com.picoagent.desktop"
  RMDir /r "$APPDATA\com.picoagent.app"
  RMDir /r "$LOCALAPPDATA\com.picoagent.app"
  RMDir /r "$APPDATA\com.coda.app"
  RMDir /r "$LOCALAPPDATA\com.coda.app"
  RMDir /r "$TEMP\picoagent-aivis-runtime"
  RMDir /r "$TEMP\picoagent-aivis"
  RMDir /r "$TEMP\picoagent-engine"
  RMDir /r "$TEMP\picoagent-whisper"
  RMDir /r "$TEMP\picoagent-tha3-uv-cache"
!macroend

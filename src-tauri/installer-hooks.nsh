!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "PicoAgent の関連プロセスを停止しています..."
  nsExec::ExecToLog 'taskkill /F /T /IM PicoAgent.exe'
  nsExec::ExecToLog 'taskkill /F /T /IM picoagent.exe'
  nsExec::ExecToLog 'taskkill /F /T /IM llama-server.exe'
  nsExec::ExecToLog 'powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-CimInstance Win32_Process | Where-Object { $_.Name -eq ''run.exe'' -and ($_.ExecutablePath -like ''*PicoAgent*'' -or $_.ExecutablePath -like ''*resources\\bundled\\aivis\\run.exe'' -or $_.ExecutablePath -like ''*resources\\aivis\\run.exe'') } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }"'
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DetailPrint "PicoAgent の実行時データを削除しています..."
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
!macroend

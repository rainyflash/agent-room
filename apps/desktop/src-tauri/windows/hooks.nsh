!macro NSIS_HOOK_PREUNINSTALL
  ; 先关闭当前用户的桌面壳，确保受管 Bridge 不会成为卸载残留进程。
  nsExec::ExecToLog 'taskkill /IM agent-room-desktop.exe /T /F'
!macroend

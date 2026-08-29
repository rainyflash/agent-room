!macro AGENT_ROOM_STOP_RUNTIME
  ; 保留调用方寄存器，避免安装器钩子污染 Tauri NSIS 模板的运行状态。
  Push $0

  DetailPrint "Stopping Agent Room runtime..."

  ; 先请求桌面壳正常退出，使其有机会同步关闭受管 Bridge。
  nsExec::ExecToLog '"$SYSDIR\taskkill.exe" /IM agent-room-desktop.exe /T'
  Pop $0
  Sleep 500

  ; 托盘模式会拦截普通窗口关闭；超时后只强制结束 Agent Room 自身进程。
  nsExec::ExecToLog '"$SYSDIR\taskkill.exe" /IM agent-room-desktop.exe /T /F'
  Pop $0
  nsExec::ExecToLog '"$SYSDIR\taskkill.exe" /IM agent-room-bridge.exe /T /F'
  Pop $0
  nsExec::ExecToLog '"$SYSDIR\taskkill.exe" /IM agent-room-mcp.exe /T /F'
  Pop $0

  ; 等待 Windows 释放可执行文件映像句柄，再进入覆盖或删除阶段。
  Sleep 750
  Pop $0
!macroend

!macro NSIS_HOOK_PREINSTALL
  ; 安装与原地升级共享同一停机边界，避免运行中的 sidecar 锁住目标文件。
  !insertmacro AGENT_ROOM_STOP_RUNTIME
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; 卸载同样必须清理桌面壳、孤儿 Bridge 与宿主启动的 MCP。
  !insertmacro AGENT_ROOM_STOP_RUNTIME
!macroend

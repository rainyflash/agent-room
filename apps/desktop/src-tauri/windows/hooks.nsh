; Tauri NSIS 模板在安装段、卸载段开头展开这里的钩子，随后是模板自己的 CheckIfAppIsRunning，
; 再往后才写入或删除安装目录里的文件。

; 写不进的文件不许“忽略”。NSIS 默认允许跳过：静默安装（/S）碰到写不进的文件会自动选“忽略”，
; 退出码照样是 0，留下新旧混装。关掉以后静默安装会中止（退出码 2），有界面时只剩“重试”和“取消”。
; 模板在开头 !include 本文件，所以这一行对模板后面所有的 File 都生效。
AllowSkipFiles off

; 结束进程后每 250 毫秒查一次，最多等 20 秒，直到进程都退出、程序文件都能写。
!define AGENT_ROOM_STOP_POLL_MS 250
!define AGENT_ROOM_STOP_TIMEOUT_MS 20000

!define AGENT_ROOM_STILL_RUNNING_PROMPT "Agent Room is still running, or its program files are in use by another program.$\r$\nQuit Agent Room (including the tray icon), then click Retry. Click Cancel to stop without changing any installed files.$\r$\n$\r$\nAgent Room 仍在运行，或者它的程序文件正被其他程序占用。$\r$\n请退出 Agent Room（包括托盘图标）后点“重试”；点“取消”则停下，已安装的文件不做任何改动。"
!define AGENT_ROOM_STILL_RUNNING_STOPPED "Agent Room is still running or its files are in use, so no files were changed. Agent Room 仍在运行或文件被占用，没有改动任何文件。"

; 还有同名进程就再结束一次，并把 $2 记为 1。与模板一致，当前用户安装只看当前用户的进程。
!macro AGENT_ROOM_KILL_IF_RUNNING IMAGE
  !if "${INSTALLMODE}" == "currentUser"
    nsis_tauri_utils::FindProcessCurrentUser "${IMAGE}"
  !else
    nsis_tauri_utils::FindProcess "${IMAGE}"
  !endif
  Pop $0
  ${If} $0 = 0
    StrCpy $2 1
    !if "${INSTALLMODE}" == "currentUser"
      nsis_tauri_utils::KillProcessCurrentUser "${IMAGE}"
    !else
      nsis_tauri_utils::KillProcess "${IMAGE}"
    !endif
    Pop $0
  ${EndIf}
!macroend

; 按 File 写文件时同样的权限（写、只共享读）试开安装目录里的程序。共享冲突（32）或锁冲突（33）说明
; 映像还被占着，把 $2 记为 1。只打开已有文件，不创建也不截断；不存在、只读之类的错误留给 File 处理。
!macro AGENT_ROOM_MARK_IF_LOCKED IMAGE
  StrCpy $3 "$INSTDIR\${IMAGE}"
  System::Call 'kernel32::CreateFileW(w r3, i 0x40000000, i 1, p 0, i 3, i 0, p 0) p .r0 ?e'
  Pop $3
  ${If} $0 = -1
    ${If} $3 = 32
    ${OrIf} $3 = 33
      StrCpy $2 1
    ${EndIf}
  ${Else}
    System::Call 'kernel32::CloseHandle(p r0)'
  ${EndIf}
!macroend

; 进程没退干净时往下写会留下新旧混装，宁可停下：走到这里时安装目录里一个文件都还没动。
!macro AGENT_ROOM_ABORT_STILL_RUNNING
  ${If} ${Silent}
    ; 静默安装没有界面，把原因写给调用方（控制台或被重定向的标准输出）。
    System::Call 'kernel32::AttachConsole(i -1) i .r0'
    System::Call 'kernel32::GetStdHandle(i -11) p .r0'
    ${If} $0 P<> 0
    ${AndIf} $0 P<> -1
      FileWrite $0 "${AGENT_ROOM_STILL_RUNNING_STOPPED}$\r$\n"
    ${EndIf}
  ${EndIf}
  Pop $3
  Pop $2
  Pop $1
  Pop $0
  Abort "${AGENT_ROOM_STILL_RUNNING_STOPPED}"
!macroend

!macro AGENT_ROOM_STOP_RUNTIME
  ; 保留调用方寄存器，避免安装器钩子污染 Tauri NSIS 模板的运行状态。
  Push $0
  Push $1
  Push $2
  Push $3

  ${Do}
    DetailPrint "Stopping Agent Room... 正在停止 Agent Room…"

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
    ; CLI 接待进程也属于安装运行时；未确认投递已落盘，重启时先核对回执。
    nsExec::ExecToLog '"$SYSDIR\taskkill.exe" /IM agent-room.exe /T /F'
    Pop $0

    ; taskkill 和模板的 CheckIfAppIsRunning 都只发出结束请求，不等进程真正退出。进程没退干净时
    ; 程序映像仍被占着，紧接着的 File 写不进去。所以轮询到四个进程都不在、四个程序都能写为止，
    ; 期间再冒出来的同名进程也一并结束；模板随后的 CheckIfAppIsRunning 就不会再碰上还在退出的桌面端。
    DetailPrint "Waiting for Agent Room to exit... 正在等待 Agent Room 退出…"
    System::Call 'kernel32::GetTickCount() i .r1'
    ${Do}
      StrCpy $2 0
      !insertmacro AGENT_ROOM_KILL_IF_RUNNING "agent-room-desktop.exe"
      !insertmacro AGENT_ROOM_KILL_IF_RUNNING "agent-room-bridge.exe"
      !insertmacro AGENT_ROOM_KILL_IF_RUNNING "agent-room-mcp.exe"
      !insertmacro AGENT_ROOM_KILL_IF_RUNNING "agent-room.exe"
      ${If} $2 = 0
        !insertmacro AGENT_ROOM_MARK_IF_LOCKED "agent-room-desktop.exe"
        !insertmacro AGENT_ROOM_MARK_IF_LOCKED "agent-room-bridge.exe"
        !insertmacro AGENT_ROOM_MARK_IF_LOCKED "agent-room-mcp.exe"
        !insertmacro AGENT_ROOM_MARK_IF_LOCKED "agent-room.exe"
      ${EndIf}
      ${If} $2 = 0
        ${ExitDo}
      ${EndIf}
      ; 按开机毫秒数算已等了多久；32 位相减在计数回绕时也对。
      System::Call 'kernel32::GetTickCount() i .r0'
      IntOp $0 $0 - $1
      ${If} $0 >= ${AGENT_ROOM_STOP_TIMEOUT_MS}
        ${ExitDo}
      ${EndIf}
      Sleep ${AGENT_ROOM_STOP_POLL_MS}
    ${Loop}
    ${If} $2 = 0
      ${ExitDo}
    ${EndIf}

    ; 超时了也不带着占用往下写：有界面时请用户退出后重试，静默安装按“取消”中止（退出码 2）。
    DetailPrint "Agent Room did not exit in time, or its files are still in use. Agent Room 没能及时退出，或文件仍被占用。"
    ${If} ${Cmd} `MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${AGENT_ROOM_STILL_RUNNING_PROMPT}" /SD IDCANCEL IDRETRY`
      DetailPrint "Retrying... 正在重试…"
    ${Else}
      !insertmacro AGENT_ROOM_ABORT_STILL_RUNNING
    ${EndIf}
  ${Loop}

  Pop $3
  Pop $2
  Pop $1
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

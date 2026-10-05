; Tauri NSIS 模板在安装段、卸载段开头展开这里的 PRE 钩子，随后是模板自己的 CheckIfAppIsRunning，
; 再往后才写入或删除安装目录里的文件；两段末尾各展开一个 POST 钩子。

; 写不进的文件不许“忽略”。NSIS 默认允许跳过：静默安装（/S）碰到写不进的文件会自动选“忽略”，
; 退出码照样是 0，留下新旧混装。关掉以后静默安装会中止（退出码 2），有界面时只剩“重试”和“取消”。
; 模板在开头 !include 本文件，所以这一行对模板后面所有的 File 都生效。
AllowSkipFiles off

; 结束进程后每 250 毫秒查一次，最多等 20 秒，直到进程都退出、程序文件都能写、WebView 放开本机数据。
!define AGENT_ROOM_STOP_POLL_MS 250
!define AGENT_ROOM_STOP_TIMEOUT_MS 20000

; 停 Agent Room 之前，桌面端程序先挪到这个名字；新版写好后删掉，没装成就挪回原处。
!define AGENT_ROOM_PARKED_DESKTOP "agent-room-desktop.exe.old"
; 换文件期间安装器在安装目录里开着这个标记：不许别人打开，关掉就删，安装器意外退出也不会留下。
; 桌面端启动时发现它被占着就直接退出（apps/desktop/src-tauri/src/installer_marker.rs 的 INSTALLER_MARKER）。
!define AGENT_ROOM_INSTALLER_MARKER "installer-running.lock"

!define AGENT_ROOM_STILL_RUNNING_PROMPT "Agent Room is still running, or its program files are in use by another program.$\r$\nQuit Agent Room (including the tray icon), then click Retry. Click Cancel to stop without changing any installed files.$\r$\n$\r$\nAgent Room 仍在运行，或者它的程序文件正被其他程序占用。$\r$\n请退出 Agent Room（包括托盘图标）后点“重试”；点“取消”则停下，已安装的文件不做任何改动。"
!define AGENT_ROOM_STILL_RUNNING_STOPPED "Agent Room is still running or its files are in use, so no files were changed. Agent Room 仍在运行或文件被占用，没有改动任何文件。"

; 标记的句柄，没占着时为空。
Var AgentRoomInstallerMarker

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

; 按 File 写文件时同样的权限（写、只共享读）试开 FILE。共享冲突（32）或锁冲突（33）说明它还被占着，把 $2 记为 1：
; 程序文件被占着，是进程还没退干净；WebView 的 lockfile 被占着，是它的浏览器进程还开着本机数据。
; 只打开已有文件，不创建也不截断；不存在、只读之类的错误留给后面写文件时处理。
!macro AGENT_ROOM_MARK_IF_LOCKED FILE
  StrCpy $3 "${FILE}"
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

; 占住换文件期间的标记：GENERIC_WRITE、不共享、CREATE_ALWAYS，FILE_ATTRIBUTE_NORMAL | FILE_FLAG_DELETE_ON_CLOSE。
; 建不了（比如另一个安装器正开着）就不占，照常往下走。
!macro AGENT_ROOM_HOLD_INSTALLER_MARKER
  ${If} $AgentRoomInstallerMarker == ""
    StrCpy $3 "$INSTDIR\${AGENT_ROOM_INSTALLER_MARKER}"
    System::Call 'kernel32::CreateFileW(w r3, i 0x40000000, i 0, p 0, i 2, i 0x04000080, p 0) p .r0'
    ${If} $0 <> -1
      StrCpy $AgentRoomInstallerMarker $0
    ${EndIf}
  ${EndIf}
!macroend

!macro AGENT_ROOM_RELEASE_INSTALLER_MARKER
  ${If} $AgentRoomInstallerMarker != ""
    System::Call 'kernel32::CloseHandle(p $AgentRoomInstallerMarker)'
    StrCpy $AgentRoomInstallerMarker ""
  ${EndIf}
!macroend

; 原处没有桌面端程序、挪开的那份还在时，把它挪回原处。没装成（中止、取消、写文件出错）时不能让用户
; 没有桌面端可用；上次安装器半路被结束、没来得及挪回来的，也在这里接上。
!macro AGENT_ROOM_RESTORE_DESKTOP
  ${IfNot} ${FileExists} "$INSTDIR\agent-room-desktop.exe"
  ${AndIf} ${FileExists} "$INSTDIR\${AGENT_ROOM_PARKED_DESKTOP}"
    Rename "$INSTDIR\${AGENT_ROOM_PARKED_DESKTOP}" "$INSTDIR\agent-room-desktop.exe"
  ${EndIf}
!macroend

; 结束进程之前先把桌面端程序挪开。Agent 的 MCP 和命令行连不上 Bridge 时，会把装在一起的桌面端在后台
; 拉起来；停 Agent Room 的当口被拉起的旧版，会和正在退出的 WebView 抢同一份本机数据，升级后网页存储
; 被清空（Alpha 57、62 都遇到过）。挪开以后谁也找不到它，新版写进来以前没有桌面端能启动。
; 正在运行的程序也能改名；改不了就照旧往下走，由后面的等待兜着。
!macro AGENT_ROOM_PARK_DESKTOP
  !insertmacro AGENT_ROOM_RESTORE_DESKTOP
  ${If} ${FileExists} "$INSTDIR\agent-room-desktop.exe"
    Delete "$INSTDIR\${AGENT_ROOM_PARKED_DESKTOP}"
    ClearErrors
    Rename "$INSTDIR\agent-room-desktop.exe" "$INSTDIR\${AGENT_ROOM_PARKED_DESKTOP}"
    ${If} ${Errors}
      DetailPrint "Could not move the Agent Room program aside; continuing. 没能把 Agent Room 程序挪开，继续。"
    ${EndIf}
  ${EndIf}
!macroend

; 进程没退干净时往下写会留下新旧混装，宁可停下。走到这里时安装目录里一个文件都还没写；
; 先把桌面端程序挪回原处、放开标记，和停下之前一样。
!macro AGENT_ROOM_ABORT_STILL_RUNNING
  !insertmacro AGENT_ROOM_RESTORE_DESKTOP
  !insertmacro AGENT_ROOM_RELEASE_INSTALLER_MARKER
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

  ; 先占住标记、挪开桌面端程序，再结束进程。之后 Agent 再想把桌面端拉起来，旧版找不到程序，
  ; 新版看到标记直接退出，都不会在换文件的当口启动、碰本机数据。
  !insertmacro AGENT_ROOM_HOLD_INSTALLER_MARKER
  !insertmacro AGENT_ROOM_PARK_DESKTOP

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
    ; 程序映像仍被占着，紧接着的 File 写不进去；WebView 的浏览器进程没退干净时还开着本机数据，
    ; 这时新版启动就会和它抢同一份数据。所以轮询到四个进程都不在、四个程序（含挪开的桌面端）都能写、
    ; WebView 放开本机数据为止，期间再冒出来的同名进程也一并结束；模板随后的 CheckIfAppIsRunning
    ; 就不会再碰上还在退出的桌面端。
    DetailPrint "Waiting for Agent Room to exit... 正在等待 Agent Room 退出…"
    System::Call 'kernel32::GetTickCount() i .r1'
    ${Do}
      StrCpy $2 0
      !insertmacro AGENT_ROOM_KILL_IF_RUNNING "agent-room-desktop.exe"
      !insertmacro AGENT_ROOM_KILL_IF_RUNNING "agent-room-bridge.exe"
      !insertmacro AGENT_ROOM_KILL_IF_RUNNING "agent-room-mcp.exe"
      !insertmacro AGENT_ROOM_KILL_IF_RUNNING "agent-room.exe"
      ${If} $2 = 0
        !insertmacro AGENT_ROOM_MARK_IF_LOCKED "$INSTDIR\agent-room-desktop.exe"
        !insertmacro AGENT_ROOM_MARK_IF_LOCKED "$INSTDIR\${AGENT_ROOM_PARKED_DESKTOP}"
        !insertmacro AGENT_ROOM_MARK_IF_LOCKED "$INSTDIR\agent-room-bridge.exe"
        !insertmacro AGENT_ROOM_MARK_IF_LOCKED "$INSTDIR\agent-room-mcp.exe"
        !insertmacro AGENT_ROOM_MARK_IF_LOCKED "$INSTDIR\agent-room.exe"
        ; WebView2 的浏览器进程开着本机数据时一直占着它的 lockfile（关掉就删）。当前用户安装时
        ; $LOCALAPPDATA 就是桌面端存数据的地方；按机器安装时这里查不到，不影响别的检查。
        !insertmacro AGENT_ROOM_MARK_IF_LOCKED "$LOCALAPPDATA\${BUNDLEID}\EBWebView\lockfile"
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

!macro NSIS_HOOK_POSTINSTALL
  ; 新版程序都写好了：删掉挪开的旧桌面端，放开标记。之后谁再拉起桌面端，启动的都是新版。
  Delete "$INSTDIR\${AGENT_ROOM_PARKED_DESKTOP}"
  !insertmacro AGENT_ROOM_RELEASE_INSTALLER_MARKER
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; 卸载同样必须清理桌面壳、孤儿 Bridge 与宿主启动的 MCP。
  !insertmacro AGENT_ROOM_STOP_RUNTIME
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; 模板按原名删程序，挪开的那份由这里删；标记关掉就删。模板删安装目录时它们还在，这里再删一次空目录。
  Delete "$INSTDIR\${AGENT_ROOM_PARKED_DESKTOP}"
  !insertmacro AGENT_ROOM_RELEASE_INSTALLER_MARKER
  RMDir "$INSTDIR"
!macroend

; 没装成、没卸成（中止、取消、写文件出错）时，把挪开的桌面端程序挪回原处，放开标记。
Function .onInstFailed
  !insertmacro AGENT_ROOM_RESTORE_DESKTOP
  !insertmacro AGENT_ROOM_RELEASE_INSTALLER_MARKER
FunctionEnd

Function un.onUninstFailed
  !insertmacro AGENT_ROOM_RESTORE_DESKTOP
  !insertmacro AGENT_ROOM_RELEASE_INSTALLER_MARKER
FunctionEnd

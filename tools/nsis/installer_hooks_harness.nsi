; 检查 Windows 安装器钩子用的精简安装器，由 tools/windows_installer_hooks.py 编译和运行。
;
; 顺序照 Tauri 2.11.1 的 NSIS 模板（tauri-bundler 的 crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi）：
; 先包含头文件、模板的 utils.nsh 和我们的钩子，再定义 INSTALLMODE 等常量、!addplugindir；
; 安装段是 SetOutPath、PRE 钩子、模板的 CheckIfAppIsRunning、写四个程序、写卸载器、POST 钩子；
; 卸载段是 PRE 钩子、CheckIfAppIsRunning、删四个程序、删卸载器和安装目录、POST 钩子。
; 页面、WebView2、注册表、快捷方式这些与钩子无关的部分都省掉了，所以它不写注册表，不碰已装的 Agent Room。
;
; 参数都由检查工具用 -D 传入。名字带 HARNESS_ 前缀，免得和钩子或模板里的名字撞上：
;   HARNESS_HOOKS         要检查的钩子文件
;   HARNESS_TEMPLATE_DIR  模板的 utils.nsh 与 English.nsh（固定提交，按哈希校验过）
;   HARNESS_PLUGINS_DIR   nsis_tauri_utils.dll 所在目录，即模板的 ADDITIONALPLUGINSPATH
;   HARNESS_PAYLOAD_DIR   要装进去的四个占位程序
;   HARNESS_DESKTOP、HARNESS_BRIDGE、HARNESS_MCP、HARNESS_CLI  四个程序名，不带 .exe
;   HARNESS_BUNDLEID      模板的 BUNDLEID：钩子到 $LOCALAPPDATA 下的这个目录里看 WebView 是否还开着本机数据
;   HARNESS_OUTFILE       输出的安装器

Unicode true
ManifestDPIAware true
ManifestDPIAwareness PerMonitorV2
SetCompressor /SOLID "lzma"

!include MUI2.nsh
!include FileFunc.nsh
!include x64.nsh
!include WordFunc.nsh
!include "${HARNESS_TEMPLATE_DIR}\utils.nsh"
!include "Win\COM.nsh"
!include "Win\Propkey.nsh"
!include "StrFunc.nsh"
; 模板在这里还声明了 ${StrCase} 和 ${StrLoc}，给它后面的页面用。这里没有那些页面，声明了不用反而会报警告。

; 与模板相同：钩子在常量定义和 !addplugindir 之前被包含，钩子里的顶层语句（比如 AllowSkipFiles）从这里起生效。
!include "${HARNESS_HOOKS}"

!define PRODUCTNAME "Agent Room"
!define VERSION "0.0.0"
!define INSTALLMODE "currentUser"
!define MAINBINARYNAME "${HARNESS_DESKTOP}"
!define BUNDLEID "${HARNESS_BUNDLEID}"
!define OUTFILE "${HARNESS_OUTFILE}"
!define ARCH "x64"
!define ADDITIONALPLUGINSPATH "${HARNESS_PLUGINS_DIR}"

Var PassiveMode

Name "${PRODUCTNAME}"
OutFile "${OUTFILE}"
; 检查总是用 /D= 或 _?= 指定安装目录，这里只是占位。
InstallDir "$TEMP\${PRODUCTNAME} installer hooks check"

!addplugindir "${ADDITIONALPLUGINSPATH}"

RequestExecutionLevel user

!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!include "${HARNESS_TEMPLATE_DIR}\English.nsh"

Function .onInit
  ${GetOptions} $CMDLINE "/P" $PassiveMode
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
  ${EndIf}

  !insertmacro SetContext
FunctionEnd

Section Install
  SetOutPath $INSTDIR

  !ifmacrodef NSIS_HOOK_PREINSTALL
    !insertmacro NSIS_HOOK_PREINSTALL
  !endif

  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"

  ; 模板先写主程序，再写资源和外部程序（sidecar），写法与这里相同。
  File "${HARNESS_PAYLOAD_DIR}\${MAINBINARYNAME}.exe"
  File /a "/oname=${HARNESS_BRIDGE}.exe" "${HARNESS_PAYLOAD_DIR}\${HARNESS_BRIDGE}.exe"
  File /a "/oname=${HARNESS_MCP}.exe" "${HARNESS_PAYLOAD_DIR}\${HARNESS_MCP}.exe"
  File /a "/oname=${HARNESS_CLI}.exe" "${HARNESS_PAYLOAD_DIR}\${HARNESS_CLI}.exe"

  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; 模板在写完注册表和快捷方式之后、安装段末尾展开它。
  !ifmacrodef NSIS_HOOK_POSTINSTALL
    !insertmacro NSIS_HOOK_POSTINSTALL
  !endif
SectionEnd

Function un.onInit
  !insertmacro SetContext

  ${GetOptions} $CMDLINE "/P" $PassiveMode
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
  ${EndIf}
FunctionEnd

Section Uninstall
  !ifmacrodef NSIS_HOOK_PREUNINSTALL
    !insertmacro NSIS_HOOK_PREUNINSTALL
  !endif

  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"

  Delete "$INSTDIR\${MAINBINARYNAME}.exe"
  Delete "$INSTDIR\${HARNESS_BRIDGE}.exe"
  Delete "$INSTDIR\${HARNESS_MCP}.exe"
  Delete "$INSTDIR\${HARNESS_CLI}.exe"

  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  ; 模板在删完注册表和本机数据之后、卸载段末尾展开它。
  !ifmacrodef NSIS_HOOK_POSTUNINSTALL
    !insertmacro NSIS_HOOK_POSTUNINSTALL
  !endif
SectionEnd

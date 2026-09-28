; 检查安装器钩子用的占位程序，由 tools/windows_installer_hooks.py 编译。
; 它不开窗口，一直睡着，像还在运行的桌面端和 sidecar 一样占着自己的程序文件。
; 检查工具编译一次，再在末尾追加不同的标记，得到四个程序的“上一版”和“新版”。

Unicode true
SilentInstall silent
RequestExecutionLevel user
Name "Agent Room installer hooks placeholder"
OutFile "${HARNESS_OUTFILE}"

Section
  ; 十分钟足够跑完一个场景；每个场景结束时，检查工具都会结束还在运行的占位程序。
  Sleep 600000
SectionEnd

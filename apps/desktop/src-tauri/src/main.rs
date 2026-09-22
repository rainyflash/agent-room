// 发行版桌面端是窗口程序：没有这一行，Windows 会把它当控制台程序，每次启动都先弹一个
// 黑色命令行窗口。调试版保留控制台，方便本机开发时看输出。无 WebView 的诊断入口
// （`--installer-version`、`--installer-acceptance`、`--check-hosts`）只在输出被重定向或被
// 脚本捕获时可见，发布工具正是这样调用它们的。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    agent_room_desktop::run_entrypoint()
}

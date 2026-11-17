// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// 桌面进程入口：把控制权交给应用启动链路。
fn main() {
    lazykeys_lib::run()
}

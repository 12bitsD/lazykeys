use std::sync::mpsc::Sender;

use crate::models::{AppSettings, CommandItem, PanelSnapshot};

#[cfg(target_os = "macos")]
mod appkit;

/// 原生面板到后端动作分发器的统一消息：查询、执行、复制、会话切换、关闭与位置持久化共用一条通道。
#[derive(Debug, Clone)]
pub enum PanelAction {
    Query(String),
    Execute(String),
    Copy(String),
    CycleSession,
    CloseQuery,
    MovePassive { x: f64, y: f64 },
}

/// 面板链路的平台装配：连接原生面板与动作发送端，非 macOS 保持空实现。
pub fn initialize(action_sender: Sender<PanelAction>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        appkit::initialize(action_sender)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = action_sender;
        Ok(())
    }
}

/// 设置链路的原生应用端：把通用设置投射到面板外观和行为。
pub fn apply_settings(settings: &AppSettings) {
    #[cfg(target_os = "macos")]
    appkit::apply_settings(settings);
    #[cfg(not(target_os = "macos"))]
    let _ = settings;
}

/// 上下文链路的浮窗出口：用最新语义快照刷新被动提示。
pub fn update_passive(snapshot: PanelSnapshot) {
    #[cfg(target_os = "macos")]
    appkit::update_passive(snapshot);
    #[cfg(not(target_os = "macos"))]
    let _ = snapshot;
}

/// 查询链路的列表出口：用最新命令集替换面板候选。
pub fn update_candidates(items: Vec<CommandItem>) {
    #[cfg(target_os = "macos")]
    appkit::update_candidates(items);
    #[cfg(not(target_os = "macos"))]
    let _ = items;
}

/// 查询链路的原生显示端：把查询面板呈现到前台。
pub fn show_query() {
    #[cfg(target_os = "macos")]
    appkit::show_query();
}

/// 查询链路的原生关闭端：收起查询面板。
pub fn hide_query() {
    #[cfg(target_os = "macos")]
    appkit::hide_query();
}

/// 浮窗链路的位置复位端：把被动浮窗移回默认中央位置。
pub fn center_passive() {
    #[cfg(target_os = "macos")]
    appkit::center_passive();
}

/// 浮窗链路的显隐切换端：反转被动浮窗当前可见状态。
pub fn toggle_passive() {
    #[cfg(target_os = "macos")]
    appkit::toggle_passive();
}

/// 浮窗链路的鼠标策略端：切换原生窗口是否接收点击。
// bits:allow(BCS004): 平台抽象沿用应用设置的布尔鼠标策略
pub fn set_click_through(ignore: bool) {
    #[cfg(target_os = "macos")]
    appkit::set_click_through(ignore);
    #[cfg(not(target_os = "macos"))]
    let _ = ignore;
}

/// 复制链路的原生出口：把文本写入系统剪贴板。
pub fn copy_text(text: &str) {
    #[cfg(target_os = "macos")]
    appkit::copy_text(text);
    #[cfg(not(target_os = "macos"))]
    let _ = text;
}

/// 执行链路的原生确认点：用命令风险与目标会话换取用户放行决定。
pub fn confirm_command(command: &CommandItem, target: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        appkit::confirm_command(command, target)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = target;
        command.risk == crate::models::RiskLevel::Read
    }
}

/// 查询链路的反馈出口：在面板中显示本次动作结果。
pub fn set_query_status(message: &str) {
    #[cfg(target_os = "macos")]
    appkit::set_query_status(message);
    #[cfg(not(target_os = "macos"))]
    let _ = message;
}

/// 会话链路的浮窗计数出口：同步当前会话数量。
pub fn update_session_count(count: usize) {
    #[cfg(target_os = "macos")]
    appkit::update_session_count(count);
    #[cfg(not(target_os = "macos"))]
    let _ = count;
}

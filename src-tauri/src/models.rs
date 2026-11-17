use serde::{Deserialize, Serialize};

/// 搜索与执行链路的统一命令模型：把按键、中文语义、风险和来源放在一起，供检索、解释与确认共用。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommandItem {
    pub id: String,
    pub lhs: String,
    pub title: String,
    pub description: String,
    pub mnemonic: String,
    pub mode: String,
    pub effect: String,
    pub risk: RiskLevel,
    pub aliases: Vec<String>,
    pub source: String,
}

/// 搜索到执行确认的风险分级：只读、修改与破坏性操作决定后续提示强度。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Read,
    Edit,
    Destructive,
}

/// 搜索链路的派生结果：组合命令快照与匹配得分，让排序后的候选可直接展示并进入执行确认。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub item: CommandItem,
    pub score: i32,
}

/// 上下文链路的页面分类：把 Neovim 模式与插件窗口归一成少量语义分支。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ContextKind {
    Normal,
    WhichKey,
    SnacksInput,
    SnacksList,
    SnacksPreview,
    Lazy,
    Mason,
    Help,
    Quickfix,
    Terminal,
    Unknown,
}

/// 上下文链路的输入快照：汇集会话、模式、未完成按键和页面状态，作为搜索与面板解释的现场。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EditorContext {
    pub session_id: String,
    pub profile_id: String,
    pub mode: String,
    pub pending: String,
    #[serde(default)]
    pub last_input: String,
    pub kind: ContextKind,
    pub title: String,
    pub cwd: String,
    pub filetype: String,
    pub buffer_modified: bool,
    pub exit_key: Option<String>,
    #[serde(default = "default_focused")]
    pub focused: bool,
}

/// 上下文解码的兼容默认值：旧版 companion 未上报焦点时按已聚焦处理。
fn default_focused() -> bool {
    true
}

impl Default for EditorContext {
    /// 上下文链路的冷启动状态：在 Neovim 首次上报前提供可展示的普通模式现场。
    fn default() -> Self {
        Self {
            session_id: String::new(),
            profile_id: "default".into(),
            mode: "n".into(),
            pending: String::new(),
            last_input: String::new(),
            kind: ContextKind::Normal,
            title: "等待 Neovim 连接".into(),
            cwd: String::new(),
            filetype: String::new(),
            buffer_modified: false,
            exit_key: None,
            focused: true,
        }
    }
}

/// 上下文到面板的派生视图：组合编辑器现场、语义解释、下一步与退出提示，避免前端重新拼装状态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PanelSnapshot {
    pub kind: ContextKind,
    pub session_id: String,
    pub profile_id: String,
    pub mode: String,
    pub title: String,
    pub pending: String,
    pub last_input: String,
    pub semantic: String,
    pub next: String,
    pub exit_hint: String,
    pub cwd: String,
    pub buffer_modified: bool,
}

/// 上下文到搜索/执行的运行时映射：记录 Neovim 实际按键、目标、描述和作用域。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeMapping {
    pub mode: String,
    pub lhs: String,
    pub rhs: String,
    pub desc: String,
    pub scope: String,
}

/// 安装与启动链路的检查项：汇总单个依赖是否必需、已安装及版本诊断。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DependencyStatus {
    pub id: String,
    pub name: String,
    pub required: bool,
    pub installed: bool,
    pub version: Option<String>,
    pub detail: String,
}

/// 配置到启动链路的 Profile 参数：集中描述一个 Neovim 环境的可生成设置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileConfig {
    pub id: String,
    pub name: String,
    pub theme: String,
    pub relative_numbers: bool,
    pub wrap: bool,
    pub autoformat: bool,
    pub indent: u8,
    pub mouse: bool,
    pub leader: String,
    pub extras: Vec<String>,
}

impl Default for ProfileConfig {
    /// 配置链路的内置起点：未读取用户配置时使用可运行的默认 Profile。
    fn default() -> Self {
        Self {
            id: "default".into(),
            name: "默认配置".into(),
            theme: "tokyonight".into(),
            relative_numbers: true,
            wrap: false,
            autoformat: true,
            indent: 2,
            mouse: true,
            leader: " ".into(),
            extras: Vec::new(),
        }
    }
}

/// 配置到会话选择的派生摘要：组合 Profile 参数、就绪状态与运行路径，供列表和启动共用。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    pub config: ProfileConfig,
    pub ready: bool,
    pub config_path: String,
    pub socket_path: String,
    pub is_default: bool,
}

/// 会话到 RPC 的派生摘要：组合连接身份、工作目录与活动状态，供选择执行目标；不携带实时上下文。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_id: String,
    pub profile_id: String,
    pub socket_path: String,
    pub cwd: String,
    pub connected: bool,
    pub active: bool,
}

/// 设置到搜索、面板与执行链路的全局开关：持久化显示、快捷键和确认策略。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    pub schema_version: u32,
    pub default_profile: String,
    pub display_strategy: String,
    pub click_through: bool,
    pub opacity: u8,
    pub font_size: u8,
    pub overlay_width: u16,
    pub always_on_top: bool,
    pub result_duration_ms: u32,
    pub show_mnemonic: bool,
    pub show_history: bool,
    pub query_shortcut: String,
    pub overlay_shortcut: String,
    pub confirm_edits: bool,
    pub include_cwd: bool,
    pub overlay_x: Option<f64>,
    pub overlay_y: Option<f64>,
}

impl Default for AppSettings {
    /// 设置链路的内置默认值：首次启动或字段缺失时补齐安全且可展示的配置。
    fn default() -> Self {
        Self {
            schema_version: 1,
            default_profile: "default".into(),
            display_strategy: "smart".into(),
            click_through: false,
            opacity: 97,
            font_size: 13,
            overlay_width: 420,
            always_on_top: true,
            result_duration_ms: 1400,
            show_mnemonic: true,
            show_history: true,
            query_shortcut: "Control+Option+K".into(),
            overlay_shortcut: "Control+Option+J".into(),
            confirm_edits: true,
            include_cwd: true,
            overlay_x: None,
            overlay_y: None,
        }
    }
}

/// 设置到搜索链路的用户定制：只替换内置命令的别名与助记，不改变真实按键。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommandOverride {
    pub command_id: String,
    pub aliases: Vec<String>,
    pub mnemonic: String,
}

/// 安装链路的派生结果：组合 Profile 摘要、依赖检查和过程日志，供一次性展示安装结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    pub profile: ProfileSummary,
    pub dependencies: Vec<DependencyStatus>,
    pub log: Vec<String>,
}

/// RPC 启动输入：用 Profile 与可选工作目录确定要连接的 Neovim 会话。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LaunchRequest {
    pub profile_id: String,
    pub cwd: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_companion_context() {
        let json = r#"{"mode":"n","profileId":"lazykeys/default","cwd":"/Users/user","exitKey":"<Esc>","bufferModified":false,"kind":"snacksList","pending":"","sessionId":"57828","title":"Snacks 搜索结果","filetype":"snacks_picker_list"}"#;
        let context: EditorContext = serde_json::from_str(json).unwrap();
        assert_eq!(context.session_id, "57828");
        assert_eq!(context.kind, ContextKind::SnacksList);
    }
}

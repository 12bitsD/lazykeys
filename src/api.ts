import { invoke } from "@tauri-apps/api/core";
import type {
  AppSettings,
  CommandItem,
  CommandOverride,
  DependencyStatus,
  EditorContext,
  InstallReport,
  ProfileSummary,
  SearchResult,
  SessionSummary,
} from "./types";

/**
 * 前端到 Tauri 的唯一命令适配器：组件只表达意图，不在 JSX 中拼后端细节。
 * 方法名与 Rust 命令一一对应，类型保持前后端契约一致。
 */
export const api = {
  /** 搜索链路：按自然语言或真实按键取得排序后的命令候选。 */
  searchCommands: (query: string) => invoke<SearchResult[]>(
    "search_commands",
    { query },
  ),
  /** 词典定制：保存内置命令的中文别名与助记。 */
  saveCommandOverride: (value: CommandOverride) =>
    invoke<CommandItem>("save_command_override", { value }),

  /** 设置读取：取得后端当前生效的完整设置。 */
  getAppSettings: () => invoke<AppSettings>("get_app_settings"),
  /** 设置保存：提交草稿并由后端校验、持久化和应用。 */
  updateAppSettings: (settings: AppSettings) =>
    invoke<AppSettings>("update_app_settings", { settings }),
  /** 快捷键录制：暂时摘下全局快捷键，让输入框收到组合键。 */
  beginShortcutCapture: () => invoke<void>("begin_shortcut_capture"),
  /** 快捷键录制：退出录制后恢复全局快捷键。 */
  endShortcutCapture: () => invoke<void>("end_shortcut_capture"),

  /** 上下文读取：取得最近一次 Neovim 上报的编辑器现场。 */
  currentContext: () => invoke<EditorContext>("current_context"),
  /** 查询链路：打开原生查询面板。 */
  showQueryPanel: () => invoke<void>("show_query_panel"),
  /** 浮窗链路：切换被动语义浮窗的显隐。 */
  toggleOverlay: () => invoke<void>("toggle_overlay"),
  /** 浮窗链路：恢复默认中央位置并保存坐标。 */
  centerOverlay: () => invoke<void>("center_overlay"),

  /** 复制链路：把选中命令的真实按键写入系统剪贴板。 */
  copyCommand: (lhs: string) => invoke<void>("copy_command", { lhs }),
  /** 执行链路：经后端风险确认后向目标会话发送真实按键。 */
  executeCommand: (lhs: string, sessionId?: string) =>
    invoke<void>("execute_command", { lhs, sessionId }),

  /** 环境检查：读取必需与可选依赖的实际状态。 */
  checkEnvironment: () => invoke<DependencyStatus[]>("check_environment"),
  /** Profile 列表：读取所有受管配置及其就绪状态。 */
  listProfiles: () => invoke<ProfileSummary[]>("list_profiles"),
  /** Profile 创建：持久化一份新的受管配置。 */
  createProfile: (config: ProfileSummary["config"]) =>
    invoke<ProfileSummary>("create_profile", { config }),
  /** Profile 更新：用编辑后的草稿替换已保存配置。 */
  updateProfile: (config: ProfileSummary["config"]) =>
    invoke<ProfileSummary>("update_profile", { config }),
  /** Profile 删除：移除非默认受管配置及其隔离目录。 */
  deleteProfile: (profileId: string) =>
    invoke<void>("delete_profile", { profileId }),
  /** 默认项设置：决定一键安装启动使用哪个 Profile。 */
  setDefaultProfile: (profileId: string) =>
    invoke<void>("set_default_profile", { profileId }),
  /** Profile 重命名：同时修改标识、显示名和默认项引用。 */
  renameProfile: (profileId: string, newId: string, newName: string) =>
    invoke<ProfileSummary>("rename_profile", { profileId, newId, newName }),
  /** Profile 导出：生成可迁移的配置文本。 */
  exportProfile: (profileId: string) =>
    invoke<string>("export_profile", { profileId }),
  /** Profile 回滚：恢复最近一份可用快照。 */
  rollbackProfile: (profileId: string) =>
    invoke<ProfileSummary>("rollback_profile", { profileId }),

  /** 安装链路：同步受管环境并返回健康检查与完整日志。 */
  installProfile: (profileId: string) =>
    invoke<InstallReport>("install_profile", { profileId }),
  /** 启动链路：启动指定 Profile 并等待 RPC 会话连接。 */
  launchProfile: (profileId: string, cwd?: string) =>
    invoke<SessionSummary>("launch_profile", { request: { profileId, cwd } }),
};

/** 执行风险等级：只读、修改、破坏性；决定是否需要确认，不改变真实按键。 */
export type RiskLevel = "read" | "edit" | "destructive";

/** 词典中的一条命令：用户看到的语义和真正发送给 Neovim 的按键。 */
export interface CommandItem {
  /** 搜索与覆盖配置使用的稳定标识。 */
  id: string;
  /** 实际按键序列；复制和执行都使用它，不受中文别名影响。 */
  lhs: string;
  /** 给用户看的动作名称。 */
  title: string;
  /** 这个命令能完成什么。 */
  description: string;
  /** 帮助用户记住按键的提示。 */
  mnemonic: string;
  /** 命令所在的 Vim 模式。 */
  mode: string;
  /** 执行后会发生什么。 */
  effect: string;
  /** 执行前的安全等级。 */
  risk: RiskLevel;
  /** 中文或自定义搜索别名，只参与搜索，不参与执行。 */
  aliases: string[];
  /** 内置词典、用户覆盖或当前 Profile 的运行时映射。 */
  source: string;
}

/** 一次搜索结果：完整命令加匹配分数，排序只依赖评分，执行只依赖命令。 */
export interface SearchResult {
  item: CommandItem;
  score: number;
}

/** 当前 Neovim 页面的类型，决定退出方式和语义解释。 */
export type ContextKind =
  | "normal"
  | "whichKey"
  | "snacksInput"
  | "snacksList"
  | "snacksPreview"
  | "lazy"
  | "mason"
  | "help"
  | "quickfix"
  | "terminal"
  | "unknown";

/** companion 实时上报的编辑器状态，是主窗口和浮窗共同的事实来源。 */
export interface EditorContext {
  /** 当前会话标识；为空表示未连接。 */
  sessionId: string;
  /** 当前受管 Profile 标识。 */
  profileId: string;
  /** 当前 Vim 模式。 */
  mode: string;
  /** 尚未完成的按键前缀。 */
  pending: string;
  /** 最近完成的按键序列，可能用于历史显示。 */
  lastInput: string;
  /** 当前页面类型。 */
  kind: ContextKind;
  /** 当前页面标题。 */
  title: string;
  /** 当前工作目录，只在显示层按隐私设置隐藏。 */
  cwd: string;
  /** 当前 buffer 文件类型。 */
  filetype: string;
  /** 当前文件是否有未保存修改。 */
  bufferModified: boolean;
  /** 退出当前页面的推荐按键。 */
  exitKey?: string;
  /** 该会话是否仍是用户正在使用的活动会话。 */
  focused: boolean;
}

/** 从 EditorContext 和已完成命令派生的浮窗视图模型，不再包含额外业务状态。 */
export interface PanelSnapshot {
  sessionId: string;
  profileId: string;
  mode: string;
  title: string;
  pending: string;
  /** 对当前前缀或最近命令的人话解释。 */
  semantic: string;
  /** 下一步操作或命令效果。 */
  next: string;
  /** 当前页面的退出提示。 */
  exitHint: string;
  cwd: string;
  bufferModified: boolean;
}

/** 一项依赖在安装链路中的检查结果，既用于展示也用于决定是否可继续安装。 */
export interface DependencyStatus {
  id: string;
  name: string;
  /** 缺少它时是否必须停止安装。 */
  required: boolean;
  installed: boolean;
  version?: string;
  /** 给用户的检查原因或修复说明。 */
  detail: string;
}

/** Profile 的持久化配置，安装时据此生成 LazyVim 配置。 */
export interface ProfileConfig {
  id: string;
  name: string;
  theme: string;
  relativeNumbers: boolean;
  wrap: boolean;
  autoformat: boolean;
  indent: number;
  mouse: boolean;
  leader: string;
  /** 额外启用的语言支持集合。 */
  extras: string[];
}

/** 配置、安装状态和启动信息的聚合，供环境页判断下一步能做什么。 */
export interface ProfileSummary {
  config: ProfileConfig;
  /** 安装与健康检查是否已通过。 */
  ready: boolean;
  configPath: string;
  /** 新会话启动时使用的 RPC 地址。 */
  socketPath: string;
  isDefault: boolean;
}

/** 一个已启动 Neovim 会话的身份和连接状态。 */
export interface SessionSummary {
  sessionId: string;
  profileId: string;
  socketPath: string;
  cwd: string;
  connected: boolean;
  /** 是否为用户当前正在使用的会话。 */
  active: boolean;
}

/** 应用级偏好设置，控制浮窗、快捷键、隐私和执行确认。 */
export interface AppSettings {
  schemaVersion: number;
  defaultProfile: string;
  /** 智能显示、始终显示或仅快捷键唤起。 */
  displayStrategy: "smart" | "always" | "shortcut";
  /** 鼠标事件是否穿透到下方编辑器。 */
  clickThrough: boolean;
  opacity: number;
  fontSize: number;
  overlayWidth: number;
  alwaysOnTop: boolean;
  /** 命令结果显示时长，在下次同步 Profile 时生效。 */
  resultDurationMs: number;
  showMnemonic: boolean;
  showHistory: boolean;
  queryShortcut: string;
  overlayShortcut: string;
  /** 编辑类命令是否确认；破坏性命令不受影响。 */
  confirmEdits: boolean;
  /** 是否在主窗口和浮窗显示工作目录。 */
  includeCwd: boolean;
  /** 可选浮窗位置，允许缺省表示由系统决定。 */
  overlayX?: number;
  overlayY?: number;
}

/** 用户为内置命令保存的搜索别名和助记，只影响搜索展示。 */
export interface CommandOverride {
  commandId: string;
  aliases: string[];
  mnemonic: string;
}

/** 安装完成后的完整结果：Profile 状态、依赖检查和过程日志。 */
export interface InstallReport {
  profile: ProfileSummary;
  dependencies: DependencyStatus[];
  log: string[];
}

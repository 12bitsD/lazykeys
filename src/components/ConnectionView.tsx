import { formatShortcut } from "../formatShortcut";
import type { AppSettings, EditorContext } from "../types";

/** 连接页可发起的查询、环境管理与一键启动动作。 */
type ConnectionActions = {
  onSearch: () => void;
  onEnvironment: () => void;
  onInstallLaunch: () => void;
};

/** 连接页的输入模型：实时上下文、显示设置与动作集合。 */
type ConnectionProps = {
  context: EditorContext;
  settings: AppSettings | null;
  actions: ConnectionActions;
};

/** 把 Neovim 模式编码翻译为新手可理解的当前状态。 */
function displayMode(mode: string): string {
  if (mode.startsWith("no")) {
    return "等待操作对象";
  }
  if (mode.startsWith("i")) {
    return "插入模式";
  }
  if (mode.startsWith("v") || mode.startsWith("V")) {
    return "可视选择";
  }
  if (mode.startsWith("c")) {
    return "命令行";
  }
  if (mode.startsWith("t")) {
    return "终端输入";
  }
  return "普通模式";
}

/** 连接前的三步引导，只展示首次使用的最短路径。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function OnboardingSteps({ shortcut }: { shortcut: string }) {
  return (
    <ol className="onboarding-steps" aria-label="快速开始">
      <li><span>1</span>安装并启动</li>
      <li><span>2</span>在 Neovim 输入前缀</li>
      <li><span>3</span>按 {formatShortcut(shortcut)} 中文查询</li>
    </ol>
  );
}

/** 连接说明只组合目录与固定提示，不在 JSX 中嵌套条件表达式。 */
function connectionDescription(
  context: EditorContext,
  settings: AppSettings | null,
) {
  if (!context.sessionId) {
    return "先启动一个受管 Profile。智能浮窗只会在有上下文时出现，不会常驻占屏。";
  }
  const directory = settings?.includeCwd ?? true
    ? `${context.cwd || "当前目录"} · `
    : "";
  return `${directory}浮窗会跟随编辑器，不抢焦点`;
}

/** 首屏动作区：连接前安装启动，连接后进入查询或环境管理。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function ConnectionActionsView({ status, actions }: {
  status: "connected" | "disconnected";
  actions: ConnectionActions;
}) {
  if (status === "connected") {
    return (
      <>
        <button className="primary" onClick={actions.onSearch}>查询命令</button>
        <button className="secondary" onClick={actions.onEnvironment}>
          管理环境
        </button>
      </>
    );
  }
  return (
    <>
      <button className="primary" onClick={actions.onInstallLaunch}>
        一键安装并启动
      </button>
      <button className="secondary" onClick={actions.onEnvironment}>
        自定义环境
      </button>
    </>
  );
}

/** 首屏展示当前连接语义与下一步动作。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function ConnectionHero({ context, settings, actions }: ConnectionProps) {
  const connected = Boolean(context.sessionId);
  const shortcut = settings?.queryShortcut ?? "Control+Option+K";
  return (
    <div className={`connection-hero ${connected ? "connected" : ""}`}>
      <div className="connection-state-row">
        <span className="connection-state">
          <span className={`status-dot ${connected ? "connected" : ""}`} />
          {connected ? "实时语义已连接" : "还没有 Neovim 会话"}
        </span>
        {connected && <span className="profile-chip">{context.profileId}</span>}
      </div>
      <h2>{connected ? context.title : "先连接一个 LazyVim 环境"}</h2>
      <p>{connectionDescription(context, settings)}</p>
      <p className="overlay-note">浮窗可拖动，也可随时关闭；查询与显隐都有快捷键。</p>
      <div className="hero-actions">
        <ConnectionActionsView
          status={connected ? "connected" : "disconnected"}
          actions={actions}
        />
      </div>
      {!connected && <OnboardingSteps shortcut={shortcut} />}
    </div>
  );
}

/** 根据实时上下文生成退出提示，区分不同页面的 Esc 含义。 */
function exitText(context: EditorContext) {
  if (context.exitKey) {
    return `按 ${context.exitKey} 关闭当前页面`;
  }
  if (context.pending) {
    return "按 Esc 取消当前输入";
  }
  return "按 Esc 返回普通模式";
}

/** 输入区优先展示未完成前缀，其次才是可选的最近输入。 */
function pendingText(context: EditorContext, settings: AppSettings | null) {
  if (context.pending) {
    return context.pending;
  }
  if ((settings?.showHistory ?? true) && context.lastInput) {
    return `最近输入 ${context.lastInput}`;
  }
  return "没有未完成命令";
}

/** 已连接时的实时现场摘要，所有值直接来自编辑器上下文。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function ContextGrid({ context, settings }: ConnectionProps) {
  return (
    <div className="context-grid">
      <div className="context-item">
        <span>当前模式</span><strong>{displayMode(context.mode)}</strong>
      </div>
      <div className="context-item">
        <span>正在输入</span><strong>{pendingText(context, settings)}</strong>
      </div>
      <div className="context-item wide">
        <span>当前页面</span><strong>{context.title}</strong>
      </div>
      <div className="context-item wide">
        <span>退出方式</span><strong>{exitText(context)}</strong>
      </div>
    </div>
  );
}

/** 新手需要区分的四类退出语义，连接前后共用同一份内容。 */
function usageItems() {
  return (
    <ul>
      <li>取消未完成输入</li>
      <li>返回命令菜单的上一级</li>
      <li>关闭当前插件页面或窗口</li>
      <li>保存、放弃修改或退出 Neovim</li>
    </ul>
  );
}

/** 主窗口的实时语义入口：连接前引导，连接后展示上下文与下一步。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
export function ConnectionView({
  context,
  settings,
  actions,
}: ConnectionProps) {
  if (!context.sessionId) {
    return (
      <section className="connection-section">
        <ConnectionHero
          context={context}
          settings={settings}
          actions={actions}
        />
        <details className="onboarding-notes">
          <summary>连接后浮窗能做什么</summary>
          {usageItems()}
        </details>
      </section>
    );
  }
  return (
    <section className="connection-section">
      <ConnectionHero context={context} settings={settings} actions={actions} />
      <ContextGrid context={context} settings={settings} actions={actions} />
      <div className="usage-notes">
        <h3>浮窗会区分四件事</h3>
        {usageItems()}
      </div>
    </section>
  );
}

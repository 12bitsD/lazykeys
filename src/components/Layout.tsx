import { api } from "../api";
import { formatShortcut } from "../formatShortcut";
import type { ProfileActions } from "../profileActions";
import type { useActionFeedback } from "../useActionFeedback";
import type { useAppData } from "../useAppData";
import type { useCommandSearch } from "../useCommandSearch";
import type { AppSettings, EditorContext } from "../types";
import { CommandWorkspace } from "./CommandWorkspace";
import { ConnectionView } from "./ConnectionView";
import { EnvironmentView } from "./EnvironmentView";
import { SettingsView } from "./SettingsView";

/** 主窗口的四个工作区，仅决定当前展示哪条任务链路。 */
export type Tab = "connection" | "commands" | "environment" | "settings";

const tabs: { id: Tab; label: string }[] = [
  { id: "connection", label: "当前连接" },
  { id: "commands", label: "命令词典" },
  { id: "environment", label: "环境" },
  { id: "settings", label: "设置" },
];

/** 侧栏输入：当前页签、连接上下文与页签切换动作。 */
type SidebarProps = {
  tab: Tab;
  context: EditorContext;
  onSelect: (tab: Tab) => void;
};

/** 主导航与连接摘要，不持有业务状态。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
export function Sidebar({ tab, context, onSelect }: SidebarProps) {
  return (
    <aside className="sidebar">
      <div className="brand">
        <span className="brand-mark">LK</span>
        <div><strong>LazyKeys</strong><span>边用边懂</span></div>
      </div>
      <nav aria-label="主导航">
        {tabs.map((item) => (
          <button
            key={item.id}
            className={`nav-item ${tab === item.id ? "active" : ""}`}
            onClick={() => onSelect(item.id)}
            aria-current={tab === item.id ? "page" : undefined}
          >
            {item.label}
          </button>
        ))}
      </nav>
      <div className="sidebar-status">
        <span
          className={`status-dot ${context.sessionId ? "connected" : ""}`}
        />
        <span>{context.sessionId ? "Neovim 已连接" : "Neovim 未连接"}</span>
      </div>
    </aside>
  );
}

/** 顶栏提供全局浮窗与查询入口，设置只用于展示快捷键。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
export function Topbar({ tab, settings, onError }: {
  tab: Tab;
  settings: AppSettings | null;
  onError: (error: unknown) => void;
}) {
  const shortcut = settings?.queryShortcut ?? "Control+Option+K";
  return (
    <header className="topbar">
      <div>
        <p className="eyebrow">LazyVim 即时语义助手</p>
        <h1>{tabs.find((item) => item.id === tab)?.label}</h1>
      </div>
      <div className="topbar-actions">
        <button
          className="secondary"
          onClick={() => api.toggleOverlay().catch(onError)}
        >
          切换浮窗
        </button>
        <button
          className="secondary"
          onClick={() => api.showQueryPanel().catch(onError)}
        >
          查询命令 <kbd>{formatShortcut(shortcut)}</kbd>
        </button>
      </div>
    </header>
  );
}

/** 全局操作反馈，焦点进入提示时暂停自动消失。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
export function NoticeView({ feedback }: {
  feedback: ReturnType<typeof useActionFeedback>;
}) {
  const { notice } = feedback;
  if (!notice) {
    return null;
  }
  return (
    <div
      className={`notice ${notice.tone}`}
      role={notice.tone === "error" ? "alert" : "status"}
      onFocus={() => feedback.setNoticeFocused(true)}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
          feedback.setNoticeFocused(false);
        }
      }}
    >
      <span>{notice.message}</span>
      <button
        className="icon-button"
        onClick={() => {
          feedback.setNotice(null);
          feedback.setNoticeFocused(false);
        }}
        aria-label="关闭提示"
      >
        ×
      </button>
    </div>
  );
}

/** 工作区共享运行时：数据、反馈、搜索、Profile 动作与页签动作的组合。 */
type WorkspaceRuntime = {
  data: ReturnType<typeof useAppData>;
  feedback: ReturnType<typeof useActionFeedback>;
  search: ReturnType<typeof useCommandSearch>;
  profileActions: ProfileActions;
  onInstallLaunch: () => void;
  onSelectTab: (tab: Tab) => void;
};

/** 设置页动作统一经过操作反馈与后端适配器。 */
function settingsActions(
  data: WorkspaceRuntime["data"],
  feedback: WorkspaceRuntime["feedback"],
) {
  return {
    onSave: (draft: AppSettings) => feedback.runAction(
      "settings",
      async () => data.setSettings(await api.updateAppSettings(draft)),
      "设置已保存并应用",
    ),
    onOpenQuery: () => api.showQueryPanel().catch(feedback.reportError),
    onCenter: () => api.centerOverlay()
      .then(() => api.getAppSettings())
      .then(data.setSettings)
      .catch(feedback.reportError),
  };
}

/** 连接页只组合当前上下文与页签动作。 */
function connectionWorkspace(
  data: WorkspaceRuntime["data"],
  onInstallLaunch: () => void,
  onSelectTab: (tab: Tab) => void,
) {
  return (
    <ConnectionView
      context={data.context}
      settings={data.settings}
      actions={{
        onSearch: () => onSelectTab("commands"),
        onEnvironment: () => onSelectTab("environment"),
        onInstallLaunch,
      }}
    />
  );
}

/** 命令页只组合搜索状态与共享运行时。 */
function commandWorkspace(
  data: WorkspaceRuntime["data"],
  feedback: WorkspaceRuntime["feedback"],
  search: WorkspaceRuntime["search"],
) {
  return (
    <CommandWorkspace
      search={search}
      runtime={{
        busy: feedback.busy,
        context: data.context,
        settings: data.settings,
        runAction: feedback.runAction,
      }}
    />
  );
}

/** 按当前页签组合工作区，共享数据但不让页面互相持有状态。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
export function Workspace({
  tab,
  runtime: {
    data,
    feedback,
    search,
    profileActions,
    onInstallLaunch,
    onSelectTab,
  },
}: {
  tab: Tab;
  runtime: WorkspaceRuntime;
}) {
  if (tab === "connection") {
    return connectionWorkspace(data, onInstallLaunch, onSelectTab);
  }
  if (tab === "commands") {
    return commandWorkspace(data, feedback, search);
  }
  if (tab === "environment") {
    return (
      <EnvironmentView
        data={data}
        busy={feedback.busy}
        actions={profileActions}
      />
    );
  }
  return (
    <SettingsView
      settings={data.settings}
      busy={feedback.busy}
      actions={settingsActions(data, feedback)}
    />
  );
}

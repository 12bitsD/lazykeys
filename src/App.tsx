import { useState } from "react";
import { api } from "./api";
import { createProfileActions } from "./profileActions";
import { useActionFeedback } from "./useActionFeedback";
import { useAppData } from "./useAppData";
import { useCommandSearch } from "./useCommandSearch";
import { NoticeView, Sidebar, Topbar, Workspace } from "./components/Layout";
import type { Tab } from "./components/Layout";
import "./App.css";

/** 主窗口共享数据集合：上下文、设置、环境和安装日志的统一来源。 */
type AppData = ReturnType<typeof useAppData>;
/** 全局操作反馈集合：互斥锁、busy 与提示生命周期的统一入口。 */
type Feedback = ReturnType<typeof useActionFeedback>;

/** 一键启动默认 Profile，未就绪时先完成安装与健康检查。 */
function launchDefaultProfile(
  data: AppData,
  feedback: Feedback,
  setTab: (tab: Tab) => void,
) {
  void feedback.runAction(
    "install-launch-default",
    async () => {
      const profile = data.profiles.find((item) => item.isDefault)
        ?? data.profiles[0];
      if (!profile) {
        throw new Error("默认 Profile 尚未加载，请稍候再试");
      }
      if (!profile.ready) {
        await data.installWithLogs(profile.config.id);
      }
      await api.launchProfile(profile.config.id);
      setTab("commands");
    },
    "Neovim 已启动，接下来可查询或复制命令",
  );
}

/** 主窗口编排层：组合共享数据、工作区和 Profile 动作。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function App() {
  const [tab, setTab] = useState<Tab>("connection");
  const feedback = useActionFeedback();
  const data = useAppData(feedback.reportError, feedback.setNotice);
  const search = useCommandSearch(feedback.reportError);
  const profileActions = createProfileActions({
    settings: data.settings,
    runAction: feedback.runAction,
    loadEnvironment: data.loadEnvironment,
    installWithLogs: data.installWithLogs,
  });

  /** 切换工作区；进入设置时重新读取后端事实。 */
  function selectTab(next: Tab) {
    setTab(next);
    if (next === "settings") {
      void data.refreshSettings();
    }
  }

  return (
    <div className="app-shell">
      <Sidebar tab={tab} context={data.context} onSelect={selectTab} />
      <main className="main-content" id="content">
        <Topbar
          tab={tab}
          settings={data.settings}
          onError={feedback.reportError}
        />
        <NoticeView feedback={feedback} />
        <Workspace
          tab={tab}
          runtime={{
            data,
            feedback,
            search,
            profileActions,
            onInstallLaunch: () => launchDefaultProfile(data, feedback, setTab),
            onSelectTab: selectTab,
          }}
        />
      </main>
    </div>
  );
}

export default App;

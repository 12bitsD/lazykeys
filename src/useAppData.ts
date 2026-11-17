import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import type { Notice } from "./useActionFeedback";
import type {
  AppSettings,
  DependencyStatus,
  EditorContext,
  ProfileSummary,
} from "./types";

const emptyContext: EditorContext = {
  sessionId: "",
  profileId: "default",
  mode: "n",
  pending: "",
  lastInput: "",
  kind: "normal",
  title: "等待 Neovim 连接",
  cwd: "",
  filetype: "",
  bufferModified: false,
  focused: true,
};

/** 后端事件到主窗口状态的两个写入口：实时上下文与操作提示。 */
type BackendEventsOptions = {
  setContext: (context: EditorContext) => void;
  setNotice: (notice: Notice | null) => void;
};

/** 后端事件适配：实时上下文与命令结果分别进入主窗口状态。 */
function useBackendEvents({ setContext, setNotice }: BackendEventsOptions) {
  useEffect(() => {
    const unlisten = listen<EditorContext>("editor-context", (event) => {
      setContext(event.payload);
    });
    const errors = listen<string>("command-error", (event) => {
      setNotice({
        message: event.payload,
        tone: event.payload.startsWith("已发送") ? "success" : "error",
      });
    });
    return () => {
      void unlisten.then((cleanup) => cleanup());
      void errors.then((cleanup) => cleanup());
    };
  }, [setContext, setNotice]);
}

/** 环境数据源：依赖、Profile 与安装日志共用刷新链路。 */
function useEnvironment(reportError: (error: unknown) => void) {
  const [dependencies, setDependencies] = useState<DependencyStatus[]>([]);
  const [profiles, setProfiles] = useState<ProfileSummary[]>([]);
  const [installLogs, setInstallLogs] = useState<Record<string, string[]>>({});

  /** 并行刷新机器依赖与 Profile，安装和维护操作共用此出口。 */
  async function loadEnvironment() {
    try {
      const [dependencyResult, profileResult] = await Promise.all([
        api.checkEnvironment(),
        api.listProfiles(),
      ]);
      setDependencies(dependencyResult);
      setProfiles(profileResult);
    } catch (error) {
      reportError(error);
    }
  }

  /** 安装后保留日志并刷新状态，让同一次操作获得完整报告。 */
  async function installWithLogs(profileId: string) {
    const report = await api.installProfile(profileId);
    setInstallLogs((current) => ({
      ...current,
      [profileId]: report.log,
    }));
    await loadEnvironment();
    return report;
  }

  return {
    dependencies,
    profiles,
    installLogs,
    loadEnvironment,
    installWithLogs,
  };
}

/** 主窗口数据源：组合上下文、设置和环境状态。 */
export function useAppData(
  reportError: (error: unknown) => void,
  setNotice: (notice: Notice | null) => void,
) {
  const [context, setContext] = useState<EditorContext>(emptyContext);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const environment = useEnvironment(reportError);
  useBackendEvents({ setContext, setNotice });

  /** 设置读取入口，进入设置页或移动浮窗后刷新。 */
  function refreshSettings() {
    return api.getAppSettings().then(setSettings).catch(reportError);
  }

  useEffect(() => {
    void api.currentContext().then(setContext).catch(reportError);
    void refreshSettings();
    void environment.loadEnvironment();
  }, []);

  return {
    context,
    ...environment,
    settings,
    setSettings,
    refreshSettings,
  };
}

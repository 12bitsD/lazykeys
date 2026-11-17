import { api } from "./api";
import type { RunAction } from "./useActionFeedback";
import type {
  AppSettings,
  InstallReport,
  ProfileConfig,
  SessionSummary,
} from "./types";

/** Profile 卡片可发起的操作契约；实现集中在应用编排层。 */
export type ProfileActions = {
  onInstall: (profileId: string) => void;
  onLaunch: (profileId: string) => void;
  onSave: (config: ProfileConfig) => void;
  onCopy: (config: ProfileConfig, newId: string) => void;
  onRename: (profileId: string, newId: string, newName: string) => void;
  onDefault: (profileId: string) => void;
  onRollback: (profileId: string) => void;
  onExport: (profileId: string) => void;
  onDelete: (profileId: string) => void;
};

/** Profile 动作编排依赖：设置、全局操作入口与环境刷新链路。 */
type ProfileActionContext = {
  settings: AppSettings | null;
  runAction: RunAction;
  loadEnvironment: () => Promise<void>;
  installWithLogs: (profileId: string) => Promise<InstallReport>;
};

/** 安装、启动、保存与复制组成的主操作子集，维护动作另行组合。 */
type PrimaryActions = Pick<
  ProfileActions,
  "onInstall" | "onLaunch" | "onSave" | "onCopy"
>;

/** 启动提示只读取设置，不增加布尔行为参数。 */
function launchMessage(session: SessionSummary, settings: AppSettings | null) {
  if (settings?.includeCwd ?? true) {
    return `已在 Terminal 中启动：${session.cwd}`;
  }
  return "已在 Terminal 中启动 Neovim";
}

/** 保存后接续安装与健康检查，形成完整的配置发布链路。 */
async function saveProfile(
  config: ProfileConfig,
  context: ProfileActionContext,
) {
  await api.updateProfile(config);
  await context.installWithLogs(config.id);
}

/** 复制配置后接续安装，复用与保存相同的发布路径。 */
async function copyProfile(
  config: ProfileConfig,
  newId: string,
  context: ProfileActionContext,
) {
  await api.createProfile({
    ...config,
    id: newId,
    name: `${config.name} 副本`,
  });
  await context.installWithLogs(newId);
}

/** 改名后刷新列表，保持卡片与默认项引用一致。 */
async function renameProfile(
  profileId: string,
  newId: string,
  newName: string,
  context: ProfileActionContext,
) {
  await api.renameProfile(profileId, newId, newName);
  await context.loadEnvironment();
}

/** 维护动作的公共收尾：执行后端命令并刷新环境事实。 */
async function refreshAfter(
  operation: () => Promise<unknown>,
  context: ProfileActionContext,
) {
  await operation();
  await context.loadEnvironment();
}

/** 安装、启动和配置发布动作。 */
function createPrimaryActions(context: ProfileActionContext): PrimaryActions {
  return {
    onInstall: (id) => context.runAction(
      `install-${id}`,
      () => context.installWithLogs(id),
      "安装完成，请点击启动 Neovim 连接环境",
    ),
    onLaunch: (id) => context.runAction(
      `launch-${id}`,
      () => api.launchProfile(id),
      (session) => launchMessage(session, context.settings),
    ),
    onSave: (config) => context.runAction(
      `save-${config.id}`,
      () => saveProfile(config, context),
      "配置已保存并通过健康检查",
    ),
    onCopy: (config, newId) => context.runAction(
      `copy-${newId}`,
      () => copyProfile(config, newId, context),
      `已复制并安装 ${newId}`,
    ),
  };
}

/** 改名、默认项、回滚、导出和删除动作。 */
function createMaintenanceActions(
  context: ProfileActionContext,
): Omit<ProfileActions, keyof PrimaryActions> {
  return {
    onRename: (id, newId, newName) => context.runAction(
      `rename-${id}`,
      () => renameProfile(id, newId, newName, context),
      `已将 ${id} 重命名为 ${newId}`,
    ),
    onDefault: (id) => context.runAction(
      `default-${id}`,
      () => refreshAfter(() => api.setDefaultProfile(id), context),
      "已设为默认 Profile",
    ),
    onRollback: (id) => context.runAction(
      `rollback-${id}`,
      () => refreshAfter(() => api.rollbackProfile(id), context),
      "已回滚到最近快照",
    ),
    onExport: (id) => context.runAction(
      `export-${id}`,
      () => api.exportProfile(id),
      (destination) => `Profile 已导出：${destination}`,
    ),
    onDelete: (id) => context.runAction(
      `delete-${id}`,
      () => refreshAfter(() => api.deleteProfile(id), context),
      "已删除受管 Profile",
    ),
  };
}

/** 创建环境页动作集合，集中定义锁名称、成功反馈和刷新顺序。 */
export function createProfileActions(
  context: ProfileActionContext,
): ProfileActions {
  return {
    ...createPrimaryActions(context),
    ...createMaintenanceActions(context),
  };
}

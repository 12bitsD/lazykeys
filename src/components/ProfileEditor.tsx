import { useEffect, useId, useRef, useState } from "react";
import type { ProfileActions } from "../profileActions";
import type { ProfileConfig, ProfileSummary } from "../types";
import {
  actionBusy,
  ProfileConfigForm,
  ProfileMaintenance,
} from "./ProfileFormFields";
import type { EditorState } from "./ProfileFormFields";

/** Profile 卡片共享输入：配置摘要、全局 busy 与生命周期动作。 */
type ProfileProps = {
  profile: ProfileSummary;
  busy: string;
  actions: ProfileActions;
};

/** Profile 摘要的安装就绪状态，统一驱动状态文案与安装按钮。 */
type SummaryStatus = "installing" | "ready" | "missing";

/** 安装状态是唯一事实来源，摘要与按钮共用同一映射。 */
function summaryStatus(profile: ProfileSummary, busy: string): SummaryStatus {
  if (busy === `install-${profile.config.id}`) {
    return "installing";
  }
  if (profile.ready) {
    return "ready";
  }
  return "missing";
}

/** 摘要状态使用完整文案，避免颜色成为唯一提示。 */
function summaryStatusText(status: SummaryStatus) {
  if (status === "installing") {
    return "正在安装、同步并检查";
  }
  if (status === "ready") {
    return "已通过健康检查";
  }
  return "尚未安装或需要修复";
}

/** 安装按钮在修复与首次安装之间切换。 */
function installButtonText(status: SummaryStatus) {
  if (status === "installing") {
    return "正在安装";
  }
  if (status === "ready") {
    return "修复并同步";
  }
  return "一键安装";
}

/** Profile 摘要与主要动作：启动、安装和修复共用当前就绪状态。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function ProfileSummaryView({ profile, editor, busy, actions }: ProfileProps & {
  editor: EditorState;
}) {
  const id = profile.config.id;
  const status = summaryStatus(profile, busy);
  const disabled = actionBusy(id, busy, editor.copyId);
  return (
    <div className="profile-summary">
      <div>
        <h3>
          {profile.config.name}
          {profile.isDefault && <span className="default-badge">默认</span>}
        </h3>
        <p>{profile.configPath}</p>
        <span className={`profile-status ${profile.ready ? "ready" : ""}`}>
          {summaryStatusText(status)}
        </span>
      </div>
      <div className="profile-actions">
        {profile.ready && (
          <button
            className="primary"
            disabled={disabled}
            onClick={() => actions.onLaunch(id)}
          >
            {busy === `launch-${id}` ? "正在启动" : "启动 Neovim"}
          </button>
        )}
        <button
          className="secondary"
          disabled={disabled}
          onClick={() => actions.onInstall(id)}
        >
          {installButtonText(status)}
        </button>
      </div>
    </div>
  );
}

/** 安装进度只读取 busy，不引入额外加载状态。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function InstallProgress({ profile, busy }: Omit<ProfileProps, "actions">) {
  if (busy !== `install-${profile.config.id}`) {
    return null;
  }
  return (
    <div className="install-progress" role="status">
      <progress aria-label={`安装 ${profile.config.name}`} />
      <span>
        正在安装、同步插件并运行健康检查。时间取决于网络，期间可以继续查看命令。
      </span>
    </div>
  );
}

/** 日志只移除 ANSI 控制序列，保持原始文本内容。 */
function cleanLogLine(line: string): string {
  return line.replace(/\u001b\[[0-9;?]*[ -/]*[@-~]/g, "");
}

/** 安装日志按需展开，空日志不占布局。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function InstallLog({ lines }: { lines?: string[] }) {
  if (!lines || lines.length === 0) {
    return null;
  }
  return (
    <details className="install-log">
      <summary>安装日志（{lines.length} 条）</summary>
      <pre>{lines.map(cleanLogLine).join("\n")}</pre>
    </details>
  );
}

/** 删除区只表达确认意图，实际删除由应用动作层执行。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function DangerZone({ profile, editor, busy, onConfirm }: {
  profile: ProfileSummary;
  editor: EditorState;
  busy: string;
  onConfirm: () => void;
}) {
  const disabled = actionBusy(profile.config.id, busy, editor.copyId);
  return (
    <details className="danger-zone">
      <summary>删除 Profile</summary>
      <p>
        只删除此受管目录，不影响现有 Neovim 配置。
        {profile.isDefault && "请先设置另一个默认 Profile，再删除当前默认项。"}
      </p>
      <button
        className="danger-link"
        disabled={profile.isDefault || disabled}
        onClick={onConfirm}
      >
        确认删除
      </button>
    </details>
  );
}

/** 确认按钮与取消按钮共享原生 dialog 关闭语义。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function ConfirmActions({ action, onCancel, onConfirm }: {
  action: "rollback" | "delete";
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <form method="dialog" className="confirm-actions">
      <button type="submit" className="secondary" autoFocus onClick={onCancel}>
        取消
      </button>
      <button
        type="submit"
        className={action === "rollback" ? "primary" : "danger"}
        onClick={onConfirm}
      >
        {action === "rollback" ? "确认回滚" : "确认删除"}
      </button>
    </form>
  );
}

/** 危险维护操作的最终确认层，用原生模态对话框隔离误触。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function ConfirmDialog({ action, profileName, onCancel, onConfirm }: {
  action: "rollback" | "delete";
  profileName: string;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const descriptionId = useId();
  const rollback = action === "rollback";
  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) {
      return;
    }
    dialog.showModal();
    return () => {
      if (dialog.open) {
        dialog.close();
      }
    };
  }, []);
  return (
    <dialog
      ref={dialogRef}
      className="confirm-dialog"
      aria-labelledby={titleId}
      aria-describedby={descriptionId}
      onCancel={(event) => {
        event.preventDefault();
        onCancel();
      }}
    >
      <h2 id={titleId}>{rollback ? "确认回滚到快照" : "确认删除 Profile"}</h2>
      <p id={descriptionId}>
        {rollback
          ? `“${profileName}”的当前受管配置会被最近快照替换。`
          : `只会删除“${profileName}”的受管目录，且无法撤销。`}
        现有 Neovim 配置不受影响。
      </p>
      <ConfirmActions
        action={action}
        onCancel={onCancel}
        onConfirm={onConfirm}
      />
    </dialog>
  );
}

/** 编辑与维护表单的本地草稿，不包含危险操作确认状态。 */
function useProfileDraft(
  profile: ProfileSummary,
  onRollback: () => void,
): EditorState {
  const [draft, setDraft] = useState<ProfileConfig>(profile.config);
  const [copyId, setCopyId] = useState(`${profile.config.id}-copy`);
  const [renameId, setRenameId] = useState(profile.config.id);
  const [renameName, setRenameName] = useState(profile.config.name);
  return {
    draft,
    setDraft,
    copyId,
    renameId,
    renameName,
    setCopyId,
    setRenameId,
    setRenameName,
    onRollback,
  };
}

/** Profile 草稿与确认状态只跟随已保存配置快照重置。 */
function useProfileEditorState(profile: ProfileSummary) {
  const [confirmation, setConfirmation] = useState<
    "rollback" | "delete" | null
  >(null);
  const editor = useProfileDraft(profile, () => setConfirmation("rollback"));
  const savedConfig = JSON.stringify(profile.config);
  useEffect(() => {
    editor.setDraft(profile.config);
    editor.setCopyId(`${profile.config.id}-copy`);
    editor.setRenameId(profile.config.id);
    editor.setRenameName(profile.config.name);
  }, [savedConfig]);
  return { editor, confirmation, setConfirmation };
}

/** 确认层把回滚与删除映射到对应 Profile 动作。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function ProfileConfirmation({ confirmation, profile, actions, onClose }: {
  confirmation: "rollback" | "delete" | null;
  profile: ProfileSummary;
  actions: ProfileActions;
  onClose: () => void;
}) {
  if (!confirmation) {
    return null;
  }
  return (
    <ConfirmDialog
      action={confirmation}
      profileName={profile.config.name}
      onCancel={onClose}
      onConfirm={() => {
        const action = confirmation;
        onClose();
        if (action === "rollback") {
          actions.onRollback(profile.config.id);
        } else {
          actions.onDelete(profile.config.id);
        }
      }}
    />
  );
}

/** 单个 Profile 的完整维护链路，组合草稿、安装启动与危险操作。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
export function ProfileEditor({
  profile,
  busy,
  actions,
  installLog,
}: ProfileProps & {
  installLog?: string[];
}) {
  const { editor, confirmation, setConfirmation } = useProfileEditorState(
    profile,
  );
  return (
    <div className="profile-card">
      <ProfileSummaryView
        profile={profile}
        editor={editor}
        busy={busy}
        actions={actions}
      />
      <InstallProgress profile={profile} busy={busy} />
      <ProfileConfigForm
        profile={profile}
        editor={editor}
        busy={busy}
        actions={{ onChange: editor.setDraft, onSave: actions.onSave }}
      />
      <ProfileMaintenance
        profile={profile}
        editor={editor}
        busy={busy}
        actions={actions}
      />
      <InstallLog lines={installLog} />
      <DangerZone
        profile={profile}
        editor={editor}
        busy={busy}
        onConfirm={() => setConfirmation("delete")}
      />
      <ProfileConfirmation
        confirmation={confirmation}
        profile={profile}
        actions={actions}
        onClose={() => setConfirmation(null)}
      />
    </div>
  );
}

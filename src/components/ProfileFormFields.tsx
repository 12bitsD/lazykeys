import type { ProfileActions } from "../profileActions";
import type { ProfileConfig, ProfileSummary } from "../types";

/** Profile 编辑草稿及维护表单的共享状态。 */
export type EditorState = {
  draft: ProfileConfig;
  setDraft: (draft: ProfileConfig) => void;
  copyId: string;
  renameId: string;
  renameName: string;
  setCopyId: (value: string) => void;
  setRenameId: (value: string) => void;
  setRenameName: (value: string) => void;
  onRollback: () => void;
};

/** 把全局操作名映射为当前卡片的互斥状态。 */
export function actionBusy(profileId: string, busy: string, copyId: string) {
  const actions = [
    "install",
    "launch",
    "save",
    "rename",
    "default",
    "rollback",
    "export",
    "delete",
  ];
  return actions.some((action) => busy === `${action}-${profileId}`)
    || busy === `copy-${copyId}`;
}

/** Extra 开关只修改草稿数组，不直接写配置。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function ExtraOption({ extra, draft, onChange }: {
  extra: "typescript" | "python";
  draft: ProfileConfig;
  onChange: (draft: ProfileConfig) => void;
}) {
  return (
    <label className="config-check">
      <input
        type="checkbox"
        checked={draft.extras.includes(extra)}
        onChange={(event) => onChange({
          ...draft,
          extras: event.target.checked
            ? [...new Set([...draft.extras, extra])]
            : draft.extras.filter((item) => item !== extra),
        })}
      />
      {extra === "typescript" ? "TypeScript Extra" : "Python Extra"}
    </label>
  );
}

/** 主题与缩进使用原生选择控件，更新完整草稿。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function ThemeIndentOptions({ draft, onChange }: {
  draft: ProfileConfig;
  onChange: (draft: ProfileConfig) => void;
}) {
  return (
    <>
      <label>
        主题
        <select
          value={draft.theme}
          onChange={(event) => onChange({
            ...draft,
            theme: event.target.value,
          })}
        >
          <option value="tokyonight">TokyoNight</option>
          <option value="catppuccin">Catppuccin</option>
        </select>
      </label>
      <label>
        缩进
        <select
          value={draft.indent}
          onChange={(event) => onChange({
            ...draft,
            indent: Number(event.target.value),
          })}
        >
          <option value={2}>2 空格</option>
          <option value={4}>4 空格</option>
        </select>
      </label>
    </>
  );
}

/** 普通布尔配置由字段名驱动，避免重复表单结构。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function ProfileToggle({ label, field, draft, onChange }: {
  label: string;
  field: "relativeNumbers" | "wrap" | "autoformat";
  draft: ProfileConfig;
  onChange: (draft: ProfileConfig) => void;
}) {
  return (
    <label className="config-check">
      <input
        type="checkbox"
        checked={draft[field]}
        onChange={(event) => onChange({
          ...draft,
          [field]: event.target.checked,
        })}
      />
      {label}
    </label>
  );
}

/** 编辑器选项字段集合，统一以完整草稿替换方式更新。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function ProfileOptions({ draft, onChange }: {
  draft: ProfileConfig;
  onChange: (draft: ProfileConfig) => void;
}) {
  return (
    <div className="profile-config-grid">
      <ThemeIndentOptions draft={draft} onChange={onChange} />
      <ProfileToggle
        label="相对行号"
        field="relativeNumbers"
        draft={draft}
        onChange={onChange}
      />
      <ProfileToggle
        label="自动换行"
        field="wrap"
        draft={draft}
        onChange={onChange}
      />
      <ProfileToggle
        label="自动格式化"
        field="autoformat"
        draft={draft}
        onChange={onChange}
      />
      <ExtraOption extra="typescript" draft={draft} onChange={onChange} />
      <ExtraOption extra="python" draft={draft} onChange={onChange} />
    </div>
  );
}

/** 编辑器选项的保存状态，统一驱动状态文案、还原和保存按钮。 */
type ConfigStatus = "saving" | "dirty" | "synced";

/** 配置保存状态与按钮使用同一份文案映射。 */
function configStatusText(status: ConfigStatus) {
  if (status === "saving") {
    return "正在保存、同步并检查";
  }
  if (status === "dirty") {
    return "有未保存的选项修改";
  }
  return "选项已同步";
}

/** 保存按钮只依赖明确状态，避免重复嵌套条件。 */
function configSaveText(status: ConfigStatus) {
  if (status === "saving") {
    return "正在保存并检查";
  }
  if (status === "dirty") {
    return "保存并同步";
  }
  return "已同步";
}

/** 配置编辑区：已保存事实与草稿比较后才允许保存或还原。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
export function ProfileConfigForm({ profile, editor, busy, actions }: {
  profile: ProfileSummary;
  editor: EditorState;
  busy: string;
  actions: {
    onChange: (draft: ProfileConfig) => void;
    onSave: (config: ProfileConfig) => void;
  };
}) {
  const id = profile.config.id;
  const dirty = JSON.stringify(editor.draft) !== JSON.stringify(profile.config);
  let status: ConfigStatus = "synced";
  if (busy === `save-${id}`) {
    status = "saving";
  } else if (dirty) {
    status = "dirty";
  }
  const disabled = actionBusy(id, busy, editor.copyId);
  return (
    <details className="profile-config">
      <summary>编辑器选项</summary>
      <ProfileOptions draft={editor.draft} onChange={actions.onChange} />
      <div className="profile-actions profile-config-actions">
        <span className="profile-config-status" role="status">
          {configStatusText(status)}
        </span>
        {status === "dirty" && (
          <button
            className="secondary"
            disabled={disabled}
            onClick={() => actions.onChange(profile.config)}
          >
            还原修改
          </button>
        )}
        <button
          className="primary"
          disabled={disabled || status !== "dirty"}
          onClick={() => actions.onSave(editor.draft)}
        >
          {configSaveText(status)}
        </button>
      </div>
    </details>
  );
}

/** 重命名与复制共用的 Profile ID 输入及原生校验。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function ProfileIdInput({ id, value, kind, onChange }: {
  id: string;
  value: string;
  kind: "rename" | "copy";
  onChange: (value: string) => void;
}) {
  const inputId = kind === "rename" ? `rename-id-${id}` : `copy-${id}`;
  const hintId = kind === "rename"
    ? `rename-id-hint-${id}`
    : `copy-id-hint-${id}`;
  const missingMessage = kind === "rename"
    ? "请输入 Profile ID"
    : "请输入新 Profile ID";
  return (
    <>
      <label htmlFor={inputId}>新 Profile ID</label>
      <span className="maintenance-hint" id={hintId}>
        小写字母开头，可含数字、-、_，最长 40 位。
      </span>
      <input
        id={inputId}
        name={kind === "rename" ? "profile-id" : "profile-copy-id"}
        value={value}
        onChange={(event) => {
          event.currentTarget.setCustomValidity("");
          onChange(event.target.value);
        }}
        onInvalid={(event) => {
          const message = event.currentTarget.validity.valueMissing
            ? missingMessage
            : "ID 需以小写字母开头，只能包含小写字母、数字、- 和 _，最长 40 位";
          event.currentTarget.setCustomValidity(message);
        }}
        required
        pattern="[a-z][a-z0-9_\-]{0,39}"
        maxLength={40}
        aria-describedby={hintId}
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
      />
    </>
  );
}

/** 重命名表单状态，独立于编辑器选项草稿驱动提交按钮。 */
type RenameStatus = "renaming" | "dirty" | "synced";

/** 重命名按钮状态由保存中与草稿差异决定。 */
function renameButtonText(status: RenameStatus) {
  if (status === "renaming") {
    return "正在重命名";
  }
  if (status === "dirty") {
    return "重命名";
  }
  return "无需重命名";
}

/** 显示名称输入与原生非空白校验，独立于 Profile ID。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function RenameNameInput({ id, value, onChange }: {
  id: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <>
      <label htmlFor={`rename-name-${id}`}>显示名称</label>
      <input
        id={`rename-name-${id}`}
        name="profile-name"
        value={value}
        onChange={(event) => {
          event.currentTarget.setCustomValidity("");
          onChange(event.target.value);
        }}
        onInvalid={(event) => event.currentTarget.setCustomValidity(
          "显示名称不能为空白",
        )}
        required
        pattern=".*\S.*"
        autoComplete="off"
      />
    </>
  );
}

/** 重命名表单：ID 和显示名独立于编辑器选项计算修改状态。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function RenameForm({ profile, editor, busy, actions }: {
  profile: ProfileSummary;
  editor: EditorState;
  busy: string;
  actions: ProfileActions;
}) {
  const id = profile.config.id;
  const dirty = editor.renameId.trim() !== id
    || editor.renameName.trim() !== profile.config.name;
  let status: RenameStatus = "synced";
  if (busy === `rename-${id}`) {
    status = "renaming";
  } else if (dirty) {
    status = "dirty";
  }
  const disabled = actionBusy(id, busy, editor.copyId);
  return (
    <fieldset>
      <legend>重命名</legend>
      <form onSubmit={(event) => {
        event.preventDefault();
        actions.onRename(id, editor.renameId.trim(), editor.renameName.trim());
      }}>
        <ProfileIdInput
          id={id}
          value={editor.renameId}
          kind="rename"
          onChange={editor.setRenameId}
        />
        <RenameNameInput
          id={id}
          value={editor.renameName}
          onChange={editor.setRenameName}
        />
        <button
          type="submit"
          className="secondary"
          disabled={disabled || status !== "dirty"}
        >
          {renameButtonText(status)}
        </button>
      </form>
    </fieldset>
  );
}

/** 复制表单：以当前草稿创建新 Profile 并进入安装链路。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function CopyForm({ profile, editor, busy, actions }: {
  profile: ProfileSummary;
  editor: EditorState;
  busy: string;
  actions: ProfileActions;
}) {
  const id = profile.config.id;
  const disabled = actionBusy(id, busy, editor.copyId);
  return (
    <fieldset>
      <legend>复制</legend>
      <form onSubmit={(event) => {
        event.preventDefault();
        actions.onCopy(editor.draft, editor.copyId.trim());
      }}>
        <ProfileIdInput
          id={id}
          value={editor.copyId}
          kind="copy"
          onChange={editor.setCopyId}
        />
        <button type="submit" className="secondary" disabled={disabled}>
          {busy === `copy-${editor.copyId}` ? "正在复制并安装" : "复制并安装"}
        </button>
      </form>
    </fieldset>
  );
}

/** 维护动作与两个编辑表单分离，保持回滚和默认项限制一致。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function MaintenanceActions({ profile, editor, busy, actions }: {
  profile: ProfileSummary;
  editor: EditorState;
  busy: string;
  actions: ProfileActions;
}) {
  const id = profile.config.id;
  const disabled = actionBusy(id, busy, editor.copyId);
  return (
    <div className="profile-actions profile-config-actions">
      <button
        className="secondary"
        disabled={disabled}
        onClick={editor.onRollback}
      >
        回滚到快照
      </button>
      <button
        className="secondary"
        disabled={disabled}
        onClick={() => actions.onExport(id)}
      >
        导出
      </button>
      <button
        className="secondary"
        disabled={profile.isDefault || disabled}
        onClick={() => actions.onDefault(id)}
      >
        设为默认
      </button>
    </div>
  );
}

/** 维护区组合复制、改名、回滚、导出和默认项设置。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
export function ProfileMaintenance({ profile, editor, busy, actions }: {
  profile: ProfileSummary;
  editor: EditorState;
  busy: string;
  actions: ProfileActions;
}) {
  return (
    <details className="profile-maintenance">
      <summary>维护与复制</summary>
      <div className="maintenance-grid">
        <RenameForm
          profile={profile}
          editor={editor}
          busy={busy}
          actions={actions}
        />
        <CopyForm
          profile={profile}
          editor={editor}
          busy={busy}
          actions={actions}
        />
      </div>
      <MaintenanceActions
        profile={profile}
        editor={editor}
        busy={busy}
        actions={actions}
      />
    </details>
  );
}

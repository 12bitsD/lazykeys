import { useEffect, useMemo, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import type { AppSettings } from "../types";
import {
  NumberSetting,
  ShortcutInput,
  comparableSettings,
  createDefaultSettings,
  numberTextFrom,
  numericSettingMeta,
  validNumericSetting,
} from "./SettingsInputs";
import type { NumericSettingKey } from "./SettingsInputs";

/** 设置页动作：保存草稿、打开查询与恢复浮窗中央位置。 */
type SettingsActions = {
  onSave: (settings: AppSettings) => void;
  onOpenQuery: () => void;
  onCenter: () => void;
};

/** 设置页输入：后端已保存设置、全局 busy 与动作集合。 */
type SettingsProps = {
  settings: AppSettings | null;
  busy: string;
  actions: SettingsActions;
};

/** 草稿校验与保存状态，统一驱动提示、保存和放弃修改。 */
type DraftStatus = "invalid" | "dirty" | "synced";

/** 完整签名跟随坐标，行为签名排除坐标以防拖动覆盖草稿。 */
function settingSignatures(settings: AppSettings | null) {
  return {
    full: settings ? comparableSettings(settings) : "",
    behavior: settings
      ? comparableSettings({
        ...settings,
        overlayX: undefined,
        overlayY: undefined,
      })
      : "",
  };
}

/** 数字文本始终更新，只有合法值进入可保存草稿。 */
function updateNumericDraft(
  key: NumericSettingKey,
  value: string,
  setNumberText: Dispatch<SetStateAction<Record<NumericSettingKey, string>>>,
  setDraft: Dispatch<SetStateAction<AppSettings | null>>,
) {
  setNumberText((current) => ({ ...current, [key]: value }));
  if (validNumericSetting(key, value)) {
    setDraft((current) => current
      ? { ...current, [key]: Number(value) }
      : current);
  }
}

/** 设置草稿状态机：已保存事实、编辑副本和恢复默认备份分离。 */
function useSettingsDraft(settings: AppSettings | null) {
  const [draft, setDraft] = useState<AppSettings | null>(settings);
  const [numberText, setNumberText] = useState(() => numberTextFrom(settings));
  const [resetBackup, setResetBackup] = useState<AppSettings | null>(null);
  const signatures = settingSignatures(settings);
  const dirty = useMemo(
    () => settings && draft
      ? comparableSettings(settings) !== comparableSettings(draft)
      : false,
    [settings, draft],
  );
  const numericKeys = Object.keys(
    numericSettingMeta,
  ) as NumericSettingKey[];
  const numbersInvalid = numericKeys.some(
    (key) => !validNumericSetting(key, numberText[key]),
  );

  /** 同步替换结构化草稿与数字文本，避免残留旧输入。 */
  function replaceDraft(next: AppSettings | null) {
    setDraft(next);
    setNumberText(numberTextFrom(next));
  }

  useEffect(() => {
    replaceDraft(settings);
    setResetBackup(null);
  }, [signatures.behavior]);
  useEffect(() => {
    setDraft((current) => current && settings
      ? { ...current, overlayX: settings.overlayX, overlayY: settings.overlayY }
      : current);
  }, [signatures.full]);

  return {
    draft,
    numberText,
    resetBackup,
    dirty,
    numbersInvalid,
    setDraft,
    setResetBackup,
    replaceDraft,
    updateNumber: (key: NumericSettingKey, value: string) => {
      updateNumericDraft(key, value, setNumberText, setDraft);
    },
  };
}

/** 两个全局快捷键共用录制组件，但分别写入草稿字段。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function ShortcutSettings({ draft, onChange }: {
  draft: AppSettings;
  onChange: (draft: AppSettings) => void;
}) {
  return (
    <>
      <label className="setting-row">
        <span>
          <strong>全局查询快捷键</strong>
          <small>聚焦后按下组合键；系统注册成功后才会保存。</small>
        </span>
        <ShortcutInput
          label="全局查询快捷键"
          value={draft.queryShortcut}
          onChange={(queryShortcut) => onChange({
            ...draft,
            queryShortcut,
          })}
        />
      </label>
      <label className="setting-row">
        <span>
          <strong>浮窗显示 / 隐藏快捷键</strong>
          <small>独立于查询，用于切换被动浮窗。</small>
        </span>
        <ShortcutInput
          label="浮窗显示 / 隐藏快捷键"
          value={draft.overlayShortcut}
          onChange={(overlayShortcut) => onChange({
            ...draft,
            overlayShortcut,
          })}
        />
      </label>
    </>
  );
}

/** 浮窗行为设置：所有编辑只更新草稿，保存按钮统一提交。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function BehaviorSettings({ draft, onChange, onOpenQuery }: {
  draft: AppSettings;
  onChange: (draft: AppSettings) => void;
  onOpenQuery: () => void;
}) {
  return (
    <>
      <label className="setting-row">
        <span>
          <strong>显示策略</strong>
          <small>智能展开、始终完整显示，或只在快捷键唤起时显示。</small>
        </span>
        <select
          value={draft.displayStrategy}
          onChange={(event) => onChange({
            ...draft,
            displayStrategy: event.target.value as AppSettings[
              "displayStrategy"
            ],
          })}
        >
          <option value="smart">智能显示</option>
          <option value="always">常驻</option>
          <option value="shortcut">仅快捷键</option>
        </select>
      </label>
      <ShortcutSettings draft={draft} onChange={onChange} />
      <div className="detail-actions">
        <button type="button" className="secondary" onClick={onOpenQuery}>
          立即打开查询
        </button>
      </div>
    </>
  );
}

/** 高级设置所需草稿、数值文本与动作集合。 */
type AdvancedProps = {
  draft: AppSettings;
  numberText: Record<NumericSettingKey, string>;
  busy: string;
  resetBackup: AppSettings | null;
  onChange: (draft: AppSettings) => void;
  onNumberChange: (key: NumericSettingKey, value: string) => void;
  onCenter: () => void;
  onReset: () => void;
};

/** 布尔偏好使用统一字段表，避免重复表单结构。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function AdvancedToggles({ draft, onChange }: {
  draft: AppSettings;
  onChange: (draft: AppSettings) => void;
}) {
  return (
    <>
      {([
        ["clickThrough", "鼠标穿透", "开启时点击到达下方编辑器；关闭后可拖动浮窗并自动记住位置。"],
        ["alwaysOnTop", "始终置顶", "关闭后使用普通窗口层级。"],
        ["showMnemonic", "显示助记", "在浮窗中显示下一步与分段助记。"],
        ["showHistory", "显示按键历史", "只保留内存中的短前缀，不持久化。"],
        ["confirmEdits", "修改前确认", "破坏性操作始终需要确认，不受此开关影响。"],
        ["includeCwd", "显示工作目录", "关闭后，主窗口与浮窗隐藏 cwd。"],
      ] as const).map(([key, title, detail]) => (
        <label className="setting-row" key={key}>
          <span><strong>{title}</strong><small>{detail}</small></span>
          <input
            type="checkbox"
            checked={draft[key]}
            onChange={(event) => onChange({
              ...draft,
              [key]: event.target.checked,
            })}
          />
        </label>
      ))}
    </>
  );
}

/** 高级设置收纳外观、隐私和恢复默认，保持主流程简洁。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function AdvancedSettings({
  draft,
  numberText,
  busy,
  resetBackup,
  onChange,
  onNumberChange,
  onCenter,
  onReset,
}: AdvancedProps) {
  return (
    <details className="settings-advanced">
      <summary>浮窗外观与高级选项</summary>
      <div className="profile-config-grid">
        {(Object.keys(numericSettingMeta) as NumericSettingKey[]).map((key) => (
          <NumberSetting
            key={key}
            setting={key}
            value={numberText[key]}
            onChange={(value) => onNumberChange(key, value)}
          />
        ))}
      </div>
      <AdvancedToggles draft={draft} onChange={onChange} />
      <div className="detail-actions">
        <button type="button" className="secondary" onClick={onCenter}>
          浮窗居中
        </button>
        <button
          type="button"
          className="danger-link"
          disabled={busy === "settings" || resetBackup !== null}
          onClick={onReset}
        >
          恢复默认
        </button>
      </div>
    </details>
  );
}

/** 保存栏文案只读取明确状态，不嵌套条件表达式。 */
function draftStatusText(status: DraftStatus) {
  if (status === "invalid") {
    return "请先修正数值输入";
  }
  if (status === "dirty") {
    return "有未保存的修改";
  }
  return "设置已同步";
}

/** 恢复默认后的撤销入口，与主保存按钮保持独立。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function ResetControls({ status, onUndo }: {
  status: DraftStatus;
  onUndo: () => void;
}) {
  return (
    <div className="detail-actions">
      <span role="status"><small>
        {status === "dirty" ? "已填入默认值，保存后生效。" : "当前已是默认值。"}
      </small></span>
      <button type="button" className="secondary" onClick={onUndo}>
        撤销恢复
      </button>
    </div>
  );
}

/** 保存栏只接收明确状态，不用多个布尔参数推导文案。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function SettingsSaveBar({ status, busy, resetBackup, actions }: {
  status: DraftStatus;
  busy: string;
  resetBackup: AppSettings | null;
  actions: {
    onDiscard: () => void;
    onSave: () => void;
    onUndoReset: () => void;
  };
}) {
  const saving = busy === "settings";
  return (
    <>
      <div className="settings-savebar">
        <div role="status"><small>{draftStatusText(status)}</small></div>
        <div className="detail-actions">
          {status !== "synced" && !resetBackup && (
            <button
              type="button"
              className="secondary"
              onClick={actions.onDiscard}
            >
              放弃修改
            </button>
          )}
          <button
            type="button"
            className="primary"
            disabled={saving || status !== "dirty"}
            onClick={actions.onSave}
          >
            {saving ? "正在保存" : "保存并应用"}
          </button>
        </div>
      </div>
      {resetBackup && (
        <ResetControls status={status} onUndo={actions.onUndoReset} />
      )}
    </>
  );
}

/** 设置说明与生效时机是静态内容，不参与草稿状态计算。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function SettingsHeading() {
  return (
    <div className="section-heading">
      <h2>浮窗行为</h2>
      <p>外观在保存后立即应用；结果显示时长在下次同步 Profile 后生效。</p>
    </div>
  );
}

/** 隐私边界固定展示，明确不采集的输入类型。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function PrivacyNote() {
  return (
    <div className="privacy-note">
      <h3>隐私边界</h3>
      <p>
        Insert 文本、命令参数和搜索内容不会发送或保存。实时按键只来自安装
        companion 的 Neovim，不使用全局键盘监听。
      </p>
    </div>
  );
}

/** 保存栏动作统一读取当前草稿与恢复备份，避免在 JSX 中编写流程。 */
function saveBarActions(
  editor: ReturnType<typeof useSettingsDraft>,
  actions: SettingsActions,
  settings: AppSettings | null,
) {
  return {
    onDiscard: () => editor.replaceDraft(settings),
    onSave: () => actions.onSave(editor.draft as AppSettings),
    onUndoReset: () => {
      editor.replaceDraft(editor.resetBackup);
      editor.setResetBackup(null);
    },
  };
}

/** 设置工作区：校验草稿后一次性保存并应用。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
export function SettingsView({ settings, busy, actions }: SettingsProps) {
  const editor = useSettingsDraft(settings);
  if (!editor.draft) {
    return <section className="settings-section empty-state">正在读取设置…</section>;
  }
  const draft = editor.draft;
  let status: DraftStatus = "synced";
  if (editor.numbersInvalid) {
    status = "invalid";
  } else if (editor.dirty) {
    status = "dirty";
  }
  return (
    <section className="settings-section">
      <SettingsHeading />
      <BehaviorSettings
        draft={draft}
        onChange={editor.setDraft}
        onOpenQuery={actions.onOpenQuery}
      />
      <AdvancedSettings
        draft={draft}
        numberText={editor.numberText}
        busy={busy}
        resetBackup={editor.resetBackup}
        onChange={editor.setDraft}
        onNumberChange={editor.updateNumber}
        onCenter={actions.onCenter}
        onReset={() => {
          editor.setResetBackup(draft);
          editor.replaceDraft(createDefaultSettings(draft.defaultProfile));
        }}
      />
      <SettingsSaveBar
        status={status}
        busy={busy}
        resetBackup={editor.resetBackup}
        actions={saveBarActions(editor, actions, settings)}
      />
      <PrivacyNote />
    </section>
  );
}

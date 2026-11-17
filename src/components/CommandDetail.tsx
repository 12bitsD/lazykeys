import { useEffect, useState } from "react";
import type { CommandItem, CommandOverride } from "../types";
import type { CommandActions, CommandRuntime } from "./CommandWorkspace";

/** 命令详情输入：当前候选、共享运行时与可执行动作。 */
type DetailProps = {
  command?: CommandItem;
  runtime: CommandRuntime;
  actions: CommandActions;
};

/** 别名草稿的三种界面状态，统一驱动状态文案、还原与保存按钮。 */
type OverrideState = "saving" | "dirty" | "synced";

/** 保存状态文案只读取明确状态，不嵌套条件表达式。 */
function overrideStatus(state: OverrideState) {
  if (state === "saving") {
    return "正在保存别名与助记…";
  }
  if (state === "dirty") {
    return "有未保存的修改";
  }
  return "已与当前命令同步";
}

/** 提交按钮沿用同一状态模型，避免文案与禁用条件分叉。 */
function overrideSubmitText(state: OverrideState) {
  if (state === "saving") {
    return "正在保存…";
  }
  if (state === "dirty") {
    return "保存别名与助记";
  }
  return "已同步";
}

/** 别名与助记字段只更新本地草稿。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function OverrideFields({ aliases, mnemonic, onAliases, onMnemonic }: {
  aliases: string;
  mnemonic: string;
  onAliases: (value: string) => void;
  onMnemonic: (value: string) => void;
}) {
  return (
    <>
      <label htmlFor="override-aliases">自定义中文别名（用逗号分隔）</label>
      <input
        id="override-aliases"
        name="aliases"
        autoComplete="off"
        value={aliases}
        onChange={(event) => onAliases(event.target.value)}
      />
      <label htmlFor="override-mnemonic">自定义助记（必填）</label>
      <input
        id="override-mnemonic"
        name="mnemonic"
        autoComplete="off"
        required
        pattern=".*\S.*"
        title="请输入至少一个非空白字符"
        value={mnemonic}
        onInvalid={(event) => event.currentTarget.setCustomValidity(
          "请输入至少一个非空白字符",
        )}
        onChange={(event) => {
          event.currentTarget.setCustomValidity("");
          onMnemonic(event.target.value);
        }}
      />
    </>
  );
}

/** 保存与还原动作共用状态，不在保存中重复提交。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function OverrideActions({ state, onReset }: {
  state: OverrideState;
  onReset: () => void;
}) {
  return (
    <div className="override-actions">
      {state === "dirty" && (
        <button type="button" className="secondary" onClick={onReset}>
          还原修改
        </button>
      )}
      <button
        type="submit"
        className="primary"
        disabled={state !== "dirty"}
      >
        {overrideSubmitText(state)}
      </button>
    </div>
  );
}

/** 表单呈现层只接收草稿与动作，不推导保存状态。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function OverrideForm({ command, draft, onSave }: {
  command: CommandItem;
  draft: {
    aliases: string;
    mnemonic: string;
    state: OverrideState;
    onAliases: (value: string) => void;
    onMnemonic: (value: string) => void;
    onReset: () => void;
  };
  onSave: (value: CommandOverride) => void;
}) {
  return (
    <form onSubmit={(event) => {
      event.preventDefault();
      onSave({
        commandId: command.id,
        aliases: draft.aliases.split(/[，,]/).map(
          (alias) => alias.trim(),
        ).filter(Boolean),
        mnemonic: draft.mnemonic.trim(),
      });
    }}>
      <OverrideFields
        aliases={draft.aliases}
        mnemonic={draft.mnemonic}
        onAliases={draft.onAliases}
        onMnemonic={draft.onMnemonic}
      />
      <div className="override-status" role="status">
        {overrideStatus(draft.state)}
      </div>
      <OverrideActions state={draft.state} onReset={draft.onReset} />
    </form>
  );
}

/** 别名编辑表单：草稿与已保存值分开，保存后刷新词典。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function OverrideEditor({ command, busy, onSave }: {
  command: CommandItem;
  busy: string;
  onSave: (value: CommandOverride) => void;
}) {
  const [aliases, setAliases] = useState(command.aliases.join("，"));
  const [mnemonic, setMnemonic] = useState(command.mnemonic);
  const savedAliases = command.aliases.join("，");
  const dirty = aliases !== savedAliases || mnemonic !== command.mnemonic;
  const saving = busy === `override-${command.id}`;
  let state: OverrideState = "synced";
  if (saving) {
    state = "saving";
  } else if (dirty) {
    state = "dirty";
  }
  useEffect(() => {
    setAliases(savedAliases);
    setMnemonic(command.mnemonic);
  }, [command.id, savedAliases, command.mnemonic]);
  return (
    <details className="override-editor">
      <summary>自定义别名与助记</summary>
      <OverrideForm
        command={command}
        onSave={onSave}
        draft={{
          aliases,
          mnemonic,
          state,
          onAliases: setAliases,
          onMnemonic: setMnemonic,
          onReset: () => {
            setAliases(savedAliases);
            setMnemonic(command.mnemonic);
          },
        }}
      />
    </details>
  );
}

/** 风险提示同时考虑连接状态、命令风险和用户确认设置。 */
function riskText(command: CommandItem, { context, settings }: CommandRuntime) {
  if (!context.sessionId) {
    return "可复制按键说明；启动受管 Profile 后才能执行到 Neovim。";
  }
  if (command.risk === "destructive") {
    return `目标：${context.profileId}。这是破坏性操作，执行前需要再次确认。`;
  }
  if (command.risk !== "edit") {
    return `目标：${context.profileId}。只发送真实按键，不拼接命令脚本。`;
  }
  if (settings?.confirmEdits ?? true) {
    return `目标：${context.profileId}。此操作会修改当前状态，执行前需要确认。`;
  }
  return `目标：${context.profileId}。此操作会修改当前状态，将直接发送按键。`;
}

/** 搜索详情的动作区：复制始终可用，执行要求已连接会话。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function DetailActions({ command, runtime, actions }: DetailProps & {
  command: CommandItem;
}) {
  const sessionReady = Boolean(runtime.context.sessionId);
  const executing = runtime.busy === `execute-${command.id}`;
  return (
    <div className="detail-actions">
      <button
        className={sessionReady ? "secondary" : "primary"}
        onClick={() => actions.onCopy(command)}
      >
        复制按键说明
      </button>
      <button
        className={command.risk === "destructive" ? "danger" : "primary"}
        disabled={!sessionReady || executing}
        onClick={() => actions.onExecute(command)}
      >
        {executing ? "正在执行" : "执行到当前 Neovim"}
      </button>
    </div>
  );
}

/** 命令详情：解释语义、编辑别名，并按风险复制或执行真实按键。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
export function CommandDetail({ command, runtime, actions }: DetailProps) {
  if (!command) {
    return <aside className="command-detail empty-state">选择一条命令查看语义。</aside>;
  }
  return (
    <aside className="command-detail" aria-label="命令详情">
      <div className="detail-source">{command.source}</div>
      <h2>{command.title}</h2>
      <kbd className="detail-key">{command.lhs}</kbd>
      <dl>
        <div><dt>功能</dt><dd>{command.description}</dd></div>
        <div><dt>助记</dt><dd>{command.mnemonic}</dd></div>
        <div><dt>模式</dt><dd>{command.mode}</dd></div>
        <div><dt>结果与副作用</dt><dd>{command.effect}</dd></div>
      </dl>
      {!command.id.startsWith("runtime:") && (
        <OverrideEditor
          command={command}
          busy={runtime.busy}
          onSave={actions.onSaveOverride}
        />
      )}
      <p className={`risk-note ${command.risk}`}>
        {riskText(command, runtime)}
      </p>
      <DetailActions command={command} runtime={runtime} actions={actions} />
    </aside>
  );
}

import { api } from "../api";
import type { RunAction } from "../useActionFeedback";
import type { useCommandSearch } from "../useCommandSearch";
import type {
  AppSettings,
  CommandItem,
  CommandOverride,
  EditorContext,
} from "../types";
import { CommandDetail } from "./CommandDetail";
import { QueryPanel } from "./QueryPanel";

/** 命令工作区共享的运行时事实与操作入口。 */
export type CommandRuntime = {
  busy: string;
  context: EditorContext;
  settings: AppSettings | null;
  runAction: RunAction;
};

/** 命令详情可发起的复制、执行与别名保存动作。 */
export type CommandActions = {
  onCopy: (command: CommandItem) => void;
  onExecute: (command: CommandItem) => void;
  onSaveOverride: (value: CommandOverride) => void;
};

/** 执行链路：只把真实按键交给当前会话，由后端完成风险确认。 */
function execute(command: CommandItem, runtime: CommandRuntime) {
  return runtime.runAction(
    `execute-${command.id}`,
    () => api.executeCommand(
      command.lhs,
      runtime.context.sessionId || undefined,
    ),
    `已发送 ${command.lhs}`,
  );
}

/** 复制链路：未连接会话时也能取得真实按键说明。 */
function copy(command: CommandItem, runtime: CommandRuntime) {
  return runtime.runAction(
    `copy-${command.id}`,
    () => api.copyCommand(command.lhs),
    `已复制；在 Neovim 普通模式依次按下 ${command.lhs}`,
  );
}

/** 搜索与详情工作区：候选、选中命令和动作读取同一份状态。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
export function CommandWorkspace({ search, runtime }: {
  search: ReturnType<typeof useCommandSearch>;
  runtime: CommandRuntime;
}) {
  const actions: CommandActions = {
    onCopy: (command) => copy(command, runtime),
    onExecute: (command) => execute(command, runtime),
    onSaveOverride: (value) => runtime.runAction(
      `override-${value.commandId}`,
      async () => {
        await api.saveCommandOverride(value);
        await search.refresh();
      },
      "中文别名与助记已保存",
    ),
  };
  return (
    <section className="command-workspace" aria-label="命令词典">
      <QueryPanel search={search} />
      <CommandDetail
        command={search.searching ? undefined : search.selected}
        runtime={runtime}
        actions={actions}
      />
    </section>
  );
}

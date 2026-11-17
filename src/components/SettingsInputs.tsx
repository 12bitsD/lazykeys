import { useEffect, useId, useState } from "react";
import type { KeyboardEvent } from "react";
import { api } from "../api";
import type { AppSettings } from "../types";

/** 设置快照统一空坐标表示，避免 undefined 与 null 造成假修改。 */
export function comparableSettings(settings: AppSettings): string {
  return JSON.stringify({
    ...settings,
    overlayX: settings.overlayX ?? null,
    overlayY: settings.overlayY ?? null,
  });
}

/** 恢复默认时保留默认 Profile，只重置用户可配置行为。 */
export function createDefaultSettings(defaultProfile: string): AppSettings {
  return {
    schemaVersion: 1,
    defaultProfile,
    displayStrategy: "smart",
    clickThrough: true,
    opacity: 97,
    fontSize: 13,
    overlayWidth: 420,
    alwaysOnTop: true,
    resultDurationMs: 1400,
    showMnemonic: true,
    showHistory: true,
    queryShortcut: "Control+Option+K",
    overlayShortcut: "Control+Option+J",
    confirmEdits: true,
    includeCwd: true,
  };
}

/** 数值设置的展示与约束表，供输入和保存前校验共用。 */
export const numericSettingMeta = {
  opacity: { label: "透明度（40–100）", min: 40, max: 100, step: 1 },
  fontSize: { label: "字号（10–24）", min: 10, max: 24, step: 1 },
  overlayWidth: { label: "宽度（320–560）", min: 320, max: 560, step: 1 },
  resultDurationMs: {
    label: "结果显示时长（毫秒）",
    min: 500,
    max: 5000,
    step: 100,
  },
} as const;

/** 可由数值设置表驱动的字段名集合。 */
export type NumericSettingKey = keyof typeof numericSettingMeta;

/** 把结构化设置转换为输入文本，加载与恢复草稿共用。 */
export function numberTextFrom(
  settings: AppSettings | null,
): Record<NumericSettingKey, string> {
  return {
    opacity: String(settings?.opacity ?? 97),
    fontSize: String(settings?.fontSize ?? 13),
    overlayWidth: String(settings?.overlayWidth ?? 420),
    resultDurationMs: String(settings?.resultDurationMs ?? 1400),
  };
}

/** 只接受配置范围内的整数，输入过程仍可暂时保留非法文本。 */
export function validNumericSetting(key: NumericSettingKey, value: string) {
  const parsed = Number(value);
  const meta = numericSettingMeta[key];
  return value.trim() !== ""
    && Number.isInteger(parsed)
    && parsed >= meta.min
    && parsed <= meta.max;
}

/** 单个数值设置：失焦后展示错误，合法输入进入草稿。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
export function NumberSetting({ setting, value, onChange }: {
  setting: NumericSettingKey;
  value: string;
  onChange: (value: string) => void;
}) {
  const meta = numericSettingMeta[setting];
  const errorId = useId();
  const [blurred, setBlurred] = useState(false);
  const valid = validNumericSetting(setting, value);
  useEffect(() => {
    if (valid) {
      setBlurred(false);
    }
  }, [valid]);
  return (
    <label>{meta.label}
      <input
        type="number"
        min={meta.min}
        max={meta.max}
        step={meta.step}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        onBlur={() => setBlurred(true)}
        aria-invalid={blurred && !valid}
        aria-describedby={blurred && !valid ? errorId : undefined}
      />
      {blurred && !valid && (
        <small className="number-setting-error" id={errorId}>
          请输入 {meta.min}–{meta.max} 之间的整数
        </small>
      )}
    </label>
  );
}

/** 快捷键录制状态：空闲或正在捕获，决定全局快捷键是否暂停。 */
type CaptureStatus = "idle" | "recording";

/** 系统捕获生命周期所需的当前状态、错误出口与停止入口。 */
type CaptureOptions = {
  status: CaptureStatus;
  onError: (message: string) => void;
  onStop: (status: CaptureStatus) => void;
};

/** 录制期间暂停全局快捷键，结束或卸载时恢复。 */
function useShortcutCapture({ status, onError, onStop }: CaptureOptions) {
  useEffect(() => {
    if (status === "idle") {
      return;
    }
    let active = true;
    let captured = false;
    const finishCapture = () => api.endShortcutCapture().catch((reason) => {
      onError(reason instanceof Error ? reason.message : String(reason));
    });
    api.beginShortcutCapture().then(() => {
      captured = true;
      if (!active) {
        void finishCapture();
      }
    }).catch((reason) => {
      if (active) {
        onStop("idle");
        onError(reason instanceof Error ? reason.message : String(reason));
      }
    });
    return () => {
      active = false;
      if (captured) {
        void finishCapture();
      }
    };
  }, [status, onError, onStop]);
}

/** 将键盘事件归一为可保存的组合键文本。 */
function shortcutFromEvent(event: KeyboardEvent<HTMLInputElement>) {
  const modifiers = [
    event.ctrlKey ? "Control" : "",
    event.altKey ? "Option" : "",
    event.metaKey ? "Command" : "",
    event.shiftKey ? "Shift" : "",
  ].filter(Boolean);
  if (modifiers.length === 0) {
    return "";
  }
  const specialKeys: Record<string, string> = {
    " ": "Space",
    Enter: "Enter",
    Backspace: "Backspace",
    Delete: "Delete",
    ArrowUp: "ArrowUp",
    ArrowDown: "ArrowDown",
    ArrowLeft: "ArrowLeft",
    ArrowRight: "ArrowRight",
  };
  const key = specialKeys[event.key]
    ?? (event.key.length === 1 ? event.key.toUpperCase() : event.key);
  return [...modifiers, key].join("+");
}

/** 键盘事件可触发的组合键写入、停止录制与错误反馈动作。 */
type ShortcutKeyActions = {
  onChange: (value: string) => void;
  onStop: (status: CaptureStatus) => void;
  onError: (message: string) => void;
};

/** 键盘事件适配器只处理录入，不管理全局快捷键生命周期。 */
function handleShortcutKey(
  event: KeyboardEvent<HTMLInputElement>,
  status: CaptureStatus,
  actions: ShortcutKeyActions,
) {
  if (event.key === "Tab") {
    actions.onStop("idle");
    actions.onError("");
    return;
  }
  if (status === "idle") {
    return;
  }
  event.preventDefault();
  if (event.key === "Escape") {
    actions.onStop("idle");
    actions.onError("");
    event.currentTarget.blur();
    return;
  }
  if (event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) {
    return;
  }
  if (["Control", "Alt", "Meta", "Shift"].includes(event.key)) {
    return;
  }
  const shortcut = shortcutFromEvent(event);
  if (!shortcut) {
    actions.onError("请至少按住 Control、Option、Command 或 Shift 中的一项");
    return;
  }
  actions.onChange(shortcut);
  actions.onStop("idle");
  actions.onError("");
  event.currentTarget.blur();
}

/** 组合键录入器：聚焦后记录真实按键，Esc 取消，Tab 离开。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
export function ShortcutInput({ value, onChange, label }: {
  value: string;
  onChange: (value: string) => void;
  label: string;
}) {
  const hintId = useId();
  const [status, setStatus] = useState<CaptureStatus>("idle");
  const [error, setError] = useState("");
  const recording = status === "recording";
  useShortcutCapture({ status, onError: setError, onStop: setStatus });
  return (
    <span className="shortcut-input">
      <input
        aria-label={label}
        aria-describedby={recording || error ? hintId : undefined}
        aria-invalid={Boolean(error)}
        readOnly
        value={value}
        onFocus={() => {
          setStatus("recording");
          setError("");
        }}
        onBlur={() => {
          setStatus("idle");
          setError("");
        }}
        onKeyDown={(event) => handleShortcutKey(event, status, {
          onChange,
          onStop: setStatus,
          onError: setError,
        })}
      />
      {(recording || error) && (
        <small id={hintId} role={error ? "alert" : "status"}>
          {error || "请按下完整组合键；Esc 取消，Tab 离开。"}
        </small>
      )}
    </span>
  );
}

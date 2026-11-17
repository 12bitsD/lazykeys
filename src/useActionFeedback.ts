import { useEffect, useRef, useState } from "react";
import type { Dispatch, SetStateAction } from "react";

/** 操作结果的统一出口；类型决定提示的播报与消失方式。 */
export type Notice = {
  message: string;
  tone: "success" | "error" | "info";
};

/** 全局操作入口的函数契约，供各工作区共享互斥与反馈。 */
export type RunAction = <T>(
  name: string,
  action: () => Promise<T>,
  success: string | ((result: T) => string),
) => Promise<void>;

/** 提示计时器的输入集合：当前提示、焦点状态及对应状态写入口。 */
type NoticeTimerOptions = {
  notice: Notice | null;
  noticeFocused: boolean;
  setNotice: Dispatch<SetStateAction<Notice | null>>;
  setNoticeFocused: Dispatch<SetStateAction<boolean>>;
};

/** 提示生命周期：普通提示四秒消失，聚焦时暂停，错误持续显示。 */
function useNoticeTimer({
  notice,
  noticeFocused,
  setNotice,
  setNoticeFocused,
}: NoticeTimerOptions) {
  useEffect(() => {
    if (!notice) {
      setNoticeFocused(false);
      return;
    }
    if (notice.tone === "error" || noticeFocused) {
      return;
    }
    const timer = window.setTimeout(() => {
      setNotice((current) => current === notice ? null : current);
    }, 4000);
    return () => window.clearTimeout(timer);
  }, [notice, noticeFocused, setNotice, setNoticeFocused]);
}

/** 操作链路的反馈层：串行执行动作，并分别提供忙碌状态与结果提示。 */
export function useActionFeedback() {
  const [busy, setBusy] = useState("");
  const [notice, setNotice] = useState<Notice | null>(null);
  const [noticeFocused, setNoticeFocused] = useState(false);
  const activeAction = useRef("");
  useNoticeTimer({ notice, noticeFocused, setNotice, setNoticeFocused });

  /** 把未知异常转换为可持续显示的错误提示。 */
  function reportError(error: unknown) {
    setNotice({
      message: error instanceof Error ? error.message : String(error),
      tone: "error",
    });
  }

  /** 全局互斥闸门：同一时刻只执行一个动作，并统一处理反馈与清理。 */
  async function runAction<T>(
    name: string,
    action: () => Promise<T>,
    success: string | ((result: T) => string),
  ) {
    if (activeAction.current) {
      setNotice({ message: "当前有操作正在进行，请等待完成后再试", tone: "info" });
      return;
    }
    activeAction.current = name;
    setBusy(name);
    setNotice(null);
    try {
      const result = await action();
      const message = typeof success === "function" ? success(result) : success;
      setNotice({ message, tone: "success" });
    } catch (error) {
      reportError(error);
    } finally {
      activeAction.current = "";
      setBusy("");
    }
  }

  return {
    busy,
    notice,
    setNotice,
    noticeFocused,
    setNoticeFocused,
    runAction,
    reportError,
  };
}

/** 把可持久化的组合键文本转换为 macOS 符号，仅用于展示。 */
export function formatShortcut(shortcut: string): string {
  return shortcut
    .replace(/Control/g, "⌃")
    .replace(/Option/g, "⌥")
    .replace(/Command/g, "⌘")
    .replace(/Shift/g, "⇧")
    .replace(/\+/g, "");
}

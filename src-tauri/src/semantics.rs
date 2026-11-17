use crate::models::{CommandItem, ContextKind, EditorContext, PanelSnapshot};

/// 上下文到面板的组装入口：优先解释已完成输入，否则解析当前前缀，并附上退出路径。
pub fn snapshot(
    context: &EditorContext,
    query_shortcut: &str,
    completed: Option<CommandItem>,
) -> PanelSnapshot {
    let (semantic, next) = if context.pending.is_empty()
        && !context.last_input.is_empty()
    {
        completed.map_or_else(
            || {
                (
                    format!("最近输入 {}", context.last_input),
                    "没有可用的完成语义".into(),
                )
            },
            |command| {
                (
                    format!("{} → {}", context.last_input, command.description),
                    command.effect,
                )
            },
        )
    } else {
        explain_prefix(context)
    };
    PanelSnapshot {
        kind: context.kind.clone(),
        session_id: context.session_id.clone(),
        profile_id: context.profile_id.clone(),
        mode: context.mode.clone(),
        title: context.title.clone(),
        pending: context.pending.clone(),
        last_input: context.last_input.clone(),
        semantic,
        next,
        exit_hint: exit_hint(context, query_shortcut),
        cwd: context.cwd.clone(),
        buffer_modified: context.buffer_modified,
    }
}

/// 上下文解释核心：把页面、模式或未完成按键归成「当前语义 + 下一步动作」。
fn explain_prefix(context: &EditorContext) -> (String, String) {
    let pending = context.pending.as_str();
    if pending.is_empty() {
        return match context.kind {
            ContextKind::WhichKey => (
                "命令菜单：继续按键执行，或返回上级".into(),
                "选择下一级命令".into(),
            ),
            ContextKind::SnacksInput => (
                "正在 Snacks 搜索框中输入".into(),
                "Enter 确认当前项，或关闭搜索".into(),
            ),
            ContextKind::SnacksList | ContextKind::SnacksPreview => (
                "正在浏览 Snacks 搜索结果".into(),
                "移动选择，Enter 打开当前项".into(),
            ),
            ContextKind::Lazy => {
                ("lazy.nvim 插件管理页面".into(), "查看或管理插件".into())
            }
            ContextKind::Mason => {
                ("Mason 外部工具管理页面".into(), "查看或管理外部工具".into())
            }
            ContextKind::Help => {
                ("Neovim 帮助页面".into(), "阅读帮助或关闭当前窗口".into())
            }
            ContextKind::Quickfix => {
                ("Quickfix 列表".into(), "选择条目或关闭列表".into())
            }
            ContextKind::Terminal => {
                ("Neovim 内置终端".into(), "先返回 Normal 再管理窗口".into())
            }
            ContextKind::Unknown => {
                ("未识别的插件页面".into(), "不猜测当前页面语义".into())
            }
            ContextKind::Normal => match context.mode.as_str() {
                "i" => (
                    "Insert：正在输入文本".into(),
                    "输入文本，或返回 Normal".into(),
                ),
                "v" | "V" => {
                    ("Visual：正在选择文本".into(), "调整选择或执行操作".into())
                }
                "c" => (
                    "Command：正在输入命令".into(),
                    "Enter 执行，或取消命令行".into(),
                ),
                "t" => (
                    "Terminal：正在与终端进程交互".into(),
                    "输入终端内容，或返回 Normal".into(),
                ),
                _ => (
                    "Normal：可以执行编辑和导航命令".into(),
                    "输入命令，或按快捷键查询".into(),
                ),
            },
        };
    }

    if pending.starts_with("<leader>") || pending.starts_with(' ') {
        let leader_pending =
            pending.trim_start_matches("<leader>").trim_start();
        let group = match leader_pending.chars().next() {
            Some('f') => "File / 文件相关",
            Some('s') => "Search / 搜索相关",
            Some('c') => "Code / 编码相关",
            Some('b') => "Buffer / 缓冲区相关",
            Some('g') => "Git / 版本控制相关",
            Some('u') => "UI / 界面开关",
            Some('q') => "Quit / Session / 退出相关",
            Some('x') => "Diagnostics / 问题列表",
            Some('d') => "Debug / 调试相关",
            _ => "Leader 命令前缀",
        };
        return (
            format!("Leader → {group}"),
            "选择完整命令；Esc 取消，Backspace 返回上级".into(),
        );
    }

    let operator = pending.chars().next().unwrap_or_default();
    if matches!(operator, 'd' | 'c' | 'y') {
        let action = match operator {
            'd' => "delete 删除",
            'c' => "change 修改并进入 Insert",
            'y' => "yank 复制",
            _ => unreachable!(),
        };
        let scope = match pending.chars().nth(1) {
            Some('i') => "inside 内部，不包含边界",
            Some('a') => "around 周围，包含边界",
            _ => "等待 motion 或 text object",
        };
        let object = match pending.chars().nth(2) {
            Some('w') => "word 单词",
            Some('p') => "paragraph 段落",
            Some('"') => "双引号内容",
            Some('\'') => "单引号内容",
            Some('(') | Some(')') | Some('b') => "括号内容",
            _ => "选择作用对象",
        };
        return (format!("{action} · {scope}"), object.into());
    }

    ("正在解析当前命令前缀".into(), "继续输入；Esc 取消".into())
}

/// 面板解释的最后一环：按页面、输入状态和修改状态给出最短可退出路径。
fn exit_hint(context: &EditorContext, query_shortcut: &str) -> String {
    if let Some(key) = context.exit_key.as_deref().filter(|key| !key.is_empty())
    {
        return format!("{key} 关闭当前页面");
    }
    if !context.pending.is_empty() {
        return "Esc 取消当前输入 · Backspace 返回上级".into();
    }
    match context.kind {
        ContextKind::WhichKey => "Esc 关闭菜单 · Backspace 返回上一级".into(),
        ContextKind::SnacksInput if context.mode.starts_with('i') => {
            "Ctrl-c 关闭搜索 · Esc 先返回 Normal".into()
        }
        ContextKind::SnacksInput | ContextKind::SnacksList => {
            "Esc 关闭搜索".into()
        }
        ContextKind::SnacksPreview => "q / Esc 关闭预览".into(),
        ContextKind::Lazy | ContextKind::Mason => {
            "q 关闭当前页面 · g? 查看页面帮助".into()
        }
        ContextKind::Help | ContextKind::Quickfix => {
            if context.buffer_modified {
                ":q 关闭窗口（未保存时会被阻止）".into()
            } else {
                ":q 关闭当前窗口".into()
            }
        }
        ContextKind::Terminal => "Ctrl-\\ Ctrl-n 返回 Normal".into(),
        ContextKind::Unknown => {
            "Esc 返回 Normal（如适用） · 未识别当前插件".into()
        }
        ContextKind::Normal => match context.mode.as_str() {
            "i" | "v" | "V" | "c" => "Esc 返回 Normal".into(),
            "t" => "Ctrl-\\ Ctrl-n 返回 Normal".into(),
            _ if context.buffer_modified => {
                ":w 保存 · :q 关闭窗口 · :q! 丢弃修改".into()
            }
            _ => format!(":q 关闭当前窗口 · {query_shortcut} 查询命令"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(kind: ContextKind, mode: &str) -> EditorContext {
        EditorContext {
            kind,
            mode: mode.into(),
            ..EditorContext::default()
        }
    }

    #[test]
    fn explains_operator_and_text_object() {
        let mut value = context(ContextKind::Normal, "no");
        value.pending = "di".into();
        let panel = snapshot(&value, "Control+Option+K", None);
        assert!(panel.semantic.contains("delete"));
        assert!(panel.semantic.contains("inside"));
        assert!(panel.next.contains("作用对象"));
        assert!(panel.exit_hint.contains("Esc"));
    }

    #[test]
    fn snacks_exit_depends_on_mode() {
        let insert = snapshot(
            &context(ContextKind::SnacksInput, "i"),
            "Control+Option+K",
            None,
        );
        assert!(insert.exit_hint.contains("Ctrl-c"));
        let normal = snapshot(
            &context(ContextKind::SnacksInput, "n"),
            "Control+Option+K",
            None,
        );
        assert_eq!(normal.exit_hint, "Esc 关闭搜索");
    }

    #[test]
    fn modified_file_distinguishes_discard() {
        let mut value = context(ContextKind::Normal, "n");
        value.buffer_modified = true;
        let panel = snapshot(&value, "Control+Option+K", None);
        assert!(panel.exit_hint.contains(":w"));
        assert!(panel.exit_hint.contains(":q!"));
    }

    #[test]
    fn completed_input_shows_real_command_result() {
        let mut value = context(ContextKind::Normal, "n");
        value.last_input = "diw".into();
        let command = crate::dictionary::Dictionary::default()
            .resolve_lhs("diw", &[])
            .unwrap();
        let panel = snapshot(&value, "Control+Option+K", Some(command));
        assert!(
            panel.semantic.contains("diw → 删除光标所在的单词"),
            "{}",
            panel.semantic
        );
        assert!(panel.next.contains("register"));
    }

    #[test]
    fn explains_leader_groups() {
        let mut value = context(ContextKind::Normal, "n");
        value.pending = "<leader>f".into();
        let panel = snapshot(&value, "Control+Option+K", None);
        assert!(panel.semantic.contains("File"));
        assert!(panel.exit_hint.contains("Backspace"));
    }
}

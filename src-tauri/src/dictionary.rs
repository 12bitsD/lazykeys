use crate::models::{
    CommandItem, CommandOverride, RiskLevel, RuntimeMapping, SearchResult,
};

macro_rules! command {
    (
        $id:expr, $lhs:expr, $title:expr, $description:expr, $mnemonic:expr,
        $mode:expr, $effect:expr, $risk:expr, $aliases:expr
    ) => {
        CommandItem {
            id: $id.into(),
            lhs: $lhs.into(),
            title: $title.into(),
            description: $description.into(),
            mnemonic: $mnemonic.into(),
            mode: $mode.into(),
            effect: $effect.into(),
            risk: $risk,
            aliases: $aliases.iter().map(|value| value.to_string()).collect(),
            source: "LazyVim 默认".into(),
        }
    };
}

/// 搜索链路的命令数据源：持有内置命令，运行时与 Neovim 映射合并后供检索和执行解析。
pub struct Dictionary {
    commands: Vec<CommandItem>,
}

impl Default for Dictionary {
    /// 搜索数据源的初始化：装载内置命令，等待运行时映射和用户别名叠加。
    fn default() -> Self {
        Self {
            commands: builtin_commands(),
        }
    }
}

impl Dictionary {
    /// 搜索与展示的基础读取：返回未合并运行时映射的内置命令集合。
    pub fn all(&self) -> &[CommandItem] {
        &self.commands
    }

    /// 设置到搜索的定制入口：更新内置命令的别名和助记，并返回更新后的可展示快照。
    pub fn apply_override(
        &mut self,
        value: &CommandOverride,
    ) -> Result<CommandItem, String> {
        let command = self
            .commands
            .iter_mut()
            .find(|command| command.id == value.command_id)
            .ok_or("只能覆盖内置词典命令；运行时映射请修改 Neovim 配置")?;
        command.aliases = value.aliases.clone();
        command.mnemonic = value.mnemonic.clone();
        Ok(command.clone())
    }

    /// 执行前的命令解析：合并实际映射后按规范化按键找到命令，复用其风险与效果说明。
    pub fn resolve_lhs(
        &self,
        lhs: &str,
        mappings: &[RuntimeMapping],
    ) -> Option<CommandItem> {
        let normalized = normalize_lhs(lhs);
        merge_runtime_mappings(self.commands.clone(), mappings)
            .into_iter()
            .find(|item| normalize_lhs(&item.lhs) == normalized)
    }

    /// 搜索主入口：把内置命令与当前 Neovim 映射合并，再按查询返回排序候选。
    pub fn search_with_mappings(
        &self,
        query: &str,
        mappings: &[RuntimeMapping],
    ) -> Vec<SearchResult> {
        let commands = self.commands.clone();
        let mut merged = merge_runtime_mappings(commands, mappings);
        merged.sort_by(|a, b| a.title.cmp(&b.title));
        search_commands(&merged, query)
    }
}

/// 搜索排序阶段：为命令打分，按得分和标题排序后截取前 12 条。
fn search_commands(commands: &[CommandItem], query: &str) -> Vec<SearchResult> {
    let query = query.trim().to_lowercase();
    let mut results: Vec<_> = commands
        .iter()
        .filter_map(|item| {
            let score = score_command(item, &query)?;
            Some(SearchResult {
                item: item.clone(),
                score,
            })
        })
        .collect();
    results.sort_by(|a, b| {
        b.score.cmp(&a.score).then(a.item.title.cmp(&b.item.title))
    });
    results.truncate(12);
    results
}

/// 运行时映射的来源只决定展示归属，不改变真实按键。
fn mapping_source(mapping: &RuntimeMapping) -> String {
    if mapping.scope == "buffer" {
        "当前 Buffer".into()
    } else {
        "当前 Profile".into()
    }
}

/// 风险合并取较高等级，运行时映射不能降低内置命令的确认要求。
fn combined_risk(builtin: RiskLevel, runtime: RiskLevel) -> RiskLevel {
    match (builtin, runtime) {
        (RiskLevel::Destructive, _) | (_, RiskLevel::Destructive) => {
            RiskLevel::Destructive
        }
        (RiskLevel::Edit, _) | (_, RiskLevel::Edit) => RiskLevel::Edit,
        _ => RiskLevel::Read,
    }
}

/// 已有命令只更新运行时描述、来源与风险，保留内置助记和别名。
fn update_existing_command(
    existing: &mut CommandItem,
    mapping: &RuntimeMapping,
) {
    existing.lhs = mapping.lhs.clone();
    existing.source = mapping_source(mapping);
    if !mapping.desc.trim().is_empty() {
        existing.description = mapping.desc.clone();
    }
    existing.risk = combined_risk(existing.risk, runtime_risk(mapping));
}

/// 未知运行时映射转换为完整命令，缺失语义显式标记而不猜测。
fn runtime_command(mapping: &RuntimeMapping) -> CommandItem {
    let description = if mapping.desc.trim().is_empty() {
        "当前配置中的映射，暂无中文语义".to_string()
    } else {
        mapping.desc.clone()
    };
    CommandItem {
        id: format!(
            "runtime:{}:{}:{}",
            mapping.scope, mapping.mode, mapping.lhs
        ),
        lhs: mapping.lhs.clone(),
        title: description.clone(),
        description,
        mnemonic: "当前配置，无内置助记".into(),
        mode: mapping.mode.clone(),
        effect: if mapping.rhs.is_empty() {
            "执行当前映射".into()
        } else {
            mapping.rhs.clone()
        },
        risk: runtime_risk(mapping),
        aliases: Vec::new(),
        source: mapping_source(mapping),
    }
}

/// 上下文到搜索的合并阶段：用实际映射覆盖同名内置命令，并补齐未知命令与风险。
fn merge_runtime_mappings(
    mut commands: Vec<CommandItem>,
    mappings: &[RuntimeMapping],
) -> Vec<CommandItem> {
    for mapping in mappings {
        let normalized_lhs = normalize_lhs(&mapping.lhs);
        if let Some(existing) = commands
            .iter_mut()
            .find(|item| normalize_lhs(&item.lhs) == normalized_lhs)
        {
            update_existing_command(existing, mapping);
            continue;
        }
        commands.push(runtime_command(mapping));
    }
    commands
}

/// 搜索与执行解析的按键归一化：统一 Leader 写法与大小写以便精确匹配。
fn normalize_lhs(lhs: &str) -> String {
    lhs.replace("<Space>", "<leader>").to_lowercase()
}

/// 执行确认前的保守风险推断：从映射文本识别破坏性特征，其余运行时命令至少按修改处理。
fn runtime_risk(mapping: &RuntimeMapping) -> RiskLevel {
    let value = format!("{} {} {}", mapping.lhs, mapping.rhs, mapping.desc)
        .to_lowercase()
        .replace("<cmd>", ":")
        .replace("<cr>", "");
    if [
        ":q!",
        ":qa!",
        "qa!",
        ":qall!",
        "quitall!",
        "uninstall",
        "force",
        ":!rm",
        "bwipeout!",
        "os.execute",
    ]
    .iter()
    .any(|term| value.contains(term))
    {
        RiskLevel::Destructive
    } else {
        RiskLevel::Edit
    }
}

/// 搜索匹配阶段：依次比较按键、标题、别名和语义字段，产出候选排序分。
fn score_command(item: &CommandItem, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(1);
    }
    let lhs = item.lhs.to_lowercase();
    let readable_lhs = lhs.replace("<leader>", "space ");
    if lhs == query || readable_lhs == query {
        return Some(1000);
    }
    if item.id == query {
        return Some(950);
    }
    if item.title.to_lowercase().contains(query) {
        return Some(800 - item.title.len() as i32);
    }
    if lhs.contains(query) || readable_lhs.contains(query) {
        return Some(700);
    }
    if item
        .aliases
        .iter()
        .any(|alias| alias.to_lowercase().contains(query))
    {
        return Some(650);
    }
    let haystack = format!(
        "{} {} {} {}",
        item.description, item.mnemonic, item.effect, item.mode
    )
    .to_lowercase();
    let terms: Vec<_> = query.split_whitespace().collect();
    if terms.iter().all(|term| haystack.contains(term)) {
        return Some(500 - terms.len() as i32);
    }
    None
}

/// 搜索链路的静态词表：提供 LazyVim 常用命令的按键、语义、效果与初始风险。
// bits:allow(BCS003): 静态命令数据表，不包含业务控制流程
fn builtin_commands() -> Vec<CommandItem> {
    use RiskLevel::{Destructive, Edit, Read};
    vec![
        command!(
            "insert",
            "i",
            "进入插入模式",
            "在光标前输入文本",
            "i = insert",
            "Normal",
            "进入 Insert，后续输入会写入 buffer",
            Edit,
            &["插入", "输入", "编辑"]
        ),
        command!(
            "escape",
            "<Esc>",
            "返回普通模式",
            "退出当前输入模式或取消未完成命令",
            "escape = 退出当前状态",
            "Insert / Visual / Command",
            "通常返回 Normal；插件页面可能有单独行为",
            Read,
            &["退出", "取消", "普通模式", "esc"]
        ),
        command!(
            "save",
            ":w",
            "保存当前文件",
            "把当前 buffer 写入磁盘",
            "w = write",
            "Command",
            "写入当前文件，不退出 Neovim",
            Edit,
            &["保存", "写入", "write"]
        ),
        command!(
            "quit",
            ":q",
            "关闭当前窗口",
            "关闭当前窗口；有未保存修改时会被阻止",
            "q = quit",
            "Command",
            "不会丢弃未保存修改，但会关闭当前窗口",
            Edit,
            &["退出", "关闭页面", "关闭窗口"]
        ),
        command!(
            "save-quit",
            ":wq",
            "保存并退出",
            "先保存当前文件，再关闭窗口",
            "w + q = write + quit",
            "Command",
            "写入文件后退出",
            Edit,
            &["保存退出", "wq"]
        ),
        command!(
            "force-quit",
            ":q!",
            "放弃修改并退出",
            "不保存当前 buffer 的修改并强制退出",
            "q! = 强制 quit",
            "Command",
            "未保存修改会永久丢失",
            Destructive,
            &["强制退出", "不保存", "放弃修改"]
        ),
        command!(
            "undo",
            "u",
            "撤销",
            "撤销上一次修改",
            "u = undo",
            "Normal",
            "回退 buffer 的修改历史",
            Edit,
            &["撤销", "undo"]
        ),
        command!(
            "redo",
            "<C-r>",
            "重做",
            "恢复被撤销的修改",
            "Ctrl + r = redo",
            "Normal",
            "前进 buffer 的修改历史",
            Edit,
            &["重做", "redo", "ctrl r"]
        ),
        command!(
            "word-next",
            "w",
            "移到下一个单词",
            "把光标移到下一个单词开头",
            "w = word",
            "Normal / Visual",
            "只移动光标，不修改文本",
            Read,
            &["移动", "单词", "word"]
        ),
        command!(
            "word-back",
            "b",
            "回到上一个单词",
            "把光标移到上一个单词开头",
            "b = back",
            "Normal / Visual",
            "只移动光标，不修改文本",
            Read,
            &["后退", "单词", "back"]
        ),
        command!(
            "line-start",
            "0",
            "移到行首",
            "把光标移到当前行第一个字符",
            "0 = 行首",
            "Normal / Visual",
            "只移动光标",
            Read,
            &["行首", "移动"]
        ),
        command!(
            "line-end",
            "$",
            "移到行尾",
            "把光标移到当前行末尾",
            "$ = 行尾",
            "Normal / Visual",
            "只移动光标",
            Read,
            &["行尾", "移动"]
        ),
        command!(
            "file-start",
            "gg",
            "移到文件开头",
            "把光标移到第一行",
            "gg = go to top",
            "Normal / Visual",
            "只移动光标",
            Read,
            &["文件开头", "顶部"]
        ),
        command!(
            "file-end",
            "G",
            "移到文件末尾",
            "把光标移到最后一行",
            "G = go to bottom",
            "Normal / Visual",
            "只移动光标",
            Read,
            &["文件末尾", "底部"]
        ),
        command!(
            "search",
            "/",
            "搜索当前文件",
            "输入搜索模式并查找匹配文本",
            "/ = search",
            "Normal",
            "进入搜索命令行；Enter 执行，Esc 取消",
            Read,
            &["搜索", "查找", "search"]
        ),
        command!(
            "search-next",
            "n",
            "下一个搜索结果",
            "沿当前搜索方向移动",
            "n = next",
            "Normal",
            "移动到下一条匹配",
            Read,
            &["下一个", "搜索结果"]
        ),
        command!(
            "search-prev",
            "N",
            "上一个搜索结果",
            "沿当前搜索反方向移动",
            "N = previous",
            "Normal",
            "移动到上一条匹配",
            Read,
            &["上一个", "搜索结果"]
        ),
        command!(
            "repeat",
            ".",
            "重复上次修改",
            "重复最近一次文本修改命令",
            ". = dot repeat",
            "Normal",
            "可能重复删除、粘贴或其他修改",
            Edit,
            &["重复", "dot"]
        ),
        command!(
            "visual",
            "v",
            "字符选择",
            "进入按字符选择的 Visual 模式",
            "v = visual",
            "Normal",
            "进入 Visual；Esc 返回 Normal",
            Read,
            &["选择", "可视模式"]
        ),
        command!(
            "visual-line",
            "V",
            "整行选择",
            "进入按行选择的 Visual Line 模式",
            "V = visual line",
            "Normal",
            "进入整行选择；Esc 返回 Normal",
            Read,
            &["行选择", "可视行"]
        ),
        command!(
            "delete-word",
            "diw",
            "删除当前单词",
            "删除光标所在的单词，不包含单词外空格",
            "d = delete，i = inside，w = word",
            "Normal",
            "删除文本并存入 register，可粘贴或撤销",
            Edit,
            &["删除单词", "diw", "delete inside word"]
        ),
        command!(
            "change-word",
            "ciw",
            "修改当前单词",
            "删除当前单词并立即进入 Insert",
            "c = change，i = inside，w = word",
            "Normal",
            "删除文本后等待输入新内容",
            Edit,
            &["修改单词", "改词", "ciw", "change inside word"]
        ),
        command!(
            "change-quotes",
            "ci\"",
            "修改引号内容",
            "删除引号内部内容并进入 Insert，保留引号",
            "c = change，i = inside，\" = 引号",
            "Normal",
            "删除引号内文本，不删除边界",
            Edit,
            &["修改引号", "引号内容", "change inside quotes"]
        ),
        command!(
            "yank-paragraph",
            "yap",
            "复制当前段落",
            "复制当前段落及其边界",
            "y = yank，a = around，p = paragraph",
            "Normal",
            "写入 register，不修改原文",
            Read,
            &["复制段落", "yap"]
        ),
        command!(
            "delete-line",
            "dd",
            "删除当前行",
            "删除整行并存入 register",
            "d + d = 对整行执行 delete",
            "Normal",
            "删除文本，可粘贴或撤销",
            Edit,
            &["删除行", "dd"]
        ),
        command!(
            "yank-line",
            "yy",
            "复制当前行",
            "复制整行到 register",
            "y + y = 对整行执行 yank",
            "Normal",
            "不修改原文",
            Read,
            &["复制行", "yy"]
        ),
        command!(
            "paste",
            "p",
            "在光标后粘贴",
            "把 register 内容放到光标之后",
            "p = put",
            "Normal",
            "修改 buffer，可撤销",
            Edit,
            &["粘贴", "put"]
        ),
        command!(
            "find-files",
            "<leader><space>",
            "查找文件",
            "在项目根目录打开文件搜索",
            "Leader → 快速文件入口",
            "Normal",
            "打开 Snacks Picker，不修改文件",
            Read,
            &["找文件", "文件搜索", "space space", "find files"]
        ),
        command!(
            "grep",
            "<leader>/",
            "全局搜索",
            "在项目根目录按文本内容搜索",
            "Leader + / = 项目搜索",
            "Normal",
            "打开 Grep Picker",
            Read,
            &["全局搜索", "项目搜索", "grep", "space /"]
        ),
        command!(
            "explorer",
            "<leader>e",
            "文件树",
            "打开或关闭项目根目录文件树",
            "Leader + e = explorer",
            "Normal",
            "切换 Snacks Explorer",
            Read,
            &["文件树", "资源管理器", "explorer", "space e"]
        ),
        command!(
            "find-files-alt",
            "<leader>ff",
            "查找文件（完整按键）",
            "与快速文件入口相同，打开文件搜索",
            "Leader + f + f = file + find",
            "Normal",
            "打开文件搜索",
            Read,
            &["找文件", "space ff", "find files"]
        ),
        command!(
            "recent-files",
            "<leader>fr",
            "最近文件",
            "列出最近打开的文件",
            "Leader + f + r = file + recent",
            "Normal",
            "打开最近文件 Picker",
            Read,
            &["最近文件", "历史文件", "space fr"]
        ),
        command!(
            "delete-buffer",
            "<leader>bd",
            "关闭当前 Buffer",
            "删除当前 buffer，保留其他窗口和 buffer",
            "Leader + b + d = buffer + delete",
            "Normal",
            "关闭 buffer；未保存修改需要处理",
            Edit,
            &["关闭buffer", "删除buffer", "space bd"]
        ),
        command!(
            "definition",
            "gd",
            "跳到定义",
            "跳转到光标符号的定义",
            "g + d = goto definition",
            "Normal",
            "移动光标或打开位置列表",
            Read,
            &["定义", "跳转", "goto definition"]
        ),
        command!(
            "references",
            "gr",
            "查看引用",
            "列出光标符号的引用",
            "g + r = goto references",
            "Normal",
            "打开引用结果",
            Read,
            &["引用", "references"]
        ),
        command!(
            "hover",
            "K",
            "查看符号文档",
            "显示光标符号的类型、签名或文档",
            "K = keyword / hover",
            "Normal",
            "打开 hover；再次操作或按页面提示退出",
            Read,
            &["文档", "悬浮", "hover"]
        ),
        command!(
            "code-action",
            "<leader>ca",
            "代码操作",
            "显示当前位置可用的修复或重构动作",
            "Leader + c + a = code + action",
            "Normal / Visual",
            "打开 Code Action 菜单",
            Read,
            &["代码操作", "修复", "code action", "space ca"]
        ),
        command!(
            "rename",
            "<leader>cr",
            "重命名符号",
            "通过 LSP 重命名当前符号及相关引用",
            "Leader + c + r = code + rename",
            "Normal",
            "修改多个位置的文本，执行前检查范围",
            Edit,
            &["重命名", "rename", "space cr"]
        ),
        command!(
            "format",
            "<leader>cf",
            "格式化",
            "使用当前配置的 formatter 格式化代码",
            "Leader + c + f = code + format",
            "Normal / Visual",
            "修改当前 buffer，可撤销",
            Edit,
            &["格式化", "format", "space cf"]
        ),
        command!(
            "diagnostic-prev",
            "[d",
            "上一个诊断",
            "移动到上一个错误、警告或提示",
            "[ + d = previous diagnostic",
            "Normal",
            "移动到上一个诊断",
            Read,
            &["诊断", "错误", "上一个"]
        ),
        command!(
            "diagnostic-next",
            "]d",
            "下一个诊断",
            "移动到下一个错误、警告或提示",
            "] + d = next diagnostic",
            "Normal",
            "移动到下一个诊断",
            Read,
            &["诊断", "错误", "下一个"]
        ),
        command!(
            "lazy",
            "<leader>l",
            "插件管理",
            "打开 lazy.nvim 插件管理页面",
            "Leader + l = lazy",
            "Normal",
            "打开 Lazy UI",
            Read,
            &["插件", "lazy", "space l"]
        ),
        command!(
            "mason",
            "<leader>cm",
            "外部工具管理",
            "打开 Mason，管理 LSP、formatter 等外部工具",
            "Leader + c + m = code + mason",
            "Normal",
            "打开 Mason UI",
            Read,
            &["mason", "工具安装", "space cm"]
        ),
        command!(
            "quit-all",
            "<leader>qq",
            "退出全部窗口",
            "退出整个 Neovim",
            "Leader + q + q = quit + quit",
            "Normal",
            "未保存修改会阻止退出；关闭整个编辑器前需要确认",
            Edit,
            &["退出neovim", "全部退出", "space qq"]
        ),
        command!(
            "buffer-keymaps",
            "<leader>?",
            "当前 Buffer 快捷键",
            "显示当前 buffer 的局部 keymap",
            "Leader + ? = 查询局部按键",
            "Normal",
            "打开 which-key 局部映射",
            Read,
            &["局部快捷键", "buffer keymaps", "space ?"]
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searches_chinese_english_and_actual_keys() {
        let dictionary = Dictionary::default();
        assert_eq!(
            dictionary.search_with_mappings("查找文件", &[])[0].item.id,
            "find-files"
        );
        assert_eq!(
            dictionary.search_with_mappings("references", &[])[0]
                .item
                .id,
            "references"
        );
        assert_eq!(
            dictionary.search_with_mappings("space ff", &[])[0].item.lhs,
            "<leader>ff"
        );
        assert_eq!(
            dictionary.search_with_mappings("diw", &[])[0].item.id,
            "delete-word"
        );
    }

    #[test]
    fn custom_aliases_change_search_without_changing_real_lhs() {
        let mut dictionary = Dictionary::default();
        dictionary
            .apply_override(&CommandOverride {
                command_id: "find-files".into(),
                aliases: vec!["找项目源码".into()],
                mnemonic: "我的文件入口".into(),
            })
            .unwrap();
        let result = dictionary.search_with_mappings("找项目源码", &[]);
        assert_eq!(result[0].item.lhs, "<leader><space>");
        assert_eq!(result[0].item.mnemonic, "我的文件入口");
        assert!(
            dictionary
                .apply_override(&CommandOverride {
                    command_id: "runtime:n:x".into(),
                    aliases: vec![],
                    mnemonic: "不允许".into(),
                })
                .is_err()
        );
    }

    #[test]
    fn empty_query_returns_bounded_candidates() {
        let dictionary = Dictionary::default();
        assert_eq!(dictionary.search_with_mappings("", &[]).len(), 12);
    }

    #[test]
    fn merges_actual_profile_mappings() {
        let dictionary = Dictionary::default();
        let mappings = vec![
            RuntimeMapping {
                mode: "n".into(),
                lhs: "<Space>ff".into(),
                rhs: "<cmd>Telescope find_files<cr>".into(),
                desc: "Find project files".into(),
                scope: "global".into(),
            },
            RuntimeMapping {
                mode: "n".into(),
                lhs: "<leader>zz".into(),
                rhs: "<cmd>CustomAction<cr>".into(),
                desc: "自定义动作".into(),
                scope: "buffer".into(),
            },
        ];
        let results =
            dictionary.search_with_mappings("find project", &mappings);
        assert!(
            results
                .iter()
                .any(|result| result.item.source == "当前 Profile")
        );
        let custom = dictionary.search_with_mappings("自定义", &mappings);
        assert_eq!(custom[0].item.lhs, "<leader>zz");
        assert_eq!(custom[0].item.source, "当前 Buffer");
        assert_eq!(custom[0].item.risk, RiskLevel::Edit);
    }

    #[test]
    fn unknown_runtime_mappings_fail_closed_and_force_quit_is_destructive() {
        let dictionary = Dictionary::default();
        let mappings = vec![
            RuntimeMapping {
                mode: "n".into(),
                lhs: "<leader>zz".into(),
                rhs: "<cmd>CustomAction<cr>".into(),
                desc: "未知动作".into(),
                scope: "global".into(),
            },
            RuntimeMapping {
                mode: "n".into(),
                lhs: "<leader>qq".into(),
                rhs: "<cmd>qa!<cr>".into(),
                desc: "Force quit all".into(),
                scope: "global".into(),
            },
            RuntimeMapping {
                mode: "n".into(),
                lhs: "K".into(),
                rhs: "<cmd>qa!<cr>".into(),
                desc: "Unknown override".into(),
                scope: "global".into(),
            },
        ];
        let custom = dictionary.search_with_mappings("未知", &mappings);
        assert_eq!(custom[0].item.risk, RiskLevel::Edit);
        let quit = dictionary.resolve_lhs("<leader>qq", &mappings).unwrap();
        assert_eq!(quit.risk, RiskLevel::Destructive);
        let overridden_read = dictionary.resolve_lhs("K", &mappings).unwrap();
        assert_eq!(overridden_read.risk, RiskLevel::Destructive);
    }
}

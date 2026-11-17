use std::cell::{Cell, OnceCell, RefCell};
use std::ptr::NonNull;
use std::sync::mpsc::Sender;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAlert, NSAlertFirstButtonReturn, NSApplication,
    NSBackingStoreType, NSButton, NSColor, NSControl,
    NSControlTextEditingDelegate, NSFloatingWindowLevel, NSFont,
    NSLayoutAttribute, NSNormalWindowLevel, NSPanel, NSPasteboard,
    NSPasteboardTypeString, NSScreen, NSScrollView, NSStackView,
    NSStackViewDistribution, NSTableColumn, NSTableView, NSTableViewDataSource,
    NSTableViewDelegate, NSTextField, NSTextFieldDelegate, NSTextView,
    NSUserInterfaceLayoutOrientation, NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowCollectionBehavior, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSIndexSet, NSInteger, NSNotification, NSObject,
    NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, ns_string,
};

use crate::models::{
    AppSettings, CommandItem, ContextKind, PanelSnapshot, RiskLevel,
};
use crate::panels::PanelAction;

/// 查询控制器状态｜集中保存候选、当前选择及输入框、候选表、详情视图的引用。
#[derive(Debug, Default)]
struct PanelControllerIvars {
    /// 查询结果段｜当前可供选择和执行的命令集合。
    candidates: RefCell<Vec<CommandItem>>,
    /// 候选视图段｜延迟接入用于刷新与选择命令的表格。
    table: OnceCell<Retained<NSTableView>>,
    /// 详情视图段｜延迟接入用于展示选中命令的文本。
    detail: OnceCell<Retained<NSTextField>>,
    /// 查询输入段｜延迟接入用于读取原始关键词的输入框。
    query: OnceCell<Retained<NSTextField>>,
    /// 候选选择段｜记录键盘导航对应的当前行。
    selected: Cell<usize>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = PanelControllerIvars]
    /// 查询交互控制器｜把 AppKit 输入与选择事件翻译成面板动作。
    struct PanelController;

    unsafe impl NSObjectProtocol for PanelController {}

    impl PanelController {
        /// 查询输入回调｜文本变化只触发重新查询，不在视图层筛选。
        #[unsafe(method(queryChanged:))]
        fn query_changed(&self, _sender: &NSTextField) {
            self.emit_query();
        }

        /// 查询执行出口｜仅在有候选和已连接会话时发送选中命令。
        #[unsafe(method(executeSelected:))]
        fn execute_selected(&self, _sender: Option<&AnyObject>) {
            REGISTRY.with(|registry| {
                let borrowed = registry.borrow();
                let Some(registry) = borrowed.as_ref() else {
                    return;
                };
                let candidate_count = self.ivars().candidates.borrow().len();
                let available = query_actions_available(
                    candidate_count,
                    registry.session_count.get(),
                );
                if !available.1 {
                    let message = if candidate_count == 0 {
                        "没有可执行的命令"
                    } else {
                        "请先启动并连接受管 Neovim Profile"
                    };
                    registry
                        .query_status
                        .setStringValue(&NSString::from_str(message));
                    return;
                }
                if let Some(command) = self.selected_command() {
                    emit(PanelAction::Execute(command.lhs));
                }
            });
        }

        /// 查询目标切换｜把多会话选择交给上层处理。
        #[unsafe(method(cycleSession:))]
        fn cycle_session(&self, _sender: Option<&AnyObject>) {
            emit(PanelAction::CycleSession);
        }

        /// 查询复制出口｜把选中命令的真实按键交给上层复制。
        #[unsafe(method(copySelected:))]
        fn copy_selected(&self, _sender: Option<&AnyObject>) {
            if let Some(command) = self.selected_command() {
                emit(PanelAction::Copy(command.lhs));
            }
        }

        /// 查询关闭出口｜视图不自行管理面板生命周期。
        #[unsafe(method(closeQuery:))]
        fn close_query(&self, _sender: Option<&AnyObject>) {
            emit(PanelAction::CloseQuery);
        }
    }

    unsafe impl NSControlTextEditingDelegate for PanelController {
        /// 查询编辑回调｜与输入框动作共用同一条查询入口。
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, _notification: &NSNotification) {
            self.emit_query();
        }

        /// 查询键盘导航｜方向键选候选，Tab 切详情，Esc 关闭，Enter 留给执行。
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn control_text_view_do_command_by_selector(
            &self,
            _control: &NSControl,
            _text_view: &NSTextView,
            selector: Sel,
        ) -> bool {
            if selector == sel!(moveUp:) {
                self.move_selection(-1);
                true
            } else if selector == sel!(moveDown:) {
                self.move_selection(1);
                true
            } else if selector == sel!(insertTab:) {
                if let Some(detail) = self.ivars().detail.get() {
                    detail.setHidden(!detail.isHidden());
                }
                true
            } else if selector == sel!(cancelOperation:) {
                emit(PanelAction::CloseQuery);
                true
            } else {
                selector == sel!(insertNewline:)
            }
        }
    }

    unsafe impl NSTextFieldDelegate for PanelController {}

    unsafe impl NSTableViewDataSource for PanelController {
        /// 候选列表数据源｜行数直接等于当前查询结果数。
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, _table_view: &NSTableView) -> NSInteger {
            self.ivars().candidates.borrow().len() as NSInteger
        }

        /// 候选列表展示｜每行由命令标题和真实按键组成。
        #[unsafe(method(tableView:objectValueForTableColumn:row:))]
        fn object_value(
            &self,
            _table_view: &NSTableView,
            _table_column: Option<&NSTableColumn>,
            row: NSInteger,
        ) -> Option<NonNull<AnyObject>> {
            let candidates = self.ivars().candidates.borrow();
            let command = candidates.get(row as usize)?;
            let text = format!("{}  ·  {}", command.title, command.lhs);
            let value = NSString::from_str(&text).into_super();
            NonNull::new(Retained::autorelease_return(value))
                .map(|pointer| pointer.cast())
        }
    }

    unsafe impl NSTableViewDelegate for PanelController {
        /// 候选到详情｜鼠标或表格选择变化后同步当前行和详情。
        #[unsafe(method(tableViewSelectionDidChange:))]
        fn selection_did_change(&self, _notification: &NSNotification) {
            let row = self
                .ivars()
                .table
                .get()
                .map(|table| table.selectedRow())
                .unwrap_or(0);
            self.ivars().selected.set(row.max(0) as usize);
            self.update_detail();
        }
    }

    unsafe impl NSWindowDelegate for PanelController {
        /// 被动浮窗位置回传｜用户拖动后把新原点交给上层持久化。
        #[unsafe(method(windowDidMove:))]
        #[allow(non_snake_case)]
        fn windowDidMove(&self, _notification: &NSNotification) {
            REGISTRY.with(|registry| {
                if let Some(registry) = registry.borrow().as_ref()
                    && !registry.suppress_move_notification.get()
                {
                    let origin = registry.passive.frame().origin;
                    emit(PanelAction::MovePassive {
                        x: origin.x,
                        y: origin.y,
                    });
                }
            });
        }
    }
);

impl PanelController {
    /// 查询链路入口｜在主线程创建带空候选状态的交互控制器。
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PanelControllerIvars::default());
        unsafe { msg_send![super(this), init] }
    }

    /// 输入到查询动作｜读取输入框全文并原样发给上层。
    fn emit_query(&self) {
        let query = self
            .ivars()
            .query
            .get()
            .map(|field| field.stringValue().to_string())
            .unwrap_or_default();
        emit(PanelAction::Query(query));
    }

    /// 选择到命令｜以表格当前行从候选中取出完整命令。
    fn selected_command(&self) -> Option<CommandItem> {
        let index = self
            .ivars()
            .table
            .get()
            .map(|table| table.selectedRow())
            .unwrap_or(0);
        self.ivars()
            .candidates
            .borrow()
            .get(index.max(0) as usize)
            .cloned()
    }

    /// 键盘到候选｜在结果范围内移动单行选择并联动表格和详情。
    fn move_selection(&self, delta: isize) {
        let len = self.ivars().candidates.borrow().len();
        if len == 0 {
            return;
        }
        let current = self.ivars().selected.get();
        let next =
            (current as isize + delta).clamp(0, len as isize - 1) as usize;
        self.ivars().selected.set(next);
        if let Some(table) = self.ivars().table.get() {
            table.selectRowIndexes_byExtendingSelection(
                &NSIndexSet::indexSetWithIndex(next),
                false,
            );
            table.scrollRowToVisible(next as NSInteger);
        }
        self.update_detail();
    }

    /// 查询结果到候选｜替换结果、回到首行并同步表格与详情。
    fn set_candidates(&self, commands: Vec<CommandItem>) {
        self.ivars().selected.set(0);
        *self.ivars().candidates.borrow_mut() = commands;
        if let Some(table) = self.ivars().table.get() {
            table.reloadData();
            if !self.ivars().candidates.borrow().is_empty() {
                table.selectRowIndexes_byExtendingSelection(
                    &NSIndexSet::indexSetWithIndex(0),
                    false,
                );
            }
        }
        self.update_detail();
    }

    /// 候选到详情｜把选中命令的说明、效果和按当前设置计算的风险提示汇总展示。
    fn update_detail(&self) {
        if let Some(detail) = self.ivars().detail.get() {
            let confirm_edits = REGISTRY.with(|registry| {
                registry
                    .borrow()
                    .as_ref()
                    .map(|registry| registry.settings.borrow().confirm_edits)
                    .unwrap_or(true)
            });
            let text = self
                .selected_command()
                .map(|command| {
                    format!(
                        "{}\n{}\n助记：{}\n结果：{} · 风险：{}",
                        command.description,
                        command.effect,
                        command.mnemonic,
                        command.mode,
                        match command.risk {
                            RiskLevel::Read => "只读",
                            RiskLevel::Edit if confirm_edits =>
                                "修改，执行前确认",
                            RiskLevel::Edit => "修改，直接发送按键",
                            RiskLevel::Destructive => "破坏性，执行前确认",
                        }
                    )
                })
                .unwrap_or_else(|| "没有匹配命令，可换个中文动作或按键".into());
            detail.setStringValue(&NSString::from_str(&text));
        }
    }
}

/// 查询视图组合｜把面板容器与输入、状态、操作控件一起交给注册表持有。
struct QueryViews {
    /// 查询容器｜承载完整交互面板。
    panel: Retained<NSPanel>,
    /// 查询输入｜与候选和详情共同组成检索路径。
    field: Retained<NSTextField>,
    /// 查询目标｜展示当前命令将发往的会话。
    target: Retained<NSTextField>,
    /// 查询反馈｜展示结果数量或不可执行原因。
    status: Retained<NSTextField>,
    /// 查询提示｜配合当前候选和会话状态说明可用按键。
    shortcut_hint: Retained<NSTextField>,
    /// 目标操作｜在多会话间切换。
    cycle_button: Retained<NSButton>,
    /// 候选操作｜复制当前命令而无需执行。
    copy_button: Retained<NSButton>,
    /// 候选操作｜执行当前命令。
    execute_button: Retained<NSButton>,
}

/// 被动视图组合｜把浮窗容器与实时状态文本一起交给注册表持有。
struct PassiveViews {
    /// 被动浮窗容器｜承载无需键盘焦点的状态展示。
    panel: Retained<NSPanel>,
    /// 会话状态｜显示模式与上下文标题。
    status: Retained<NSTextField>,
    /// 输入状态｜显示未完成按键或最近输入。
    pending: Retained<NSTextField>,
    /// 语义状态｜解释当前输入上下文。
    semantic: Retained<NSTextField>,
    /// 后续提示｜给出下一步可用操作的助记。
    next: Retained<NSTextField>,
    /// 出口提示｜组合上下文退出方式与全局快捷键。
    exit: Retained<NSTextField>,
}

/// 面板链路枢纽｜聚合设置、最新快照、会话数和两类视图的共享状态。
struct PanelRegistry {
    /// 设置输入｜为浮窗展示和风险提示提供当前配置。
    settings: RefCell<AppSettings>,
    /// 快照缓存｜设置刷新后可据此重建被动浮窗内容。
    snapshot: RefCell<Option<PanelSnapshot>>,
    /// 位置同步保护｜区分程序化移动与用户拖动，避免回写循环。
    suppress_move_notification: Cell<bool>,
    /// 被动容器｜接收显隐、层级、尺寸和位置调整。
    passive: Retained<NSPanel>,
    /// 被动文本｜属于同一浮窗的会话状态区域。
    status: Retained<NSTextField>,
    /// 被动文本｜属于同一浮窗的输入状态区域。
    pending: Retained<NSTextField>,
    /// 被动文本｜属于同一浮窗的语义区域。
    semantic: Retained<NSTextField>,
    /// 被动文本｜属于同一浮窗的后续提示区域。
    next: Retained<NSTextField>,
    /// 被动文本｜属于同一浮窗的出口提示区域。
    exit: Retained<NSTextField>,
    /// 查询容器｜独立于被动浮窗管理焦点和显隐。
    query: Retained<NSPanel>,
    /// 查询组合｜输入框与候选表、详情共同由控制器联动。
    query_field: Retained<NSTextField>,
    /// 查询组合｜目标文本复用被动快照中的会话信息。
    query_target: Retained<NSTextField>,
    /// 查询组合｜状态文本补充候选列表的执行反馈。
    query_status: Retained<NSTextField>,
    /// 查询组合｜快捷键提示随候选和会话可用性变化。
    shortcut_hint: Retained<NSTextField>,
    /// 查询组合｜目标切换按钮由会话数控制。
    cycle_button: Retained<NSButton>,
    /// 查询组合｜复制按钮由候选可用性控制。
    copy_button: Retained<NSButton>,
    /// 查询组合｜执行按钮同时依赖候选和会话。
    execute_button: Retained<NSButton>,
    /// 会话输入｜连接查询目标切换与命令执行门槛。
    session_count: Cell<usize>,
    /// 查询协调者｜共享候选状态并连接输入、候选和详情。
    controller: Retained<PanelController>,
}

thread_local! {
    /// 主线程面板状态｜AppKit 对象通过唯一注册表共享。
    static REGISTRY: RefCell<Option<PanelRegistry>>
        = const { RefCell::new(None) };
    /// 面板动作出口｜把视图事件送回上层事件循环。
    static ACTION_SENDER: RefCell<Option<Sender<PanelAction>>>
        = const { RefCell::new(None) };
}

/// 视图到上层｜统一发送面板动作，未初始化时静默丢弃。
fn emit(action: PanelAction) {
    ACTION_SENDER.with(|sender| {
        if let Some(sender) = sender.borrow().as_ref() {
            let _ = sender.send(action);
        }
    });
}

/// 面板链路初始化｜一次性组装被动浮窗、查询面板及其共享注册表。
pub fn initialize(action_sender: Sender<PanelAction>) -> Result<(), String> {
    ACTION_SENDER.with(|sender| *sender.borrow_mut() = Some(action_sender));
    let mtm =
        MainThreadMarker::new().ok_or("NSPanel 必须在 AppKit 主线程初始化")?;
    REGISTRY.with(|registry| {
        if registry.borrow().is_some() {
            return Ok(());
        }
        let passive = build_passive(mtm);
        let controller = PanelController::new(mtm);
        let query = build_query(mtm, &controller);
        passive.panel.setMovable(true);
        passive.panel.setMovableByWindowBackground(true);
        passive
            .panel
            .setDelegate(Some(ProtocolObject::from_ref(&*controller)));
        *registry.borrow_mut() = Some(PanelRegistry {
            settings: RefCell::new(AppSettings::default()),
            snapshot: RefCell::new(None),
            suppress_move_notification: Cell::new(false),
            passive: passive.panel,
            status: passive.status,
            pending: passive.pending,
            semantic: passive.semantic,
            next: passive.next,
            exit: passive.exit,
            query: query.panel,
            query_field: query.field,
            query_target: query.target,
            query_status: query.status,
            shortcut_hint: query.shortcut_hint,
            cycle_button: query.cycle_button,
            copy_button: query.copy_button,
            execute_button: query.execute_button,
            session_count: Cell::new(0),
            controller,
        });
        Ok(())
    })
}

/// 面板交互模式用显式状态区分查询窗口与被动浮窗。
#[derive(Clone, Copy)]
enum PanelInteraction {
    Interactive,
    ClickThrough,
}

impl PanelInteraction {
    /// AppKit 布尔属性统一从交互模式派生，调用点不传递裸布尔值。
    fn is_interactive(self) -> bool {
        matches!(self, Self::Interactive)
    }
}

/// 文本字重使用具名状态，避免标签构造器接收含义不明的布尔值。
#[derive(Clone, Copy)]
enum TextWeight {
    Regular,
    Bold,
}

/// 两类面板的基础容器｜以交互模式区分查询窗口与鼠标穿透的被动浮窗。
fn panel(
    mtm: MainThreadMarker,
    width: f64,
    height: f64,
    interaction: PanelInteraction,
) -> Retained<NSPanel> {
    let interactive = interaction.is_interactive();
    let style = if interactive {
        NSWindowStyleMask::Borderless
            | NSWindowStyleMask::NonactivatingPanel
            | NSWindowStyleMask::HUDWindow
    } else {
        NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel
    };
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        NSRect::new(NSPoint::new(120.0, 120.0), NSSize::new(width, height)),
        style,
        NSBackingStoreType::Buffered,
        false,
    );
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setLevel(NSFloatingWindowLevel);
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setAlphaValue(0.97);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    panel.setFloatingPanel(true);
    panel.setBecomesKeyOnlyIfNeeded(interactive);
    panel.setHidesOnDeactivate(false);
    panel.setIgnoresMouseEvents(!interactive);
    panel.setHasShadow(true);
    panel
}

/// 两类面板的视觉底层｜提供统一的半透明圆角背景，不承担交互状态。
fn background(
    mtm: MainThreadMarker,
    width: f64,
    height: f64,
) -> Retained<NSVisualEffectView> {
    let view = NSVisualEffectView::initWithFrame(
        NSVisualEffectView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height)),
    );
    view.setMaterial(NSVisualEffectMaterial::HUDWindow);
    view.setBlendingMode(NSVisualEffectBlendingMode::WithinWindow);
    view.setState(NSVisualEffectState::Active);
    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
        layer.setCornerRadius(14.0);
        layer.setMasksToBounds(true);
    }
    view
}

/// 面板文本构件｜生成统一的无边框展示标签，输入与业务状态由调用方提供。
fn label(
    mtm: MainThreadMarker,
    text: &str,
    size: f64,
    weight: TextWeight,
) -> Retained<NSTextField> {
    let label =
        NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    let font = match weight {
        TextWeight::Bold => NSFont::boldSystemFontOfSize(size),
        TextWeight::Regular => NSFont::systemFontOfSize(size),
    };
    label.setFont(Some(&font));
    label.setTextColor(Some(&NSColor::labelColor()));
    label.setDrawsBackground(false);
    label.setBordered(false);
    label
}

/// 被动浮窗布局构件｜把状态文本按统一间距纵向排列。
fn vertical_stack(
    mtm: MainThreadMarker,
    width: f64,
    height: f64,
) -> Retained<NSStackView> {
    let stack = NSStackView::initWithFrame(
        NSStackView::alloc(mtm),
        NSRect::new(
            NSPoint::new(14.0, 12.0),
            NSSize::new(width - 28.0, height - 24.0),
        ),
    );
    stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
    stack.setAlignment(NSLayoutAttribute::Leading);
    stack.setDistribution(NSStackViewDistribution::Fill);
    stack.setSpacing(6.0);
    stack
}

/// 被动展示链路装配｜组合鼠标穿透的浮窗与状态文本，并立即显示。
fn build_passive(mtm: MainThreadMarker) -> PassiveViews {
    let width = 420.0;
    let height = 190.0;
    let panel = panel(mtm, width, height, PanelInteraction::ClickThrough);
    let background = background(mtm, width, height);
    let stack = vertical_stack(mtm, width, height);
    let status = label(mtm, "等待 Neovim 连接", 12.0, TextWeight::Bold);
    let pending = label(mtm, "LazyKeys", 22.0, TextWeight::Bold);
    let semantic = label(
        mtm,
        "启动一个受管 Profile 后即可显示实时语义",
        13.0,
        TextWeight::Regular,
    );
    let next = label(mtm, "不会监听全局键盘", 12.0, TextWeight::Regular);
    let exit = label(mtm, "⌃⌥K 查询 · ⌃⌥J 显隐", 12.0, TextWeight::Regular);
    for view in [&status, &pending, &semantic, &next, &exit] {
        stack.addArrangedSubview(view);
    }
    background.addSubview(&stack);
    panel.setContentView(Some(&background));
    panel.orderFront(None);
    PassiveViews {
        panel,
        status,
        pending,
        semantic,
        next,
        exit,
    }
}

/// 查询目标区组合目录文本与多会话切换按钮。
fn query_target_views(
    mtm: MainThreadMarker,
    controller: &Retained<PanelController>,
    background: &NSVisualEffectView,
) -> (Retained<NSTextField>, Retained<NSButton>) {
    let target =
        label(mtm, "目标：等待 Neovim 连接", 11.0, TextWeight::Regular);
    target.setFrame(NSRect::new(
        NSPoint::new(18.0, 424.0),
        NSSize::new(390.0, 20.0),
    ));
    background.addSubview(&target);
    let cycle = unsafe {
        NSButton::buttonWithTitle_target_action(
            ns_string!("切换目标"),
            Some(controller),
            Some(sel!(cycleSession:)),
            mtm,
        )
    };
    cycle.setFrame(NSRect::new(
        NSPoint::new(466.0, 418.0),
        NSSize::new(116.0, 28.0),
    ));
    cycle.setEnabled(false);
    background.addSubview(&cycle);
    (target, cycle)
}

/// 查询反馈区分别承载结果状态与当前可用快捷键。
fn query_feedback_views(
    mtm: MainThreadMarker,
    background: &NSVisualEffectView,
) -> (Retained<NSTextField>, Retained<NSTextField>) {
    let status = label(mtm, "", 10.0, TextWeight::Regular);
    status.setFrame(NSRect::new(
        NSPoint::new(18.0, 52.0),
        NSSize::new(564.0, 16.0),
    ));
    background.addSubview(&status);
    let hint = label(
        mtm,
        "↑↓ 选择 · Enter 执行 · Tab 详情 · Esc 关闭",
        10.0,
        TextWeight::Regular,
    );
    hint.setFrame(NSRect::new(
        NSPoint::new(18.0, 134.0),
        NSSize::new(564.0, 18.0),
    ));
    background.addSubview(&hint);
    (status, hint)
}

/// 查询输入区连接控制器动作与文本编辑代理。
fn query_input(
    mtm: MainThreadMarker,
    controller: &Retained<PanelController>,
    background: &NSVisualEffectView,
) -> Retained<NSTextField> {
    let field = NSTextField::textFieldWithString(ns_string!(""), mtm);
    field.setFrame(NSRect::new(
        NSPoint::new(18.0, 378.0),
        NSSize::new(564.0, 30.0),
    ));
    field.setPlaceholderString(Some(ns_string!(
        "输入中文、英文或按键，例如：查找文件 / diw"
    )));
    field.setFont(Some(&NSFont::systemFontOfSize(15.0)));
    unsafe {
        field.setTarget(Some(controller));
        field.setAction(Some(sel!(queryChanged:)));
        field.setDelegate(Some(ProtocolObject::from_ref(&**controller)));
    }
    background.addSubview(&field);
    controller.ivars().query.set(field.clone()).unwrap();
    field
}

/// 候选区把表格数据源与选择代理接入同一控制器。
fn query_candidates(
    mtm: MainThreadMarker,
    controller: &Retained<PanelController>,
    background: &NSVisualEffectView,
) -> Retained<NSTableView> {
    let table =
        NSTableView::initWithFrame(NSTableView::alloc(mtm), NSRect::default());
    let column = NSTableColumn::initWithIdentifier(
        NSTableColumn::alloc(mtm),
        ns_string!("command"),
    );
    column.setWidth(564.0);
    table.addTableColumn(&column);
    table.setHeaderView(None);
    table.setRowHeight(28.0);
    table.setUsesAlternatingRowBackgroundColors(true);
    unsafe {
        table.setDataSource(Some(ProtocolObject::from_ref(&**controller)));
        table.setDelegate(Some(ProtocolObject::from_ref(&**controller)));
    }
    let scroll = NSScrollView::initWithFrame(
        NSScrollView::alloc(mtm),
        NSRect::new(NSPoint::new(18.0, 156.0), NSSize::new(564.0, 202.0)),
    );
    scroll.setDocumentView(Some(&table));
    scroll.setHasVerticalScroller(true);
    scroll.setDrawsBackground(false);
    background.addSubview(&scroll);
    controller.ivars().table.set(table.clone()).unwrap();
    table
}

/// 详情区由控制器持有，随当前候选同步更新。
fn query_detail(
    mtm: MainThreadMarker,
    controller: &Retained<PanelController>,
    background: &NSVisualEffectView,
) -> Retained<NSTextField> {
    let detail = label(
        mtm,
        "输入中文、英文或按键查询当前配置",
        12.0,
        TextWeight::Regular,
    );
    detail.setFrame(NSRect::new(
        NSPoint::new(18.0, 66.0),
        NSSize::new(564.0, 64.0),
    ));
    background.addSubview(&detail);
    controller.ivars().detail.set(detail.clone()).unwrap();
    detail
}

/// 操作区分别连接复制与执行动作，初始状态为不可用。
fn query_action_buttons(
    mtm: MainThreadMarker,
    controller: &Retained<PanelController>,
    background: &NSVisualEffectView,
) -> (Retained<NSButton>, Retained<NSButton>) {
    let copy = unsafe {
        NSButton::buttonWithTitle_target_action(
            ns_string!("复制按键说明"),
            Some(controller),
            Some(sel!(copySelected:)),
            mtm,
        )
    };
    copy.setFrame(NSRect::new(
        NSPoint::new(338.0, 18.0),
        NSSize::new(112.0, 30.0),
    ));
    copy.setEnabled(false);
    background.addSubview(&copy);
    let execute = unsafe {
        NSButton::buttonWithTitle_target_action(
            ns_string!("执行"),
            Some(controller),
            Some(sel!(executeSelected:)),
            mtm,
        )
    };
    execute.setFrame(NSRect::new(
        NSPoint::new(466.0, 18.0),
        NSSize::new(116.0, 30.0),
    ));
    execute.setEnabled(false);
    execute.setKeyEquivalent(ns_string!("\r"));
    background.addSubview(&execute);
    (copy, execute)
}

/// 查询交互链路装配｜把输入、候选、详情和操作按钮接入同一控制器。
fn build_query(
    mtm: MainThreadMarker,
    controller: &Retained<PanelController>,
) -> QueryViews {
    let panel = panel(mtm, 600.0, 460.0, PanelInteraction::Interactive);
    let background = background(mtm, 600.0, 460.0);
    let (target, cycle) = query_target_views(mtm, controller, &background);
    let (status, shortcut_hint) = query_feedback_views(mtm, &background);
    let field = query_input(mtm, controller, &background);
    let table = query_candidates(mtm, controller, &background);
    let detail = query_detail(mtm, controller, &background);
    let (copy, execute) = query_action_buttons(mtm, controller, &background);
    field.setAccessibilityLabel(Some(ns_string!("命令查询")));
    status.setAccessibilityLabel(Some(ns_string!("搜索结果状态")));
    table.setAccessibilityLabel(Some(ns_string!("命令候选列表")));
    detail.setAccessibilityLabel(Some(ns_string!("选中命令详情")));
    cycle.setAccessibilityLabel(Some(ns_string!("切换目标 Neovim 会话")));
    panel.setContentView(Some(&background));
    QueryViews {
        panel,
        field,
        target,
        status,
        shortcut_hint,
        cycle_button: cycle,
        copy_button: copy,
        execute_button: execute,
    }
}

/// 被动浮窗的展示密度使用具名状态，避免与历史设置混为两个布尔参数。
#[derive(Clone, Copy)]
enum PassiveDisplay {
    Compact,
    Expanded,
}

/// 被动浮窗精简规则｜紧凑模式，或关闭历史且一次输入已结束时隐藏语义。
fn semantic_is_hidden(
    display: PassiveDisplay,
    settings: &AppSettings,
    pending: &str,
    last_input: &str,
) -> bool {
    matches!(display, PassiveDisplay::Compact)
        || (!settings.show_history
            && pending.is_empty()
            && !last_input.is_empty())
}

/// 快照到查询目标｜把 Profile、目录和短会话标识压缩为一行，并按设置隐藏目录。
fn query_target_text(
    profile_id: &str,
    cwd: &str,
    session_id: &str,
    settings: &AppSettings,
) -> String {
    let directory = if !settings.include_cwd {
        "目录已隐藏"
    } else if cwd.is_empty() {
        "未知目录"
    } else {
        cwd
    };
    let target_session = if session_id.is_empty() {
        "未连接".to_string()
    } else {
        session_id.chars().take(8).collect()
    };
    format!("目标：{profile_id} · {directory} · {target_session}")
}

/// 查询结果反馈｜把候选数量转成结果状态或空结果提示。
fn query_result_status(count: usize) -> String {
    if count == 0 {
        "没有匹配结果，可换个中文动作或按键".into()
    } else {
        format!("{count} 条结果")
    }
}

/// 查询操作门槛｜复制只要求有候选，执行还要求存在已连接会话。
fn query_actions_available(
    candidate_count: usize,
    session_count: usize,
) -> (bool, bool) {
    let has_candidates = candidate_count > 0;
    (has_candidates, has_candidates && session_count > 0)
}

/// 查询操作提示｜用候选和会话状态选择当前实际可用的快捷键说明。
fn query_shortcut_hint(
    candidate_count: usize,
    session_count: usize,
) -> &'static str {
    if candidate_count == 0 {
        "输入关键词查询 · Esc 关闭"
    } else if session_count == 0 {
        "可复制按键 · 启动 Profile 后执行 · Esc 关闭"
    } else {
        "↑↓ 选择 · Tab 详情 · Esc 关闭"
    }
}

impl PanelRegistry {
    /// 查询结果落视图｜同步候选、详情、结果状态和操作可用性。
    fn set_query_results(&self, items: Vec<CommandItem>) {
        let status = query_result_status(items.len());
        self.controller.set_candidates(items);
        self.query_status
            .setStringValue(&NSString::from_str(&status));
        self.sync_query_actions();
    }

    /// 查询状态到操作｜按候选和会话数统一刷新按钮与快捷键提示。
    fn sync_query_actions(&self) {
        let candidate_count = self.controller.ivars().candidates.borrow().len();
        let session_count = self.session_count.get();
        let (copy_enabled, execute_enabled) =
            query_actions_available(candidate_count, session_count);
        self.copy_button.setEnabled(copy_enabled);
        self.execute_button.setEnabled(execute_enabled);
        self.shortcut_hint.setStringValue(&NSString::from_str(
            query_shortcut_hint(candidate_count, session_count),
        ));
    }
}

/// 设置到快捷键展示｜把修饰键名称压缩为 macOS 符号。
fn format_shortcut(shortcut: &str) -> String {
    shortcut
        .replace("Control", "⌃")
        .replace("Option", "⌥")
        .replace("Command", "⌘")
        .replace("Shift", "⇧")
        .replace("+", "")
}

/// 快照到被动出口提示｜保留上下文提示，并附加当前显隐快捷键。
fn format_exit_hint(hint: &str, settings: &AppSettings) -> String {
    let query = format_shortcut(&settings.query_shortcut);
    format!(
        "{} · {} 显隐",
        hint.replace(&settings.query_shortcut, &query),
        format_shortcut(&settings.overlay_shortcut)
    )
}

/// 被动浮窗定位约束｜把保存或恢复的原点限制在当前屏幕可见区域内。
fn clamp_origin(origin: NSPoint, panel: NSSize, visible: NSRect) -> NSPoint {
    let max_x = (visible.origin.x + visible.size.width - panel.width)
        .max(visible.origin.x);
    let max_y = (visible.origin.y + visible.size.height - panel.height)
        .max(visible.origin.y);
    NSPoint::new(
        origin.x.clamp(visible.origin.x, max_x),
        origin.y.clamp(visible.origin.y, max_y),
    )
}

/// 窗口外观与位置更新期间抑制移动回传，避免把程序化调整再次持久化。
fn apply_window_settings(registry: &PanelRegistry, settings: &AppSettings) {
    registry
        .passive
        .setIgnoresMouseEvents(settings.click_through);
    registry
        .passive
        .setAlphaValue(f64::from(settings.opacity) / 100.0);
    registry.passive.setLevel(if settings.always_on_top {
        NSFloatingWindowLevel
    } else {
        NSNormalWindowLevel
    });
    let current = registry.passive.frame();
    let size = NSSize::new(f64::from(settings.overlay_width), 190.0);
    let origin = match (settings.overlay_x, settings.overlay_y) {
        (Some(x), Some(y)) => {
            let origin = NSPoint::new(x, y);
            registry
                .passive
                .screen()
                .or_else(|| {
                    MainThreadMarker::new().and_then(NSScreen::mainScreen)
                })
                .map(|screen| clamp_origin(origin, size, screen.visibleFrame()))
                .unwrap_or(origin)
        }
        _ => current.origin,
    };
    registry.suppress_move_notification.set(true);
    registry
        .passive
        .setFrame_display(NSRect::new(origin, size), true);
    registry.suppress_move_notification.set(false);
}

/// 各文本区域共用基础字号与语义字重，保持原视觉层级。
fn apply_font_settings(registry: &PanelRegistry, settings: &AppSettings) {
    for (view, weight, delta) in [
        (&registry.status, TextWeight::Bold, -1.0),
        (&registry.pending, TextWeight::Bold, 8.0),
        (&registry.semantic, TextWeight::Regular, 0.0),
        (&registry.next, TextWeight::Regular, -1.0),
        (&registry.exit, TextWeight::Regular, -1.0),
    ] {
        let size = f64::from(settings.font_size) + delta;
        let font = match weight {
            TextWeight::Bold => NSFont::boldSystemFontOfSize(size),
            TextWeight::Regular => NSFont::systemFontOfSize(size),
        };
        view.setFont(Some(&font));
    }
}

/// 设置刷新链路｜应用浮窗外观、位置和显隐，再用缓存快照重建内容并刷新风险详情。
pub fn apply_settings(settings: &AppSettings) {
    let snapshot = REGISTRY.with(|registry| {
        let borrowed = registry.borrow();
        let registry = borrowed.as_ref()?;
        *registry.settings.borrow_mut() = settings.clone();
        apply_window_settings(registry, settings);
        apply_font_settings(registry, settings);
        registry.exit.setStringValue(&NSString::from_str(&format!(
            "{} 查询 · {} 显隐",
            format_shortcut(&settings.query_shortcut),
            format_shortcut(&settings.overlay_shortcut)
        )));
        if settings.display_strategy == "shortcut" {
            registry.passive.orderOut(None);
        } else {
            registry.passive.orderFront(None);
        }
        registry.controller.update_detail();
        registry.snapshot.borrow().clone()
    });
    if let Some(snapshot) = snapshot {
        update_passive(snapshot);
    }
}

/// 未连接状态只展示安装引导与全局快捷键，不保留旧会话语义。
fn update_disconnected_passive(registry: &PanelRegistry) {
    registry
        .query_target
        .setStringValue(ns_string!("目标：等待 Neovim 连接"));
    registry
        .status
        .setStringValue(ns_string!("等待 Neovim 连接"));
    registry.pending.setStringValue(ns_string!("LazyKeys"));
    registry
        .semantic
        .setStringValue(ns_string!("启动受管 Profile 后显示实时语义"));
    registry.semantic.setHidden(false);
    registry.next.setHidden(true);
    let settings = registry.settings.borrow();
    registry.exit.setStringValue(&NSString::from_str(&format!(
        "{} 查询 · {} 显隐",
        format_shortcut(&settings.query_shortcut),
        format_shortcut(&settings.overlay_shortcut)
    )));
}

/// 主输入区优先展示未完成前缀，其次才展示可选历史。
fn pending_display_text(
    snapshot: &PanelSnapshot,
    settings: &AppSettings,
) -> String {
    if !snapshot.pending.is_empty() {
        return snapshot.pending.clone();
    }
    if settings.show_history && !snapshot.last_input.is_empty() {
        return format!("最近输入 {}", snapshot.last_input);
    }
    "LazyKeys".into()
}

/// 已连接状态按当前快照更新语义、历史、出口提示和查询目标。
fn update_connected_passive(
    registry: &PanelRegistry,
    snapshot: &PanelSnapshot,
) {
    let settings = registry.settings.borrow().clone();
    let compact = settings.display_strategy == "smart"
        && snapshot.pending.is_empty()
        && snapshot.last_input.is_empty()
        && snapshot.kind == ContextKind::Normal;
    let display = if compact {
        PassiveDisplay::Compact
    } else {
        PassiveDisplay::Expanded
    };
    registry.semantic.setHidden(semantic_is_hidden(
        display,
        &settings,
        &snapshot.pending,
        &snapshot.last_input,
    ));
    registry.next.setHidden(compact || !settings.show_mnemonic);
    registry.status.setStringValue(&NSString::from_str(&format!(
        "{} · {}",
        snapshot.mode.to_uppercase(),
        snapshot.title
    )));
    registry.pending.setStringValue(&NSString::from_str(
        &pending_display_text(snapshot, &settings),
    ));
    registry
        .semantic
        .setStringValue(&NSString::from_str(&snapshot.semantic));
    registry
        .next
        .setStringValue(&NSString::from_str(&snapshot.next));
    registry
        .exit
        .setStringValue(&NSString::from_str(&format_exit_hint(
            &snapshot.exit_hint,
            &settings,
        )));
    registry.query_target.setStringValue(&NSString::from_str(
        &query_target_text(
            &snapshot.profile_id,
            &snapshot.cwd,
            &snapshot.session_id,
            &settings,
        ),
    ));
}

/// 会话快照到被动浮窗｜更新连接状态、输入语义和提示，同时联动查询目标。
pub fn update_passive(snapshot: PanelSnapshot) {
    REGISTRY.with(|registry| {
        let borrowed = registry.borrow();
        let Some(registry) = borrowed.as_ref() else {
            return;
        };
        registry.snapshot.replace(Some(snapshot.clone()));
        if snapshot.session_id.is_empty() {
            update_disconnected_passive(registry);
        } else {
            update_connected_passive(registry, &snapshot);
        }
        registry.passive.display();
    });
}

/// 上层查询结果入口｜把最新命令集合交给查询面板统一展示。
pub fn update_candidates(items: Vec<CommandItem>) {
    REGISTRY.with(|registry| {
        if let Some(registry) = registry.borrow().as_ref() {
            registry.set_query_results(items);
        }
    });
}

/// 上层查询反馈入口｜直接替换状态文本，不改变候选内容。
pub fn set_query_status(message: &str) {
    REGISTRY.with(|registry| {
        if let Some(registry) = registry.borrow().as_ref() {
            registry
                .query_status
                .setStringValue(&NSString::from_str(message));
        }
    });
}

/// 会话状态到查询面板｜更新目标切换与执行操作所需的会话数量。
pub fn update_session_count(count: usize) {
    REGISTRY.with(|registry| {
        if let Some(registry) = registry.borrow().as_ref() {
            registry.session_count.set(count);
            registry.cycle_button.setEnabled(count > 1);
            registry.sync_query_actions();
        }
    });
}

/// 查询面板显隐入口｜居中显示并把键盘焦点和选中文本交给查询输入框。
pub fn show_query() {
    REGISTRY.with(|registry| {
        if let Some(registry) = registry.borrow().as_ref() {
            registry.query.center();
            registry.query.makeKeyAndOrderFront(None);
            registry
                .query
                .makeFirstResponder(Some(&registry.query_field));
            unsafe {
                registry.query_field.selectText(None);
            }
        }
    });
}

/// 查询面板退出入口｜隐藏交互窗口但保留其状态供下次使用。
pub fn hide_query() {
    REGISTRY.with(|registry| {
        if let Some(registry) = registry.borrow().as_ref() {
            registry.query.orderOut(None);
        }
    });
}

/// 被动浮窗位置入口｜显示并居中，用于需要重新找回浮窗的场景。
pub fn center_passive() {
    REGISTRY.with(|registry| {
        if let Some(registry) = registry.borrow().as_ref() {
            registry.passive.orderFront(None);
            registry.passive.center();
        }
    });
}

/// 被动浮窗显隐入口｜只切换可见性，不改变快照或设置。
pub fn toggle_passive() {
    REGISTRY.with(|registry| {
        if let Some(registry) = registry.borrow().as_ref() {
            if registry.passive.isVisible() {
                registry.passive.orderOut(None);
            } else {
                registry.passive.orderFront(None);
            }
        }
    });
}

/// 被动浮窗鼠标策略｜切换鼠标穿透，让浮窗在可拖动与纯展示之间变化。
// bits:allow(BCS004): 原生面板沿用应用设置的布尔鼠标策略
pub fn set_click_through(ignore: bool) {
    REGISTRY.with(|registry| {
        if let Some(registry) = registry.borrow().as_ref() {
            registry.passive.setIgnoresMouseEvents(ignore);
        }
    });
}

/// 查询复制终点｜用选中命令的真实按键替换系统剪贴板内容。
pub fn copy_text(text: &str) {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    pasteboard.setString_forType(&NSString::from_str(text), unsafe {
        NSPasteboardTypeString
    });
}

/// 执行前风险闸门｜只读命令直接放行，修改或破坏性命令须经模态确认。
pub fn confirm_command(command: &CommandItem, target: &str) -> bool {
    if command.risk == crate::models::RiskLevel::Read {
        return true;
    }
    let mtm = MainThreadMarker::new()
        .expect("command confirmation must run on main thread");
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(&format!(
        "确认在 {} 中执行 {}？",
        target, command.title
    )));
    alert.setInformativeText(&NSString::from_str(&format!(
        "{}\n真实按键：{}\n风险：{}",
        command.effect,
        command.lhs,
        match command.risk {
            RiskLevel::Read => "只读",
            RiskLevel::Edit => "会修改当前状态",
            RiskLevel::Destructive => "破坏性操作，可能丢失数据",
        }
    )));
    alert.addButtonWithTitle(ns_string!("确认执行"));
    alert.addButtonWithTitle(ns_string!("取消"));
    REGISTRY.with(|registry| {
        if let Some(registry) = registry.borrow().as_ref() {
            registry.query.orderOut(None);
        }
    });
    alert.window().setLevel(NSFloatingWindowLevel);
    alert.window().setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    alert.window().center();
    alert.window().makeKeyAndOrderFront(None);
    NSApplication::sharedApplication(mtm).activate();
    let response = alert.runModal();
    if response != NSAlertFirstButtonReturn {
        show_query();
    }
    response == NSAlertFirstButtonReturn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_shortcuts_for_display() {
        assert_eq!(format_shortcut("Control+Option+K"), "⌃⌥K");
        assert_eq!(format_shortcut("Command+Shift+F12"), "⌘⇧F12");
    }

    #[test]
    fn exit_hint_preserves_global_shortcuts() {
        let settings = AppSettings::default();
        assert_eq!(
            format_exit_hint(
                ":q 关闭当前窗口 · Control+Option+K 查询命令",
                &settings
            ),
            ":q 关闭当前窗口 · ⌃⌥K 查询命令 · ⌃⌥J 显隐"
        );
    }

    #[test]
    fn semantic_hides_completed_input_only_when_history_is_disabled() {
        let mut settings = AppSettings {
            show_history: false,
            ..Default::default()
        };
        assert!(semantic_is_hidden(
            PassiveDisplay::Expanded,
            &settings,
            "",
            "n"
        ));
        assert!(!semantic_is_hidden(
            PassiveDisplay::Expanded,
            &settings,
            "di",
            "diw"
        ));
        settings.show_history = true;
        assert!(!semantic_is_hidden(
            PassiveDisplay::Expanded,
            &settings,
            "",
            "n"
        ));
        assert!(semantic_is_hidden(
            PassiveDisplay::Compact,
            &settings,
            "",
            ""
        ));
    }

    #[test]
    fn query_target_hides_cwd_only_when_requested() {
        let mut settings = AppSettings {
            include_cwd: false,
            ..Default::default()
        };
        assert_eq!(
            query_target_text(
                "default",
                "/Users/user/project",
                "1234567890",
                &settings
            ),
            "目标：default · 目录已隐藏 · 12345678"
        );
        settings.include_cwd = true;
        assert_eq!(
            query_target_text("default", "/Users/user/project", "", &settings),
            "目标：default · /Users/user/project · 未连接"
        );
        assert_eq!(
            query_target_text("default", "", "1234567890", &settings),
            "目标：default · 未知目录 · 12345678"
        );
    }

    #[test]
    fn query_result_status_describes_matches_and_empty_results() {
        assert_eq!(query_result_status(12), "12 条结果");
        assert_eq!(query_result_status(1), "1 条结果");
        assert_eq!(
            query_result_status(0),
            "没有匹配结果，可换个中文动作或按键"
        );
    }

    #[test]
    fn query_actions_require_candidates_and_execution_requires_a_session() {
        assert_eq!(query_actions_available(0, 0), (false, false));
        assert_eq!(query_actions_available(3, 0), (true, false));
        assert_eq!(query_actions_available(3, 1), (true, true));
        assert_eq!(query_actions_available(0, 1), (false, false));
    }

    #[test]
    fn query_shortcut_hint_matches_available_actions() {
        assert_eq!(query_shortcut_hint(0, 0), "输入关键词查询 · Esc 关闭");
        assert_eq!(
            query_shortcut_hint(2, 0),
            "可复制按键 · 启动 Profile 后执行 · Esc 关闭"
        );
        assert_eq!(query_shortcut_hint(2, 1), "↑↓ 选择 · Tab 详情 · Esc 关闭");
    }

    #[test]
    fn clamp_origin_keeps_panel_inside_visible_frame() {
        let visible =
            NSRect::new(NSPoint::new(0.0, 25.0), NSSize::new(1440.0, 875.0));
        let panel = NSSize::new(420.0, 190.0);
        assert_eq!(
            clamp_origin(NSPoint::new(-200.0, 900.0), panel, visible),
            NSPoint::new(0.0, 710.0)
        );
        assert_eq!(
            clamp_origin(NSPoint::new(2000.0, -100.0), panel, visible),
            NSPoint::new(1020.0, 25.0)
        );
    }

    #[test]
    fn clamp_origin_handles_panel_larger_than_visible_frame() {
        let visible =
            NSRect::new(NSPoint::new(10.0, 20.0), NSSize::new(300.0, 100.0));
        let panel = NSSize::new(420.0, 190.0);
        assert_eq!(
            clamp_origin(NSPoint::new(200.0, 200.0), panel, visible),
            NSPoint::new(10.0, 20.0)
        );
    }
}

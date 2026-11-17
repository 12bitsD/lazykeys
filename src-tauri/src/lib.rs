mod dictionary;
mod environment;
mod models;
mod panels;
mod profiles;
mod rpc;
mod semantics;
mod sessions;

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;

use dictionary::Dictionary;
use models::{
    AppSettings, CommandItem, CommandOverride, DependencyStatus, EditorContext,
    InstallReport, LaunchRequest, ProfileConfig, ProfileSummary, RiskLevel,
    SearchResult, SessionSummary,
};
use profiles::{AppPaths, ProfileStore};
use sessions::{SessionEvent, SessionManager};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// 后端全局状态容器：把词典、当前上下文、会话、Profile 与设置汇入各 Tauri 命令共用的单一事实来源。
struct AppState {
    dictionary: Mutex<Dictionary>,
    context: Mutex<EditorContext>,
    sessions: SessionManager,
    profiles: ProfileStore,
    settings: Mutex<AppSettings>,
    settings_warning: Mutex<Option<String>>,
    overlay_move_generation: AtomicU64,
    shortcut_capture_depth: AtomicUsize,
}

impl AppState {
    /// 启动链路的状态装配：把磁盘配置、词典和会话管理器汇总为全局唯一状态。
    fn new(
        event_sender: mpsc::Sender<SessionEvent>,
        paths: AppPaths,
    ) -> Result<Self, String> {
        let profiles = ProfileStore::new(paths);
        let mut dictionary = Dictionary::default();
        for value in profiles.command_overrides()? {
            dictionary.apply_override(&value)?;
        }
        let (settings, warning) = match profiles.app_settings() {
            Ok(settings) => (settings, None),
            Err(error) => (
                AppSettings::default(),
                Some(format!(
                    "应用设置无效，已临时使用默认值；保存设置可修复：{error}"
                )),
            ),
        };
        Ok(Self {
            dictionary: Mutex::new(dictionary),
            context: Mutex::new(EditorContext::default()),
            sessions: SessionManager::new(event_sender),
            profiles,
            settings: Mutex::new(settings),
            settings_warning: Mutex::new(warning),
            overlay_move_generation: AtomicU64::new(0),
            shortcut_capture_depth: AtomicUsize::new(0),
        })
    }
}

/// 查询链路的检索入口：输入关键词，输出已合并当前会话映射的候选结果。
#[tauri::command]
fn search_commands(
    query: String,
    state: tauri::State<'_, AppState>,
) -> Vec<SearchResult> {
    state
        .dictionary
        .lock()
        .expect("dictionary lock")
        .search_with_mappings(&query, &state.sessions.active_mappings())
}

/// 设置链路的词典读取：返回全部内置命令及其当前覆盖结果。
#[tauri::command]
fn all_commands(state: tauri::State<'_, AppState>) -> Vec<CommandItem> {
    state
        .dictionary
        .lock()
        .expect("dictionary lock")
        .all()
        .to_vec()
}

/// 设置链路的命令覆盖写入：先持久化自定义别名与助记，再同步到内存词典。
#[tauri::command]
fn save_command_override(
    value: CommandOverride,
    state: tauri::State<'_, AppState>,
) -> Result<CommandItem, String> {
    if !state
        .dictionary
        .lock()
        .expect("dictionary lock")
        .all()
        .iter()
        .any(|command| command.id == value.command_id)
    {
        return Err("只能覆盖内置词典命令；运行时映射请修改 Neovim 配置".into());
    }
    state.profiles.save_command_override(&value)?;
    state
        .dictionary
        .lock()
        .expect("dictionary lock")
        .apply_override(&value)
}

/// 设置链路的读取出口：返回当前设置，并补发启动时的配置异常提示。
#[tauri::command]
fn get_app_settings(app: AppHandle) -> Result<AppSettings, String> {
    let state = app.state::<AppState>();
    if let Some(warning) = state
        .settings_warning
        .lock()
        .expect("settings warning lock")
        .clone()
    {
        let _ = app.emit("command-error", warning);
    }
    Ok(state.settings.lock().expect("settings lock").clone())
}

/// 设置链路的快捷键切换：以旧快捷键换新快捷键，失败时恢复旧绑定。
fn replace_shortcut<R: Runtime>(
    app: &AppHandle<R>,
    previous: &str,
    next: &str,
) -> Result<(), String> {
    if previous == next
        || app
            .state::<AppState>()
            .shortcut_capture_depth
            .load(Ordering::Relaxed)
            > 0
    {
        return Ok(());
    }
    app.global_shortcut()
        .unregister(previous)
        .map_err(|error| format!("注销快捷键 {previous} 失败：{error}"))?;
    if let Err(error) = app.global_shortcut().register(next) {
        let _ = app.global_shortcut().register(previous);
        return Err(format!("注册快捷键 {next} 失败：{error}"));
    }
    Ok(())
}

/// 录制链路的暂停闸门：首次进入录制时摘下全局快捷键，避免按键被应用截获。
#[tauri::command]
fn begin_shortcut_capture(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.shortcut_capture_depth.fetch_add(1, Ordering::SeqCst) > 0 {
        return Ok(());
    }
    let settings = state.settings.lock().expect("settings lock").clone();
    if let Err(error) = app
        .global_shortcut()
        .unregister(settings.query_shortcut.as_str())
    {
        state.shortcut_capture_depth.fetch_sub(1, Ordering::SeqCst);
        return Err(format!("暂停查询快捷键失败：{error}"));
    }
    if let Err(error) = app
        .global_shortcut()
        .unregister(settings.overlay_shortcut.as_str())
    {
        let _ = app
            .global_shortcut()
            .register(settings.query_shortcut.as_str());
        state.shortcut_capture_depth.fetch_sub(1, Ordering::SeqCst);
        return Err(format!("暂停浮窗快捷键失败：{error}"));
    }
    Ok(())
}

/// 录制链路的恢复闸门：最后一层录制结束时重新挂回全局快捷键。
#[tauri::command]
fn end_shortcut_capture(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let Ok(previous) = state.shortcut_capture_depth.try_update(
        Ordering::SeqCst,
        Ordering::SeqCst,
        |depth| depth.checked_sub(1),
    ) else {
        return Ok(());
    };
    if previous != 1 {
        return Ok(());
    }
    let settings = state.settings.lock().expect("settings lock").clone();
    app.global_shortcut()
        .register(settings.query_shortcut.as_str())
        .map_err(|error| format!("恢复查询快捷键失败：{error}"))?;
    app.global_shortcut()
        .register(settings.overlay_shortcut.as_str())
        .map_err(|error| format!("恢复浮窗快捷键失败：{error}"))?;
    Ok(())
}

/// 新设置先通过格式与互斥校验，再进入系统快捷键切换。
fn validate_shortcuts(settings: &AppSettings) -> Result<(), String> {
    settings
        .query_shortcut
        .parse::<tauri_plugin_global_shortcut::Shortcut>()
        .map_err(|error| format!("快捷键格式无效：{error}"))?;
    settings
        .overlay_shortcut
        .parse::<tauri_plugin_global_shortcut::Shortcut>()
        .map_err(|error| format!("浮窗快捷键格式无效：{error}"))?;
    if settings.query_shortcut == settings.overlay_shortcut {
        return Err("浮窗与查询不能使用同一个快捷键".into());
    }
    Ok(())
}

/// 保存失败时恢复两份旧快捷键，避免系统状态与磁盘设置分叉。
fn restore_shortcuts<R: Runtime>(
    app: &AppHandle<R>,
    previous: &AppSettings,
    failed: &AppSettings,
) {
    let _ =
        replace_shortcut(app, &failed.query_shortcut, &previous.query_shortcut);
    let _ = replace_shortcut(
        app,
        &failed.overlay_shortcut,
        &previous.overlay_shortcut,
    );
}

/// 先切换查询快捷键；第二个失败时立即恢复第一个。
fn switch_shortcuts<R: Runtime>(
    app: &AppHandle<R>,
    previous: &AppSettings,
    next: &AppSettings,
) -> Result<(), String> {
    replace_shortcut(
        app,
        &previous.query_shortcut,
        next.query_shortcut.as_str(),
    )?;
    if let Err(error) = replace_shortcut(
        app,
        &previous.overlay_shortcut,
        next.overlay_shortcut.as_str(),
    ) {
        let _ = replace_shortcut(
            app,
            &next.query_shortcut,
            &previous.query_shortcut,
        );
        return Err(error);
    }
    Ok(())
}

/// 持久化成功后应用面板行为并更新内存中的唯一设置事实。
fn apply_settings_update<R: Runtime>(
    app: &AppHandle<R>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    let state = app.state::<AppState>();
    let applied = settings.clone();
    app.run_on_main_thread(move || panels::apply_settings(&applied))
        .map_err(|error| error.to_string())?;
    *state.settings.lock().expect("settings lock") = settings.clone();
    *state
        .settings_warning
        .lock()
        .expect("settings warning lock") = None;
    Ok(settings)
}

/// 设置链路的保存入口：校验并持久化新设置，同时切换快捷键和浮窗行为。
#[tauri::command]
fn update_app_settings(
    mut settings: AppSettings,
    app: AppHandle,
) -> Result<AppSettings, String> {
    let _settings_guard = profiles::settings_lock()
        .lock()
        .map_err(|_| "应用设置锁已失效".to_string())?;
    let state = app.state::<AppState>();
    let previous = state.settings.lock().expect("settings lock").clone();
    settings.default_profile = previous.default_profile.clone();
    validate_shortcuts(&settings)?;
    switch_shortcuts(&app, &previous, &settings)?;
    if let Err(error) = state.profiles.save_app_settings_unlocked(&settings) {
        restore_shortcuts(&app, &previous, &settings);
        return Err(error);
    }
    apply_settings_update(&app, settings)
}

/// 上下文链路的快照读取：返回最近一次发布的编辑器状态。
#[tauri::command]
fn current_context(state: tauri::State<'_, AppState>) -> EditorContext {
    state.context.lock().expect("context lock").clone()
}

/// 查询链路的数据适配：把词典检索结果精简为面板所需命令列表。
fn panel_query_items<R: Runtime>(
    app: &AppHandle<R>,
    query: &str,
) -> Vec<CommandItem> {
    app.state::<AppState>()
        .dictionary
        .lock()
        .expect("dictionary lock")
        .search_with_mappings(
            query,
            &app.state::<AppState>().sessions.active_mappings(),
        )
        .into_iter()
        .map(|result| result.item)
        .collect()
}

/// 查询链路的显示入口：在主线程展示面板并装入初始候选。
#[tauri::command]
fn show_query_panel(app: AppHandle) -> Result<(), String> {
    let items = panel_query_items(&app, "");
    app.run_on_main_thread(move || {
        panels::show_query();
        panels::update_candidates(items);
    })
    .map_err(|error| error.to_string())
}

/// 查询链路的关闭出口：在主线程隐藏查询面板。
#[tauri::command]
fn hide_query_panel(app: AppHandle) -> Result<(), String> {
    app.run_on_main_thread(panels::hide_query)
        .map_err(|error| error.to_string())
}

/// 浮窗链路的开关入口：在主线程切换被动浮窗显隐。
#[tauri::command]
fn toggle_overlay(app: AppHandle) -> Result<(), String> {
    app.run_on_main_thread(panels::toggle_passive)
        .map_err(|error| error.to_string())
}

/// 浮窗链路的位置复位：在主线程把浮窗移回默认中央位置。
#[tauri::command]
fn center_overlay(app: AppHandle) -> Result<(), String> {
    app.run_on_main_thread(panels::center_passive)
        .map_err(|error| error.to_string())
}

/// 浮窗链路的鼠标策略切换：在主线程决定点击落在浮窗还是穿透到下层。
// bits:allow(BCS004): Tauri 命令沿用既有布尔设置契约
#[tauri::command]
fn set_overlay_click_through(
    ignore: bool,
    app: AppHandle,
) -> Result<(), String> {
    app.run_on_main_thread(move || panels::set_click_through(ignore))
        .map_err(|error| error.to_string())
}

/// 复制链路的系统出口：在主线程把按键序列写入剪贴板。
#[tauri::command]
fn copy_command(lhs: String, app: AppHandle) -> Result<(), String> {
    app.run_on_main_thread(move || panels::copy_text(&lhs))
        .map_err(|error| error.to_string())
}

/// 上下文链路的预览入口：把前端构造的上下文按真实上下文同样发布。
#[tauri::command]
fn preview_context(
    context: EditorContext,
    app: AppHandle,
) -> Result<(), String> {
    publish_context(app, context)
}

/// 执行链路的命令入口：把按键解析、风险确认和会话发送串成一次执行。
#[tauri::command(async)]
fn execute_command(
    lhs: String,
    session_id: Option<String>,
    app: AppHandle,
) -> Result<(), String> {
    execute_with_confirmation(&app, &lhs, session_id.as_deref())
}

/// 会话链路的列表读取：返回当前已知会话摘要。
#[tauri::command]
fn list_sessions(state: tauri::State<'_, AppState>) -> Vec<SessionSummary> {
    state.sessions.list()
}

/// 会话链路的接入口：连接指定 Neovim socket 并登记工作目录。
#[tauri::command]
fn connect_session(
    profile_id: String,
    socket_path: String,
    cwd: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<SessionSummary, String> {
    state
        .sessions
        .connect(&profile_id, socket_path, cwd.as_deref())
}

/// 环境链路的依赖体检：按当前数据目录检查外部依赖状态。
#[tauri::command]
fn check_environment(
    state: tauri::State<'_, AppState>,
) -> Vec<DependencyStatus> {
    environment::check_with_paths(&state.profiles.paths)
}

/// Profile 管理链路的列表读取：返回全部配置的摘要。
#[tauri::command]
fn list_profiles(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ProfileSummary>, String> {
    state.profiles.list()
}

/// Profile 管理链路的创建入口：校验并保存一份新配置。
#[tauri::command]
fn create_profile(
    config: ProfileConfig,
    state: tauri::State<'_, AppState>,
) -> Result<ProfileSummary, String> {
    state.profiles.create(config)
}

/// Profile 管理链路的更新入口：用提交内容替换已有配置。
#[tauri::command]
fn update_profile(
    config: ProfileConfig,
    state: tauri::State<'_, AppState>,
) -> Result<ProfileSummary, String> {
    state.profiles.update(config)
}

/// Profile 管理链路的删除入口：移除指定配置及其关联数据。
#[tauri::command]
fn delete_profile(
    profile_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state.profiles.delete(&profile_id)
}

/// Profile 设置链路的默认项入口：把指定配置设为后续启动的默认选择。
#[tauri::command]
fn set_default_profile(
    profile_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    set_default_profile_state(&state, &profile_id)
}

/// Profile 管理链路的重命名入口：修改标识与显示名并保持默认项一致。
#[tauri::command]
fn rename_profile(
    profile_id: String,
    new_id: String,
    new_name: String,
    state: tauri::State<'_, AppState>,
) -> Result<ProfileSummary, String> {
    rename_profile_state(&state, &profile_id, &new_id, &new_name)
}

/// Profile 设置链路的默认项落盘：同步更新磁盘设置和内存默认值。
fn set_default_profile_state(
    state: &AppState,
    profile_id: &str,
) -> Result<(), String> {
    let _guard = profiles::settings_lock()
        .lock()
        .map_err(|_| "应用设置锁已失效".to_string())?;
    if !state.profiles.paths.profile_dir(profile_id).is_dir() {
        return Err("Profile 不存在".into());
    }
    let mut settings = state.settings.lock().expect("settings lock").clone();
    settings.default_profile = profile_id.to_string();
    state.profiles.save_app_settings_unlocked(&settings)?;
    *state.settings.lock().expect("settings lock") = settings;
    Ok(())
}

/// Profile 设置链路的重命名事务：目录改名后同步默认项，失败时回滚。
fn rename_profile_state(
    state: &AppState,
    profile_id: &str,
    new_id: &str,
    new_name: &str,
) -> Result<ProfileSummary, String> {
    let _guard = profiles::settings_lock()
        .lock()
        .map_err(|_| "应用设置锁已失效".to_string())?;
    let previous = state.profiles.summary(profile_id)?;
    let previous_default =
        state.settings.lock().expect("settings lock").clone();
    let summary = state.profiles.rename(profile_id, new_id, new_name)?;
    if previous_default.default_profile == profile_id {
        let mut settings = previous_default;
        settings.default_profile = new_id.to_string();
        if let Err(error) = state.profiles.save_app_settings_unlocked(&settings)
        {
            let rollback = state.profiles.rename(
                new_id,
                profile_id,
                &previous.config.name,
            );
            return Err(match rollback {
                Ok(_) => {
                    format!("默认 Profile 设置保存失败，已恢复重命名：{error}")
                }
                Err(rollback) => {
                    format!(
                        "默认 Profile 设置保存失败：{error}；恢复重命名也失败：{rollback}"
                    )
                }
            });
        }
        *state.settings.lock().expect("settings lock") = settings;
    }
    Ok(summary)
}

/// Profile 管理链路的导出口：把指定配置打包为可迁移文本。
#[tauri::command]
fn export_profile(
    profile_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    state.profiles.export(&profile_id)
}

/// Profile 管理链路的安装入口：在阻塞线程安装依赖并汇总结果。
#[tauri::command]
async fn install_profile(
    profile_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<InstallReport, String> {
    let profiles = state.profiles.clone();
    tauri::async_runtime::spawn_blocking(move || profiles.install(&profile_id))
        .await
        .map_err(|error| format!("安装任务失败：{error}"))?
}

/// Profile 管理链路的版本回退：在阻塞线程恢复上一份可用安装。
#[tauri::command]
async fn rollback_profile(
    profile_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<ProfileSummary, String> {
    let profiles = state.profiles.clone();
    tauri::async_runtime::spawn_blocking(move || profiles.rollback(&profile_id))
        .await
        .map_err(|error| format!("回滚任务失败：{error}"))?
}

/// 启动链路的会话建立：启动目标 Profile，再等待并连接其 Neovim socket。
#[tauri::command]
async fn launch_profile(
    request: LaunchRequest,
    state: tauri::State<'_, AppState>,
) -> Result<SessionSummary, String> {
    let profiles = state.profiles.clone();
    let sessions = state.sessions.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let target = profiles::launch(
            &profiles,
            &request.profile_id,
            request.cwd.as_deref(),
        )?;
        sessions.connect_with_retry(
            &target.profile_id,
            &target.socket_path,
            Some(&target.cwd),
        )
    })
    .await
    .map_err(|error| format!("启动任务失败：{error}"))?
}

/// 上下文链路的统一发布点：一份编辑器快照同时写入状态、事件和浮窗。
fn publish_context<R: Runtime>(
    app: AppHandle<R>,
    context: EditorContext,
) -> Result<(), String> {
    let settings = app
        .state::<AppState>()
        .settings
        .lock()
        .expect("settings lock")
        .clone();
    let completed =
        if context.pending.is_empty() && !context.last_input.is_empty() {
            app.state::<AppState>()
                .sessions
                .mappings_for(&context.session_id)
                .ok()
                .and_then(|mappings| {
                    app.state::<AppState>()
                        .dictionary
                        .lock()
                        .expect("dictionary lock")
                        .resolve_lhs(&context.last_input, &mappings)
                })
        } else {
            None
        };
    let snapshot = semantics::snapshot(
        &context,
        settings.query_shortcut.as_str(),
        completed,
    );
    *app.state::<AppState>()
        .context
        .lock()
        .expect("context lock") = context.clone();
    app.emit("editor-context", &context)
        .map_err(|error| error.to_string())?;
    let session_count = app.state::<AppState>().sessions.list().len();
    app.run_on_main_thread(move || {
        panels::update_passive(snapshot);
        panels::update_session_count(session_count);
    })
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// 执行链路的风险闸门：破坏性操作始终确认，编辑操作按设置确认。
fn requires_execution_confirmation(
    risk: RiskLevel,
    settings: &AppSettings,
) -> bool {
    risk == RiskLevel::Destructive
        || (risk == RiskLevel::Edit && settings.confirm_edits)
}

/// 执行链路的安全核心：先锁定会话、映射和风险，再确认并按版本发送按键。
fn execute_with_confirmation<R: Runtime>(
    app: &AppHandle<R>,
    lhs: &str,
    session_id: Option<&str>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let target = state.sessions.target_session(session_id)?;
    let target_description = state.sessions.target_description(&target)?;
    let (mappings, mapping_revision) =
        state.sessions.mapping_snapshot(&target)?;
    let command = state
        .dictionary
        .lock()
        .expect("dictionary lock")
        .resolve_lhs(lhs, &mappings)
        .ok_or("无法确认该按键的风险，已拒绝执行")?;
    let settings = state.settings.lock().expect("settings lock").clone();
    let requires_confirmation =
        requires_execution_confirmation(command.risk, &settings);
    if requires_confirmation {
        let (sender, receiver) = mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            let _ = sender
                .send(panels::confirm_command(&command, &target_description));
        })
        .map_err(|error| error.to_string())?;
        if !receiver.recv().unwrap_or(false) {
            return Err("已取消执行".into());
        }
    }
    state
        .sessions
        .execute_with_revision(&target, lhs, mapping_revision)
}

/// 面板链路的后台分发器：把用户动作路由到查询、复制、会话或执行分支。
fn handle_panel_action<R: Runtime>(
    app: &AppHandle<R>,
    action: panels::PanelAction,
) {
    match action {
        panels::PanelAction::Query(query) => {
            let items = panel_query_items(app, &query);
            let _ = app
                .run_on_main_thread(move || panels::update_candidates(items));
        }
        panels::PanelAction::CycleSession => {
            if let Some(context) =
                app.state::<AppState>().sessions.cycle_active()
                && let Err(error) = publish_context(app.clone(), context)
            {
                let _ = app.emit("command-error", error);
            }
        }
        panels::PanelAction::MovePassive { x, y } => {
            if !x.is_finite() || !y.is_finite() {
                return;
            }
            let generation = {
                let state = app.state::<AppState>();
                let mut settings =
                    state.settings.lock().expect("settings lock");
                if settings.overlay_x == Some(x)
                    && settings.overlay_y == Some(y)
                {
                    return;
                }
                settings.overlay_x = Some(x);
                settings.overlay_y = Some(y);
                state
                    .overlay_move_generation
                    .fetch_add(1, Ordering::Relaxed)
                    + 1
            };
            let app = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(300));
                let state = app.state::<AppState>();
                if state.overlay_move_generation.load(Ordering::Relaxed)
                    != generation
                {
                    return;
                }
                let settings =
                    state.settings.lock().expect("settings lock").clone();
                if let Err(error) =
                    state.profiles.save_app_settings_unlocked(&settings)
                {
                    let _ = app.emit(
                        "command-error",
                        format!("浮窗位置保存失败：{error}"),
                    );
                }
            });
        }
        panels::PanelAction::Copy(lhs) => {
            let _ = app.run_on_main_thread(move || {
                panels::copy_text(&lhs);
                panels::set_query_status(&format!(
                    "已复制；在普通模式依次按下 {lhs}"
                ));
            });
        }
        panels::PanelAction::CloseQuery => {
            let _ = app.run_on_main_thread(panels::hide_query);
        }
        panels::PanelAction::Execute(lhs) => {
            match execute_with_confirmation(app, &lhs, None) {
                Ok(()) => {
                    let _ = app.run_on_main_thread(panels::hide_query);
                    let _ = app.emit("command-error", format!("已发送 {lhs}"));
                }
                Err(error) => {
                    let _ = app.run_on_main_thread(move || {
                        panels::set_query_status(&error)
                    });
                }
            }
        }
    }
}

/// 全局快捷键只负责区分查询与被动浮窗，录制期间不响应。
fn handle_global_shortcut<R: Runtime>(
    app: &AppHandle<R>,
    shortcut: &tauri_plugin_global_shortcut::Shortcut,
) {
    if app
        .state::<AppState>()
        .shortcut_capture_depth
        .load(Ordering::Relaxed)
        > 0
    {
        return;
    }
    let query_shortcut = app
        .state::<AppState>()
        .settings
        .lock()
        .expect("settings lock")
        .query_shortcut
        .parse::<tauri_plugin_global_shortcut::Shortcut>()
        .ok()
        .map(|shortcut| shortcut.id());
    if Some(shortcut.id()) == query_shortcut {
        let items = panel_query_items(app, "");
        let _ = app.run_on_main_thread(move || {
            panels::update_candidates(items);
            panels::show_query();
        });
    } else {
        let _ = app.run_on_main_thread(panels::toggle_passive);
    }
}

/// 快捷键插件只保留注册配置，实际动作交给统一分发器。
fn global_shortcut_plugin(
    settings: &AppSettings,
) -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_shortcut(settings.query_shortcut.as_str())
        .expect("valid query shortcut")
        .with_shortcut(settings.overlay_shortcut.as_str())
        .expect("valid overlay shortcut")
        .with_handler(|app, shortcut, event| {
            if event.state == ShortcutState::Pressed {
                handle_global_shortcut(app, shortcut);
            }
        })
        .build()
}

/// 上下文事件先校验会话身份，再记录并发布最新现场。
fn handle_context_event<R: Runtime>(
    app: &AppHandle<R>,
    session_id: &str,
    json: &str,
) {
    match serde_json::from_str::<EditorContext>(json) {
        Ok(context) if context.session_id == session_id => {
            if let Some(context) =
                app.state::<AppState>().sessions.record_context(context)
                && let Err(error) = publish_context(app.clone(), context)
            {
                let _ = app.emit("command-error", error);
            }
        }
        Ok(_) => {
            let _ = app
                .emit("command-error", "忽略了会话 ID 不匹配的 Neovim 上下文");
        }
        Err(error) => {
            let _ = app.emit(
                "command-error",
                format!("无法解析 Neovim 上下文：{error}"),
            );
        }
    }
}

/// 会话事件按映射、上下文、关闭与错误分别进入对应状态链路。
fn handle_session_event<R: Runtime>(app: &AppHandle<R>, event: SessionEvent) {
    match event {
        SessionEvent::Mappings {
            session_id,
            mappings,
        } => {
            app.state::<AppState>()
                .sessions
                .record_mappings(&session_id, mappings);
        }
        SessionEvent::Context { session_id, json } => {
            handle_context_event(app, &session_id, &json);
        }
        SessionEvent::Closed(session_id) => {
            if app.state::<AppState>().sessions.disconnect(&session_id) {
                let _ = publish_context(app.clone(), EditorContext::default());
            }
            let _ = app.emit("session-closed", session_id);
        }
        SessionEvent::Error { session_id, error } => {
            let _ = app.emit(
                "command-error",
                format!("Neovim 会话 {session_id}：{error}"),
            );
        }
    }
}

/// 两类后台事件泵在启动时统一创建，接收端关闭即自然退出。
fn spawn_event_pumps<R: Runtime>(
    app: &AppHandle<R>,
    action_receiver: mpsc::Receiver<panels::PanelAction>,
    session_receiver: mpsc::Receiver<SessionEvent>,
) {
    let panel_app = app.clone();
    std::thread::spawn(move || {
        while let Ok(action) = action_receiver.recv() {
            handle_panel_action(&panel_app, action);
        }
    });
    let session_app = app.clone();
    std::thread::spawn(move || {
        while let Ok(event) = session_receiver.recv() {
            handle_session_event(&session_app, event);
        }
    });
}

/// Tauri 命令注册表集中声明前端可调用的后端边界。
fn command_handler()
-> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        search_commands,
        all_commands,
        save_command_override,
        get_app_settings,
        begin_shortcut_capture,
        end_shortcut_capture,
        update_app_settings,
        current_context,
        show_query_panel,
        hide_query_panel,
        toggle_overlay,
        center_overlay,
        set_overlay_click_through,
        copy_command,
        preview_context,
        execute_command,
        list_sessions,
        connect_session,
        check_environment,
        list_profiles,
        create_profile,
        update_profile,
        delete_profile,
        set_default_profile,
        rename_profile,
        export_profile,
        install_profile,
        rollback_profile,
        launch_profile,
    ]
}

/// 应用启动链路的总装入口：构建共享状态、插件和事件泵后进入 Tauri 主循环。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (action_sender, action_receiver) = mpsc::channel();
    let (session_sender, session_receiver) = mpsc::channel();
    let paths = AppPaths::discover().expect("无法确定 LazyKeys 数据目录");
    let app_state =
        AppState::new(session_sender, paths).expect("无法加载 LazyKeys 配置");
    let initial_settings =
        app_state.settings.lock().expect("settings lock").clone();
    tauri::Builder::default()
        .plugin(global_shortcut_plugin(&initial_settings))
        .manage(app_state)
        .invoke_handler(command_handler())
        .setup(move |app| {
            panels::initialize(action_sender)?;
            panels::apply_settings(&initial_settings);
            spawn_event_pumps(app.handle(), action_receiver, session_receiver);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running lazykeys");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renaming_default_profile_updates_memory_and_disk_atomically() {
        let temp = tempfile::tempdir().unwrap();
        let (sender, _) = mpsc::channel();
        let state =
            AppState::new(sender, AppPaths::new(temp.path().to_path_buf()))
                .unwrap();
        state.profiles.create(ProfileConfig::default()).unwrap();
        set_default_profile_state(&state, "default").unwrap();
        rename_profile_state(&state, "default", "renamed", "新名称").unwrap();
        assert_eq!(state.settings.lock().unwrap().default_profile, "renamed");
        assert_eq!(
            state.profiles.app_settings().unwrap().default_profile,
            "renamed"
        );
        let other = ProfileConfig {
            id: "other".into(),
            ..Default::default()
        };
        state.profiles.create(other).unwrap();
        rename_profile_state(&state, "other", "other-new", "其他").unwrap();
        assert_eq!(state.settings.lock().unwrap().default_profile, "renamed");
    }

    #[test]
    fn backend_confirmation_never_disables_destructive_actions() {
        let mut settings = AppSettings {
            confirm_edits: false,
            ..Default::default()
        };
        assert!(!requires_execution_confirmation(RiskLevel::Read, &settings));
        assert!(!requires_execution_confirmation(RiskLevel::Edit, &settings));
        assert!(requires_execution_confirmation(
            RiskLevel::Destructive,
            &settings
        ));
        settings.confirm_edits = true;
        assert!(requires_execution_confirmation(RiskLevel::Edit, &settings));
    }
}

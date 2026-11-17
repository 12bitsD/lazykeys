use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::environment;
use crate::models::{
    AppSettings, CommandOverride, InstallReport, ProfileConfig, ProfileSummary,
};

/// 路径模型｜安装、配置、回滚与启动共用的目录地图；所有受管文件都从这些根目录派生。
#[derive(Debug, Clone)]
pub struct AppPaths {
    /// 配置根目录：每个 Profile 的 manifest 与 Neovim 配置都以其 ID 为子目录。
    pub config_root: PathBuf,
    /// 数据根目录：承载应用设置、命令覆盖等跨 Profile 数据。
    pub data_root: PathBuf,
    /// 工具根目录：依赖检查与安装在此放置受管 Neovim 和 tree-sitter。
    pub tools: PathBuf,
    /// 快照根目录：安装前按 Profile 保存可回滚的完整配置副本。
    pub snapshots: PathBuf,
    /// 运行时根目录：启动阶段在此生成 Unix socket 与一次性启动脚本。
    pub runtime: PathBuf,
    /// 构建根目录：首次安装时暂存 Starter 克隆，避免污染正式配置。
    pub build: PathBuf,
}

impl AppPaths {
    #[cfg(test)]
    pub fn new(root: PathBuf) -> Self {
        Self {
            config_root: root.join("config/lazykeys"),
            data_root: root.join("data/lazykeys"),
            tools: root.join("tools"),
            snapshots: root.join("snapshots"),
            runtime: root.join("run"),
            build: root.join("build"),
        }
    }

    /// 路径发现｜安装与启动的入口；把 HOME/XDG 环境映射成统一的受管目录模型。
    pub fn discover() -> Result<Self, String> {
        let home = std::env::var_os("HOME").ok_or("无法确定 HOME")?;
        let home = PathBuf::from(home);
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        Ok(Self {
            config_root: config_home.join("lazykeys"),
            data_root: data_home.join("lazykeys"),
            tools: data_home.join("lazykeys/tools"),
            snapshots: data_home.join("lazykeys/snapshots"),
            runtime: runtime_root(&home),
            build: data_home.join("lazykeys/build"),
        })
    }

    /// 配置路径派生｜以 Profile ID 定位其独立配置目录；后续读写都不越过这一边界。
    pub fn profile_dir(&self, id: &str) -> PathBuf {
        self.config_root.join(id)
    }

    /// 配置路径派生｜manifest 是 Profile 配置与安装就绪状态的持久化入口。
    pub fn manifest_path(&self, id: &str) -> PathBuf {
        self.profile_dir(id).join("lazykeys-manifest.json")
    }

    /// 配置路径派生｜pending 文件保存待安装配置；发布前不替换当前可用 manifest。
    fn pending_path(&self, id: &str) -> PathBuf {
        self.profile_dir(id).join("lazykeys-pending.json")
    }

    /// 启动路径派生｜为摘要提供 Profile 的稳定 socket 地址；实际启动另用唯一地址避免冲突。
    pub fn socket_path(&self, id: &str) -> PathBuf {
        self.runtime.join(format!("{id}.sock"))
    }
}

/// 启动路径派生｜用 HOME 哈希缩短运行时目录；长用户目录也不会撑破 Unix socket 路径上限。
fn runtime_root(home: &Path) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;

    let digest = Sha256::digest(home.as_os_str().as_bytes());
    let id = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    PathBuf::from("/tmp")
        .join(format!("lazykeys-{id}"))
        .join("run")
}

/// 安装与启动边界｜创建并收紧运行时父目录和运行目录；socket 与脚本只落在私有真实目录中。
fn create_runtime_dir(path: &Path) -> Result<(), String> {
    let parent = path.parent().ok_or("runtime 目录缺少受管父目录")?;
    ensure_private_directory(parent)?;
    ensure_private_directory(path)
}

/// 安装与启动边界｜校验目录归属、真实类型和 0700 权限；拒绝符号链接及他人拥有的运行时路径。
fn ensure_private_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    unsafe extern "C" {
        fn getuid() -> u32;
    }
    let current_uid = unsafe { getuid() };
    let inspect = |path: &Path| {
        let metadata =
            fs::symlink_metadata(path).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "runtime 路径必须是真实目录，不能是符号链接：{}",
                path.display()
            ));
        }
        if metadata.uid() != current_uid {
            return Err(format!(
                "runtime 目录不属于当前用户：{}",
                path.display()
            ));
        }
        Ok(metadata)
    };
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| error.to_string())?;
        }
        Err(error) => return Err(error.to_string()),
    }
    inspect(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    let metadata = inspect(path)?;
    if metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(format!(
            "runtime 目录权限无法收紧到 0700：{}",
            path.display()
        ));
    }
    Ok(())
}

/// manifest 写入状态使用显式分类，避免布尔参数隐藏发布语义。
#[derive(Clone, Copy)]
enum ManifestStatus {
    Ready,
    Pending,
}

/// 磁盘清单模型｜配置读写与安装发布的元数据；一份 JSON 同时记录格式版本、配置和就绪状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProfileManifest {
    /// 清单格式版本：读取时据此拒绝不兼容结构。
    schema_version: u32,
    /// 已发布的 `ProfileConfig`：编辑中的 pending 配置不会提前覆盖它。
    config: ProfileConfig,
    /// 安装健康检查结论：只有与磁盘 `init.lua` 同时成立才允许启动。
    ready: bool,
}

impl ProfileManifest {
    /// 重命名等维护操作沿用磁盘上的发布状态，不重新推断。
    fn status(&self) -> ManifestStatus {
        if self.ready {
            ManifestStatus::Ready
        } else {
            ManifestStatus::Pending
        }
    }
}

/// 配置存储入口｜列表、编辑、安装、回滚与启动共用的仓储；`AppPaths` 是唯一路径依据。
#[derive(Clone)]
pub struct ProfileStore {
    /// 当前仓储所有配置、快照、工具与运行时文件的目录地图。
    pub paths: AppPaths,
}

/// 安装暂存模型｜把正式发布目录、隔离暂存目录与既有配置状态放在一起，供发布和清理共用。
struct InstallWorkspace {
    /// 本次安装唯一的临时根目录，结束后必须清理。
    home: PathBuf,
    /// 正式 Profile 目录，只有健康检查通过后才被替换。
    directory: PathBuf,
    /// 隔离生成与同步的工作目录，不直接影响现有配置。
    staging: PathBuf,
    /// 安装前是否存在可运行配置，决定快照更新还是首次克隆。
    had_working_profile: bool,
}

impl ProfileStore {
    /// 仓储构造｜把目录地图绑定到配置操作；安装、回滚与启动据此共享同一文件布局。
    pub fn new(paths: AppPaths) -> Self {
        Self { paths }
    }

    /// 配置列表｜扫描受管目录并生成摘要；没有 Profile 时以默认配置提供首个入口。
    pub fn list(&self) -> Result<Vec<ProfileSummary>, String> {
        let mut profiles = Vec::new();
        match fs::read_dir(&self.paths.config_root) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry.map_err(|error| error.to_string())?;
                    if entry
                        .file_type()
                        .map_err(|error| error.to_string())?
                        .is_dir()
                    {
                        let id =
                            entry.file_name().to_string_lossy().to_string();
                        profiles.push(self.summary(&id)?);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        if profiles.is_empty() {
            profiles.push(self.summary("default")?);
        }
        profiles.sort_by(|a, b| a.config.id.cmp(&b.config.id));
        Ok(profiles)
    }

    /// 配置摘要｜列表与启动前的读模型；`config` 取自已发布配置，
    /// `ready` 合并 manifest 与 `init.lua`，其余字段派生路径和默认标记。
    pub fn summary(&self, id: &str) -> Result<ProfileSummary, String> {
        validate_id(id)?;
        let manifest = self.read_manifest(id)?;
        let config = manifest
            .as_ref()
            .map(|manifest| manifest.config.clone())
            .unwrap_or_else(|| ProfileConfig {
                id: id.to_string(),
                ..ProfileConfig::default()
            });
        Ok(ProfileSummary {
            ready: manifest.map(|manifest| manifest.ready).unwrap_or(false)
                && self.paths.profile_dir(id).join("init.lua").exists(),
            config,
            config_path: self.paths.profile_dir(id).display().to_string(),
            socket_path: self.paths.socket_path(id).display().to_string(),
            is_default: id == self.default_profile()?,
        })
    }

    /// 配置创建｜校验并原子写入未就绪 manifest；只建立配置身份，安装链路随后才生成可用环境。
    pub fn create(
        &self,
        config: ProfileConfig,
    ) -> Result<ProfileSummary, String> {
        let lock = profile_lock(&config.id);
        let _guard = lock
            .lock()
            .map_err(|_| "Profile 创建锁已失效".to_string())?;
        validate_profile(&config)?;
        if self.paths.profile_dir(&config.id).exists() {
            return Err("Profile ID 已存在".into());
        }
        self.write_manifest(&config, ManifestStatus::Pending)?;
        self.summary(&config.id)
    }

    /// 配置编辑｜把新 `ProfileConfig` 写为 pending；当前 manifest 与可启动环境保持原样，直到下次安装发布。
    pub fn update(
        &self,
        config: ProfileConfig,
    ) -> Result<ProfileSummary, String> {
        let lock = profile_lock(&config.id);
        let _guard = lock
            .lock()
            .map_err(|_| "Profile 更新锁已失效".to_string())?;
        validate_profile(&config)?;
        self.read_manifest(&config.id)?
            .ok_or("Profile 不存在，请先创建")?;
        atomic_write(
            &self.paths.pending_path(&config.id),
            &serde_json::to_vec_pretty(&config)
                .map_err(|error| error.to_string())?,
        )?;
        self.summary(&config.id)
    }

    #[cfg(test)]
    pub fn set_default(&self, id: &str) -> Result<(), String> {
        let _guard = settings_lock()
            .lock()
            .map_err(|_| "应用设置锁已失效".to_string())?;
        validate_id(id)?;
        if !self.paths.profile_dir(id).is_dir() {
            return Err("Profile 不存在".into());
        }
        let mut settings = self.app_settings()?;
        settings.default_profile = id.to_string();
        self.save_app_settings_unlocked(&settings)
    }

    /// 配置重命名｜在双 ID 锁内移动目录并改写 manifest；发布写失败时恢复旧目录，避免身份半迁移。
    pub fn rename(
        &self,
        id: &str,
        new_id: &str,
        new_name: &str,
    ) -> Result<ProfileSummary, String> {
        let first = profile_lock(id.min(new_id));
        let _first_guard = first
            .lock()
            .map_err(|_| "Profile 重命名锁已失效".to_string())?;
        let second = profile_lock(id.max(new_id));
        let _second_guard = if id == new_id {
            None
        } else {
            Some(
                second
                    .lock()
                    .map_err(|_| "Profile 重命名锁已失效".to_string())?,
            )
        };
        validate_id(id)?;
        validate_id(new_id)?;
        if new_name.trim().is_empty() {
            return Err("Profile 名称不能为空".into());
        }
        if id == new_id {
            let mut manifest =
                self.read_manifest(id)?.ok_or("Profile 不存在")?;
            manifest.config.name = new_name.to_string();
            self.write_manifest(&manifest.config, manifest.status())?;
            return self.summary(new_id);
        }
        if self.paths.profile_dir(new_id).exists() {
            return Err("目标 Profile ID 已存在".into());
        }
        let current = self.paths.profile_dir(id);
        let mut manifest = self.read_manifest(id)?.ok_or("Profile 不存在")?;
        let target = self.paths.profile_dir(new_id);
        fs::rename(&current, &target).map_err(|error| error.to_string())?;
        manifest.config.id = new_id.to_string();
        manifest.config.name = new_name.to_string();
        if let Err(error) =
            self.write_manifest(&manifest.config, manifest.status())
        {
            let _ = fs::rename(&target, &current);
            return Err(error);
        }
        self.summary(new_id)
    }

    /// 配置导出｜把已安装的完整 Profile 目录压缩到桌面；未安装配置不进入导出链路。
    pub fn export(&self, id: &str) -> Result<String, String> {
        validate_id(id)?;
        let source = self.paths.profile_dir(id);
        if !source.join("init.lua").exists() {
            return Err("Profile 尚未安装，无法导出".into());
        }
        let desktop = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("无法确定 HOME")?
            .join("Desktop");
        fs::create_dir_all(&desktop).map_err(|error| error.to_string())?;
        let destination =
            desktop.join(format!("lazykeys-{id}-{}.zip", timestamp()));
        let status = Command::new("ditto")
            .arg("-c")
            .arg("-k")
            .arg("--sequesterRsrc")
            .arg("--keepParent")
            .arg(&source)
            .arg(&destination)
            .status()
            .map_err(|error| format!("导出 Profile 失败：{error}"))?;
        if !status.success() {
            return Err("导出 Profile 失败".into());
        }
        Ok(destination.display().to_string())
    }

    /// 配置派生｜从应用设置读取默认 Profile ID；摘要的默认标记与删除保护共用这一事实来源。
    fn default_profile(&self) -> Result<String, String> {
        Ok(self.app_settings()?.default_profile)
    }

    /// 应用配置读取｜加载并校验跨 Profile 设置；缺失时使用默认值，损坏时阻止静默覆盖。
    pub fn app_settings(&self) -> Result<AppSettings, String> {
        let path = self.paths.data_root.join("settings.json");
        let settings = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<AppSettings>(&bytes)
                .map_err(|error| format!("应用设置已损坏：{error}"))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                AppSettings::default()
            }
            Err(error) => return Err(error.to_string()),
        };
        validate_settings(&settings)?;
        Ok(settings)
    }

    #[cfg(test)]
    pub fn save_app_settings(
        &self,
        settings: &AppSettings,
    ) -> Result<(), String> {
        let _guard = settings_lock()
            .lock()
            .map_err(|_| "应用设置锁已失效".to_string())?;
        self.save_app_settings_unlocked(settings)
    }

    /// 应用配置写入｜在调用方持锁的前提下校验并原子替换 settings；避免半文件状态被后续读取。
    pub(crate) fn save_app_settings_unlocked(
        &self,
        settings: &AppSettings,
    ) -> Result<(), String> {
        validate_settings(settings)?;
        atomic_write(
            &self.paths.data_root.join("settings.json"),
            &serde_json::to_vec_pretty(settings)
                .map_err(|error| error.to_string())?,
        )
    }

    /// 应用配置读取｜加载命令助记与别名覆盖；文件缺失等价于没有自定义项。
    pub fn command_overrides(&self) -> Result<Vec<CommandOverride>, String> {
        let path = self.paths.data_root.join("dictionary-overrides.json");
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| format!("自定义词典配置已损坏：{error}")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(Vec::new())
            }
            Err(error) => Err(error.to_string()),
        }
    }

    /// 应用配置写入｜按命令 ID 合并覆盖项并原子保存；排序让持久化结果保持稳定。
    pub fn save_command_override(
        &self,
        value: &CommandOverride,
    ) -> Result<(), String> {
        if value.command_id.trim().is_empty()
            || value.mnemonic.trim().is_empty()
        {
            return Err("命令 ID 和助记不能为空".into());
        }
        if value.aliases.iter().any(|alias| alias.trim().is_empty()) {
            return Err("中文别名不能为空".into());
        }
        let mut overrides = self.command_overrides()?;
        if let Some(existing) = overrides
            .iter_mut()
            .find(|existing| existing.command_id == value.command_id)
        {
            *existing = value.clone();
        } else {
            overrides.push(value.clone());
        }
        overrides.sort_by(|a, b| a.command_id.cmp(&b.command_id));
        atomic_write(
            &self.paths.data_root.join("dictionary-overrides.json"),
            &serde_json::to_vec_pretty(&overrides)
                .map_err(|error| error.to_string())?,
        )
    }

    /// 安装暂存路径只计算一次，发布与清理读取同一份目录事实。
    fn install_workspace(&self, id: &str) -> Result<InstallWorkspace, String> {
        let home = self
            .paths
            .config_root
            .parent()
            .ok_or("Profile 配置根目录无效")?
            .join(format!(".lazykeys-install-{}", Uuid::new_v4().simple()));
        let directory = self.paths.profile_dir(id);
        let staging = home.join("lazykeys").join(id);
        let had_working_profile = directory.join("init.lua").exists();
        Ok(InstallWorkspace {
            home,
            directory,
            staging,
            had_working_profile,
        })
    }

    /// 文件准备区分快照更新与首次克隆，随后生成受管配置。
    fn prepare_profile_files(
        &self,
        workspace: &InstallWorkspace,
        config: &ProfileConfig,
        log: &mut Vec<String>,
    ) -> Result<(), String> {
        if workspace.had_working_profile {
            self.snapshot(&config.id)?;
            copy_directory(&workspace.directory, &workspace.staging)?;
            log.push("已保存当前配置快照，并在隔离暂存区更新".into());
        } else {
            self.clone_starter(&workspace.staging)?;
            log.push("已获取 LazyVim Starter".into());
        }
        self.generate_files_at(&workspace.staging, config)?;
        log.push("已生成受管配置，保留 custom.lua".into());
        Ok(())
    }

    /// 插件同步与健康检查全部通过后才生成就绪 manifest。
    fn sync_and_check(
        &self,
        workspace: &InstallWorkspace,
        config: &ProfileConfig,
        log: &mut Vec<String>,
    ) -> Result<(), String> {
        let sync = self.run_nvim_at(
            &workspace.staging,
            &workspace.home,
            &config.id,
            &["+Lazy! sync", "+qa"],
            240,
        )?;
        log.push(sync);
        let health = self.run_nvim_at(
            &workspace.staging,
            &workspace.home,
            &config.id,
            &["+Lazy health", "+qa"],
            240,
        )?;
        if has_health_error(&health) {
            return Err(format!("Lazy health 检查发现错误：\n{health}"));
        }
        log.push(health);
        if !workspace.staging.join("lazy-lock.json").exists() {
            return Err("插件同步结束但未生成 lazy-lock.json".into());
        }
        write_manifest_at(
            &workspace.staging.join("lazykeys-manifest.json"),
            config,
            ManifestStatus::Ready,
        )
    }

    /// 发布链路只在准备与检查全部成功后替换正式目录。
    fn publish_profile(
        &self,
        workspace: &InstallWorkspace,
        config: &ProfileConfig,
        log: &mut Vec<String>,
    ) -> Result<(), String> {
        self.prepare_profile_files(workspace, config, log)?;
        self.sync_and_check(workspace, config, log)?;
        replace_directory(&workspace.staging, &workspace.directory)?;
        log.push("LazyHealth 与启动检查通过，已原子发布 Profile".into());
        if let Err(error) = fs::remove_file(self.paths.pending_path(&config.id))
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log.push(format!("Profile 已发布，但待应用配置清理失败：{error}"));
        }
        Ok(())
    }

    /// 安装主链路｜依赖预检后在隔离目录生成、同步并体检；已有配置先快照，全部通过才原子发布并清除 pending。
    pub fn install(&self, id: &str) -> Result<InstallReport, String> {
        let lock = profile_lock(id);
        let _guard = lock
            .lock()
            .map_err(|_| "Profile 安装锁已失效".to_string())?;
        validate_id(id)?;
        let config = self.config_for_install(id)?;
        fs::create_dir_all(&self.paths.config_root)
            .map_err(|error| error.to_string())?;
        fs::create_dir_all(&self.paths.tools)
            .map_err(|error| error.to_string())?;
        create_runtime_dir(&self.paths.runtime)?;
        let mut log = Vec::new();
        let dependencies = environment::ensure_required_tools(&self.paths)?;
        log.push("依赖预检通过".into());
        let workspace = self.install_workspace(id)?;
        let result = self.publish_profile(&workspace, &config, &mut log);
        let cleanup = fs::remove_dir_all(&workspace.home);
        match (result, cleanup) {
            (Err(error), Ok(())) => return Err(error),
            (Err(error), Err(cleanup)) => {
                return Err(format!(
                    "{error}\n暂存目录清理失败：{cleanup}；路径：{}",
                    workspace.home.display()
                ));
            }
            (Ok(()), Err(cleanup)) => {
                log.push(format!(
                    "新配置已发布，旧配置暂存目录清理失败：{cleanup}；路径：{}",
                    workspace.home.display()
                ));
            }
            (Ok(()), Ok(())) => {}
        }
        Ok(InstallReport {
            profile: self.summary(id)?,
            dependencies,
            log,
        })
    }

    /// 安装配置选择｜pending 优先于已发布 manifest，首次安装回退默认 `ProfileConfig`；生成文件只消费这一份配置快照。
    fn config_for_install(&self, id: &str) -> Result<ProfileConfig, String> {
        let pending = self.paths.pending_path(id);
        match fs::read(&pending) {
            Ok(bytes) => {
                let config: ProfileConfig = serde_json::from_slice(&bytes)
                    .map_err(|error| {
                        format!("待应用 Profile 配置已损坏：{error}")
                    })?;
                validate_profile(&config)?;
                if config.id != id {
                    return Err("待应用配置的 Profile ID 与目录不一致".into());
                }
                Ok(config)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => self
                .read_manifest(id)?
                .map(|manifest| manifest.config)
                .or_else(|| {
                    Some(ProfileConfig {
                        id: id.to_string(),
                        ..ProfileConfig::default()
                    })
                })
                .ok_or("无法确定 Profile 配置".into()),
            Err(error) => Err(error.to_string()),
        }
    }

    /// 回滚主链路｜选择该 Profile 最新快照，在隔离目录复制后原子交换到正式位置；发布结果再转为摘要。
    pub fn rollback(&self, id: &str) -> Result<ProfileSummary, String> {
        let lock = profile_lock(id);
        let _guard = lock
            .lock()
            .map_err(|_| "Profile 回滚锁已失效".to_string())?;
        validate_id(id)?;
        let parent = self.paths.snapshots.join(id);
        let mut snapshots: Vec<_> = fs::read_dir(&parent)
            .map_err(|_| "没有可回滚的快照".to_string())?
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .collect();
        snapshots.sort_by_key(|entry| entry.file_name());
        let latest = snapshots.pop().ok_or("没有可回滚的快照")?;
        let staging_home = self
            .paths
            .config_root
            .parent()
            .ok_or("Profile 配置根目录无效")?
            .join(format!(".lazykeys-rollback-{}", Uuid::new_v4().simple()));
        let staging = staging_home.join("lazykeys").join(id);
        let current = self.paths.profile_dir(id);
        let result = copy_directory(&latest.path(), &staging)
            .and_then(|_| replace_directory(&staging, &current));
        let cleanup = fs::remove_dir_all(&staging_home);
        result?;
        cleanup.map_err(|error| {
            format!("回滚已发布，但暂存目录清理失败：{error}")
        })?;
        self.summary(id)
    }

    /// 配置删除｜默认 Profile 与越界路径先被拒绝；只允许移除受管配置根目录内的指定目录。
    pub fn delete(&self, id: &str) -> Result<(), String> {
        let lock = profile_lock(id);
        let _guard = lock
            .lock()
            .map_err(|_| "Profile 删除锁已失效".to_string())?;
        validate_id(id)?;
        if id == self.default_profile()? {
            return Err(
                "不能删除当前默认 Profile；请先设置另一个默认 Profile".into()
            );
        }
        let directory = self.paths.profile_dir(id);
        if !directory.starts_with(&self.paths.config_root) {
            return Err("拒绝删除受管目录之外的路径".into());
        }
        if directory.exists() {
            fs::remove_dir_all(directory).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// 配置读取边界｜解析 manifest 并校验格式版本与目录 ID；缺失返回空，损坏或不一致直接报错。
    fn read_manifest(
        &self,
        id: &str,
    ) -> Result<Option<ProfileManifest>, String> {
        validate_id(id)?;
        match fs::read(self.paths.manifest_path(id)) {
            Ok(bytes) => {
                let manifest: ProfileManifest = serde_json::from_slice(&bytes)
                    .map_err(|error| {
                        format!("Profile {id} 的 manifest 已损坏：{error}")
                    })?;
                if manifest.schema_version != 1 {
                    return Err(format!(
                        "Profile {id} 的 manifest 版本不受支持"
                    ));
                }
                if manifest.config.id != id {
                    return Err(format!(
                        "Profile {id} 的 manifest ID 与目录不一致"
                    ));
                }
                Ok(Some(manifest))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(None)
            }
            Err(error) => Err(error.to_string()),
        }
    }

    /// 配置写入边界｜校验 `ProfileConfig` 后持久化到其 manifest 路径；就绪状态由安装链路显式给出。
    fn write_manifest(
        &self,
        config: &ProfileConfig,
        status: ManifestStatus,
    ) -> Result<(), String> {
        validate_profile(config)?;
        write_manifest_at(&self.paths.manifest_path(&config.id), config, status)
    }

    /// 首次安装准备｜浅克隆 LazyVim Starter 到构建暂存区，去除 Git 元数据后移入配置暂存目录。
    fn clone_starter(&self, destination: &Path) -> Result<(), String> {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let staging =
            self.paths.build.join(format!("starter-{}", Uuid::new_v4()));
        if let Some(parent) = staging.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let result = (|| {
            run_logged(
                Command::new("git")
                    .args(["clone", "--depth", "1", "--filter=blob:none"])
                    .arg("https://github.com/LazyVim/starter")
                    .arg(&staging),
                120,
            )?;
            fs::remove_dir_all(staging.join(".git"))
                .map_err(|error| error.to_string())?;
            fs::rename(&staging, destination)
                .map_err(|error| error.to_string())?;
            Ok(())
        })();
        if staging.exists() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }

    #[cfg(test)]
    fn generate_files(&self, config: &ProfileConfig) -> Result<(), String> {
        self.generate_files_at(&self.paths.profile_dir(&config.id), config)
    }

    /// 安装配置生成｜把编辑器选项、主题和 extras 映射为受管 Lua 文件；
    /// `custom.lua` 仅缺失时创建，用户内容始终保留。
    fn generate_files_at(
        &self,
        directory: &Path,
        config: &ProfileConfig,
    ) -> Result<(), String> {
        ensure_real_directory(directory, Path::new("lua/config"))?;
        ensure_real_directory(directory, Path::new("lua/plugins"))?;
        let custom_options = directory.join("lua/config/custom.lua");
        if !custom_options.exists() {
            atomic_write(&custom_options, b"")?;
        }
        let options = format!(
            "vim.g.mapleader = {}\nvim.g.maplocalleader = \"\\\\\"\nvim.opt.relativenumber = {}\nvim.opt.wrap = {}\nvim.opt.shiftwidth = {}\nvim.opt.tabstop = {}\nvim.opt.mouse = {}\nrequire('config.custom')\n",
            lua_string(&config.leader),
            config.relative_numbers,
            config.wrap,
            config.indent,
            config.indent,
            if config.mouse { "\"a\"" } else { "\"\"" },
        );
        atomic_write(
            &directory.join("lua/config/options.lua"),
            options.as_bytes(),
        )?;
        let extras = config
            .extras
            .iter()
            .filter_map(|extra| match extra.as_str() {
                "typescript" => Some("lazyvim.plugins.extras.lang.typescript"),
                "python" => Some("lazyvim.plugins.extras.lang.python"),
                _ => None,
            })
            .map(|extra| format!("  {{ import = \"{extra}\" }},"))
            .collect::<Vec<_>>()
            .join("\n");
        let companion = companion_path()?;
        let settings = self.app_settings()?;
        let spec = format!(
            "vim.opt.rtp:prepend({})\nrequire('lazykeys').setup({{ result_duration = {}, show_history = {} }})\nreturn {{\n  {{\n    \"LazyVim/LazyVim\",\n    opts = {{ colorscheme = {}, autoformat = {} }},\n  }},\n{}\n}}\n",
            lua_string(&companion.display().to_string()),
            settings.result_duration_ms,
            settings.show_history,
            lua_string(&config.theme),
            config.autoformat,
            extras
        );
        atomic_write(
            &directory.join("lua/plugins/lazykeys.lua"),
            spec.as_bytes(),
        )
    }

    /// 安装健康检查执行｜在隔离 XDG 环境中无头运行指定 Profile；插件同步与健康检查不触碰用户默认 Neovim 配置。
    fn run_nvim_at(
        &self,
        directory: &Path,
        config_home: &Path,
        id: &str,
        arguments: &[&str],
        timeout_seconds: u64,
    ) -> Result<String, String> {
        let nvim = environment::preferred_nvim(&self.paths);
        let mut command = Command::new(nvim);
        command
            .arg("--headless")
            .arg("-u")
            .arg(directory.join("init.lua"))
            .args(arguments)
            .env("NVIM_APPNAME", format!("lazykeys/{id}"))
            .env("XDG_CONFIG_HOME", config_home)
            .env("PATH", environment::augmented_path(&self.paths))
            .current_dir(directory);
        run_logged(&mut command, timeout_seconds)
    }

    /// 安装前快照｜把当前完整 Profile 复制到“时间戳-UUID”目录；每次更新都有独立回滚点且不会覆盖旧快照。
    fn snapshot(&self, id: &str) -> Result<(), String> {
        let directory = self.paths.profile_dir(id);
        if !directory.exists() {
            return Ok(());
        }
        let destination = self.paths.snapshots.join(id).join(format!(
            "{}-{}",
            timestamp(),
            Uuid::new_v4().simple()
        ));
        copy_directory(&directory, &destination)
    }
}

/// 清单持久化｜把配置与就绪状态包装为当前版本 manifest，再通过原子写入替换目标文件。
fn write_manifest_at(
    path: &Path,
    config: &ProfileConfig,
    status: ManifestStatus,
) -> Result<(), String> {
    validate_profile(config)?;
    let manifest = ProfileManifest {
        schema_version: 1,
        config: config.clone(),
        ready: matches!(status, ManifestStatus::Ready),
    };
    atomic_write(
        path,
        &serde_json::to_vec_pretty(&manifest)
            .map_err(|error| error.to_string())?,
    )
}

/// 启动结果模型｜Terminal 启动成功后的连接契约；调用方凭 Profile、唯一 socket 与工作目录接续会话。
pub struct LaunchTarget {
    /// 已启动的 Profile 身份：对应其独立 `NVIM_APPNAME` 配置。
    pub profile_id: String,
    /// 本次启动的唯一 socket：后续会话连接该 Neovim 实例。
    pub socket_path: PathBuf,
    /// 已规范化的工作目录：Neovim 在此目录打开项目。
    pub cwd: String,
}

/// 工作目录在启动前规范化，未显式提供时使用 HOME。
fn launch_cwd(cwd: Option<&str>) -> Result<PathBuf, String> {
    let cwd = match cwd {
        Some(path) => fs::canonicalize(path)
            .map_err(|error| format!("工作目录无效：{error}"))?,
        None => std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("无法确定 HOME")?,
    };
    if !cwd.is_dir() {
        return Err("工作目录不存在".into());
    }
    Ok(cwd)
}

/// 一次性启动脚本只包含本次 Profile、PATH 与唯一 socket。
fn launcher_command(
    store: &ProfileStore,
    id: &str,
    cwd: &Path,
    socket: &Path,
) -> String {
    let nvim = environment::preferred_nvim(&store.paths);
    let path = environment::augmented_path(&store.paths);
    format!(
        "cd {}\nexport NVIM_APPNAME={}\nexport PATH={}\n{} --listen {} .",
        shell_quote(&cwd.display().to_string()),
        shell_quote(&format!("lazykeys/{id}")),
        shell_quote(&path),
        shell_quote(&nvim.display().to_string()),
        shell_quote(&socket.display().to_string()),
    )
}

/// 启动脚本原子写入并收紧为仅当前用户可执行。
fn create_launcher(
    store: &ProfileStore,
    id: &str,
    command: &str,
) -> Result<PathBuf, String> {
    let launcher = store
        .paths
        .runtime
        .join(format!("{id}-{}.command", Uuid::new_v4()));
    atomic_write(&launcher, launcher_script(command).as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&launcher)
            .map_err(|error| error.to_string())?
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&launcher, permissions)
            .map_err(|error| error.to_string())?;
    }
    Ok(launcher)
}

/// Terminal 失败时保留清理结果，避免把脚本残留隐藏在成功信息之后。
fn open_terminal(launcher: &Path) -> Result<(), String> {
    let result = Command::new("open")
        .arg("-a")
        .arg("Terminal")
        .arg(launcher)
        .status();
    let message = match result {
        Ok(status) if status.success() => return Ok(()),
        Ok(_) => "Terminal 启动 Neovim 失败".to_string(),
        Err(error) => format!("无法启动 Terminal：{error}"),
    };
    Err(match fs::remove_file(launcher) {
        Ok(()) => format!("{message}；启动脚本已清理"),
        Err(cleanup) => format!(
            "{message}；启动脚本清理也失败：{cleanup}；路径：{}",
            launcher.display()
        ),
    })
}

/// 启动主链路｜就绪检查后生成私有、一次性 Terminal 脚本，以独立 socket 启动 Neovim；成功才返回连接目标。
pub fn launch(
    store: &ProfileStore,
    id: &str,
    cwd: Option<&str>,
) -> Result<LaunchTarget, String> {
    validate_id(id)?;
    if !store.summary(id)?.ready {
        return Err("Profile 尚未通过安装与健康检查".into());
    }
    let cwd = launch_cwd(cwd)?;
    create_runtime_dir(&store.paths.runtime)?;
    let socket = store
        .paths
        .runtime
        .join(format!("{id}-{}.sock", Uuid::new_v4()));
    let command = launcher_command(store, id, &cwd, &socket);
    let launcher = create_launcher(store, id, &command)?;
    open_terminal(&launcher)?;
    Ok(LaunchTarget {
        profile_id: id.to_string(),
        socket_path: socket,
        cwd: cwd.display().to_string(),
    })
}

/// 安装依赖定位｜依次从显式环境变量、应用 Resources 和开发目录寻找 companion；其 Lua 入口必须真实存在。
fn companion_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("LAZYKEYS_COMPANION_PATH") {
        let path = PathBuf::from(path);
        if path.join("lua/lazykeys/init.lua").is_file() {
            return Ok(path);
        }
        return Err(
            "LAZYKEYS_COMPANION_PATH 不是有效的 lazykeys.nvim 目录".into()
        );
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(contents) = executable.parent().and_then(Path::parent)
    {
        let bundled = contents.join("Resources/lazykeys.nvim");
        if bundled.join("lua/lazykeys/init.lua").is_file() {
            return Ok(bundled);
        }
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../companion/lazykeys.nvim");
    if !path.join("lua/lazykeys/init.lua").is_file() {
        return Err("找不到 lazykeys.nvim companion".into());
    }
    Ok(path)
}

/// 配置与启动边界｜把 Profile ID 限定为安全目录名；所有磁盘派生路径先经过同一校验。
fn validate_id(id: &str) -> Result<(), String> {
    let valid = !id.is_empty()
        && id.len() <= 40
        && id.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '-'
                || character == '_'
        })
        && id
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_lowercase());
    if valid {
        Ok(())
    } else {
        Err(
            "Profile ID 只能包含小写字母、数字、- 和 _，且必须以字母开头"
                .into(),
        )
    }
}

/// 应用配置边界｜集中校验设置版本、默认 Profile、显示参数与快捷键；非法输入不会进入原子写入。
fn validate_settings(settings: &AppSettings) -> Result<(), String> {
    if settings.schema_version != 1 {
        return Err("应用设置版本不受支持".into());
    }
    validate_id(&settings.default_profile)?;
    if !matches!(
        settings.display_strategy.as_str(),
        "smart" | "always" | "shortcut"
    ) {
        return Err("浮窗显示策略不受支持".into());
    }
    if !(40..=100).contains(&settings.opacity)
        || !(10..=24).contains(&settings.font_size)
        || !(320..=560).contains(&settings.overlay_width)
        || !(500..=5000).contains(&settings.result_duration_ms)
    {
        return Err("浮窗字号、宽度、透明度或结果显示时长超出允许范围".into());
    }
    if settings.query_shortcut.trim().is_empty() {
        return Err("全局查询快捷键不能为空".into());
    }
    settings
        .query_shortcut
        .parse::<tauri_plugin_global_shortcut::Shortcut>()
        .map_err(|error| format!("全局查询快捷键格式无效：{error}"))?;
    if settings.overlay_shortcut.trim().is_empty() {
        return Err("浮窗显示 / 隐藏快捷键不能为空".into());
    }
    settings
        .overlay_shortcut
        .parse::<tauri_plugin_global_shortcut::Shortcut>()
        .map_err(|error| format!("浮窗快捷键格式无效：{error}"))?;
    if settings.overlay_shortcut == settings.query_shortcut {
        return Err("浮窗与查询不能使用同一个快捷键".into());
    }
    Ok(())
}

/// Profile 配置边界｜校验 ID、名称及首版支持的编辑器选项；只有受支持的 `ProfileConfig` 才能持久化或生成 Lua。
fn validate_profile(config: &ProfileConfig) -> Result<(), String> {
    validate_id(&config.id)?;
    if config.name.trim().is_empty() {
        return Err("Profile 名称不能为空".into());
    }
    if !matches!(config.indent, 2 | 4) {
        return Err("缩进只能是 2 或 4".into());
    }
    if !matches!(config.theme.as_str(), "tokyonight" | "catppuccin") {
        return Err("不支持的主题".into());
    }
    if config
        .extras
        .iter()
        .any(|extra| !matches!(extra.as_str(), "typescript" | "python"))
    {
        return Err("包含首版不支持的语言 Extra".into());
    }
    Ok(())
}

/// 安装配置生成｜把用户字符串转义为 Lua 字符串字面量；配置值不会破坏生成文件结构。
fn lua_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// 启动脚本生成｜用 EXIT trap 删除一次性脚本并保留 Neovim 退出状态；启动目录不积累可执行残留。
fn launcher_script(command: &str) -> String {
    format!(
        "#!/bin/bash\ntrap 'status=$?; if ! rm -f -- \"$0\"; then printf \"LazyKeys 启动脚本清理失败：%s\\n\" \"$0\" >&2; fi; exit \"$status\"' EXIT\n{}\n",
        command
    )
}

/// 启动命令构造｜把路径与环境值转义为单引号 shell 参数；特殊字符不会改变启动命令结构。
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// 快照与导出命名｜以 Unix 毫秒时间戳提供可读、可排序的名称前缀；UUID 再保证并发名称唯一。
fn timestamp() -> String {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

/// 安装写入边界｜逐级创建受管子目录并拒绝符号链接；Lua 生成不会借目录跳转写出配置根。
fn ensure_real_directory(
    root: &Path,
    relative: &Path,
) -> Result<PathBuf, String> {
    let root_metadata =
        fs::symlink_metadata(root).map_err(|error| error.to_string())?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err("受管配置根目录必须是真实目录，不能是符号链接".into());
    }
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata)
                if metadata.file_type().is_symlink() || !metadata.is_dir() =>
            {
                return Err(format!(
                    "受管配置目录不能包含符号链接或文件：{}",
                    current.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|error| error.to_string())?;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(current)
}

/// 配置原子写入｜先完整写入同目录唯一临时文件，再以 rename 替换目标；读取方只会看到旧文件或完整新文件。
fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("目标路径没有父目录")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    fs::write(&temporary, contents).map_err(|error| error.to_string())?;
    fs::rename(temporary, path).map_err(|error| error.to_string())
}

/// 安装与回滚发布｜以完整暂存目录替换正式目录；首次直接 rename，已有版本走原子交换或恢复式替换。
pub(crate) fn replace_directory(
    staging: &Path,
    current: &Path,
) -> Result<(), String> {
    if !staging.is_dir() {
        return Err("待发布配置目录不存在，原配置未修改".into());
    }
    if let Some(parent) = current.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    if !current.exists() {
        return fs::rename(staging, current).map_err(|error| error.to_string());
    }
    swap_directories(staging, current)
}

/// macOS 原子发布｜用 `RENAME_SWAP` 一次交换新旧目录；系统调用失败时正式配置保持原位。
#[cfg(target_os = "macos")]
fn swap_directories(first: &Path, second: &Path) -> Result<(), String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    unsafe extern "C" {
        fn renamex_np(from: *const i8, to: *const i8, flags: u32) -> i32;
    }
    const RENAME_SWAP: u32 = 0x00000002;
    let first = CString::new(first.as_os_str().as_bytes())
        .map_err(|error| error.to_string())?;
    let second = CString::new(second.as_os_str().as_bytes())
        .map_err(|error| error.to_string())?;
    if unsafe { renamex_np(first.as_ptr(), second.as_ptr(), RENAME_SWAP) } == 0
    {
        Ok(())
    } else {
        Err(format!(
            "原子交换 Profile 失败，原配置未修改：{}",
            std::io::Error::last_os_error()
        ))
    }
}

/// 非 macOS 发布兜底｜先把旧目录移至备份再发布新目录；第二步失败会恢复旧配置。
#[cfg(not(target_os = "macos"))]
fn swap_directories(first: &Path, second: &Path) -> Result<(), String> {
    let backup = second.with_extension(format!("backup-{}", Uuid::new_v4()));
    fs::rename(second, &backup).map_err(|error| error.to_string())?;
    if let Err(error) = fs::rename(first, second) {
        fs::rename(&backup, second).map_err(|restore| {
            format!("发布失败：{error}；恢复原配置也失败：{restore}")
        })?;
        return Err(error.to_string());
    }
    fs::remove_dir_all(backup).map_err(|error| error.to_string())
}

/// 快照与暂存复制｜递归复制完整目录并保留符号链接；安装更新和回滚都以整棵配置树为单位。
fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    if source.is_symlink() {
        return Err("配置目录不能是符号链接".into());
    }
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        let target = destination.join(entry.file_name());
        if file_type.is_symlink() {
            #[cfg(unix)]
            {
                let link = fs::read_link(entry.path())
                    .map_err(|error| error.to_string())?;
                std::os::unix::fs::symlink(link, &target)
                    .map_err(|error| error.to_string())?;
            }
            #[cfg(not(unix))]
            {
                return Err("当前平台无法保留配置中的符号链接".into());
            }
        } else if file_type.is_dir() {
            copy_directory(&entry.path(), &target)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), target)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

/// 安装健康判定｜逐行去除 ANSI 后识别 LazyHealth 错误标记；只有显式错误会阻断发布。
fn has_health_error(output: &str) -> bool {
    output.lines().any(|line| {
        let clean = strip_ansi(line);
        clean.trim_start().starts_with("- ERROR")
            || clean.trim_start().starts_with("ERROR")
    })
}

/// 健康输出归一化｜移除终端 ANSI 转义序列；颜色控制不会影响错误文本识别。
fn strip_ansi(value: &str) -> String {
    let mut result = String::new();
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            if character.is_ascii_alphabetic() {
                escaped = false;
            }
            continue;
        }
        if character == '\u{1b}' {
            escaped = true;
        } else {
            result.push(character);
        }
    }
    result
}

/// 命令日志清理模型｜持有唯一临时日志路径；守卫生命周期结束即删除安装过程产物。
struct LogFile(PathBuf);

impl Drop for LogFile {
    /// 命令执行收尾｜无论成功或失败都尝试清理临时日志，避免运行时残留。
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// 安装命令执行｜捕获合并输出并设置总时限；超时终止整个进程组，结果与错误都携带同一日志。
fn run_logged(
    command: &mut Command,
    timeout_seconds: u64,
) -> Result<String, String> {
    use std::os::unix::process::CommandExt;

    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    const SIGKILL: i32 = 9;

    let log_path =
        std::env::temp_dir().join(format!("lazykeys-{}.log", Uuid::new_v4()));
    let log_guard = LogFile(log_path.clone());
    let log = fs::File::create(&log_path).map_err(|error| error.to_string())?;
    let stderr = log.try_clone().map_err(|error| error.to_string())?;
    command
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .process_group(0);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let deadline =
        std::time::Instant::now() + Duration::from_secs(timeout_seconds);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => return Err(error.to_string()),
        }
        if std::time::Instant::now() > deadline {
            unsafe {
                kill(-(child.id() as i32), SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            let output = fs::read_to_string(&log_path).unwrap_or_default();
            drop(log_guard);
            return Err(format!(
                "命令执行超过 {timeout_seconds} 秒，已终止整个进程组：\n{output}"
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let output = fs::read_to_string(&log_path).unwrap_or_default();
    drop(log_guard);
    if status.success() {
        Ok(output)
    } else {
        Err(format!("命令失败（{status}）：\n{output}"))
    }
}

/// 应用设置串行化｜提供进程级共享锁；设置读取、修改与原子保存作为同一临界区执行。
pub(crate) fn settings_lock() -> &'static Mutex<()> {
    static LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
    &LOCK
}

/// Profile 操作串行化｜按 ID 缓存共享锁；同一 Profile 的创建、安装、回滚等写操作互不交错。
fn profile_lock(id: &str) -> Arc<Mutex<()>> {
    static LOCKS: LazyLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    LOCKS
        .lock()
        .expect("profile locks")
        .entry(id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, ProfileStore) {
        let temp = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(AppPaths::new(temp.path().to_path_buf()));
        (temp, store)
    }

    #[test]
    fn generates_config_and_preserves_custom_lua() {
        let (_temp, store) = store();
        let config = ProfileConfig::default();
        store
            .write_manifest(&config, ManifestStatus::Pending)
            .unwrap();
        store.generate_files(&config).unwrap();
        let custom = store
            .paths
            .profile_dir("default")
            .join("lua/config/custom.lua");
        fs::write(&custom, "vim.opt.colorcolumn = '100'").unwrap();
        store.generate_files(&config).unwrap();
        assert_eq!(
            fs::read_to_string(custom).unwrap(),
            "vim.opt.colorcolumn = '100'"
        );
        let spec = fs::read_to_string(
            store
                .paths
                .profile_dir("default")
                .join("lua/plugins/lazykeys.lua"),
        )
        .unwrap();
        assert!(spec.contains("lazykeys.nvim"));
        assert!(spec.contains("tokyonight"));
    }

    #[test]
    fn runtime_rejects_preoccupied_symlinks_and_preserves_outside_target() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("sentinel"), "unchanged").unwrap();
        let outside_mode = fs::metadata(&outside).unwrap().permissions().mode();
        let parent = temp.path().join("runtime-parent");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &parent).unwrap();
        let run = parent.join("run");
        assert!(create_runtime_dir(&run).is_err());
        assert_eq!(
            fs::read_to_string(outside.join("sentinel")).unwrap(),
            "unchanged"
        );
        assert_eq!(
            fs::metadata(&outside).unwrap().permissions().mode(),
            outside_mode
        );
        let valid = temp.path().join("valid-parent/run");
        create_runtime_dir(&valid).unwrap();
        assert_eq!(
            fs::metadata(valid.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&valid).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn launcher_cleans_itself_and_preserves_exit_status() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        for (command, expected) in [("true", true), ("false", false)] {
            let path = temp.path().join(format!("launch-{expected}.command"));
            fs::write(&path, launcher_script(command)).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .unwrap();
            let status = Command::new("bash").arg(&path).status().unwrap();
            assert_eq!(status.success(), expected);
            assert!(!path.exists(), "launcher was not removed for {command}");
        }
    }

    #[test]
    fn runtime_socket_stays_short_for_long_home_paths() {
        let home = PathBuf::from("/tmp").join("very-long-home-".repeat(20));
        let paths = AppPaths {
            config_root: home.join(".config/lazykeys"),
            data_root: home.join(".local/share/lazykeys"),
            tools: home.join("tools"),
            snapshots: home.join("snapshots"),
            runtime: runtime_root(&home),
            build: home.join("build"),
        };
        let socket = paths
            .runtime
            .join(format!("default-{}.sock", Uuid::new_v4()));
        assert!(socket.as_os_str().len() < 100, "{}", socket.display());
        assert!(!socket.display().to_string().contains("very-long-home"));
    }

    #[test]
    fn rejects_unsafe_profile_ids_before_launch_writes() {
        let (_temp, store) = store();
        for id in ["../nvim", "UPPER", "a/b", ""] {
            assert!(validate_id(id).is_err());
            assert!(launch(&store, id, None).is_err());
        }
        assert!(validate_id("frontend_1").is_ok());
        assert!(!store.paths.runtime.exists());
    }

    #[test]
    fn corrupted_manifest_is_an_error_not_a_default_profile() {
        let (_temp, store) = store();
        let manifest = store.paths.manifest_path("default");
        fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        fs::write(&manifest, "not-json").unwrap();
        assert!(store.summary("default").unwrap_err().contains("损坏"));
        assert!(store.list().is_err());
    }

    #[test]
    fn update_keeps_ready_manifest_until_install_publishes() {
        let (_temp, store) = store();
        let config = ProfileConfig::default();
        store
            .write_manifest(&config, ManifestStatus::Ready)
            .unwrap();
        let mut changed = config.clone();
        changed.wrap = true;
        store.update(changed).unwrap();
        let manifest = store.read_manifest("default").unwrap().unwrap();
        assert!(manifest.ready);
        assert!(!manifest.config.wrap);
        assert!(store.paths.pending_path("default").is_file());
    }

    #[test]
    fn snapshot_and_rollback_restore_managed_files_and_symlinks() {
        let (_temp, store) = store();
        let config = ProfileConfig::default();
        store
            .write_manifest(&config, ManifestStatus::Pending)
            .unwrap();
        store.generate_files(&config).unwrap();
        let custom = store
            .paths
            .profile_dir("default")
            .join("lua/config/custom.lua");
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            "custom.lua",
            custom.with_file_name("linked.lua"),
        )
        .unwrap();
        store.snapshot("default").unwrap();
        let options = store
            .paths
            .profile_dir("default")
            .join("lua/config/options.lua");
        fs::write(&options, "broken").unwrap();
        store.rollback("default").unwrap();
        assert!(fs::read_to_string(options).unwrap().contains("mapleader"));
        #[cfg(unix)]
        {
            let link = custom.with_file_name("linked.lua");
            assert_eq!(
                fs::read_link(link).unwrap(),
                PathBuf::from("custom.lua")
            );
        }
        assert_eq!(fs::read_to_string(custom).unwrap(), "");
    }

    #[test]
    fn failed_publish_does_not_remove_current_configuration() {
        let (_temp, store) = store();
        let current = store.paths.profile_dir("default");
        fs::create_dir_all(&current).unwrap();
        fs::write(current.join("sentinel"), "working-profile").unwrap();
        let missing = current.with_file_name("missing-staging");
        assert!(replace_directory(&missing, &current).is_err());
        assert_eq!(
            fs::read_to_string(current.join("sentinel")).unwrap(),
            "working-profile"
        );
    }

    #[test]
    fn successful_publish_swaps_complete_directories() {
        let (_temp, store) = store();
        let current = store.paths.profile_dir("default");
        let staging = current.with_file_name("staging");
        fs::create_dir_all(&current).unwrap();
        fs::create_dir_all(&staging).unwrap();
        fs::write(current.join("version"), "old").unwrap();
        fs::write(staging.join("version"), "new").unwrap();
        replace_directory(&staging, &current).unwrap();
        assert_eq!(fs::read_to_string(current.join("version")).unwrap(), "new");
        assert_eq!(fs::read_to_string(staging.join("version")).unwrap(), "old");
    }

    #[test]
    fn deleting_default_profile_is_rejected() {
        let (_temp, store) = store();
        store
            .write_manifest(&ProfileConfig::default(), ManifestStatus::Pending)
            .unwrap();
        assert!(store.delete("default").unwrap_err().contains("默认"));
        assert!(store.paths.profile_dir("default").exists());
    }

    #[test]
    fn delete_is_limited_to_managed_profile() {
        let (_temp, store) = store();
        assert!(store.delete("../nvim").is_err());
    }

    #[test]
    fn settings_and_dictionary_overrides_round_trip_without_losing_preferences()
    {
        let (_temp, store) = store();
        store
            .write_manifest(&ProfileConfig::default(), ManifestStatus::Pending)
            .unwrap();
        let settings = AppSettings {
            opacity: 72,
            query_shortcut: "Control+Option+L".into(),
            ..Default::default()
        };
        store.save_app_settings(&settings).unwrap();
        store.set_default("default").unwrap();
        let loaded = store.app_settings().unwrap();
        assert_eq!(loaded.opacity, 72);
        assert_eq!(loaded.query_shortcut, "Control+Option+L");
        assert_eq!(loaded.default_profile, "default");
        let value = CommandOverride {
            command_id: "find-files".into(),
            aliases: vec!["找源码".into()],
            mnemonic: "项目文件入口".into(),
        };
        store.save_command_override(&value).unwrap();
        assert_eq!(store.command_overrides().unwrap(), vec![value]);
        let mut invalid = loaded;
        invalid.opacity = 1;
        assert!(store.save_app_settings(&invalid).is_err());
        assert_eq!(store.app_settings().unwrap().opacity, 72);
    }

    #[test]
    fn managed_generation_refuses_directory_symlink_escape() {
        let (_temp, store) = store();
        let staging = _temp.path().join("staging");
        let outside = _temp.path().join("outside");
        fs::create_dir_all(staging.join("lua")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("options.lua"), "sentinel").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, staging.join("lua/config"))
            .unwrap();
        let error = store
            .generate_files_at(&staging, &ProfileConfig::default())
            .unwrap_err();
        assert!(error.contains("符号链接"), "{error}");
        assert_eq!(
            fs::read_to_string(outside.join("options.lua")).unwrap(),
            "sentinel"
        );
        let valid = _temp.path().join("valid");
        fs::create_dir_all(&valid).unwrap();
        store
            .generate_files_at(&valid, &ProfileConfig::default())
            .unwrap();
        assert!(valid.join("lua/config/options.lua").is_file());
    }

    #[test]
    fn invalid_shortcut_is_rejected_without_replacing_settings() {
        let (_temp, store) = store();
        let mut settings = AppSettings {
            opacity: 72,
            ..Default::default()
        };
        store.save_app_settings(&settings).unwrap();
        settings.query_shortcut = "not a valid shortcut !!!".into();
        assert!(store.save_app_settings(&settings).is_err());
        assert_eq!(store.app_settings().unwrap().opacity, 72);
        assert_eq!(
            store.app_settings().unwrap().query_shortcut,
            "Control+Option+K"
        );
    }

    #[test]
    fn health_output_must_not_contain_errors() {
        assert!(has_health_error("lazy.nvim\n- ERROR broken plugin"));
        assert!(has_health_error("\u{1b}[31mERROR broken\u{1b}[0m"));
        assert!(!has_health_error(
            "lazy.nvim\n- OK all plugins loaded\n- WARNING optional"
        ));
    }

    #[test]
    fn command_timeout_kills_process_group_and_cleans_log() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("printf timeout-output; sleep 30");
        let error = run_logged(&mut command, 1).unwrap_err();
        assert!(error.contains("timeout-output"));
        assert!(error.contains("进程组"));
        let logs: Vec<_> = fs::read_dir(std::env::temp_dir())
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry.file_name().to_string_lossy().starts_with("lazykeys-")
            })
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "log")
            })
            .collect();
        assert!(logs.is_empty(), "temporary logs were not cleaned: {logs:?}");
    }
}

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{LazyLock, Mutex, MutexGuard};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::models::DependencyStatus;
use crate::profiles::{AppPaths, replace_directory};

const NVIM_VERSION: &str = "0.12.6";
const NVIM_RELEASE_TAG: &str = "v0.12.6";
const NVIM_ARCHIVE: &str = "nvim-macos-arm64.tar.gz";
const TREE_SITTER_VERSION: &str = "0.27.2";
const TREE_SITTER_RELEASE_TAG: &str = "v0.27.2";
const TREE_SITTER_ARCHIVE: &str = "tree-sitter-macos-arm64.gz";
const MINIMUM_NVIM: (u64, u64, u64) = (0, 11, 2);
const MINIMUM_GIT: (u64, u64, u64) = (2, 19, 0);

/// 工具路径派生｜受管依赖的版本化安装根目录；版本升级对应新的隔离目录。
fn nvim_root(paths: &AppPaths) -> PathBuf {
    paths.tools.join(format!("nvim-{NVIM_VERSION}"))
}

/// 工具路径派生｜tree-sitter 的版本化安装根目录；与 Neovim 安装互不覆盖。
fn tree_sitter_root(paths: &AppPaths) -> PathBuf {
    paths
        .tools
        .join(format!("tree-sitter-{TREE_SITTER_VERSION}"))
}

/// 启动依赖定位｜返回受管 Neovim 的固定可执行路径；系统版本不可用时作为后备。
pub fn managed_nvim(paths: &AppPaths) -> PathBuf {
    nvim_root(paths).join("nvim-macos-arm64/bin/nvim")
}

/// 启动依赖定位｜返回受管 tree-sitter 的固定可执行路径；系统缺失时作为后备。
fn managed_tree_sitter(paths: &AppPaths) -> PathBuf {
    tree_sitter_root(paths).join("tree-sitter")
}

/// 系统依赖发现｜按 PATH 顺序查找第一个可执行文件；只识别路径，不判断版本。
pub fn find_executable(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&paths) {
        let candidate = directory.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// 依赖检查基础｜把“文件存在且有执行权限”视为可运行；版本与用途由上层判断。
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// 依赖检查基础｜执行版本命令并取首行输出；执行失败或退出异常统一视为未探测到版本。
fn version_output(command: &str, argument: &str) -> Option<String> {
    let output = Command::new(command).arg(argument).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .and_then(|text| text.lines().next().map(str::to_string))
}

/// 依赖检查基础｜从工具输出提取前三段数字版本；无法得到完整三元组时不做可用性判断。
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let numbers: Vec<u64> = text
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .take(3)
        .collect();
    if numbers.len() == 3 {
        Some((numbers[0], numbers[1], numbers[2]))
    } else {
        None
    }
}

/// 依赖检查基础｜按语义化版本三元组比较最低要求；决定系统工具能否直接使用。
fn version_at_least(
    version: &(u64, u64, u64),
    minimum: (u64, u64, u64),
) -> bool {
    version >= &minimum
}

/// 系统依赖判定｜PATH 中的 Neovim 必须可执行且达到最低版本；否则进入受管后备链路。
fn system_nvim_is_usable() -> bool {
    find_executable("nvim")
        .and_then(|path| version_output(path.to_str()?, "--version"))
        .as_deref()
        .and_then(parse_version)
        .is_some_and(|version| version_at_least(&version, MINIMUM_NVIM))
}

/// 安装与启动选择｜优先使用合格系统 Neovim，否则选择受管版本；调用方获得同一个可执行路径模型。
pub fn preferred_nvim(paths: &AppPaths) -> PathBuf {
    if system_nvim_is_usable() {
        find_executable("nvim").expect("usable Neovim disappeared from PATH")
    } else {
        managed_nvim(paths)
    }
}

/// 依赖是否阻断安装使用显式分类，避免布尔参数隐藏行为差异。
#[derive(Clone, Copy)]
enum Requirement {
    Required,
    Optional,
}

/// 版本检查统一处理缺失、解析失败与最低版本比较。
fn version_meets(version: &Option<String>, minimum: (u64, u64, u64)) -> bool {
    version
        .as_deref()
        .and_then(parse_version)
        .is_some_and(|parsed| version_at_least(&parsed, minimum))
}

/// Neovim 状态包含系统版本可用性与受管安装补救说明。
fn nvim_status() -> DependencyStatus {
    let version = version_output("nvim", "--version");
    let valid = version_meets(&version, MINIMUM_NVIM);
    DependencyStatus {
        id: "nvim".into(),
        name: format!(
            "Neovim ≥ {}.{}.{}",
            MINIMUM_NVIM.0, MINIMUM_NVIM.1, MINIMUM_NVIM.2
        ),
        required: true,
        installed: valid,
        version,
        detail: if valid {
            "系统版本可直接使用".into()
        } else {
            "缺失或过旧时将下载官方 macOS arm64 构建".into()
        },
    }
}

/// Git 是获取 Starter 与插件的必需依赖。
fn git_status() -> DependencyStatus {
    let version = version_output("git", "--version");
    DependencyStatus {
        id: "git".into(),
        name: format!(
            "Git ≥ {}.{}.{}",
            MINIMUM_GIT.0, MINIMUM_GIT.1, MINIMUM_GIT.2
        ),
        required: true,
        installed: version_meets(&version, MINIMUM_GIT),
        version,
        detail: "用于获取 LazyVim Starter 和插件".into(),
    }
}

/// 普通命令行工具共用同一探测与状态构造路径。
fn tool_status(
    id: &str,
    name: &str,
    detail: &str,
    requirement: Requirement,
) -> DependencyStatus {
    let version = version_output(id, "--version");
    DependencyStatus {
        id: id.into(),
        name: name.into(),
        required: matches!(requirement, Requirement::Required),
        installed: version.is_some(),
        version,
        detail: detail.into(),
    }
}

/// 字体只影响图标，不阻断命令与编辑链路。
fn font_status() -> DependencyStatus {
    DependencyStatus {
        id: "nerd-font".into(),
        name: "Nerd Font v3+".into(),
        required: false,
        installed: has_nerd_font(),
        version: None,
        detail: "影响图标显示；不影响命令和编辑".into(),
    }
}

/// 依赖检查读模型｜逐项组合实际探测结果，必需项与增强项分别展示。
pub fn check() -> Vec<DependencyStatus> {
    let mut dependencies = vec![nvim_status(), git_status()];
    for (id, name, detail) in [
        ("curl", "curl", "用于受控下载与校验"),
        (
            "clang",
            "C compiler",
            "Treesitter 编译需要 Xcode Command Line Tools",
        ),
        ("tree-sitter", "tree-sitter-cli", "nvim-treesitter 需要"),
    ] {
        dependencies.push(tool_status(id, name, detail, Requirement::Required));
    }
    for (id, name, detail) in [
        ("rg", "ripgrep", "增强全局文本搜索"),
        ("fd", "fd", "增强文件查找"),
        ("fzf", "fzf", "可选的模糊查找后端；Snacks Picker 不依赖它"),
        ("lazygit", "lazygit", "可选的 Git 界面"),
    ] {
        dependencies.push(tool_status(id, name, detail, Requirement::Optional));
    }
    dependencies.push(font_status());
    dependencies
}

/// 可选依赖检查｜扫描用户与系统字体目录中的 Nerd Font 命名；只影响图标展示，不阻断安装。
fn has_nerd_font() -> bool {
    let Some(home) = std::env::var_os("HOME") else {
        return false;
    };
    for directory in [
        PathBuf::from(&home).join("Library/Fonts"),
        PathBuf::from("/Library/Fonts"),
    ] {
        if let Ok(entries) = fs::read_dir(directory) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.contains("nerd") {
                    return true;
                }
            }
        }
    }
    false
}

/// 安装依赖闸门｜串行补齐受管 Neovim 与 tree-sitter，再复查全部必需项；任何缺失都会阻止 Profile 安装继续。
pub fn ensure_required_tools(
    paths: &AppPaths,
) -> Result<Vec<DependencyStatus>, String> {
    fs::create_dir_all(&paths.tools).map_err(|error| error.to_string())?;
    let _lock = ToolLock::acquire(&paths.tools.join("lazykeys-tools.lock"))?;
    if !system_nvim_is_usable() && !is_executable(&managed_nvim(paths)) {
        install_neovim(paths)?;
    }
    if find_executable("tree-sitter").is_none()
        && !is_executable(&managed_tree_sitter(paths))
    {
        install_tree_sitter(paths)?;
    }
    let statuses = check_with_paths(paths);
    let missing: Vec<_> = statuses
        .iter()
        .filter(|dependency| dependency.required && !dependency.installed)
        .map(|dependency| dependency.name.clone())
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "仍有必需依赖未满足：{}。请按依赖页的提示完成系统授权或安装。",
            missing.join("、")
        ));
    }
    Ok(statuses)
}

/// 依赖状态合并｜在系统检查结果上覆盖可用受管工具；同一 `DependencyStatus` 同时表达系统与 LazyKeys 后备能力。
pub fn check_with_paths(paths: &AppPaths) -> Vec<DependencyStatus> {
    let mut statuses = check();
    for dependency in &mut statuses {
        let managed = match dependency.id.as_str() {
            "nvim" => Some(managed_nvim(paths)),
            "tree-sitter" => Some(managed_tree_sitter(paths)),
            _ => None,
        };
        if let Some(executable) = managed.filter(|path| is_executable(path)) {
            dependency.installed = true;
            dependency.version =
                version_output(executable.to_str().unwrap(), "--version")
                    .or(dependency.version.take());
            dependency.detail = "使用 LazyKeys 受管工具".into();
        }
    }
    statuses
}

/// 工具安装锁模型｜进程内互斥与文件锁组合；同一受管工具目录只执行一条安装链路。
struct ToolLock {
    /// 跨进程锁载体：文件描述符的生命周期就是文件锁的生命周期。
    file: fs::File,
    /// 进程内守卫：先串行化线程，再进入跨进程文件锁。
    _process_guard: MutexGuard<'static, ()>,
}

impl ToolLock {
    /// 工具安装入口｜依次取得线程互斥和跨进程文件锁；防止并发下载或发布同一工具。
    fn acquire(path: &Path) -> Result<Self, String> {
        use std::os::unix::io::AsRawFd;

        unsafe extern "C" {
            fn flock(fd: i32, operation: i32) -> i32;
        }
        const LOCK_EX: i32 = 2;
        static PROCESS_LOCK: LazyLock<Mutex<()>> =
            LazyLock::new(|| Mutex::new(()));
        let process_guard = PROCESS_LOCK
            .lock()
            .map_err(|_| "工具安装进程锁已失效".to_string())?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| format!("无法创建工具安装锁：{error}"))?;
        if unsafe { flock(file.as_raw_fd(), LOCK_EX) } != 0 {
            return Err(format!(
                "无法取得跨进程工具安装锁：{}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self {
            file,
            _process_guard: process_guard,
        })
    }
}

impl Drop for ToolLock {
    /// 工具安装收尾｜守卫离开作用域即释放文件锁；后续安装链路才能继续。
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        unsafe extern "C" {
            fn flock(fd: i32, operation: i32) -> i32;
        }
        const LOCK_UN: i32 = 8;
        unsafe {
            flock(self.file.as_raw_fd(), LOCK_UN);
        }
    }
}

/// 官方发布模型｜工具下载的元数据入口；只保留安装链路需要的资源列表。
#[derive(Debug, Deserialize)]
struct Release {
    /// 同一发布版本中的候选资源；按目标文件名选择唯一下载项。
    assets: Vec<ReleaseAsset>,
}

/// 官方资源模型｜受管工具的最小下载契约；名称定位文件，摘要校验内容，地址提供下载入口。
#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    /// 发布资源文件名：与固定平台归档名精确匹配。
    name: String,
    /// 官方 SHA-256 摘要：下载后必须一致才允许发布。
    digest: String,
    /// 官方下载地址：只用于获取已定位的归档。
    browser_download_url: String,
}

/// 工具下载基础｜构造带连接与总时限的 curl 命令；元数据查询和归档下载共享失败即报错的行为。
fn curl_command(url: &str, max_time_seconds: u64) -> Command {
    let mut command = Command::new("curl");
    command.args([
        "--fail",
        "--location",
        "--silent",
        "--show-error",
        "--connect-timeout",
        "15",
        "--max-time",
        &max_time_seconds.to_string(),
        url,
    ]);
    command
}

/// 工具下载定位｜查询固定仓库与版本的发布元数据；只选择名称匹配的官方资源。
fn release_asset(
    repo: &str,
    tag: &str,
    name: &str,
) -> Result<ReleaseAsset, String> {
    let url =
        format!("https://api.github.com/repos/{repo}/releases/tags/{tag}");
    let output = curl_command(&url, 30)
        .output()
        .map_err(|error| format!("查询官方 release 失败：{error}"))?;
    if !output.status.success() {
        return Err(format!("查询 {repo} release 失败：{}", output.status));
    }
    let release: Release = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("解析官方 release 失败：{error}"))?;
    release
        .assets
        .into_iter()
        .find(|asset| asset.name == name)
        .ok_or_else(|| format!("{repo} 的 {tag} 中找不到 {name}"))
}

/// 工具下载校验｜先写入唯一临时文件，再核对官方 SHA-256 并重命名；未通过校验的内容不会成为安装输入。
fn download_verified(
    repo: &str,
    tag: &str,
    asset_name: &str,
    destination: &Path,
) -> Result<(), String> {
    let asset = release_asset(repo, tag, asset_name)?;
    let expected = asset
        .digest
        .strip_prefix("sha256:")
        .ok_or_else(|| format!("官方资源 {asset_name} 未提供 SHA-256"))?
        .to_lowercase();
    if expected.len() != 64
        || !expected
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(format!("官方资源 {asset_name} 的 SHA-256 格式无效"));
    }
    let temporary =
        destination.with_extension(format!("download-{}", Uuid::new_v4()));
    if let Some(parent) = temporary.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let result = (|| {
        let output: Output = curl_command(&asset.browser_download_url, 300)
            .arg("--output")
            .arg(&temporary)
            .output()
            .map_err(|error| format!("下载 {asset_name} 失败：{error}"))?;
        if !output.status.success() {
            return Err(format!("下载 {asset_name} 失败：{}", output.status));
        }
        let bytes = fs::read(&temporary).map_err(|error| error.to_string())?;
        let actual = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if actual != expected {
            return Err(format!(
                "{asset_name} 校验失败：期望 {expected}，实际 {actual}"
            ));
        }
        fs::rename(&temporary, destination).map_err(|error| error.to_string())
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// 受管工具安装｜在版本目录的暂存区下载、校验、解压 Neovim；完整可执行目录通过原子发布替换旧版本。
fn install_neovim(paths: &AppPaths) -> Result<(), String> {
    let root = nvim_root(paths);
    if is_executable(&managed_nvim(paths)) {
        return Ok(());
    }
    let staging = paths
        .tools
        .join(format!(".nvim-install-{}", Uuid::new_v4()));
    fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
    let archive = staging.join("nvim.tar.gz");
    let result = (|| {
        download_verified(
            "neovim/neovim",
            NVIM_RELEASE_TAG,
            NVIM_ARCHIVE,
            &archive,
        )?;
        let output = Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(&staging)
            .output()
            .map_err(|error| format!("解压 Neovim 失败：{error}"))?;
        if !output.status.success() {
            return Err(format!(
                "解压 Neovim 失败：{}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let executable = staging.join("nvim-macos-arm64/bin/nvim");
        if !is_executable(&executable) {
            return Err("官方 Neovim 归档中缺少可执行文件".into());
        }
        replace_directory(&staging, &root)
    })();
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// 受管工具安装｜下载并校验 tree-sitter，解压为可执行文件后原子发布；暂存目录始终在安装收尾清理。
fn install_tree_sitter(paths: &AppPaths) -> Result<(), String> {
    let root = tree_sitter_root(paths);
    if is_executable(&managed_tree_sitter(paths)) {
        return Ok(());
    }
    let staging = paths
        .tools
        .join(format!(".tree-sitter-install-{}", Uuid::new_v4()));
    fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
    let archive = staging.join("tree-sitter.gz");
    let result = (|| {
        download_verified(
            "tree-sitter/tree-sitter",
            TREE_SITTER_RELEASE_TAG,
            TREE_SITTER_ARCHIVE,
            &archive,
        )?;
        let output = Command::new("gzip")
            .arg("-dc")
            .arg(&archive)
            .output()
            .map_err(|error| format!("解压 tree-sitter 失败：{error}"))?;
        if !output.status.success() {
            return Err(format!(
                "解压 tree-sitter 失败：{}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let executable = staging.join("tree-sitter");
        fs::write(&executable, output.stdout)
            .map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&executable)
                .map_err(|error| error.to_string())?
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&executable, permissions)
                .map_err(|error| error.to_string())?;
        }
        replace_directory(&staging, &root)
    })();
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// 安装与启动环境｜把受管 Neovim、tree-sitter 和工具目录前置到 PATH；子进程优先解析受管依赖。
pub fn augmented_path(paths: &AppPaths) -> String {
    let current = std::env::var("PATH").unwrap_or_default();
    format!(
        "{}:{}/nvim-macos-arm64/bin:{}:{}",
        nvim_root(paths).display(),
        paths.tools.display(),
        tree_sitter_root(paths).display(),
        current
    )
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn parses_lazyvim_requirements() {
        assert!(version_at_least(&(0, 12, 6), MINIMUM_NVIM));
        assert!(!version_at_least(&(0, 10, 9), MINIMUM_NVIM));
        assert!(version_at_least(&(2, 54, 0), MINIMUM_GIT));
    }

    #[test]
    fn verifies_path_does_not_include_unmanaged_nvim_config() {
        let path =
            augmented_path(&AppPaths::new(PathBuf::from("/tmp/lazykeys-test")));
        assert!(!path.contains("/.config/nvim"));
        assert!(path.contains(NVIM_VERSION));
        assert!(path.contains(TREE_SITTER_VERSION));
    }

    #[test]
    fn curl_has_a_real_deadline_for_slow_servers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            std::thread::sleep(Duration::from_secs(2));
        });
        let started = Instant::now();
        let output = curl_command(&format!("http://{address}/slow"), 1)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(started.elapsed() < Duration::from_secs(4));
        server.join().unwrap();
    }

    #[test]
    fn tool_lock_excludes_threads_and_uses_a_stable_process_lock_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("tools.lock");
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let mut threads = Vec::new();
        for _ in 0..4 {
            let path = path.clone();
            let active = active.clone();
            let maximum = maximum.clone();
            threads.push(std::thread::spawn(move || {
                let _lock = ToolLock::acquire(&path).unwrap();
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(current, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(30));
                active.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(maximum.load(Ordering::SeqCst), 1);
        assert!(path.is_file());
    }
}

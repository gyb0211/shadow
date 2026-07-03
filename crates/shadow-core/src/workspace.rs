//! Workspace -- agent 工作空间路径布局
//!
//! 集中所有子路径常量, 替换散落各 crate 的 `workspace_dir.join("xxx")`.
//! 设计目标:
//! - 路径常量单点维护 (新增子目录只改这里)
//! - 为 multi-user profile 留接口 (`Workspace::open(profile_root)`)
//! - 不依赖其他 shadow-* crate (微内核零内部依赖原则)
//!
//! 阶段 1: struct 落地, main.rs 用 `Workspace::open(config_dir())` 创建,
//!         各后端继续接受 `&Path` (从 `workspace.root()` 派生).
//! 阶段 2: 各 crate 改签名直接接受 `&Workspace`, 调用 `.sessions_dir()` 等.

use std::path::{Path, PathBuf};

/// Agent 工作空间 -- 一个 profile 的所有数据目录
///
/// 一个 `Workspace` 实例对应文件系统上一个目录树, 包含 sessions / memory /
/// skills / logs / workspace 等子目录. 默认布局参考 Hermes / ZeroClaw.
///
/// # 示例
/// ```
/// use shadow_core::Workspace;
/// let ws = Workspace::open("/tmp/.shadow");
/// assert_eq!(ws.sessions_dir(), std::path::PathBuf::from("/tmp/.shadow/sessions"));
/// assert_eq!(ws.memory_db_path(), std::path::PathBuf::from("/tmp/.shadow/memory/brain.db"));
/// ```
#[derive(Debug, Clone)]
pub struct Workspace {
    /// workspace 根目录 (如 `~/.shadow/` 或 `~/.shadow/profiles/<name>/`)
    root: PathBuf,
}

impl Workspace {
    /// 打开一个 workspace (不校验目录存在, 不创建子目录)
    ///
    /// 调用方负责后续调用 [`Self::ensure_layout`] 创建子目录.
    #[must_use]
    pub fn open(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// workspace 根目录
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    // ── 子路径访问 (单点维护所有路径常量) ──

    /// `sessions/` -- 会话 JSONL 文件目录
    #[must_use]
    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }

    /// `memory/` -- 记忆数据目录
    #[must_use]
    pub fn memory_dir(&self) -> PathBuf {
        self.root.join("memory")
    }

    /// `memory/brain.db` -- SQLite 记忆库路径
    #[must_use]
    pub fn memory_db_path(&self) -> PathBuf {
        self.memory_dir().join("brain.db")
    }

    /// `memory/MEMORY.md` -- Markdown 记忆文件路径
    #[must_use]
    pub fn markdown_memory_path(&self) -> PathBuf {
        self.memory_dir().join("MEMORY.md")
    }

    /// `skills/` -- 技能目录 (SKILL.md 加载位置)
    #[must_use]
    pub fn skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    /// `logs/` -- 日志目录
    #[must_use]
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// `logs/runtime-trace.jsonl` -- 运行时追踪日志
    #[must_use]
    pub fn logs_path(&self) -> PathBuf {
        self.logs_dir().join("runtime-trace.jsonl")
    }

    /// `workspace/` -- 用户工作树 (git 仓库工作目录)
    ///
    /// 注意: 这是 profile 内的"用户工作区", 与 `root()` 不同.
    /// `root()` 是 Shadow 数据目录, `workspace_root()` 是用户项目工作树.
    #[must_use]
    pub fn workspace_root(&self) -> PathBuf {
        self.root.join("workspace")
    }

    /// `cron.db` -- Cron 调度器 SQLite 路径
    #[must_use]
    pub fn cron_db_path(&self) -> PathBuf {
        self.root.join("cron.db")
    }

    /// `config.toml` -- Shadow 配置文件
    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    /// `SOUL.md` -- agent 人格文件 (Hermes 风格, 未来用)
    #[must_use]
    pub fn soul_path(&self) -> PathBuf {
        self.root.join("SOUL.md")
    }

    /// 创建所有运行时子目录 (幂等)
    ///
    /// 创建: `sessions/`, `memory/`, `skills/`, `logs/`, `workspace/`.
    /// 已存在的目录跳过, 不报错.
    ///
    /// # 错误
    /// 返回 `io::Error` 如果创建失败 (权限不足 / 磁盘满等).
    pub fn ensure_layout(&self) -> std::io::Result<()> {
        for dir in [
            self.sessions_dir(),
            self.memory_dir(),
            self.skills_dir(),
            self.logs_dir(),
            self.workspace_root(),
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_accepts_str_and_pathbuf() {
        let _ws1 = Workspace::open("/tmp/foo");
        let _ws2 = Workspace::open(PathBuf::from("/tmp/foo"));
        let _ws3 = Workspace::open(Path::new("/tmp/foo"));
    }

    #[test]
    fn root_returns_input_path() {
        let ws = Workspace::open("/tmp/.shadow");
        assert_eq!(ws.root(), Path::new("/tmp/.shadow"));
    }

    #[test]
    fn paths_match_existing_layout() {
        let ws = Workspace::open("/root");

        assert_eq!(ws.sessions_dir(), PathBuf::from("/root/sessions"));
        assert_eq!(ws.memory_dir(), PathBuf::from("/root/memory"));
        assert_eq!(ws.memory_db_path(), PathBuf::from("/root/memory/brain.db"));
        assert_eq!(ws.markdown_memory_path(), PathBuf::from("/root/memory/MEMORY.md"));
        assert_eq!(ws.skills_dir(), PathBuf::from("/root/skills"));
        assert_eq!(ws.logs_dir(), PathBuf::from("/root/logs"));
        assert_eq!(ws.logs_path(), PathBuf::from("/root/logs/runtime-trace.jsonl"));
        assert_eq!(ws.workspace_root(), PathBuf::from("/root/workspace"));
        assert_eq!(ws.cron_db_path(), PathBuf::from("/root/cron.db"));
        assert_eq!(ws.config_path(), PathBuf::from("/root/config.toml"));
        assert_eq!(ws.soul_path(), PathBuf::from("/root/SOUL.md"));
    }

    #[test]
    fn ensure_layout_creates_all_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::open(tmp.path());

        ws.ensure_layout().unwrap();

        assert!(ws.sessions_dir().exists());
        assert!(ws.memory_dir().exists());
        assert!(ws.skills_dir().exists());
        assert!(ws.logs_dir().exists());
        assert!(ws.workspace_root().exists());
    }

    #[test]
    fn ensure_layout_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::open(tmp.path());

        ws.ensure_layout().unwrap();
        // 第二次不应报错
        ws.ensure_layout().unwrap();
    }

    #[test]
    fn ensure_layout_does_not_touch_files() {
        // ensure_layout 只创建目录, 不创建文件 (如 config.toml / SOUL.md)
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::open(tmp.path());

        ws.ensure_layout().unwrap();

        assert!(!ws.config_path().exists());
        assert!(!ws.soul_path().exists());
        assert!(!ws.memory_db_path().exists());
        assert!(!ws.logs_path().exists());
    }

    #[test]
    fn clone_then_root_match() {
        let ws = Workspace::open("/foo");
        let cloned = ws.clone();
        assert_eq!(ws.root(), cloned.root());
    }
}

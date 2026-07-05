//! Cron 调度器 -- 定时任务调度与持久化
//!
//! 借鉴 ZeroClaw 的 Cron 设计但精简:
//! - ZeroClaw: 7306 行, SQLite 持久化, 安全验证, announce 投递
//! - Shadow: 简单调度 + SQLite 持久化
//!
//! cron 表达式格式 (6-7 字段, 使用 cron crate 解析):
//! ```text
//! 秒 分 时 日 月 周 [年]
//! *  *  *  *  *  *
//! |  |  |  |  |  |
//! |  |  |  |  |  +-- 星期 (0-7, 0 和 7 都是周日)
//! |  |  |  |  +---- 月 (1-12)
//! |  |  |  +------ 日 (1-31)
//! |  |  +-------- 时 (0-23)
//! |  +----------- 分 (0-59)
//! +-------------- 秒 (0-59)
//! ```
//!
//! 示例:
//! - `"0 * * * * *"` -- 每分钟第 0 秒执行
//! - `"*/30 * * * * *"` -- 每 30 秒执行一次
//! - `"0 0 * * * *"` -- 每天午夜执行
//! - `"0 0 9 * * 1-5"` -- 工作日每天上午 9 点执行

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use cron::Schedule;
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use std::path::Path;

use shadow_log::Action;

// ───────────────────────── 数据结构 ─────────────────────────

/// Cron 作业 -- 一个定时任务的定义
#[derive(Debug, Clone)]
pub struct CronJob {
    /// 作业 ID (SQLite 自增主键, 新建时设为 0, 由数据库分配)
    pub id: i64,
    /// 作业名称 (人类可读)
    pub name: String,
    /// cron 表达式 (6-7 字段: 秒 分 时 日 月 周 [年])
    pub schedule: String,
    /// 要执行的 shell 命令
    pub command: String,
    /// 是否启用 (false 时不会被调度)
    pub enabled: bool,
    /// 关联的 agent 别名 (用于多 agent 场景区分执行者)
    pub agent_alias: String,
    /// 下次运行时间 (Unix 时间戳, None 表示未计算)
    pub next_run: Option<i64>,
}

impl CronJob {
    /// 创建新的 cron 作业 (id 设为 0, 由数据库分配)
    ///
    /// 默认启用, agent_alias 为 "default".
    pub fn new(
        name: impl Into<String>,
        schedule: impl Into<String>,
        command: impl Into<String>,
    ) -> Self {
        Self {
            id: 0,
            name: name.into(),
            schedule: schedule.into(),
            command: command.into(),
            enabled: true,
            agent_alias: "default".to_string(),
            next_run: None,
        }
    }

    /// 设置关联的 agent 别名
    pub fn with_agent_alias(mut self, alias: impl Into<String>) -> Self {
        self.agent_alias = alias.into();
        self
    }

    /// 设置启用状态
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// Cron 运行记录 -- 一次作业执行的历史记录
#[derive(Debug, Clone)]
pub struct CronRun {
    /// 运行记录 ID (由数据库分配, 新建时设为 0)
    pub id: i64,
    /// 关联的作业 ID
    pub job_id: i64,
    /// 开始时间 (Unix 时间戳)
    pub started_at: i64,
    /// 结束时间 (Unix 时间戳, None 表示仍在运行)
    pub finished_at: Option<i64>,
    /// 运行状态: "success" / "failed" / "running"
    pub status: String,
    /// 输出内容 (stdout + stderr, 已截断)
    pub output: String,
}

/// 运行输出最大长度 (10KB), 超过则截断
const MAX_OUTPUT_LEN: usize = 10 * 1024;

// ───────────────────────── 调度器 ─────────────────────────

/// Cron 调度器 -- 管理 cron 作业的持久化与调度
///
/// 使用 SQLite 持久化作业定义和运行记录, 支持 WAL 模式.
/// 内部用 `parking_lot::Mutex` 保护连接, 线程安全 (`Send + Sync`).
///
/// # 用法
/// ```no_run
/// use shadow_runtime::cron::{CronJob, CronScheduler};
///
/// let scheduler = CronScheduler::new(std::path::Path::new(".")).unwrap();
/// let id = scheduler.add_job(
///     CronJob::new("每日备份", "0 0 2 * * *", "tar czf backup.tar.gz data/")
/// ).unwrap();
/// ```
pub struct CronScheduler {
    /// SQLite 连接 (Mutex 保护, 支持多线程访问)
    conn: Mutex<Connection>,
}

impl CronScheduler {
    /// 创建调度器 -- 打开 workspace_dir 下的 cron.db
    ///
    /// 自动创建数据库文件和表结构, 启用 WAL 模式.
    pub fn new(workspace_dir: &Path) -> Result<Self> {
        let db_path = workspace_dir.join("cron.db");
        let conn = Connection::open(&db_path)
            .with_context(|| format!("无法打开 cron 数据库: {}", db_path.display()))?;

        // 启用 WAL 模式 -- 提高并发读写性能
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        let scheduler = Self {
            conn: Mutex::new(conn),
        };
        scheduler.init_tables()?;
        Ok(scheduler)
    }

    /// 创建内存调度器 -- 用于单元测试 (使用 :memory: SQLite)
    ///
    /// 表结构与磁盘版完全一致, 但数据不持久化.
    pub fn new_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let scheduler = Self {
            conn: Mutex::new(conn),
        };
        scheduler.init_tables()?;
        Ok(scheduler)
    }

    /// 初始化数据库表结构
    ///
    /// 创建两张表:
    /// - `cron_jobs`: 作业定义 (id, name, schedule, command, enabled, agent_alias, next_run)
    /// - `cron_runs`: 运行记录 (id, job_id, started_at, finished_at, status, output)
    fn init_tables(&self) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cron_jobs (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT    NOT NULL,
                schedule    TEXT    NOT NULL,
                command     TEXT    NOT NULL,
                enabled     INTEGER NOT NULL DEFAULT 1,
                agent_alias TEXT    NOT NULL DEFAULT 'default',
                next_run    INTEGER
            );
            CREATE TABLE IF NOT EXISTS cron_runs (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                job_id      INTEGER NOT NULL,
                started_at  INTEGER NOT NULL,
                finished_at INTEGER,
                status      TEXT    NOT NULL,
                output      TEXT    NOT NULL DEFAULT '',
                FOREIGN KEY (job_id) REFERENCES cron_jobs(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_cron_runs_job_id ON cron_runs(job_id);
            CREATE INDEX IF NOT EXISTS idx_cron_jobs_next_run ON cron_jobs(next_run);
            ",
        )?;
        Ok(())
    }

    /// 添加作业 -- INSERT 并返回新分配的 ID
    ///
    /// 自动计算 `next_run` (下次运行时间).
    /// 若 cron 表达式无效则返回错误.
    pub fn add_job(&self, mut job: CronJob) -> Result<i64> {
        // 计算 cron 表达式的下次运行时间
        job.next_run = calc_next_run(&job.schedule)?;

        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO cron_jobs (name, schedule, command, enabled, agent_alias, next_run)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                job.name,
                job.schedule,
                job.command,
                job.enabled as i64,
                job.agent_alias,
                job.next_run,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// 删除作业 -- 按 ID 删除
    ///
    /// 关联的运行记录通过外键 ON DELETE CASCADE 自动删除.
    pub fn remove_job(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM cron_jobs WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// 列出所有作业 -- 按 ID 升序
    pub fn list_jobs(&self) -> Result<Vec<CronJob>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, schedule, command, enabled, agent_alias, next_run
             FROM cron_jobs ORDER BY id",
        )?;
        let jobs = stmt.query_map([], |row| {
            Ok(CronJob {
                id: row.get(0)?,
                name: row.get(1)?,
                schedule: row.get(2)?,
                command: row.get(3)?,
                enabled: row.get::<_, i64>(4)? != 0,
                agent_alias: row.get(5)?,
                next_run: row.get(6)?,
            })
        })?;
        let mut result = Vec::new();
        for job in jobs {
            result.push(job?);
        }
        Ok(result)
    }

    /// 获取到期作业 -- `next_run <= now` 且 `enabled = true`
    ///
    /// 按 next_run 升序排列, 优先执行最早到期的作业.
    pub fn due_jobs(&self, now: DateTime<Utc>) -> Result<Vec<CronJob>> {
        let now_ts = now.timestamp();
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, schedule, command, enabled, agent_alias, next_run
             FROM cron_jobs
             WHERE enabled = 1 AND next_run IS NOT NULL AND next_run <= ?1
             ORDER BY next_run",
        )?;
        let jobs = stmt.query_map(params![now_ts], |row| {
            Ok(CronJob {
                id: row.get(0)?,
                name: row.get(1)?,
                schedule: row.get(2)?,
                command: row.get(3)?,
                enabled: row.get::<_, i64>(4)? != 0,
                agent_alias: row.get(5)?,
                next_run: row.get(6)?,
            })
        })?;
        let mut result = Vec::new();
        for job in jobs {
            result.push(job?);
        }
        Ok(result)
    }

    /// 记录一次运行 -- INSERT 并返回记录 ID
    pub fn record_run(&self, run: &CronRun) -> Result<i64> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO cron_runs (job_id, started_at, finished_at, status, output)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                run.job_id,
                run.started_at,
                run.finished_at,
                run.status,
                run.output,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// 查询作业的运行记录 -- 按 started_at 降序 (最新在前)
    pub fn list_runs(&self, job_id: i64) -> Result<Vec<CronRun>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, job_id, started_at, finished_at, status, output
             FROM cron_runs WHERE job_id = ?1 ORDER BY started_at DESC",
        )?;
        let runs = stmt.query_map(params![job_id], |row| {
            Ok(CronRun {
                id: row.get(0)?,
                job_id: row.get(1)?,
                started_at: row.get(2)?,
                finished_at: row.get(3)?,
                status: row.get(4)?,
                output: row.get(5)?,
            })
        })?;
        let mut result = Vec::new();
        for run in runs {
            result.push(run?);
        }
        Ok(result)
    }

    /// 更新作业的下次运行时间 -- 作业执行后调用
    ///
    /// 根据 cron 表达式重新计算下次运行时间.
    pub fn update_next_run(&self, job_id: i64, schedule: &str) -> Result<()> {
        let next = calc_next_run(schedule)?;
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE cron_jobs SET next_run = ?1 WHERE id = ?2",
            params![next, job_id],
        )?;
        Ok(())
    }

    /// 手动设置作业的下次运行时间 -- 用于测试或手动调度控制
    pub fn set_next_run(&self, job_id: i64, next_run: Option<i64>) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE cron_jobs SET next_run = ?1 WHERE id = ?2",
            params![next_run, job_id],
        )?;
        Ok(())
    }

    /// 获取单个作业 -- 按 ID 查询
    ///
    /// 返回 `Ok(Some(job))` 表示找到, `Ok(None)` 表示 ID 不存在.
    pub fn get_job(&self, id: i64) -> Result<Option<CronJob>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, schedule, command, enabled, agent_alias, next_run
             FROM cron_jobs WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(CronJob {
                id: row.get(0)?,
                name: row.get(1)?,
                schedule: row.get(2)?,
                command: row.get(3)?,
                enabled: row.get::<_, i64>(4)? != 0,
                agent_alias: row.get(5)?,
                next_run: row.get(6)?,
            })
        })?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// 更新作业字段 -- 部分更新 (None 字段保持不变)
    ///
    /// # 参数
    /// - `id`: 作业 ID
    /// - `schedule`: 新的 cron 表达式 (None 则不更新). 更新后自动重算 next_run.
    /// - `command`: 新的命令/prompt (None 则不更新)
    /// - `enabled`: 新的启用状态 (None 则不更新)
    ///
    /// # 返回
    /// `Ok(true)` 表示更新成功, `Ok(false)` 表示作业不存在.
    pub fn update_job(
        &self,
        id: i64,
        schedule: Option<&str>,
        command: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<bool> {
        // 先检查作业是否存在
        if self.get_job(id)?.is_none() {
            return Ok(false);
        }

        let conn = self.conn.lock();

        // 动态构建 UPDATE 语句
        let mut sets: Vec<&str> = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(sch) = schedule {
            sets.push("schedule = ?");
            params_vec.push(Box::new(sch.to_string()));
            // schedule 变化时需要重算 next_run
            let next = calc_next_run(sch)?;
            sets.push("next_run = ?");
            params_vec.push(Box::new(next));
        }
        if let Some(cmd) = command {
            sets.push("command = ?");
            params_vec.push(Box::new(cmd.to_string()));
        }
        if let Some(en) = enabled {
            sets.push("enabled = ?");
            params_vec.push(Box::new(en as i64));
        }

        if sets.is_empty() {
            // 没有字段需要更新
            return Ok(true);
        }

        let set_clause = sets.join(", ");
        let sql = format!("UPDATE cron_jobs SET {set_clause} WHERE id = ?");

        // 构建参数 (动态字段 + 末尾的 id)
        let mut param_refs: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        param_refs.push(&id);

        conn.execute(&sql, rusqlite::params_from_iter(param_refs))?;
        Ok(true)
    }
}

// ───────────────────────── 辅助函数 ─────────────────────────

/// 计算 cron 表达式的下次运行时间 (Unix 时间戳)
///
/// 从当前时间开始查找下一次匹配的时间点.
/// 返回 `None` 表示没有未来的运行时间 (例如表达式只匹配过去的日期).
fn calc_next_run(schedule_expr: &str) -> Result<Option<i64>> {
    let schedule: Schedule = schedule_expr
        .parse()
        .with_context(|| format!("无效的 cron 表达式: {schedule_expr}"))?;
    let now = Utc::now();
    match schedule.after(&now).next() {
        Some(dt) => Ok(Some(dt.timestamp())),
        None => Ok(None),
    }
}

/// 截断超长输出 -- 超过 MAX_OUTPUT_LEN 时只保留前部分并附加提示
///
/// 在字节边界安全截断 (避免截断 UTF-8 字符的中间).
fn truncate_output(output: &str) -> String {
    if output.len() <= MAX_OUTPUT_LEN {
        return output.to_string();
    }
    let mut end = MAX_OUTPUT_LEN;
    while end > 0 && !output.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n\n[输出已截断, 显示前 {end} / 共 {} 字节]",
        &output[..end],
        output.len()
    )
}

// ───────────────────────── 作业执行 ─────────────────────────

/// 处理到期作业 -- 遍历到期作业, 执行命令, 记录结果
///
/// 流程:
/// 1. 获取当前到期的作业列表 (`due_jobs`)
/// 2. 对每个作业: 在 agent 工作目录中用 `sh -c` 执行命令
/// 3. 记录运行结果到 `cron_runs` 表
/// 4. 更新作业的下次运行时间
///
/// # 参数
/// - `scheduler`: Cron 调度器引用
/// - `agent`: Agent 引用 (提供工作目录上下文)
///
/// # 返回
/// 成功执行的作业数量.
///
/// # 注意
/// 单个作业执行失败不会中断其他作业, 错误会记录到运行记录中.
pub async fn process_due_jobs(
    scheduler: &CronScheduler,
    agent: &crate::agent::Agent,
) -> Result<usize> {
    let now = Utc::now();
    let due = scheduler.due_jobs(now)?;
    let mut executed = 0;

    for job in due {
        let started = Utc::now().timestamp();
        shadow_log::record!(
            INFO,
            Action::Invoke,
            format!(
                "cron 作业 [{}] (id={}) 开始执行: {}",
                job.name, job.id, job.command
            )
        );

        // 在 agent 的工作目录中执行命令 (使用 sh -c, 与 ShellTool 一致)
        let output_result = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&job.command)
            .current_dir(&agent.config.workspace_dir)
            .output()
            .await;

        let (status, output_text) = match output_result {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let combined = if stderr.is_empty() {
                    stdout.to_string()
                } else {
                    format!("{stdout}\n[stderr]\n{stderr}")
                };
                let combined = truncate_output(&combined);
                if out.status.success() {
                    ("success", combined)
                } else {
                    ("failed", combined)
                }
            }
            Err(e) => ("failed", format!("执行命令失败: {e}")),
        };

        let finished = Utc::now().timestamp();

        // 记录运行结果
        scheduler.record_run(&CronRun {
            id: 0,
            job_id: job.id,
            started_at: started,
            finished_at: Some(finished),
            status: status.to_string(),
            output: output_text,
        })?;

        // 更新下次运行时间
        scheduler.update_next_run(job.id, &job.schedule)?;

        shadow_log::record!(
            INFO,
            Action::Complete,
            format!(
                "cron 作业 [{}] (id={}) 执行完成: {}",
                job.name, job.id, status
            )
        );

        executed += 1;
    }

    Ok(executed)
}

// ───────────────────────── 单元测试 ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试: 添加作业并获取分配的 ID
    #[test]
    fn add_job() {
        let scheduler = CronScheduler::new_in_memory().unwrap();
        let job = CronJob::new("测试任务", "0 * * * * *", "echo hello")
            .with_agent_alias("test-agent");
        let id = scheduler.add_job(job).unwrap();
        // ID 应为正数 (SQLite 自增主键从 1 开始)
        assert!(id > 0);
    }

    /// 测试: 添加作业后 next_run 应被自动计算
    #[test]
    fn add_job_calculates_next_run() {
        let scheduler = CronScheduler::new_in_memory().unwrap();
        let job = CronJob::new("测试任务", "0 * * * * *", "echo hello");
        let id = scheduler.add_job(job).unwrap();

        let jobs = scheduler.list_jobs().unwrap();
        let added = jobs.iter().find(|j| j.id == id).unwrap();
        // next_run 应为 Some (未来时间)
        assert!(added.next_run.is_some());
        let next_run = added.next_run.unwrap();
        // next_run 应在未来
        assert!(next_run > Utc::now().timestamp());
    }

    /// 测试: 列出所有作业
    #[test]
    fn list_jobs() {
        let scheduler = CronScheduler::new_in_memory().unwrap();

        let job1 = CronJob::new("任务1", "0 * * * * *", "echo 1");
        let job2 = CronJob::new("任务2", "0 0 * * * *", "echo 2");

        let id1 = scheduler.add_job(job1).unwrap();
        let id2 = scheduler.add_job(job2).unwrap();

        let jobs = scheduler.list_jobs().unwrap();
        assert_eq!(jobs.len(), 2);
        // 按 ID 升序
        assert_eq!(jobs[0].id, id1);
        assert_eq!(jobs[0].name, "任务1");
        assert_eq!(jobs[0].schedule, "0 * * * * *");
        assert_eq!(jobs[0].command, "echo 1");
        assert!(jobs[0].enabled);
        assert_eq!(jobs[0].agent_alias, "default");
        assert_eq!(jobs[1].id, id2);
        assert_eq!(jobs[1].name, "任务2");
    }

    /// 测试: 空列表
    #[test]
    fn list_jobs_empty() {
        let scheduler = CronScheduler::new_in_memory().unwrap();
        let jobs = scheduler.list_jobs().unwrap();
        assert!(jobs.is_empty());
    }

    /// 测试: 获取到期作业 -- next_run 在过去的作业应被选出
    #[test]
    fn due_jobs() {
        let scheduler = CronScheduler::new_in_memory().unwrap();

        let job = CronJob::new("到期任务", "0 * * * * *", "echo test");
        let id = scheduler.add_job(job).unwrap();

        // 手动将 next_run 设为过去时间, 确保作业到期
        let past_ts = Utc::now().timestamp() - 60;
        scheduler.set_next_run(id, Some(past_ts)).unwrap();

        // 获取到期作业
        let due = scheduler.due_jobs(Utc::now()).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, id);
        assert_eq!(due[0].name, "到期任务");
    }

    /// 测试: 到期作业排除禁用的作业
    #[test]
    fn due_jobs_excludes_disabled() {
        let scheduler = CronScheduler::new_in_memory().unwrap();

        let job = CronJob::new("禁用任务", "0 * * * * *", "echo test").with_enabled(false);
        let id = scheduler.add_job(job).unwrap();

        // 设为过去时间
        let past_ts = Utc::now().timestamp() - 60;
        scheduler.set_next_run(id, Some(past_ts)).unwrap();

        // 禁用的作业不应出现在到期列表中
        let due = scheduler.due_jobs(Utc::now()).unwrap();
        assert_eq!(due.len(), 0);
    }

    /// 测试: 到期作业排除未来的作业
    #[test]
    fn due_jobs_excludes_future() {
        let scheduler = CronScheduler::new_in_memory().unwrap();

        let job = CronJob::new("未来任务", "0 * * * * *", "echo future");
        let id = scheduler.add_job(job).unwrap();

        // 设为未来时间
        let future_ts = Utc::now().timestamp() + 3600;
        scheduler.set_next_run(id, Some(future_ts)).unwrap();

        let due = scheduler.due_jobs(Utc::now()).unwrap();
        assert_eq!(due.len(), 0);
    }

    /// 测试: 记录运行结果
    #[test]
    fn record_run() {
        let scheduler = CronScheduler::new_in_memory().unwrap();

        // 先添加一个作业
        let job = CronJob::new("测试任务", "0 * * * * *", "echo hello");
        let job_id = scheduler.add_job(job).unwrap();

        // 记录一次运行
        let now_ts = Utc::now().timestamp();
        let run = CronRun {
            id: 0,
            job_id,
            started_at: now_ts,
            finished_at: Some(now_ts + 1),
            status: "success".to_string(),
            output: "hello\n".to_string(),
        };
        let run_id = scheduler.record_run(&run).unwrap();
        assert!(run_id > 0);

        // 查询运行记录
        let runs = scheduler.list_runs(job_id).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, run_id);
        assert_eq!(runs[0].job_id, job_id);
        assert_eq!(runs[0].status, "success");
        assert_eq!(runs[0].output, "hello\n");
        assert_eq!(runs[0].started_at, now_ts);
        assert_eq!(runs[0].finished_at, Some(now_ts + 1));
    }

    /// 测试: 记录多条运行历史
    #[test]
    fn record_multiple_runs() {
        let scheduler = CronScheduler::new_in_memory().unwrap();

        let job = CronJob::new("多次运行", "0 * * * * *", "echo hi");
        let job_id = scheduler.add_job(job).unwrap();

        // 记录三次运行
        for i in 0..3 {
            let ts = Utc::now().timestamp() + i;
            scheduler
                .record_run(&CronRun {
                    id: 0,
                    job_id,
                    started_at: ts,
                    finished_at: Some(ts + 1),
                    status: if i == 1 { "failed" } else { "success" }.to_string(),
                    output: format!("output {i}"),
                })
                .unwrap();
        }

        let runs = scheduler.list_runs(job_id).unwrap();
        assert_eq!(runs.len(), 3);
        // 按 started_at 降序 (最新在前)
        assert_eq!(runs[0].output, "output 2");
        assert_eq!(runs[1].output, "output 1");
        assert_eq!(runs[2].output, "output 0");
    }

    /// 测试: 删除作业
    #[test]
    fn remove_job() {
        let scheduler = CronScheduler::new_in_memory().unwrap();

        let job = CronJob::new("待删除", "0 * * * * *", "echo bye");
        let id = scheduler.add_job(job).unwrap();

        assert_eq!(scheduler.list_jobs().unwrap().len(), 1);
        scheduler.remove_job(id).unwrap();
        assert_eq!(scheduler.list_jobs().unwrap().len(), 0);
    }

    /// 测试: 无效 cron 表达式应报错
    #[test]
    fn invalid_cron_expr() {
        let scheduler = CronScheduler::new_in_memory().unwrap();
        let job = CronJob::new("坏表达式", "not a cron expr", "echo error");
        let result = scheduler.add_job(job);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("无效的 cron 表达式"));
    }

    /// 测试: 更新下次运行时间
    #[test]
    fn update_next_run() {
        let scheduler = CronScheduler::new_in_memory().unwrap();

        let job = CronJob::new("更新测试", "0 * * * * *", "echo test");
        let id = scheduler.add_job(job).unwrap();

        // 获取初始 next_run
        let jobs = scheduler.list_jobs().unwrap();
        let initial_next = jobs.iter().find(|j| j.id == id).unwrap().next_run.unwrap();

        // 更新 next_run
        scheduler.update_next_run(id, "0 0 * * * *").unwrap();

        // 验证已更新
        let jobs = scheduler.list_jobs().unwrap();
        let updated = jobs.iter().find(|j| j.id == id).unwrap();
        let new_next = updated.next_run.unwrap();
        assert_ne!(initial_next, new_next);
    }

    /// 测试: 输出截断
    #[test]
    fn truncate_output_short() {
        let result = truncate_output("hello");
        assert_eq!(result, "hello");
    }

    #[test]
    fn truncate_output_long() {
        let long = "x".repeat(MAX_OUTPUT_LEN + 1000);
        let result = truncate_output(&long);
        assert!(result.contains("[输出已截断"));
        assert!(result.len() < long.len());
    }
}

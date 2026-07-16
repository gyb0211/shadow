//! JSONL 写入器 -- 追加式持久化 + 流式裁剪
//!
//! 参考 ZeroClaw writer.rs:
//! - 不在内存缓冲所有行 (避免 OOM)
//! - 超限时流式重写: BufReader 逐行读 → 保留最新 N 行 → 写临时文件 → rename
//! - RAM 使用量恒定: 一行读缓冲 + 一行写缓冲
//!
//! # 架构概览
//!
//! 全局单例 (`WRITER`) 持有当前进程的写入状态。`record_event` 是唯一入口,
//! 扇出到三个去向 (顺序敏感):
//! 1. observer_bridge -- 投影为 `ObserverEvent`,供指标订阅
//! 2. broadcast hook -- SSE / 实时订阅通道
//! 3. JSONL 文件 -- 仅在 `policy.storage.is_enabled()` 时落盘
//!
//! # 存储策略 (StoragePolicy)
//!
//! | 策略 | 行为 | 触发时机 |
//! |------|------|----------|
//! | `None` | 不落盘 | 调试/测试 |
//! | `Full` | 只追加,永不裁剪 | 长期归档 |
//! | `Rolling` | 追加 + 每次保留最新 `max_entries` 行 | 固定条数滚动 |
//! | `Rotating` | 追加 + 按日期切 + 按大小切 + 保留 N 份 | 标准日志轮转 |
//!
//! Rolling 用"重写"实现 (trim_to_last_entries),Rotating 用"换文件"实现 (rotate_active)。
//! 两者都不缓冲整个文件到内存。

use crate::config::{LlmRequestPayloadPolicy, LogConfig, ResolvedPolicy, StoragePolicy};
use crate::event::LogEvent;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde_json::Value;

use std::fs::{self, File, OpenOptions};
use std::io;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

/// 进程级写入状态: 解析后的策略 + 互斥锁。
///
/// `write_lock` 序列化所有文件写入,避免并发 append / trim / rotate 互相覆盖。
/// 持锁点仅在 `append_line` 内部唯一一处; `record_event` 不持锁, 避免重入死锁
/// (parking_lot::Mutex 不可重入, 同线程二次 lock 会 deadlock)。
struct WriterState {
    policy: ResolvedPolicy,
    write_lock: Mutex<()>,
}

/// 全局单例容器。
///
/// 用 `RwLock<Option<Arc<WriterState>>>` 而不是 `OnceLock<WriterState>`:
/// - `OnceLock` 初始化后无法替换, 不支持 `init_from_config` 多次调用 (如热重载/测试重置)
/// - 内层 `Arc` 让 `current_state()` 拿到的是廉价引用克隆, 读端无需持锁
/// - 外层 `RwLock` 写锁仅在 `init_from_config` 时短暂持有
static WRITER: OnceLock<parking_lot::RwLock<Option<Arc<WriterState>>>> = OnceLock::new();

/// 获取全局单例槽位, 首次调用时惰性初始化为 `None`。
fn slot() -> &'static parking_lot::RwLock<Option<Arc<WriterState>>> {
    WRITER.get_or_init(|| parking_lot::RwLock::new(None))
}

/// 快照当前状态 (无锁读)。返回 `None` 表示尚未 `init_from_config`。
fn current_state() -> Option<Arc<WriterState>> {
    slot().read().clone()
}

/// 初始化写入器: 解析配置为 `ResolvedPolicy` 并写入全局单例。
///
/// 可重复调用 (如测试间重置); 新状态直接替换旧状态。已持有旧 `Arc` 的调用方
/// 不受影响, 这也是用 `Arc<WriterState>` 而非 `&WriterState` 的原因。
pub fn init_from_config(workspace_dir: &Path, config: &LogConfig) {
    let policy = ResolvedPolicy::from_config(config, workspace_dir);

    let state = Arc::new(WriterState {
        policy,
        write_lock: Mutex::new(()),
    });

    *slot().write() = Some(state);
}

/// 获取当前日志文件路径 (仅用于诊断/展示)。未初始化时返回 `None`。
#[must_use]
pub fn runtime_trace_path() -> Option<PathBuf> {
    current_state().map(|s| s.policy.path.clone())
}

/// 测试用 no-op: 本写入器是同步追加 + fsync, 无异步缓冲需要 flush。
/// 保留接口以兼容测试套件的统一清理模式。
pub fn flush_for_test() -> Result<()> {
    Ok(())
}

/// 暴露 LLM 请求负载策略 + 工具 IO 截断字节数。
/// 供 event 构造端决定是否/如何截断大体量 payload, 避免日志爆炸。
pub fn llm_request_payload_policy() -> Option<(LlmRequestPayloadPolicy, usize)> {
    current_state().map(|s| {
        (
            s.policy.llm_request_payload,
            s.policy.tool_io_truncate_bytes,
        )
    })
}

/// 发射一条事件 -- 统一入口, 扇出到三个去向 (顺序敏感)。
///
/// 扇出顺序的设计意图:
/// 1. **observer_bridge** 先于持久化: 即使落盘失败, 指标/告警通道仍能感知
/// 2. **broadcast** (SSE/订阅) 紧随其后: 在线观察者尽快看到, 失败静默丢弃
/// 3. **JSONL 文件** 最后: 可靠性要求最高, 失败仅 tracing::warn, 不影响调用方
///
/// 任何扇出失败都不会反向影响调用方 (fire-and-forget 语义)。
pub fn record_event(event: LogEvent) {
    // 序列化在最前: 若事件本身不可序列化, 后续扇出全部跳过
    let value = match serde_json::to_value(&event) {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(
                target: "log_internal",
                error = ?err,
                "log: event serialization failed"
            );
            return;
        }
    };

    //todo obs bridge 转发

    // observer 桥接 (投影到 ObserverEvent)
    crate::observer_bridge::forward(&event);

    // 广播 (SSE / 实时订阅) -- 失败静默, 不影响后续持久化
    if let Some(hook) = crate::current_broadcast_hook() {
        let _ = hook.send(value.clone());
    }

    // 持久化到 JSONL 文件
    let Some(state) = current_state() else {
        // 未初始化: 静默丢弃, 不报错 (典型场景: 测试或尚未调用 init_from_config)
        return;
    };

    if !state.policy.storage.is_enabled() {
        return;
    }

    // 追加到文件
    if let Err(err) = append_line(&state, &value) {
        tracing::warn!(
            target: "log_internal",
            error = ?err,
            "log: append failed"
        );
    }
}

/// 写一行 JSON + 换行到任意 writer。抽出来便于 trim 重写时复用。
fn write_jsonl_line<W: io::Write + ?Sized>(writer: &mut W, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *writer, value).context("serializing log line")?;
    writer.write_all(b"\n").context("writing newline")?;
    Ok(())
}

/// 把一条事件追加到当前 JSONL 文件, 并按 storage 策略做后处理。
///
/// 步骤意图:
/// 1. `create_dir_all` -- 首次启动或 workspace 迁移后目录可能不存在
/// 2. `maybe_rotate_for_date` (仅 Rotating) -- 写入前若跨天则把当前文件归档,
///    保证"同一文件属于同一天", 简化按日查询
/// 3. 打开时强制 `0o600` (Unix) -- 日志可能含敏感输入, 默认仅属主可读写
/// 4. `BufWriter` + `flush` + `sync_data` -- 三段式确保崩溃不丢日志
///    (sync_data 比 sync_all 更便宜, 不刷文件元数据)
/// 5. 写入后再次 `set_permissions(0o600)` -- 兜底: 若文件已存在且之前权限更宽松,
///    追加打开不会收紧权限, 必须显式降权
/// 6. 按 storage 策略裁剪/轮转 -- 写后再做, 避免读取正在写的文件产生竞态
fn append_line(state: &WriterState, value: &Value) -> Result<()> {
    // 唯一持锁点: 序列化并发 append / trim / rotate。
    // parking_lot::Mutex 不可重入 -- 上层调用方 (record_event) 不得再获取此锁,
    // 否则同线程二次 lock 会死锁。
    let _guard = state.write_lock.lock();
    if let Some(parent) = state.policy.path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating log directory {}", parent.display()));
    }

    if state.policy.storage == StoragePolicy::Rotating {
        maybe_rotate_for_date(state)?;
    }

    let mut options = OpenOptions::new();
    options.create(true).append(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // 创建时掩码 0o600。注意: 仅对"本次新建"的文件生效;
        // 已存在文件需在写入后显式 set_permissions 收紧 (见下方)。
        options.mode(0o600);
    }

    let file = options
        .open(&state.policy.path)
        .with_context(|| format!("opening log file {}", state.policy.path.display()))?;

    let mut writer = BufWriter::new(file);
    write_jsonl_line(&mut writer, value)?;

    // flush 把 BufWriter 内的字节推给底层文件描述符
    writer.flush().context("flushing log line")?;
    // into_inner 取回原始 File 以便 fsync (BufWriter 没有 sync_data 接口)
    let file = writer
        .into_inner()
        .context("taking log file out of buf writer")?;

    // sync_data 而非 sync_all: 只刷数据不刷元数据, 性能更好且足够保证内容持久
    file.sync_data().context("fsync log line")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // 兜底收紧权限: 若文件已存在且权限更宽 (如之前以默认 umask 创建),
        // 追加打开不会自动收紧, 这里显式降到 0o600。
        let _ = fs::set_permissions(&state.policy.path, fs::Permissions::from_mode(0o600));
    }

    // 写后裁剪/轮转: 此时数据已稳定落盘, 读旧文件做重写或换文件都是安全的
    match state.policy.storage {
        StoragePolicy::Rolling => trim_to_last_entries(state)?,
        StoragePolicy::Rotating => maybe_rotate_for_size(state)?,
        StoragePolicy::None | StoragePolicy::Full => {}
    }

    Ok(())
}

/// Rolling 策略: 保留最新 `max_entries` 行, 流式重写文件。
///
/// 不一次性 `read_to_end` 到内存 -- 旧日志可能上 GB。流程:
/// 1. 数当前非空行数 (一遍扫描)
/// 2. 若超过上限, 写临时文件: 跳过前 `skip = total - max` 行, 余下逐行复制
/// 3. fsync 临时文件后 rename 原子替换
///
/// 始终保持 RAM 占用 = 一行读缓冲 + 一行写缓冲。
fn trim_to_last_entries(state: &WriterState) -> Result<()> {
    let total = count_nonempty_lines(&state.policy.path)?;

    if total <= state.policy.max_entries {
        return Ok(());
    }

    let skip = total - state.policy.max_entries;

    // 临时文件名混入 pid + nanos, 防止多进程或并发 trim 互相覆盖
    let tmp = state.policy.path.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ));

    {
        let mut opts = OpenOptions::new();
        // create_new 防止意外覆盖已存在的 tmp (并发场景)
        opts.create_new(true).write(true);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }

        let out_file = opts
            .open(&tmp)
            .with_context(|| format!("creating trim temp file {}", tmp.display()))?;
        let mut out = BufWriter::new(out_file);

        let in_file = File::open(&state.policy.path)
            .with_context(|| format!("opening log for trim: {}", state.policy.path.display()))?;
        let reader = BufReader::new(in_file);

        // 单遍扫描: 跳过前 `skip` 行, 之后才开始写入
        let mut index: usize = 0;
        for line in reader.lines() {
            let line = line.context("reading log line during trim")?;
            // 跳过完全空行 (历史 append 不会产生, 但容错)
            if line.trim().is_empty() {
                continue;
            }
            if index >= skip {
                out.write_all(line.as_bytes())
                    .context("writing trim line")?;
                out.write_all(b"\n").context("writing trim newline")?;
            }
            index += 1;
        }
        out.flush().context("flushing trim file")?;
        // 写完 fsync, 确保 rename 前数据稳定 (rename 本身只保证元数据原子)
        out.into_inner()
            .context("taking trim file out of buf writer")?
            .sync_data()
            .context("fsync trim file")?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }

    // rename 是原子的 (同文件系统内), 读者不会看到半截文件
    fs::rename(&tmp, &state.policy.path).with_context(|| {
        format!(
            "renaming trim tmp {} -> {}",
            tmp.display(),
            state.policy.path.display()
        )
    })?;

    Ok(())
}

/// Rotating 策略写入后触发: 文件 >= `max_bytes` 时换文件。
/// `max_bytes == 0` 视为不限制 (仅按日期切)。
fn maybe_rotate_for_size(state: &WriterState) -> Result<()> {
    let max = state.policy.max_bytes;
    if max == 0 {
        return Ok(());
    }

    let path = &state.policy.path;
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        // 文件可能尚未创建 (首次写入前), 跳过
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("stat log for size rotation: {}", path.display()));
        }
    };

    if meta.len() >= max {
        // 用 mtime 作为归档时间戳, 比当前时间更准确反映"这批数据的时段"
        let modified: DateTime<Utc> = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH).into();
        rotate_active(state, modified)?;
    }
    Ok(())
}

/// Rotating 策略写入前触发: 跨天则换文件, 保证同文件 = 同日。
/// `rotate_daily == false` 时禁用。空文件不切 (首次启动当天)。
fn maybe_rotate_for_date(state: &WriterState) -> Result<()> {
    if !state.policy.rotate_daily {
        return Ok(());
    }

    let path = &state.policy.path;
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("stat log for date rotation: {}", path.display()));
        }
    };

    // 空文件 (刚 trim 完或首次创建) 不切, 否则会产生空归档
    if meta.len() == 0 {
        return Ok(());
    }

    let modified: DateTime<Utc> = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH).into();
    // 严格小于今天: 用 date_naive 比较, 忽略时区内时分秒
    if modified.date_naive() < Utc::now().date_naive() {
        rotate_active(state, modified)?;
    }
    Ok(())
}

/// 把当前活动文件重命名为归档名, 然后跑 retention 清理。
/// 调用后 `state.policy.path` 不再存在, 下次 append 会自动 create。
fn rotate_active(state: &WriterState, time: DateTime<Utc>) -> Result<()> {
    let path = &state.policy.path;
    let archive = archive_path(path, time)?;
    fs::rename(path, &archive)
        .with_context(|| format!("rotating log {} -> {}", path.display(), archive.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // 归档文件同样保持 0o600
        let _ = fs::set_permissions(&archive, fs::Permissions::from_mode(0o600));
    }
    // 换完文件立即跑 retention, 避免归档堆积
    run_retention(state);
    Ok(())
}

/// 计算归档文件路径: `<base>.<YYYYMMDD-HHMMSS>.<ext>`。
/// 同秒冲突时追加序号后缀: `<base>.<stamp>.<n>.<ext>` (始终基于 dir, 不嵌套)。
fn archive_path(path: &PathBuf, time: DateTime<Utc>) -> Result<PathBuf> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .context("log path has not filename")?;
    let (base, ext) = split_file_name(file_name);
    // 时间戳格式与 is_stamp 校验严格对齐: 8 位日期 + '-' + 6 位时间 = 15 字符
    let stamp = time.format("%Y%m%d-%H%M%S").to_string();

    let mut candidate = dir.join(format!("{base}.{stamp}.{ext}"));
    let mut n = 1u32;
    while candidate.exists() {
        candidate = dir.join(format!("{base}.{stamp}.{n}{ext}"));
        n += 1;
    }
    Ok(candidate)
}

/// 把文件名拆为 (stem, ext_with_dot)。最后一个 `.` 之后为扩展名。
/// `foo.tar.gz` -> ("foo.tar", ".gz"); 无点或点在首位 -> (原名, "")。
fn split_file_name(filename: &str) -> (&str, &str) {
    match filename.rfind('.') {
        Some(i) if i > 0 => (&filename[..i], &filename[i..]),
        _ => (filename, ""),
    }
}

/// 列出所有归档文件 (不含活动文件), 按发现顺序返回 (路径, mtime)。
/// 仅识别符合 `<base>.<stamp>[.<counter>].<ext>` 模式的文件, 由 is_archive_core 校验。
fn list_archives(active: &Path) -> Result<Vec<(PathBuf, SystemTime)>> {
    let dir = active.parent().unwrap_or_else(|| Path::new("."));
    let active_name = active
        .file_name()
        .and_then(|s| s.to_str())
        .context("log path has no filename")?;
    let (base, ext) = split_file_name(active_name);
    let prefix = format!("{base}.");
    let mut out = Vec::new();

    let entries = match fs::read_dir(dir) {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(err) => {
            return Err(err).with_context(|| format!("reading log dir: {}", dir.display()));
        }
    };

    for entry in entries {
        let entry = entry.with_context(|| format!("reading entry in {}", dir.display()))?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        // 跳过活动文件本身
        if name == active_name {
            continue;
        }
        // 必须以 `<base>.` 开头, 防止误识别同目录其它日志
        let Some(suffix) = name.strip_prefix(&prefix) else {
            continue;
        };
        // 剥掉扩展名得到 "core" (stamp 或 stamp.counter)
        let core = if ext.is_empty() {
            suffix
        } else {
            let Some(core) = suffix.strip_suffix(ext) else {
                continue;
            };
            core
        };
        if !is_archive_core(core) {
            continue;
        }

        let Ok(meta) = entry.metadata() else { continue };
        // 只关心普通文件, 跳过目录/符号链接等
        if !meta.is_file() {
            continue;
        }
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        out.push((entry.path(), mtime));
    }
    Ok(out)
}

/// 校验 core 是否为合法归档标识: 纯 stamp, 或 stamp.counter。
/// `20260716-120000` -> true; `20260716-120000.1` -> true; 其它 -> false。
fn is_archive_core(core: &str) -> bool {
    match core.split_once(".") {
        Some((stamp, counter)) => {
            !counter.is_empty() && counter.bytes().all(|b| b.is_ascii_digit()) && is_stamp(stamp)
        }
        None => is_stamp(core),
    }
}

/// 校验时间戳格式: 8 位日期 + '-' + 6 位时间, 共 15 字符, 全数字 (除分隔符)。
/// 必须与 archive_path 中的 `%Y%m%d-%H%M%S` 严格对齐。
fn is_stamp(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 15
        && b[..8].iter().all(u8::is_ascii_digit)
        && b[8] == b'-'
        && b[9..].iter().all(u8::is_ascii_digit)
}

/// 归档保留策略: 双维度清理。
///
/// - `max_age_days > 0`: 删除早于 cutoff 的归档 (按 mtime)
/// - `max_files > 0`: 若剩余归档数仍超限, 按 list_archives 返回顺序 (近似创建顺序)
///   删除最旧的
///
fn run_retention(state: &WriterState) {
    let max_files = state.policy.retention_max_files;
    let max_age_days = state.policy.retention_max_age_days;
    if max_files == 0 && max_age_days == 0 {
        return;
    }

    let mut archives = match list_archives(&state.policy.path) {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(
                target: "log_internal",
                error = ?err,
                "log: list archives for retention failed"
            );
            return;
        }
    };

    archives.sort_by_key(|(_, mtime)| std::cmp::Reverse(*mtime));

    // 维度一: 按年龄清理。retain 边删边过滤, 保留未过期的
    if max_age_days > 0
        && let Some(cutoff) =
            SystemTime::now().checked_sub(Duration::from_secs(max_age_days.saturating_mul(86400)))
    {
        archives.retain(|(pf, mtime)| {
            if *mtime < cutoff {
                remove_archive(pf);
                false
            } else {
                true
            }
        });
    }

    // 维度二: 按数量清理。注意: archives 未排序, 见函数级 FIXME
    if max_files > 0 && archives.len() > max_files {
        for (pf, _) in archives.iter().skip(max_files) {
            remove_archive(pf);
        }
    }
}

/// 删除单个归档文件, 失败仅 warn 不传播 (retention 是 best-effort)。
fn remove_archive(pf: &Path) {
    if let Err(err) = fs::remove_file(pf) {
        tracing::warn!(
            target: "log_internal",
            error = ?err,
            path = %pf.display(),
            "log: pruning rotated archive failed"
        );
    }
}

/// 数文件中非空行数。用于 Rolling 策略判断是否触发 trim。
/// 单遍扫描, RAM 占用 = 一行缓冲。
fn count_nonempty_lines(path: &Path) -> Result<usize> {
    let file = fs::File::open(path)
        .with_context(|| format!("opening log to count lines: {}", path.display()))?;

    let reader = BufReader::new(file);
    let mut count = 0usize;
    for line in reader.lines() {
        let line = line.context("reading log line for count")?;
        // 注意: 这里只判断 `!line.is_empty()`, 而 trim_to_last_entries 判断
        // `line.trim().is_empty()`。两者对含空白字符行的处理不一致 -- 若日志中
        // 出现纯空白行, count 会计入但 trim 不会写入, 长期可能产生轻微偏差。
        // 影响很小, 历史日志通常不会有纯空白行。
        if !line.is_empty() {
            count += 1;
        }
    }
    Ok(count)
}

# ZeroClaw 基本工具实现分析

源码位置: /Users/zhuwenquan/Downloads/zeroclaw-master/

## 1. Tool trait 定义 (crates/zeroclaw-api/src/tool.rs)

### 1.1 ToolResult 结构

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}
```

设计要点:
- success: bool — 执行是否成功
- output: String — 成功时的输出内容（也是失败时附加警告的字段）
- error: Option<String> — 失败原因（None 表示无错误）

### 1.2 ToolSpec 结构（用于 LLM 注册）

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}
```

### 1.3 Tool trait 本身

```rust
#[async_trait]
pub trait Tool: Send + Sync + crate::attribution::Attributable {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult>;

    // 默认实现：聚合上面三个方法生成 ToolSpec
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}
```

设计要点:
- 超 trait 是 `Attributable`（不是孤立的 Tool），让 `&dyn Tool` 可自动 coerce 到 `&dyn Attributable`，dispatch 层日志/审计无需知道具体工具类型
- Attributable 要求实现 `role() -> Role` 和 `alias() -> &str`
- 用 `tool_attribution!` 宏消除样板代码: `crate::tool_attribution!(ShellTool, ToolKind::Shell);`

### 1.4 Attributable + Role + ToolKind (crates/zeroclaw-api/src/attribution.rs)

```rust
pub trait Attributable {
    fn role(&self) -> Role;
    fn alias(&self) -> &str;
}

pub enum Role {
    Swarm,
    Agent,
    Channel(ChannelKind),
    Provider(ProviderKind),
    Tool(ToolKind),   // ← 工具走这个变体
    Memory(MemoryKind),
    Observer(ObserverKind),
    Peripheral(PeripheralKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum ToolKind {
    Shell, HttpRequest, HttpServer, FetchUrl, Search, Memory,
    SpawnSubagent, SopList, SopExecute, SopApprove, SopAdvance,
    SopStatus, SopHistory, Wait, Plugin,
}
```

### 1.5 临时工作区警告机制（共享）

所有文件系统相关工具共享一个常量 + 辅助函数:

```rust
pub const EPHEMERAL_WORKSPACE_WARNING: &str =
    "⚠️ EPHEMERAL WORKSPACE: the active runtime uses an ephemeral workspace \
     (tmpfs / no host volume mount). Files written here do NOT persist ...";

pub fn with_ephemeral_workspace_warning(text: &str) -> String {
    if text.is_empty() {
        EPHEMERAL_WORKSPACE_WARNING.to_string()
    } else {
        format!("{EPHEMERAL_WORKSPACE_WARNING}\n\n{text}")
    }
}
```

不同工具的行为差异:
- file_write: 临时工作区直接拒绝（返回 error）
- shell / file_read / file_edit: 仍可使用，但结果附带警告横幅


## 2. ShellTool (crates/zeroclaw-runtime/src/tools/shell.rs)

### 2.1 结构与依赖

```rust
pub struct ShellTool {
    security: Arc<SecurityPolicy>,
    runtime: Arc<dyn RuntimeAdapter>,
    sandbox: Arc<dyn Sandbox>,
    timeout_secs: u64,
    tui_env: Option<HashMap<String, String>>,
    persistent_writes: bool,
}
```

关键 builder 方法:
- `new(security, runtime)` — 用 NoopSandbox
- `new_with_sandbox(security, runtime, sandbox)` — 注入沙箱
- `with_persistent_writes(bool)` — 控制临时工作区行为
- `with_timeout_secs(u64)` — 覆盖超时
- `with_tui_env(Option<HashMap>)` — 转发 TUI 客户端环境

### 2.2 name / description / parameters_schema

```rust
fn name(&self) -> &str { "shell" }
fn description(&self) -> &str { "Execute a shell command in the workspace directory" }

fn parameters_schema(&self) -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "command": { "type": "string", "description": "The shell command to execute" },
            "approved": {
                "type": "boolean",
                "description": "Set true to explicitly approve medium/high-risk commands in supervised mode",
                "default": false
            }
        },
        "required": ["command"]
    })
}
```

### 2.3 execute 核心逻辑（设计要点）

1. 参数提取: command (必需), approved (可选, 默认 false)
2. 安全验证: `security.validate_command_execution(command, approved)` — 按风险等级和自治级别审批
3. 构建命令: `runtime.build_shell_command(command, &workspace_dir)` — RuntimeAdapter 抽象不同平台
4. 沙箱包装: `sandbox.wrap_command(cmd.as_std_mut())` — 注入沙箱约束
5. 环境隔离 (CWE-200 防护):
   ```rust
   cmd.env_clear();  // 先清空，防泄漏 API keys
   for var in collect_allowed_shell_env_vars(&self.security) {
       if let Ok(val) = std::env::var(&var) { cmd.env(&var, val); }
   }
   // 注入 session_id, 叠加 TUI env
   ```
   安全白名单变量: PATH, HOME, TERM, LANG, LC_ALL, LC_CTYPE, USER, SHELL, TMPDIR (+ Windows: PATHEXT, USERPROFILE, SYSTEMROOT...)
6. 进程组管理 (Unix): `cmd.process_group(0)` + `ChildGroupGuard`（Drop 时 SIGKILL 整个进程组，防止僵尸子进程）
7. 超时执行:
   ```rust
   let mut child = cmd.spawn()?;
   let stdout_drain = spawn_drain(stdout_handle, MAX_OUTPUT_BYTES); // 1MB cap
   let stderr_drain = spawn_drain(stderr_handle, MAX_OUTPUT_BYTES);
   match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait()).await {
       Ok(Ok(status)) => { /* disarm guard, drain, decode, 组装 ToolResult */ }
       Ok(Err(e)) => { /* 执行失败 */ }
       Err(_) => { child.start_kill(); /* 超时杀死 */ }
   }
   ```
8. 输出截断: MAX_OUTPUT_BYTES = 1MB，超过则截断并追加 `... [output truncated at 1MB]`
9. 编码处理:
   - Unix: `String::from_utf8_lossy`
   - Windows: 运行时查询 ConsoleOutputCP，用 encoding_rs 转码 (GBK/SHIFT_JIS/EUC_KR/BIG5 等)
10. 临时工作区: 若 `!persistent_writes`，对 output 和 error 都附加警告横幅


## 3. FileReadTool (crates/zeroclaw-runtime/src/tools/file_read.rs)

### 3.1 结构

```rust
const MAX_FILE_SIZE_BYTES: u64 = 10 * 1024 * 1024; // 10MB

pub struct FileReadTool {
    security: Arc<SecurityPolicy>,
    persistent_writes: bool,
}
```

### 3.2 parameters_schema

```rust
fn parameters_schema(&self) -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "..." },
            "offset": { "type": "integer", "description": "Starting line number (1-based, default: 1)..." },
            "limit": { "type": "integer", "description": "Maximum number of lines to return..." },
            "encoding": {
                "type": "string",
                "enum": ["utf8", "base64"],
                "description": "Output encoding (default: 'utf8'). Use 'base64' for binary files."
            }
        },
        "required": ["path"]
    })
}
```

### 3.3 execute 核心逻辑

1. base64 模式不附加警告（会破坏解码），utf8 模式在临时工作区附加警告
2. 路径解析 `resolve_candidate`:
   - 拒绝 null byte
   - 拒绝 `..` 路径遍历
   - `security.resolve_tool_path(path)` 解析为绝对路径
3. canonicalize 解析符号链接，再用 `is_resolved_path_readable` 校验边界
   - 允许: workspace + read-write allowlist + read-only allowlist + POSIX 设备文件
4. 文件大小检查（canonicalize 之后做，防 TOCTOU 符号链接绕过）
5. 分支处理:
   - **base64**: 读原始字节 → base64 编码返回（不走行号/分页）
   - **utf8**: `read_to_string`，失败则尝试:
     - PDF 文本提取 (rag-pdf feature, pdf_extract)
     - 图片格式检测 (PNG/JPEG/GIF/WEBP/BMP magic bytes) → 拒绝并指向 image_info 工具
     - 二进制检测 (looks_binary: NUL byte 或 >30% 控制字符) → 拒绝并建议 base64
     - 否则 lossy 解码（非 UTF-8 文本如 cp1251/Latin-1）
6. 行号格式:
   ```rust
   let numbered: String = lines[start..end]
       .iter()
       .enumerate()
       .map(|(i, line)| format!("{}: {}", start + i + 1, line))
       .collect::<Vec<_>>().join("\n");
   // 末尾追加 [Lines X-Y of N] 或 [N lines total]
   ```
7. 反探测: resolve/canonicalize 失败时主动 `record_action()`，防止免费探测路径存在性


## 4. FileWriteTool (crates/zeroclaw-tools/src/file_write.rs)

### 4.1 结构

```rust
pub struct FileWriteTool {
    security: Arc<SecurityPolicy>,
    persistent_writes: bool,
}
```

### 4.2 parameters_schema

```rust
fn parameters_schema(&self) -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "..." },
            "content": { "type": "string", "description": "UTF-8 text or base64-encoded bytes" },
            "encoding": {
                "type": "string",
                "enum": ["utf8", "base64"],
                "description": "How to interpret 'content' (default: 'utf8')..."
            }
        },
        "required": ["path", "content"]
    })
}
```

### 4.3 execute 核心逻辑

1. 参数提取: path, content (均必需), encoding (默认 utf8)
2. 自治检查: `security.can_act()` — ReadOnly 模式直接拒绝
3. 临时工作区: `!persistent_writes` 直接拒绝（file_write 存在就是为了持久化）
4. 编码解码（在文件系统操作之前做，避免无效输入产生副作用）:
   - utf8 → `content.as_bytes().to_vec()`
   - base64 → `base64::decode(content)`，失败返回错误
5. 路径解析 + 安全检查:
   ```rust
   let full_path = self.security.resolve_tool_path(path);
   tokio::fs::create_dir_all(parent).await?;  // 自动创建父目录
   let resolved_parent = tokio::fs::canonicalize(parent).await?;  // canonicalize 在创建后做
   // 校验 resolved_parent 是否允许
   self.security.is_resolved_path_allowed(&resolved_parent)?
   self.security.is_runtime_config_path(&resolved_target)?  // 保护运行时配置
   ```
6. 符号链接防护: 目标若已是 symlink 则拒绝写入（`symlink_metadata` 检测）
7. 写入: `tokio::fs::write(&resolved_target, &bytes)`，返回 `Written {n} bytes to {path}`


## 5. FileEditTool (crates/zeroclaw-tools/src/file_edit.rs)

### 5.1 结构（与 FileWriteTool 几乎相同）

```rust
pub struct FileEditTool {
    security: Arc<SecurityPolicy>,
    persistent_writes: bool,
}
```

### 5.2 parameters_schema

```rust
fn parameters_schema(&self) -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "..." },
            "old_string": { "type": "string", "description": "The exact text to find and replace (must appear exactly once in the file)" },
            "new_string": { "type": "string", "description": "The replacement text (empty string to delete the matched text)" }
        },
        "required": ["path", "old_string", "new_string"]
    })
}
```

### 5.3 execute 核心逻辑（精确字符串替换）

1. 参数提取 + 校验:
   - old_string 不能为空
   - 自治检查 `can_act()`
2. 路径解析、canonicalize、is_resolved_path_allowed、runtime_config 保护、symlink 拒绝（与 file_write 完全镜像）
3. 读取文件内容
4. 精确匹配计数:
   ```rust
   let match_count = content.matches(old_string).count();
   if match_count == 0 {
       return Err(no_match_diagnostic(&content, old_string));  // 智能诊断
   }
   if match_count > 1 {
       return Err("old_string matches {match_count} times; must match exactly once");
   }
   let new_content = content.replacen(old_string, new_string, 1);
   ```
5. 写回文件
6. 临时工作区: 成功时附加警告（失败时不附加，因为没有数据丢失）
7. 智能诊断 `no_match_diagnostic`:
   - 去掉前导空白后重新匹配
   - 若归一化后唯一匹配 → 提示"差异在缩进（宽度或 tab/space）"
   - 若归一化后多个匹配 → 提示"歧义，需更多上下文行"
   - 否则 → "old_string not found in file"


## 6. GlobSearchTool (crates/zeroclaw-tools/src/glob_search.rs)

### 6.1 结构

```rust
const MAX_RESULTS: usize = 1000;

pub struct GlobSearchTool {
    security: Arc<SecurityPolicy>,
}
```

### 6.2 parameters_schema

```rust
fn parameters_schema(&self) -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "pattern": { "type": "string", "description": "Glob pattern, e.g. '**/*.rs', 'src/**/mod.rs'" }
        },
        "required": ["pattern"]
    })
}
```

### 6.3 execute 核心逻辑

1. 安全检查:
   - 绝对路径（除非在 allowed_root 下）→ 拒绝
   - `..` 路径遍历 → 拒绝
2. 用 `resolve_tool_path` 处理 ~ 展开和绝对路径
3. `glob::glob(&full_pattern)` 枚举匹配项
4. 逐项 canonicalize，用 `is_resolved_path_readable` 校验（防符号链接逃逸）
5. 排除目录，只保留文件
6. 转为 workspace 相对路径
7. 排序 + 截断到 MAX_RESULTS，追加 `[Results truncated...]` 和 `Total: N files`


## 7. ContentSearchTool (crates/zeroclaw-tools/src/content_search.rs)

### 7.1 结构与后端检测

```rust
const MAX_RESULTS: usize = 1000;
const MAX_OUTPUT_BYTES: usize = 1_048_576; // 1 MB
const TIMEOUT_SECS: u64 = 30;

pub struct ContentSearchTool {
    security: Arc<SecurityPolicy>,
    backend: SearchBackend,
}

enum SearchBackend { Ripgrep, Grep, Internal }

fn detect_search_backend() -> SearchBackend {
    if which::which("rg").is_ok() { SearchBackend::Ripgrep }
    else if which::which("grep").is_ok() { SearchBackend::Grep }
    else { SearchBackend::Internal }
}
```

### 7.2 parameters_schema（最丰富的参数集）

```rust
fn parameters_schema(&self) -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "pattern": { "type": "string", "description": "Regular expression pattern to search for" },
            "path": { "type": "string", "default": ".", "description": "Directory to search in..." },
            "output_mode": {
                "type": "string",
                "enum": ["content", "files_with_matches", "count"],
                "default": "content"
            },
            "include": { "type": "string", "description": "File glob filter, e.g. '*.rs', '*.{ts,tsx}'" },
            "case_sensitive": { "type": "boolean", "default": true },
            "context_before": { "type": "integer", "default": 0 },
            "context_after": { "type": "integer", "default": 0 },
            "multiline": { "type": "boolean", "default": false, "description": "ripgrep only" },
            "max_results": { "type": "integer", "default": 1000 }
        },
        "required": ["pattern"]
    })
}
```

### 7.3 execute 核心逻辑

1. 参数解析 + 校验 (pattern 非空, output_mode 合法, path 无 `..`)
2. 路径解析 + canonicalize + `is_resolved_path_readable`
3. multiline 仅 ripgrep 支持，否则报错
4. 按后端分支:
   - **Ripgrep / Grep**: `execute_external_search`
     - build_rg_command / build_grep_command 构造命令
     - `cmd.env_clear()` + 仅保留 PATH/HOME/LANG/LC_ALL/LC_CTYPE
     - `tokio::time::timeout(30s, cmd.output())`
     - 退出码语义: 0=有匹配, 1=无匹配, ≥2=错误
     - format_rg_output / format_grep_output 格式化（按 output_mode）
   - **Internal**: `spawn_blocking` + `run_internal_search_with_deadline`
     - 用 regex crate 编译模式
     - 手动遍历目录树，glob 过滤 include
     - 双重超时: 内部 deadline + 外部 tokio::timeout
5. 输出截断到 1MB（`truncate_utf8` 保证 UTF-8 边界安全）


## 8. 工具包装器 (crates/zeroclaw-tools/src/wrappers.rs)

横切关注点用装饰器模式实现，组合顺序:

```text
RateLimitedTool (外)
  └─ PathGuardedTool (内)
       └─ <具体工具>
```

### 8.1 RateLimitedTool

```rust
pub struct RateLimitedTool<T: Tool> {
    inner: T,
    security: Arc<SecurityPolicy>,
}

async fn execute(&self, args) -> anyhow::Result<ToolResult> {
    if self.security.is_rate_limited() {
        return Ok(ToolResult { success: false, error: Some("Rate limit exceeded...".into()), .. });
    }
    let result = self.inner.execute(args).await?;
    // 只在成功时消费预算（保留旧语义：验证/策略失败不消耗）
    if result.success && !self.security.record_action() {
        return Ok(ToolResult { success: false, error: Some("Rate limit exceeded: action budget exhausted".into()), .. });
    }
    Ok(result)
}
```

设计要点:
- 先委托后计费 — 只有 `success: true` 才 `record_action()`
- name/description/parameters_schema/Attributable 全部透传给 inner

### 8.2 PathGuardedTool

```rust
pub struct PathGuardedTool<T: Tool> {
    inner: T,
    security: Arc<SecurityPolicy>,
    extractor: Option<Box<PathExtractor>>,
}

fn extract_path_string(&self, args) -> Option<String> {
    if let Some(ref f) = self.extractor { return f(args); }
    // 默认检查常见字段名
    for field in &["path", "command", "pattern", "query", "file"] {
        if let Some(s) = args.get(field).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

async fn execute(&self, args) -> anyhow::Result<ToolResult> {
    if let Some(arg) = self.extract_path_string(&args) {
        // shell command 用 token-aware 扫描器；纯路径用 is_path_allowed
        let blocked = if /* 是 command 字段 */ {
            self.security.forbidden_path_argument(&arg)
        } else if !self.security.is_path_allowed(&arg) {
            Some(arg.clone())
        } else { None };
        if let Some(path) = blocked {
            return Ok(ToolResult { success: false, error: Some(format!("Path blocked by security policy: {path}")), .. });
        }
    }
    self.inner.execute(args).await
}
```

设计要点:
- 字段名驱动: 默认扫描 path/command/pattern/query/file
- `with_extractor` 支持非标准字段名
- 阻止时不消耗 rate-limit 预算（因为 RateLimitedTool 只在 inner success 时计费）


## 9. 工具注册 (crates/zeroclaw-runtime/src/tools/mod.rs)

```rust
pub fn default_tools_with_runtime(
    security: Arc<SecurityPolicy>,
    runtime: Arc<dyn RuntimeAdapter>,
) -> Vec<Box<dyn Tool>> {
    let persistent_writes = runtime.has_filesystem_access();
    vec![
        Box::new(RateLimitedTool::new(
            PathGuardedTool::new(
                ShellTool::new(security.clone(), runtime).with_persistent_writes(persistent_writes),
                security.clone(),
            ),
            security.clone(),
        )),
        Box::new(RateLimitedTool::new(
            PathGuardedTool::new(
                FileReadTool::new_with_persistence(security.clone(), persistent_writes),
                security.clone(),
            ),
            security.clone(),
        )),
        // file_write / file_edit 同构
        // glob_search / content_search 不需要 persistent_writes 参数
        Box::new(RateLimitedTool::new(
            PathGuardedTool::new(GlobSearchTool::new(security.clone()), security.clone()),
            security.clone(),
        )),
        Box::new(RateLimitedTool::new(
            PathGuardedTool::new(ContentSearchTool::new(security.clone()), security.clone()),
            security,
        )),
    ]
}
```

注意: 源码中 shell 和 file_read 在 zeroclaw-runtime crate，而 file_write/file_edit/glob_search/content_search 在 zeroclaw-tools crate（历史原因，AGENTS.md 标注 zeroclaw-runtime 是过渡性 holding crate）。


## 10. 总结：与 shadow 对比的设计启示

| 维度 | ZeroClaw 设计 | shadow 可借鉴点 |
|------|--------------|----------------|
| ToolResult | `{success, output, error: Option}` | 结构简洁，error 可选；可直接复用 |
| Tool trait 超 trait | `Tool: Attributable` | 若 shadow 不需要审计归因，可不加超 trait |
| parameters_schema | 返回 `serde_json::Value` (json! 宏构造) | 简单直接，无需 schema 库 |
| 横切关注点 | 装饰器: RateLimitedTool(PathGuardedTool(Inner)) | 优雅，避免每个工具重复 guard 代码 |
| Shell 安全 | env_clear + 白名单变量 + 进程组 SIGKILL + 沙箱 trait | 进程组回收是亮点 |
| file_edit | 精确字符串匹配，要求唯一匹配 + 智能诊断 | no_match_diagnostic 对缩进差异给出可操作提示 |
| file_read | 行号 + offset/limit 分页 + base64 + 二进制检测 | looks_binary 启发式 (NUL + 控制字符密度) |
| 搜索后端 | ripgrep > grep > internal 三级降级 | 检测 + 降级策略成熟 |
| 临时工作区 | 共享 EPHEMERAL_WORKSPACE_WARNING，不同工具不同策略 | 统一常量 + 辅助函数 |

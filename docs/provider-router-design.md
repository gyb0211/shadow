# Provider Router 设计 -- 3 层架构 (Router + Reliable + Compat)

> 状态: Phase 1-4 已实现 (feature/provider-router 分支)
> 关联: 实现细节见 `crates/shadow-providers/src/{router,reliable,openai,error,rate_limit}.rs`

## 1. 背景与动机

影子在与 LLM 交互时需要处理:

1. **多家 provider** -- OpenAI / Anthropic / OpenRouter / Ollama / 各种 OpenAI-compatible
2. **错误恢复** -- 网络抖动、5xx、限流 (429)、key 失效
3. **key 轮换** -- 多个 API key 分摊配额、规避单 key 限流
4. **限流** -- 自带 RPM 上限, 避免打爆上游
5. **跨 provider fallback** -- 主 provider 整体不可用时切到备选

单层 `OpenAiProvider` 无法承担这些职责. 借鉴 zeroclaw 的 3 层架构, 我们用
decorator 模式逐层包装:

```
Agent  (Arc<dyn Provider>)
  ↓
RouterModelProvider  (顶层) -- hint 路由 + 跨 provider fallback
  ↓ per-route
ReliableModelProvider  (中层) -- 重试 + 退避 + key 轮换 + 限流 + fallback_models
  ↓
OpenAiProvider  (底层) -- OpenAI-compat 协议 (auth/path/payload 适配)
```

每层只面对 `dyn Provider`, 互不感知. Agent 拿到的是最外层的 `Arc<dyn Provider>`.

## 2. 三层职责

### 2.1 Compat 层 -- `OpenAiProvider`

**职责**: 把 OpenAI-compat 协议家族的差异 (auth style、API path、payload 形态)
适配为统一 `Provider` trait.

**关键设计**:
- `client: reqwest::Client` -- 构造时 build 一次, 复用连接池 (不再 per-call)
- `api_key: Arc<RwLock<Option<String>>>` -- 运行时可切换 (key 轮换依赖此)
- `set_api_key(&self, key: Option<String>)` -- 写锁更新
- 实现 `KeyRotator` trait (`fn set_key(&self, key: Option<&str>)`)
- 错误透传为 `ChatError` (带 HTTP status), 不再 bail 字符串

**支持的家族**: `openai` / `openrouter` / `ollama` / `compatible`. 不同家族
主要差异是 `base_url` 和某些字段名.

### 2.2 Reliable 层 -- `ReliableModelProvider`

**职责**: 在 inner provider 之上加重试 / 退避 / key 轮换 / 限流 / fallback_models.

**字段**:
```rust
pub struct ReliableModelProvider {
    alias: String,
    inner: Arc<dyn Provider>,
    policy: RetryPolicy,             // max_retries, backoff_ms, jitter_pct
    keys: Vec<String>,               // 多 key 列表
    key_idx: AtomicUsize,            // 当前 key 索引 (round-robin)
    rotator: Option<Arc<dyn KeyRotator>>,  // 把 key 推到 inner
    rate_limiter: Option<Arc<TokenBucket>>,
    fallback_models: Vec<String>,    // 模型级 fallback
}
```

**chat_with_retry 流程** (核心算法):
```
'outer: for model in [primary, *fallback_models] {
    let mut keys_tried = 0;
    for attempt in 0..=max_retries {
        if attempt > 0 { sleep(policy.backoff(attempt)) }
        self.pre_call().await;  // 限流 acquire + key 注入
        match inner.chat(req).await {
            Ok(resp) => return Ok(resp),
            Err(err) => match classify(err) {
                Auth if keys.len() > 0 => {
                    keys_tried += 1;
                    if keys_tried >= keys.len() {
                        last_err = err; break; // 所有 key 都失败, 跳到下一个 model
                    }
                    advance_key(); continue;  // 立即切 key 重试 (无 backoff)
                }
                Transient | Network | RateLimit => {
                    last_err = err; continue; // 退避后重试
                }
                Permanent => {
                    last_err = err; continue 'outer;  // 跳到下一个 model
                }
            }
        }
    }
}
return Err(last_err);
```

**关键策略**:
- **Auth 错误**: 切换 key 立即重试 (无 backoff). 试遍所有 key 仍失败才跳到下个 model.
- **Transient / Network**: 指数退避重试.
- **RateLimit**: 退避重试, 尊重服务器 Retry-After (如果有).
- **Permanent**: 不重试当前 model, 直接跳到 fallback_models 的下一个.
- **stream 重试**: 只在 pre-stream 阶段 (建立连接失败) 重试. 一旦 Ok(BoxStream)
  返回, mid-stream 错误直接透传, 不再重试 (避免 chunk 序列错乱).

**RetryPolicy.compute_backoff(attempt)**:
```
base = min(initial * 2^(attempt-1), max_backoff_ms)
jitter = base * jitter_pct / 100 * (LCG_pseudo_random * 2 - 1)
backoff = base + jitter
```
LCG (线性同余) 用 `AtomicU64` 维护种子, 避免每次重试都调系统 RNG.

### 2.3 Router 层 -- `RouterModelProvider`

**职责**: hint 路由 + 跨 provider fallback.

**字段**:
```rust
pub struct RouterModelProvider {
    alias: String,
    routes: HashMap<String, (usize, String)>,  // hint → (idx, model)
    model_providers: Vec<(String, Box<dyn Provider>)>,
    default_index: usize,
    fallback_chains: HashMap<String, Vec<usize>>,  // hint_or_"default" → [idx, ...]
    default_model: String,
}
```

**chat 流程**:
```
let (idx, model) = router.resolve(req.model);  // "hint:x" 查 routes, 否则 default
match providers[idx].chat(req with model).await {
    Ok(r) => return Ok(r),
    Err(_) => warn "primary failed",
}
// 走 fallback chain
let chain = fallback_chains[hint].or(fallback_chains["default"]);
for fallback_idx in chain {
    if fallback_idx == idx { continue; }  // 主 provider 已试过
    match providers[fallback_idx].chat(req).await {
        Ok(r) => return Ok(r),
        Err(_) => continue,
    }
}
return Err("all providers failed");
```

**关键策略**:
- Router **不分类错误** -- 任何 Err 都触发 fallback (因为 inner 已是 Reliable-wrapped,
  内部已耗尽重试).
- Chain 中的 provider 用 request.model 原值 (不再做 hint→model 替换).
- 未指定 hint 的 fallback chain 时, 回退到 `"default"` chain.
- Stream 路径同样支持 fallback (只在 pre-stream 阶段).

## 3. 配置 Schema

### 3.1 完整示例

```toml
schema_version = 2

[agent]
alias = "default"
model_provider = "openai.default"
model = "gpt-4o-mini"
temperature = 0.7
autonomy = "supervised"

# ── Providers ──

[providers.openai.default]
api_keys = ["sk-1", "sk-2"]   # 多 key 轮换; 也接受 api_key = "sk-1"
model = "gpt-4o-mini"

[providers.openai.default.reliable]
max_retries = 3
initial_backoff_ms = 1000
max_backoff_ms = 60000
jitter_pct = 25
requests_per_minute = 60       # 0 = 无限流

[providers.anthropic.claude]
api_keys = ["sk-ant-xxx"]
model = "claude-sonnet-4-20250514"
fallback_models = ["claude-3-5-haiku-20241022"]  # 同 provider 内换模型重试

[providers.custom.glm]
api_keys = ["glm-key"]
base_url = "https://open.bigmodel.cn/api/paas/v4"
model = "glm-4-flash"

# ── Router (可选) ──

[router]
default = "openai.default"

[router.routes.reasoning]
provider = "anthropic.claude"
model = "claude-sonnet-4-20250514"

[router.fallback_chains]
default = ["anthropic.claude", "custom.glm"]   # 主失败时依次尝试
reasoning = ["openai.default"]                  # hint 特定 chain

# ── Memory ──
[memory]
backend = "none"
```

### 3.2 字段参考

#### ProviderEntry

| 字段 | 类型 | 默认 | 说明 |
|------|------|------|------|
| `api_key` | string | - | 单 key (向后兼容) |
| `api_keys` | string[] | `[]` | 多 key (推荐形态) |
| `model` | string | - | 默认模型 |
| `base_url` | string | family 默认 | API 端点 |
| `temperature` | f64 | 0.7 | 采样温度 |
| `max_tokens` | u32 | - | 响应上限 |
| `timeout_secs` | u64 | - | 超时 |
| `fallback_models` | string[] | `[]` | 同 provider 内换模型 |
| `reliable` | ReliableConfig | 见下 | 重试/限流配置 |

`api_key` 与 `api_keys` 同时出现时合并去重 (api_key 优先).

#### ReliableConfig

| 字段 | 类型 | 默认 | 说明 |
|------|------|------|------|
| `max_retries` | u32 | 3 | 最大重试次数 (0=只调一次) |
| `initial_backoff_ms` | u64 | 1000 | 初始退避 |
| `max_backoff_ms` | u64 | 60000 | 退避上限 |
| `jitter_pct` | u8 | 25 | Jitter 百分比 (0-100) |
| `requests_per_minute` | u32 | 0 | RPM 限流 (0=无限流) |

全字段等于默认值时整个 `[reliable]` 段不序列化.

#### RouterConfig

| 字段 | 类型 | 说明 |
|------|------|------|
| `default` | string | 默认 provider 引用 (family.alias) |
| `routes` | map\<string, RouteEntry\> | hint → (provider, model) |
| `fallback_chains` | map\<string, string[]\> | hint (或 "default") → provider 引用列表 |

`Config.router = None` 表示单 provider 模式 (向后兼容, 不构造 Router).

## 4. 错误分类 -- ChatError

`shadow-providers` 内部类型 (不污染 shadow-core):

```rust
pub enum RetryClass {
    Transient,                          // 5xx -- 退避重试
    RateLimit { retry_after_secs },     // 429 -- 退避重试, 尊重 Retry-After
    Auth,                               // 401/403 -- 切 key 重试
    Permanent,                          // 400/404/... -- 不重试
    Network,                            // 连接失败/DNS -- 退避重试
}

pub struct ChatError {
    pub status: Option<u16>,
    pub message: String,
    pub class: RetryClass,
}
```

`ChatError::from_status(status, body)` 按 HTTP 码自动分类. Compat 层把所有 HTTP
错误转成 `ChatError`, Reliable 层 `downcast_ref::<ChatError>()` 提取分类决策重试.

## 5. 迁移指南

### v1 → v2

**触发**: schema_version 1 → 2 (含 api_keys 字段)

**自动转换**: 加载配置时自动运行, 用户无感:
- `api_key = "sk-xxx"` → `api_keys = ["sk-xxx"]`
- 同时存在 `api_key` 与 `api_keys` → 合并去重 (api_key 优先)

**手动迁移**: 也可以直接编辑 config.toml, 把字段名改了即可.

**密钥加密**: api_keys 中的每个 key 独立加密 (enc2: 前缀). 已加密的 key 在
迁移时透传, 不重复加密.

### 调试技巧

- **trace log**: `RUST_LOG=shadow_providers=trace` 查看 backoff / key 切换 / fallback
  决策
- **config show**: `shadow config show` 显示 api_keys (掩码) / reliable 段 /
  router 段
- **哪些 provider 服务了请求**: 日志里 `provider_idx=0` (主) → `fallback=backup`
  (切换到 backup) 的轨迹一目了然

## 6. 实施阶段

### Phase 1: ChatError + ReliableModelProvider + Client fix ✅

- `error.rs` -- ChatError + RetryClass
- `reliable.rs` -- ReliableModelProvider 骨架 (retry + backoff)
- `openai.rs` -- Arc\<Client\>, Arc\<RwLock\<key\>\>, 发 ChatError

### Phase 2: Key 轮换 + 限流 + fallback_models + Config schema ✅

- `rate_limit.rs` -- TokenBucket
- `reliable.rs` -- KeyRotator trait, advance_key, with_fallback_models
- `provider.rs` -- api_keys: Vec\<String\>, ReliableConfig, 手动 Deserialize
- `migration.rs` -- v1→v2 step (api_key → api_keys)

### Phase 3: Factory wiring ✅

- `lib.rs` -- create_reliable_provider 一站式工厂
- `main.rs` / `shadow-tui` -- 从 ProviderEntry 提取字段构造 Reliable

### Phase 4: 跨 provider fallback ✅

- `router.rs` -- fallback_chains, chat/chat_stream 走 chain
- `provider.rs` -- RouterConfig + RouteEntry
- `schema.rs` -- Config.router: Option\<RouterConfig\>
- `docs/provider-router-design.md` (本文档)

## 7. 不做的事 (YAGNI)

- **AnthropicNative provider** -- 当前只有 OpenAI-compat, 原生协议推到后续.
- **流式 mid-stream 重试** -- stream 建立后不再重试.
- **per-key 限流** -- 默认 per-provider, 够用.
- **请求级 retry budget** -- 不引入全局 token bucket.
- **Circuit breaker** -- Provider 失败过多走 fallback_chain 就够了.
- **Observability hook** -- 不加 Prometheus, trace log 够用.

## 8. 相关文档

- [Session 元信息扩展](./session-design.md) -- SessionStore 侧的 sidecar 设计
- `crates/shadow-providers/src/reliable.rs::tests` -- 完整的重试/退避/key 轮换测试
- `crates/shadow-providers/src/router.rs::tests` -- 完整的 fallback chain 测试
- `crates/shadow-config/tests/config.rs` -- 配置解析与迁移测试

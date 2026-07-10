# Shadow 能力分析: Security 安全

> 对比 ZeroClaw 与 Shadow 的安全设计

## 1. 安全组件清单

| 组件 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| SecurityPolicy | 25+ 字段, 5765行 | 9 字段, 338行 | 严重不足 |
| ApprovalManager | 3模式 + session allowlist + audit + 跨通道路由 | 无 | 全缺 |
| Sandbox trait | 5后端: Landlock/Firejail/Bubblewrap/Docker/SandboxExec | NoopSandbox (直通) | 全缺 |
| scrub_credentials | KV正则 + LeakDetector 8类 + outbound策略 | 注释代码 3正则 | 未激活 |
| LeakDetector | 8类: API keys(7种)/AWS/JWT/PEM/DB URL/bot token/high-entropy | 无 | 全缺 |
| PathGuardedTool | 参数提取 + token-aware shell扫描 + RW/RO/WO三层 | 有源码未注册 | 未接入 |
| RateLimitedTool | max_actions_per_hour + 仅success消耗 + anti-probing | 有源码未注册 | 未接入 |
| PerSenderTracker | per-sender 滑动窗口 + Arc共享(SubAgent继承) | 无 | 全缺 |
| PromptGuard | system override/role confusion/tool injection/jailbreak | InjectionGuard 10正则 | 基础版有 |
| EstopManager | KillAll/NetworkKill/DomainBlock/ToolFreeze | 无 | 全缺 |
| AuditLogger | 安全事件审计 | 无 | 全缺 |
| SecretStore | 加密凭证存储 | 有(secrets.rs) | 已有 |
| Shell命令解析 | quote-aware分割 + subshell拦截 + redirect检测 | 子串匹配 | 严重不足 |
| SubAgent权限检测 | 11种 EscalationViolation | 无 | 全缺 |
| OutboundPolicy | scan/sanitize/frame/scrub_outbound | 无 | 全缺 |

## 2. 命令风险分级对比

| 风险级别 | ZeroClaw 命令 | Shadow |
|----------|--------------|--------|
| Low | git/npm/cargo/ls/cat 等 20+ 安全命令 | 所有命令 (stub 永远 Low) |
| Medium | git push/npm install/cargo publish/mkdir/mv/cp | 无 |
| High | rm/mkfs/dd/sudo/su/chmod/ssh/curl/wget | 无 (仅黑名单子串匹配) |
| 拦截模式 | rm -rf /, :(){:\|:&};:, curl\|sh 等 | 15条黑名单 |

## 3. 命令允许检查对比

| 检查层 | ZeroClaw | Shadow |
|--------|----------|--------|
| subshell 操作符 | 拦截 ` $() <( >( | 无 |
| 重定向检测 | 不安全输出重定向拦截 | 无 |
| tee 命令 | 全拦截 | 无 |
| 后台 & | 单 & 拦截 (保留 &&) | 无 |
| per-segment allowlist | 逐段验证 | 无 (永远 true) |
| 白名单 | 20+ 默认安全命令 | 空 Vec |

## 4. 审批系统对比

| 能力 | ZeroClaw | Shadow |
|------|----------|--------|
| 模式 | Interactive(CLI) / Non-interactive(自动拒绝) / Backchannel(ACP/WS) | 无 |
| session allowlist | "Always" 后本 session 不再提示 | 无 |
| auto_approve | 通配符 * 匹配 | 无 |
| always_ask | 通配符 * 匹配 | 无 |
| ApprovalResponse | Yes/No/Always/ReplaceWith | 无 |
| 跨通道路由 | ApprovalRoute (approver != originator, 120s timeout) | 无 |
| OnNoApprover | Deny (fail-closed) | 无 |
| 审计日志 | ApprovalLogEntry | 无 |
| Shadow 注释代码 | - | requires_approval() 简单跳过, 无 audit |

## 5. 凭证脱敏对比

| 模式 | ZeroClaw | Shadow (注释) |
|------|----------|---------------|
| KV 格式 | token/api_key/password/secret/credential = "xxx" | sk-/Bearer/token= (3正则) |
| API keys | Stripe/OpenAI/Anthropic/Groq/Google/GitHub (6种) | 无 |
| AWS | AKIA + secret_access_key | 无 |
| JWT | eyJ.xxx.xxx | 无 |
| Private keys | PEM blocks | 无 |
| DB URLs | postgres://mysql:// | 无 |
| Bot tokens | xoxb- (Slack) | 无 |
| High-entropy | 24+ chars, Shannon > 4.0 | 无 |
| 替换格式 | 前4字符 + *[REDACTED] | sk-*** |

## 6. 路径安全对比

| 能力 | ZeroClaw | Shadow |
|------|----------|--------|
| 三层 root | RW / RO / WO 分区 | 无 (仅 workspace) |
| canonicalize | symlink 解析 + fallback | 无 |
| forbidden_paths | 18条 (含 ~/.ssh, ~/.gnupg, ~/.aws, ~/.config) | 6条 (无敏感目录) |
| PathGuardedTool | 参数提取 + token-aware shell 扫描 | 有源码未注册 |
| /dev/null 例外 | 允许 | 无 |
| .. traversal | rootless_path 拒绝 | 无 |

## 7. Shadow 差距表

| # | 能力 | 需要做什么 | 优先级 | 为什么 |
|---|------|-----------|--------|--------|
| 1 | 命令风险分级 | 实现 command_risk_level() | P0 | 当前所有命令被视为 Low |
| 2 | 命令白名单 | 实现 default_allowed_commands() | P0 | 无白名单 = 黑名单外的命令都允许 |
| 3 | PathGuardedTool | default_tools 中用 wrapper 包装 | P0 | forbidden_paths 仅注入 prompt 非强制 |
| 4 | scrub_credentials | 激活 + 扩充正则 | P0 | 凭证可能泄露到日志 |
| 5 | forbidden_paths 补充 | 加 ~/.ssh, ~/.gnupg, ~/.aws, ~/.config | P0 | 当前可读 SSH 私钥 |
| 6 | ApprovalManager | 实现 3 模式 + audit | P1 | 注释代码仅 skip 无 audit |
| 7 | RateLimitedTool | 实现滑动窗口 + PerSenderTracker | P1 | 无限流 = 无 DoS 防护 |
| 8 | Sandbox 后端 | 至少 Firejail(Linux) + SandboxExec(macOS) | P1 | NoopSandbox = 无隔离 |
| 9 | Shell 命令解析 | quote-aware 分割 + subshell 检测 | P1 | 子串匹配可被绕过 |
| 10 | LeakDetector | 8类模式检测 | P2 | scrub_credentials 补充 |
| 11 | AuditLogger | 安全事件审计 | P2 | 不可追溯 |
| 12 | EstopManager | KillAll + ToolFreeze | P2 | 失控时无法快速冻结 |
| 13 | OutboundPolicy | 出站内容扫描 | P2 | 输出可能含注入/凭证 |
| 14 | SubAgent 权限检测 | 11种 EscalationViolation | P3 | 有 SubAgent 时需要 |
| 15 | SecretStore | 已有 secrets.rs | 已有 | 无需修改 |

## Shadow 已有优势
- InjectionGuard (10正则 + 不可见 Unicode 检测)
- SafetyInjectionSection (prompt 安全注入)
- AutonomyLevel (Full/Supervised/ReadOnly)
- LoopDetector (完整实现, 独立文件)

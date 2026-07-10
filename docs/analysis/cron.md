# Shadow 能力分析: Cron 定时调度

> ZeroClaw ~10,337行 vs Shadow ~944行 (9%完成度)

## 1. 架构对比

| 组件 | ZeroClaw (文件/行数) | Shadow (文件/行数) | 差距 |
|------|---------------------|-------------------|------|
| 模块入口 | cron/mod.rs (879行) | cron/mod.rs (147行) | stub |
| 类型定义 | cron/types.rs (280行) | cron/types.rs (7行) | 极简 |
| 调度计算 | cron/schedule.rs (519行) | cron/schedule.rs (187行) | 基本完整,缺weekday翻译 |
| 存储层 | cron/store.rs (2863行) | 无 | 全缺 |
| 调度器 | cron/scheduler.rs (2765行) | 无 | 全缺 |
| 工具-add | tools/cron_add.rs (1373行) | tools/cron/add.rs (460行) | 底层stub |
| 工具-update | tools/cron_update.rs (871行) | 不存在 | 全缺 |
| 工具-list | tools/cron_list.rs (169行) | 空文件 | 全缺 |
| 工具-remove | tools/cron_remove.rs (370行) | 空文件 | 全缺 |
| 工具-run | tools/cron_run.rs (426行) | 空文件 | 全缺 |
| 工具-runs | tools/cron_runs.rs (200行) | 空文件 | 全缺 |
| 工具-common | tools/cron_common.rs (123行) | tools/cron/common.rs (97行) | 基本对齐 |
| 工具-types | - | tools/cron/types.rs (30行) | Shadow独有 |
| 总计 | ~10,337行 | ~944行 | 9% |

## 2. CronJob 结构体字段对比

| 字段 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| id | String (UUID) | String | OK |
| name | Option<String> | String (非Option) | 类型不一致 |
| expression | String | String | OK |
| schedule | Schedule | Schedule | OK |
| job_type | JobType | JobType | OK |
| enabled | bool | bool | OK |
| next_run | DateTime<Utc> | DateTime<Utc> | OK |
| command | String | 缺 | 缺(shell job需要) |
| prompt | Option<String> | 缺 | 缺(agent job需要) |
| session_target | SessionTarget | 缺 | 缺(agent隔离需要) |
| model | Option<String> | 缺 | 缺(模型覆盖) |
| agent_alias | String | 缺 | 缺(归属agent) |
| delivery | DeliveryConfig | 缺 | 缺(投递配置) |
| delete_after_run | bool | 缺 | 缺(一次性任务) |
| allowed_tools | Option<Vec<String>> | 缺 | 缺(工具白名单) |
| uses_memory | bool | 缺 | 缺(记忆注入开关) |
| source | String | 缺 | 缺(来源标记) |
| created_at | DateTime<Utc> | 缺 | 缺(持久化需要) |
| last_run | Option<DateTime<Utc>> | 缺 | 缺(状态追踪) |
| last_status | Option<String> | 缺 | 缺(状态追踪) |
| last_output | Option<String> | 缺 | 缺(结果追踪) |

## 3. CronRun 结构体字段对比

| 字段 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| id | i64 | i64 | OK |
| job_id | String | i64 | 类型不一致 |
| started_at | DateTime<Utc> | i64 (Unix) | 类型不一致 |
| finished_at | DateTime<Utc> | Option<i64> | 类型不一致 |
| status | String | String | OK |
| output | Option<String> | String | 类型不一致 |
| duration_ms | Option<i64> | 缺 | 缺 |

## 4. DeliveryConfig 对比

| 字段 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| mode | String ("none"/"announce") | String | OK |
| channel | Option<String> | Option<String> | OK |
| to | Option<String> | Option<String> | OK |
| thread_id | Option<String> | 缺 | 缺(webhook投递需要) |
| best_effort | bool (默认true) | 缺 | 缺(失败行为控制) |

## 5. SQLite 持久化对比

| 组件 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| cron_jobs 表 | 完整 (21列+4索引) | 缺 | 全缺 |
| cron_runs 表 | 完整 (7列+3索引) | 缺 | 全缺 |
| schema 初始化 | initialize_schema() | 缺 | 全缺 |
| 列迁移 | add_column_if_missing() | 缺 | 全缺 |
| 连接管理 | with_initialized/read_connection | 缺 | 全缺 |
| DB 路径 | data_dir/cron/jobs.db | 缺 | 全缺 |

## 6. 调度器对比

| 方法 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| run() 主循环 | poll->due->claim->并发执行->persist | 缺 | 全缺 |
| execute_job_now | agent解析+安全+retry | 缺 | 全缺 |
| execute_job_with_retry | retry+backoff+jitter | 缺 | 全缺 |
| run_agent_job | SubAgent spawn+记忆注入+agent loop | 缺 | 全缺 |
| run_job_command | tokio::process+超时+安全 | 缺 | 全缺 |
| run_manual_job | 执行+投递+持久化+广播 | 缺 | 全缺 |
| catch_up_overdue_jobs | 启动全量执行 | 缺 | 全缺 |
| skip_missed_jobs | 启动推进next_run | 缺 | 全缺 |
| claim_job | UPDATE原子锁 | 缺 | 全缺 |
| release_job | 释放锁 | 缺 | 全缺 |
| persist_job_result | reschedule/disable/delete | 缺 | 全缺 |
| deliver_if_configured | announce投递+NO_REPLY抑制 | 缺 | 全缺 |

## 7. Agent 集成对比

| 步骤 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| 解析owning agent | agent_alias->config反查 | 缺 | 全缺 |
| 安全策略 | SecurityPolicy::for_agent | 缺 | 全缺 |
| SubAgent spawn | 继承身份+权限 | 缺 | 全缺 |
| 记忆注入 | recall(prompt,5)+过滤Conversation | 缺 | 全缺 |
| Prompt构造 | format!("[cron:{id}] {prompt}") | 缺 | 全缺 |
| Session隔离 | Isolated(cron-{uuid})/Main | 缺 | 全缺 |
| 工具排除 | 默认排除scheduler工具 | 缺 | 全缺 |
| 失败记忆清理 | purge_session | 缺 | 全缺 |
| Retry | 指数退避+抖动 | 缺 | 全缺 |

## 8. schedule.rs 对比

| 方法 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| validate_schedule | 完整 | 完整 | OK |
| next_run_for_schedule | 完整 | 完整 | OK |
| normalize_expression | 5字段补秒+weekday翻译 | 5字段仅补秒 | 缺weekday翻译 |
| normalize_weekday_field | 单值/范围/列表/步进/Sunday别名 | 缺 | 全缺 |
| validate_delivery_config | 完整 | 完整 | OK |
| schedule_cron_expression | 完整 | 完整 | OK |

## 9. 工具对比

| 工具 | ZeroClaw | Shadow | 需要做什么 |
|------|----------|--------|-----------|
| cron_add | 完整(1373行) | 有(460行)但底层stub | 修复stub(add_shell_job/add_agent_job) |
| cron_update | 完整(871行) | 不存在 | 创建CronUpdateTool |
| cron_list | 完整(169行) | 空文件 | 创建CronListTool |
| cron_remove | 完整(370行) | 空文件 | 创建CronRemoveTool |
| cron_run | 完整(426行) | 空文件 | 创建CronRunTool |
| cron_runs | 完整(200行) | 空文件 | 创建CronRunsTool |

## 10. 优先级

| 优先级 | 任务 | 为什么 |
|--------|------|--------|
| P0 | 补齐CronJob 14个缺失字段 | 当前结构体无法持久化完整job |
| P0 | 实现 store.rs SQLite CRUD | 所有工具和调度器的基础 |
| P0 | 实现 cron_list/remove/run/runs 工具 | 基本CRUD能力 |
| P0 | 创建 cron_update 工具 | job修改能力 |
| P0 | 修复 add.rs 底层 stub | cron_add 工具当前无法真正创建job |
| P1 | 实现调度器 run() 主循环 | 定时执行能力 |
| P1 | 添加 weekday 翻译逻辑 | cron表达式兼容性 |
| P2 | 实现 run_agent_job | agent cron job执行 |
| P2 | 实现投递系统 | announce模式投递 |
| P2 | 实现 catch_up/skip_missed | 停机恢复 |

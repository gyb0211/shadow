# Shadow 能力分析: Channel 渠道

> 对比 ZeroClaw 与 Shadow 的 channel 实现

## 1. Channel Trait 对比

| 项目 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| trait bound | Send + Sync + Attributable | Attributable (缺 Send+Sync) | 需补 |
| 方法数 | 30 | 3 | 缺 27 |
| name() | 必填 | 必填 | 无 |
| send() | 必填 | 必填 | 无 |
| listen() | 必填 (mpsc::Sender) | 无 | 缺 |
| health_check() | 默认 true | 无 | 缺 |
| start/stop_typing() | 默认 Ok(()) | 无 | 缺 |
| draft 系列 (6个) | send/update/update_progress/finalize/cancel | 无 | 缺 |
| reaction 系列 (4个) | add/remove/pin/unpin | 无 | 缺 |
| room 管理 (2个) | create_room/invite_user | 无 | 缺 |
| 审批 (2个) | request_approval/request_choice | supports_approval() -> bool | 严重简化 |
| self_handle/drop_self_messages | 有 | 无 | 缺 |
| is_direct_message | 有 | 无 | 缺 |

## 2. ChannelMessage 对比

| 字段 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| id | String | String | 无 |
| sender | String | String | 无 |
| content | String | String | 无 |
| reply_target | Option<String> | reply_to: Option<String> | 命名不同 |
| channel | String | 无 | 缺 |
| channel_alias | String | 无 | 缺 |
| timestamp | String | 无 | 缺 |
| thread_ts | Option<String> | 无 | 缺 |
| attachments | Vec<MediaAttachment> | 无 | 缺 |
| subject | Option<String> | 无 | 缺 |
| passive_context | bool | 无 | 缺 |
| conversation_scope | enum | 无 | 缺 |
| interruption_scope_id | Option<String> | 无 | 缺 |

## 3. SendMessage 对比

| 字段 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| content | String | String | 无 |
| recipient | String | String | 无 |
| thread_ts | Option<String> | 无 | 缺 |
| cancellation_token | Option<CancellationToken> | 无 | 缺 |
| attachments | Vec<MediaAttachment> | 无 | 缺 |
| in_reply_to | Option<String> | 无 | 缺 |
| subject | Option<String> | 无 | 缺 |
| suppress_voice | bool | 无 | 缺 |

## 4. ZeroClaw 渠道实现 (~42个)

| 类别 | 渠道 |
|------|------|
| 始终编译 | CliChannel, LinkEnricher, Transcription, Tts, Voice |
| IM/聊天 | Telegram, Discord, Slack, Mattermost, Matrix, IRC, Signal, Line, QQ, Lark, DingTalk, WeCom, WeComWs, WeChat, MoChat, NextcloudTalk, ClawdTalk, Notion |
| 邮件 | EmailChannel, GmailPush |
| 社交 | Bluesky, Nostr, Twitch, Twitter, Reddit |
| WhatsApp | WhatsAppCloud, WhatsAppWeb |
| 消息队列 | Amqp, Mqtt |
| Web/API | Webhook, AcpChannel, Wati, Linq, IMessage, Filesystem |
| 语音 | VoiceCall, VoiceWake |

## 5. ZeroClaw 消息流转

```
1. INBOUND: Channel.listen() -> mpsc bus
2. ORCHESTRATOR: self-loop guard + 会话路由 + ack reaction
3. PRE-PROCESSING: thinking解析 + media pipeline + link enricher + runtime命令
4. AGENT TURN: 构建 history + memory recall + system prompt -> run_tool_call_loop
5. OUTBOUND: sanitize + send/finalize_draft + ack + stop_typing
6. POST: memory consolidation + session save
```

## 6. ZeroClaw 媒体处理

| 类型 | 处理 | 输出格式 |
|------|------|----------|
| Audio | TranscriptionManager 转录 | [Audio transcription: <text>] |
| Image | vision模型 -> base64内联 | [IMAGE:data:mime;base64,...] |
| Video | 占位标注 | [Video: <file> attached] |
| WebP | ->PNG 转换 | image/png base64 |

## 7. Shadow 差距表

| # | 能力 | 需要做什么 | 优先级 | 为什么 |
|---|------|-----------|--------|--------|
| 1 | trait bound | 补 Send + Sync | P0 | Arc<dyn Channel> 跨 task 共享需要 |
| 2 | listen() | 添加 listen 方法 | P0 | 渠道核心职责: 接收入站 |
| 3 | ChannelMessage 字段 | 补 reply_target/timestamp/channel/thread_ts | P0 | 会话路由和线程回复需要 |
| 4 | CliChannel | 实现 shadow-channels crate | P0 | 最小可用渠道: stdin/stdout |
| 5 | typing 指示器 | start/stop_typing | P1 | 用户体验, LLM 耗时数十秒 |
| 6 | self_handle | 防自循环 | P1 | 多 agent 环境必须 |
| 7 | draft 系列 | send_draft/update/finalize/cancel | P1 | 流式输出, 支持消息编辑 |
| 8 | reaction | add/remove/pin/unpin/redact | P1 | ack 反馈机制 |
| 9 | 审批系统 | ChannelApprovalRequest/Response + request_approval | P2 | Supervised 模式工具审批 |
| 10 | MediaAttachment | 类型 + ChannelMessage.attachments | P2 | 多模态交互 |
| 11 | MediaPipeline | 音频转录/图片描述/视频摘要 | P2 | 媒体预处理 |
| 12 | Orchestrator | start_channels + process_message | P3 | 渠道子系统核心 (最大工作量) |
| 13 | ChannelKind 枚举 | 补充 Cli/Webhook/Telegram 等 | P2 | 归因分类需要 |

# shadow-providers 差距分析 -- 对照 ZeroClaw

## 当前状态 (Shadow)
- OpenAI 兼容 provider (1 个实现)
- 支持 OpenAI/OpenRouter/Ollama/DeepSeek 等 (同 base_url)
- function calling 支持
- 工厂函数 create_provider
- 共 ~320 行

## ZeroClaw 对应 (zeroclaw-providers: 49664行, 34文件)
- 72 种 ModelProviderKind
- ReliableModelProvider: 重试 + 指数退避 + API key 轮换 + fallback 通知
- CompatFamilySpec blanket impl: 声明 3 常量即可注册
- for_each_model_provider_slot! 宏: 单一真相源
- 中国厂商 OAuth (Qwen/MiniMax token refresh)
- ProviderDispatch: 归因 span 自动包裹
- 流式 SSE 解析
- Anthropic 原生 tool_use 格式
- Gemini 原生格式
- Bedrock (AWS)
- 错误分类: 可重试 vs 不可重试

## 缺失项
| 功能 | 严重度 | ZeroClaw 实现 | Shadow 状态 |
|------|--------|--------------|-------------|
| Anthropic 原生 | P1 | tool_use 格式 | 缺失 |
| 重试/退避 | P1 | ReliableModelProvider | 缺失 |
| API key 轮换 | P1 | 多 key fallback | 缺失 |
| 流式 SSE | P0 | stream_chat | 缺失 |
| OAuth | P2 | Qwen/MiniMax | 缺失 |
| Bedrock | P2 | AWS | 缺失 |
| Gemini 原生 | P2 | 原生格式 | 缺失 |
| 错误分类 | P1 | 可重试/不可重试 | 缺失 |
| 多 provider 工厂 | P1 | 72 种注册 | 1 种 |

## 开发建议
1. P0: 流式 SSE (stream_chat -> BoxStream)
2. P1: Anthropic provider (原生 tool_use)
3. P1: ReliableModelProvider (重试+退避+key轮换)
4. P1: 错误分类 (可重试 vs 不可重试)
5. P2: Gemini 原生
6. P2: OAuth 流程

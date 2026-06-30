# shadow-config 差距分析 -- 对照 ZeroClaw

## 当前状态 (Shadow)
- TOML 配置 + 多 provider (flatten HashMap)
- ProviderEntry: api_key/model/base_url/temperature/max_tokens/fallback_models
- ProviderRef: "family.alias" 解析
- AgentSection: alias/model/temperature/autonomy/max_iterations/max_history/system_prompt
- config_set CLI 实现
- 共 ~650 行

## ZeroClaw 对应 (zeroclaw-config: 65171行, 36文件)
- ChaCha20-Poly1305 AEAD 密钥加密 (SecretStore)
- 版本迁移 V1->V2->V3 纯函数链
- SecurityPolicy (5765行): 安全策略定义
- CostTracker: 成本追踪
- env_overrides: ZEROCLAW_* 环境变量覆盖
- Configurable derive 宏 (2917行): 自动生成密钥管理代码
- field_visibility: 字段可见性控制
- presets: 配置预设
- pairing: 设备配对
- domain_matcher: 域名匹配
- multi_agent: 多代理配置

## 缺失项
| 功能 | 严重度 | ZeroClaw 实现 | Shadow 状态 |
|------|--------|--------------|-------------|
| 密钥加密 | P1 | ChaCha20-Poly1305, enc2: 前缀 | 明文存储 |
| 版本迁移 | P1 | V1->V2->V3 纯函数 | 缺失 |
| SecurityPolicy | P1 | 命令白名单/risk gate | 缺失 |
| CostTracker | P2 | 成本追踪+预算 | 缺失 |
| env 覆盖 | P1 | SHADOW_* 环境变量 | 缺失 |
| 配置预设 | P2 | presets.rs | 缺失 |
| Configurable 宏 | P2 | derive 宏自动生成 | 手动 impl |
| 设备配对 | P2 | pairing.rs | 缺失 |

## 开发建议
1. P1: 密钥加密 (AES 或 ChaCha20-Poly1305)
2. P1: env 覆盖 (SHADOW_* 环境变量)
3. P1: SecurityPolicy 基础 (命令白名单)
4. P1: 版本迁移框架
5. P2: CostTracker
6. P2: 配置预设

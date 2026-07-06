//! Agent ↔ MemoryStrategy 集成测试
//!
//! 验证:
//! 1. before_chat 注入的 memory_context 出现在传给 provider 的 system 消息中
//! 2. after_chat 在对话后被调用, 记忆被存储 (可被下一轮 recall)

use anyhow::Result;
use async_trait::async_trait;
use shadow_core::{Attributable, ChatRequest, ChatResponse, Memory, ModelProvider, Role, TokenUsage};
use shadow_memory::DefaultMemoryStrategy;
use shadow_memory::sqlite::SqliteMemory;
use shadow_runtime::agent::{Agent, StreamDelta};
use std::sync::{Arc, Mutex};

/// 捕获请求 + 返回预设响应的 mock provider
struct CapturingProvider {
    last_system: Arc<Mutex<String>>,
    reply: String,
}

impl Attributable for CapturingProvider {
    fn role(&self) -> Role {
        Role::Provider
    }
    fn alias(&self) -> &str {
        "mock"
    }
}

#[async_trait]
impl ModelProvider for CapturingProvider {
    fn provider_type(&self) -> &str {
        "mock"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        // 捕获 system 消息内容
        if let Some(sys) = request.messages.first() {
            if sys.role == "system" {
                *self.last_system.lock().unwrap() = sys.content.clone();
            }
        }
        Ok(ChatResponse {
            content: self.reply.clone(),
            tool_calls: vec![],
            usage: TokenUsage::default(),
            reasoning_content: None,
        })
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        Ok(vec!["mock-model".to_string()])
    }
}

#[tokio::test]
async fn before_chat_injects_memory_into_system_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let mem = Arc::new(SqliteMemory::new(dir.path()).unwrap());

    // 预置一条记忆, recall "Rust" 时应命中
    mem.store(
        "rust",
        "Rust 是一门系统编程语言",
        shadow_core::MemoryCategory::Core,
        None,
    )
    .await
    .unwrap();

    let captured_system: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let provider = Arc::new(CapturingProvider {
        last_system: captured_system.clone(),
        reply: "Rust 是一门系统编程语言".to_string(),
    });

    let strategy = Arc::new(DefaultMemoryStrategy::new(mem.clone()));
    let agent = Agent::builder()
        .alias("test")
        .provider(provider)
        .memory(mem.clone())
        .memory_strategy(strategy)
        .build()
        .unwrap();

    let _ = agent.chat("Rust").await.unwrap();

    let sys = captured_system.lock().unwrap().clone();
    assert!(
        sys.contains("[memory_context]"),
        "system prompt 应包含 memory_context, 实际: {sys}"
    );
    assert!(
        sys.contains("Rust 是一门系统编程语言"),
        "system prompt 应包含召回的记忆内容, 实际: {sys}"
    );
}

#[tokio::test]
async fn after_chat_stores_turn_memory() {
    let dir = tempfile::tempdir().unwrap();
    let mem = Arc::new(SqliteMemory::new(dir.path()).unwrap());

    let provider = Arc::new(CapturingProvider {
        last_system: Arc::new(Mutex::new(String::new())),
        reply: "Rust 是一门系统编程语言, 由 Mozilla 开发".to_string(),
    });

    let strategy = Arc::new(DefaultMemoryStrategy::new(mem.clone()));
    let agent = Agent::builder()
        .alias("test")
        .provider(provider)
        .memory(mem.clone())
        .memory_strategy(strategy)
        .build()
        .unwrap();

    // 对话前: 记忆为空
    let before = mem.list(None).await.unwrap();
    assert!(before.is_empty());

    let _ = agent.chat("什么是 Rust?").await.unwrap();

    // 对话后: 应存了一条 Conversation 记忆
    let after = mem.list(None).await.unwrap();
    assert_eq!(after.len(), 1, "after_chat 应存储 1 条记忆");
    assert_eq!(after[0].category, shadow_core::MemoryCategory::Conversation);
    assert!(after[0].content.contains("什么是 Rust?"));
}

#[tokio::test]
async fn no_strategy_no_injection_no_storage() {
    // 不设置 memory_strategy -- 行为应与之前完全一致
    let dir = tempfile::tempdir().unwrap();
    let mem = Arc::new(SqliteMemory::new(dir.path()).unwrap());

    let captured_system: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let provider = Arc::new(CapturingProvider {
        last_system: captured_system.clone(),
        reply: "短的回复".to_string(),
    });

    let agent = Agent::builder()
        .alias("test")
        .provider(provider)
        .memory(mem.clone())
        // 不设置 memory_strategy
        .build()
        .unwrap();

    let _ = agent.chat("hello").await.unwrap();

    let sys = captured_system.lock().unwrap().clone();
    assert!(
        !sys.contains("[memory_context]"),
        "无 strategy 时不应注入, 实际: {sys}"
    );

    let stored = mem.list(None).await.unwrap();
    assert!(stored.is_empty(), "无 strategy 时不应存储");
}

// 占位: 显式引用 StreamDelta 防止未使用 import 警告 (当以上测试改动时)
#[allow(dead_code)]
fn _stream_delta_unused(_d: StreamDelta) {}

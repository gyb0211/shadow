//! 任务路由器 -- 按名称或能力将任务路由到正确的 transport

use std::collections::HashMap;
use std::sync::Arc;
use anyhow::Result;
use parking_lot::RwLock;
use tokio::sync::Mutex;

use crate::registry::AgentRegistry;
use crate::task::{Task, TaskStatus};
use crate::transport::AgentTransport;

/// 任务路由器 -- 持有所有 transport 实例, 负责路由任务
pub struct TaskRouter {
    /// agent 注册表 (元数据)
    pub registry: Arc<AgentRegistry>,
    /// transport 实例 (按 agent 名称)
    transports: RwLock<HashMap<String, Arc<dyn AgentTransport>>>,
    /// 任务存储 (内存, 简化版)
    tasks: Mutex<HashMap<String, Task>>,
}

impl TaskRouter {
    pub fn new(registry: Arc<AgentRegistry>) -> Self {
        Self {
            registry,
            transports: RwLock::new(HashMap::new()),
            tasks: Mutex::new(HashMap::new()),
        }
    }

    /// 注册 transport
    pub fn register_transport(&self, name: &str, transport: Arc<dyn AgentTransport>) {
        self.transports.write().insert(name.to_string(), transport);
    }

    /// 派发任务 (同步等待结果)
    pub async fn dispatch(&self, from: &str, to: &str, prompt: &str) -> Result<Task> {
        let mut task = Task::new(from, to, prompt);

        // 查找 transport
        let transport = {
            let transports = self.transports.read();
            transports.get(to).cloned()
        };

        let Some(transport) = transport else {
            task.fail(format!("agent '{to}' 未注册"));
            self.tasks.lock().await.insert(task.id.clone(), task.clone());
            return Ok(task);
        };

        task.start();
        self.tasks.lock().await.insert(task.id.clone(), task.clone());

        // 执行
        match transport.chat(prompt).await {
            Ok(result) => {
                task.complete(result);
            }
            Err(e) => {
                task.fail(format!("{e:#}"));
            }
        }

        self.tasks.lock().await.insert(task.id.clone(), task.clone());
        Ok(task)
    }

    /// 按能力派发 (选第一个匹配的 agent)
    pub async fn dispatch_by_capability(
        &self,
        from: &str,
        capability: &str,
        prompt: &str,
    ) -> Result<Task> {
        let candidates = self.registry.find_by_capability(capability);
        if candidates.is_empty() {
            let mut task = Task::new(from, "unknown", prompt);
            task.fail(format!("没有 agent 具备能力 '{capability}'"));
            return Ok(task);
        }

        // 选第一个有 transport 的 (先收集名称, 释放锁, 再 dispatch)
        let target_name = {
            let transports = self.transports.read();
            candidates
                .iter()
                .find(|card| transports.contains_key(&card.name))
                .map(|card| card.name.clone())
        };

        if let Some(name) = target_name {
            return self.dispatch(from, &name, prompt).await;
        }

        let mut task = Task::new(from, &candidates[0].name, prompt);
        task.fail("找到 agent 元数据但无 transport 实例".into());
        Ok(task)
    }

    /// 查询任务状态
    pub async fn get_task(&self, task_id: &str) -> Option<Task> {
        self.tasks.lock().await.get(task_id).cloned()
    }

    /// 列出任务
    pub async fn list_tasks(&self, status_filter: Option<TaskStatus>) -> Vec<Task> {
        let tasks = self.tasks.lock().await;
        tasks
            .values()
            .filter(|t| {
                if let Some(ref status) = status_filter {
                    &t.status == status
                } else {
                    true
                }
            })
            .cloned()
            .collect()
    }

    /// 取消任务 (简化版: 只标记状态)
    pub async fn cancel_task(&self, task_id: &str) -> Option<Task> {
        let mut tasks = self.tasks.lock().await;
        if let Some(task) = tasks.get_mut(task_id) {
            if !task.is_terminal() {
                task.cancel();
            }
            return Some(task.clone());
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::LocalAgent;
    use shadow_core::{Attributable, ChatResponse, ChatRequest, ModelProvider, Role, TokenUsage};

    struct MockProvider;

    impl Attributable for MockProvider {
        fn role(&self) -> Role { Role::Agent }
        fn alias(&self) -> &str { "mock" }
    }

    #[async_trait::async_trait]
    impl ModelProvider for MockProvider {
        fn provider_type(&self) -> &str { "mock" }
        async fn chat(&self, _: ChatRequest) -> Result<ChatResponse> {
            Ok(ChatResponse {
                content: "hello from mock".into(),
                reasoning_content: None,
                tool_calls: vec![],
                usage: TokenUsage::default(),
            })
        }
        // chat_stream 使用默认实现
        async fn list_models(&self) -> Result<Vec<String>> {
            Ok(vec!["mock-model".into()])
        }
    }

    #[tokio::test]
    async fn dispatch_to_local_agent() {
        let registry = Arc::new(AgentRegistry::new());
        let router = TaskRouter::new(Arc::clone(&registry));

        let provider: Arc<dyn ModelProvider> = Arc::new(MockProvider);
        let agent = LocalAgent::new("worker", vec!["coding".into()], provider, "model".into());

        registry.register(agent.card().clone()).unwrap();
        router.register_transport("worker", Arc::new(agent));

        let task = router.dispatch("main", "worker", "do something").await.unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
        assert_eq!(task.result.as_deref(), Some("hello from mock"));
    }

    #[tokio::test]
    async fn dispatch_to_unregistered() {
        let registry = Arc::new(AgentRegistry::new());
        let router = TaskRouter::new(registry);

        let task = router.dispatch("main", "missing", "hello").await.unwrap();
        assert_eq!(task.status, TaskStatus::Failed);
        assert!(task.error.as_deref().unwrap().contains("未注册"));
    }

    #[tokio::test]
    async fn dispatch_by_capability() {
        let registry = Arc::new(AgentRegistry::new());
        let router = TaskRouter::new(Arc::clone(&registry));

        let provider: Arc<dyn ModelProvider> = Arc::new(MockProvider);
        let agent = LocalAgent::new("coder", vec!["coding".into()], provider, "model".into());

        registry.register(agent.card().clone()).unwrap();
        router.register_transport("coder", Arc::new(agent));

        let task = router.dispatch_by_capability("main", "coding", "write code").await.unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
    }

    #[tokio::test]
    async fn dispatch_by_capability_no_match() {
        let registry = Arc::new(AgentRegistry::new());
        let router = TaskRouter::new(registry);

        let task = router.dispatch_by_capability("main", "nonexistent", "hello").await.unwrap();
        assert_eq!(task.status, TaskStatus::Failed);
    }

    #[tokio::test]
    async fn get_and_cancel_task() {
        let registry = Arc::new(AgentRegistry::new());
        let router = TaskRouter::new(registry);

        let task = router.dispatch("main", "missing", "hello").await.unwrap();
        let id = task.id.clone();

        let retrieved = router.get_task(&id).await;
        assert!(retrieved.is_some());

        let cancelled = router.cancel_task(&id).await;
        // 任务已结束 (Failed), 不能取消
        assert!(cancelled.is_some());
        assert_eq!(cancelled.unwrap().status, TaskStatus::Failed);
    }
}

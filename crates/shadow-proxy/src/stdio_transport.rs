//! stdio 传输 -- JSON-RPC 2.0 over stdin/stdout
//!
//! 用于 CLI/IDE 集成, 像 ACP/MCP 一样作为子进程被调用.
//! 其他 agent 可以通过 spawn shadow proxy --stdio 来与 proxy 通信.
//!
//! JSON-RPC 方法:
//!   agents.register   -- 注册 agent
//!   agents.list       -- 列出所有 agent
//!   agents.get        -- 查看指定 agent
//!   agents.deregister -- 注销 agent
//!   tasks.create      -- 派发任务
//!   tasks.get         -- 查询任务
//!   tasks.list        -- 列出任务
//!   tasks.cancel      -- 取消任务
//!   health            -- 健康检查
//!   catalog           -- A2A 发现卡片

use anyhow::Result;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::core::ProxyCore;

/// stdio JSON-RPC server -- 从 stdin 读请求, 往 stdout 写响应
pub struct StdioTransport {
    core: ProxyCore,
}

impl StdioTransport {
    pub fn new(core: ProxyCore) -> Self {
        Self { core }
    }

    /// 启动 stdio 循环: 读一行 JSON-RPC → 处理 → 写一行响应
    pub async fn serve(&self) -> Result<()> {
        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        shadow_log::record!(
            INFO,
            shadow_log::Action::Start,
            "Proxy stdio server 启动 (JSON-RPC over stdin/stdout)"
        );

        while let Some(line) = lines.next_line().await? {
            // 空行跳过
            if line.trim().is_empty() {
                continue;
            }

            // 解析 JSON-RPC 请求
            let resp = match serde_json::from_str::<Value>(&line) {
                Ok(req) => self.handle_request(&req).await,
                Err(e) => json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": format!("解析失败: {e}") }
                }),
            };

            // 写响应 (一行 JSON)
            let mut out = serde_json::to_string(&resp)?;
            out.push('\n');
            stdout.write_all(out.as_bytes()).await?;
            stdout.flush().await?;
        }

        Ok(())
    }

    /// 处理单个 JSON-RPC 请求
    async fn handle_request(&self, req: &Value) -> Value {
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(Value::Null);

        let result = match method {
            "agents.register" => self.register_agent(&params),
            "agents.list" => Ok(self.core.list_agents()),
            "agents.get" => self.get_agent(&params),
            "agents.deregister" => self.deregister_agent(&params),
            "tasks.create" => self.create_task(&params).await,
            "tasks.get" => self.get_task(&params).await,
            "tasks.list" => self.list_tasks(&params).await,
            "tasks.cancel" => self.cancel_task(&params).await,
            "health" => Ok(self.core.health()),
            "catalog" => Ok(self.core.catalog()),
            _ => Err(format!("未知方法: {method}")),
        };

        match result {
            Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
            Err(msg) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32000, "message": msg }
            }),
        }
    }

    // ── 方法实现 ──

    fn register_agent(&self, params: &Value) -> Result<Value, String> {
        let card: AgentCard = serde_json::from_value(params.clone())
            .map_err(|e| format!("解析 AgentCard 失败: {e}"))?;
        self.core.register_agent(card).map_err(|e| e.to_string())
    }

    fn get_agent(&self, params: &Value) -> Result<Value, String> {
        let name = params.get("name").and_then(|v| v.as_str())
            .ok_or("缺少 name 参数")?;
        self.core.get_agent(name)
            .ok_or_else(|| format!("agent '{name}' 不存在"))
    }

    fn deregister_agent(&self, params: &Value) -> Result<Value, String> {
        let name = params.get("name").and_then(|v| v.as_str())
            .ok_or("缺少 name 参数")?;
        if self.core.deregister_agent(name) {
            Ok(json!({"deregistered": true, "name": name}))
        } else {
            Err(format!("agent '{name}' 不存在"))
        }
    }

    async fn create_task(&self, params: &Value) -> Result<Value, String> {
        let to = params.get("to").and_then(|v| v.as_str())
            .ok_or("缺少 to 参数")?;
        let prompt = params.get("prompt").and_then(|v| v.as_str())
            .ok_or("缺少 prompt 参数")?;
        let from = params.get("from").and_then(|v| v.as_str()).unwrap_or("stdio");
        let capability = params.get("capability").and_then(|v| v.as_str());

        self.core.create_task(from, to, prompt, capability)
            .await
            .map_err(|e| e.to_string())
    }

    async fn get_task(&self, params: &Value) -> Result<Value, String> {
        let task_id = params.get("task_id").and_then(|v| v.as_str())
            .ok_or("缺少 task_id 参数")?;
        self.core.get_task(task_id)
            .await
            .ok_or_else(|| format!("任务 '{task_id}' 不存在"))
    }

    async fn list_tasks(&self, params: &Value) -> Result<Value, String> {
        let status = params.get("status").and_then(|v| v.as_str()).and_then(|s| match s {
            "pending" => Some(TaskStatus::Pending),
            "running" => Some(TaskStatus::Running),
            "completed" => Some(TaskStatus::Completed),
            "failed" => Some(TaskStatus::Failed),
            "cancelled" => Some(TaskStatus::Cancelled),
            _ => None,
        });
        Ok(self.core.list_tasks(status).await)
    }

    async fn cancel_task(&self, params: &Value) -> Result<Value, String> {
        let task_id = params.get("task_id").and_then(|v| v.as_str())
            .ok_or("缺少 task_id 参数")?;
        self.core.cancel_task(task_id)
            .await
            .ok_or_else(|| format!("任务 '{task_id}' 不存在"))
    }
}

use crate::card::AgentCard;
use crate::task::TaskStatus;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::{AgentRegistry, TaskRouter, AgentCard};

    #[tokio::test]
    async fn stdio_handle_health() {
        let registry = Arc::new(AgentRegistry::new());
        let router = Arc::new(TaskRouter::new(Arc::clone(&registry)));
        let core = ProxyCore::new(router);
        let transport = StdioTransport::new(core);

        let req = json!({"jsonrpc": "2.0", "id": 1, "method": "health"});
        let resp = transport.handle_request(&req).await;
        assert_eq!(resp["result"]["status"], "ok");
        assert_eq!(resp["id"], 1);
    }

    #[tokio::test]
    async fn stdio_handle_list_agents() {
        let registry = Arc::new(AgentRegistry::new());
        registry.register(AgentCard::local("test", vec!["coding".into()])).unwrap();
        let router = Arc::new(TaskRouter::new(Arc::clone(&registry)));
        let core = ProxyCore::new(router);
        let transport = StdioTransport::new(core);

        let req = json!({"jsonrpc": "2.0", "id": 2, "method": "agents.list"});
        let resp = transport.handle_request(&req).await;
        assert!(resp["result"].is_array());
        assert_eq!(resp["result"][0]["name"], "test");
    }

    #[tokio::test]
    async fn stdio_unknown_method() {
        let registry = Arc::new(AgentRegistry::new());
        let router = Arc::new(TaskRouter::new(registry));
        let core = ProxyCore::new(router);
        let transport = StdioTransport::new(core);

        let req = json!({"jsonrpc": "2.0", "id": 3, "method": "nonexistent"});
        let resp = transport.handle_request(&req).await;
        assert!(resp.get("error").is_some());
        assert_eq!(resp["error"]["code"], -32000);
    }

    #[tokio::test]
    async fn stdio_register_and_get() {
        let registry = Arc::new(AgentRegistry::new());
        let router = Arc::new(TaskRouter::new(Arc::clone(&registry)));
        let core = ProxyCore::new(router);
        let transport = StdioTransport::new(core);

        // 注册
        let card = AgentCard::local("worker", vec!["coding".into()]);
        let req = json!({"jsonrpc": "2.0", "id": 1, "method": "agents.register", "params": card});
        let resp = transport.handle_request(&req).await;
        assert_eq!(resp["result"]["registered"], true);

        // 查询
        let req = json!({"jsonrpc": "2.0", "id": 2, "method": "agents.get", "params": {"name": "worker"}});
        let resp = transport.handle_request(&req).await;
        assert_eq!(resp["result"]["name"], "worker");
    }
}

//! 任务数据结构

use serde::{Deserialize, Serialize};

/// 任务状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    /// 等待执行
    Pending,
    /// 执行中
    Running,
    /// 已完成
    Completed,
    /// 失败
    Failed,
    /// 已取消
    Cancelled,
}

/// 任务 -- 一次 agent 委派
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// 任务 ID (UUID)
    pub id: String,
    /// 发起方 agent 名称
    pub from: String,
    /// 目标 agent 名称
    pub to: String,
    /// 任务描述
    pub prompt: String,
    /// 任务状态
    pub status: TaskStatus,
    /// 结果 (Completed 时有值)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// 错误信息 (Failed 时有值)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 创建时间 (RFC3339)
    pub created_at: String,
    /// 完成时间 (RFC3339)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
}

impl Task {
    /// 创建新任务
    pub fn new(from: &str, to: &str, prompt: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            from: from.to_string(),
            to: to.to_string(),
            prompt: prompt.to_string(),
            status: TaskStatus::Pending,
            result: None,
            error: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
        }
    }

    /// 标记为运行中
    pub fn start(&mut self) {
        self.status = TaskStatus::Running;
    }

    /// 标记为完成
    pub fn complete(&mut self, result: String) {
        self.status = TaskStatus::Completed;
        self.result = Some(result);
        self.finished_at = Some(chrono::Utc::now().to_rfc3339());
    }

    /// 标记为失败
    pub fn fail(&mut self, error: String) {
        self.status = TaskStatus::Failed;
        self.error = Some(error);
        self.finished_at = Some(chrono::Utc::now().to_rfc3339());
    }

    /// 标记为取消
    pub fn cancel(&mut self) {
        self.status = TaskStatus::Cancelled;
        self.finished_at = Some(chrono::Utc::now().to_rfc3339());
    }

    /// 是否已结束
    pub fn is_terminal(&self) -> bool {
        matches!(self.status, TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_lifecycle() {
        let mut task = Task::new("main", "claude", "review code");
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(!task.is_terminal());

        task.start();
        assert_eq!(task.status, TaskStatus::Running);
        assert!(!task.is_terminal());

        task.complete("looks good".into());
        assert_eq!(task.status, TaskStatus::Completed);
        assert_eq!(task.result.as_deref(), Some("looks good"));
        assert!(task.is_terminal());
        assert!(task.finished_at.is_some());
    }

    #[test]
    fn task_fail() {
        let mut task = Task::new("main", "claude", "review code");
        task.start();
        task.fail("timeout".into());
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.error.as_deref(), Some("timeout"));
        assert!(task.is_terminal());
    }
}

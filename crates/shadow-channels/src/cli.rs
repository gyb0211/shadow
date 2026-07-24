use async_trait::async_trait;
use shadow_core::{Attributable, Channel, ChannelKind, ChannelMessage, Role, SendMessage};
use tokio::io;
use tokio::io::{AsyncBufReadExt, BufReader};
use uuid::Uuid;

pub struct CliChannel {
    alias: String,
}

impl CliChannel {
    pub fn new(alias: impl Into<String>) -> Self {
        Self {
            alias: alias.into(),
        }
    }
}

impl Attributable for CliChannel {
    fn role(&self) -> Role {
        Role::Channel(ChannelKind::Cli)
    }

    fn alias(&self) -> &str {
        &self.alias
    }
}

#[async_trait]
impl Channel for CliChannel {
    fn name(&self) -> &str {
        "cli"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        println!("{}", message.content);
        Ok(())
    }

    async fn listen(&self, tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        let stdin = io::stdin();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }

            if line == "/exit" || line == "/quit" {
                break;
            }

            let message = ChannelMessage {
                id: Uuid::new_v4().to_string(),
                sender: "user".to_string(),
                content: line.to_string(),
                reply_target: "user".to_string(),
            };

            if tx.send(message).await.is_err() {
                break;
            }
        }
        Ok(())
    }
}

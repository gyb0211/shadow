//! quickstart -- 交互式配置引导 (TUI 选择器)
//!
//! `shadow quickstart`              选择/创建 agent → 菜单循环
//! `shadow quickstart -a wanba`     直接进入 wanba agent 菜单

use anyhow::Result;
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Input, Select};
use shadow_config::Config;
use shadow_config::LarkConfig;
use shadow_config::autonomy::AutonomyLevel;
use shadow_config::model_provider::{CustomModelProviderConfig, ModelProviderConfig};
use shadow_config::multi::alias_agent::{AliasedAgentConfig, MemoryBackendKind};
use shadow_config::providers::{ModelProviderRef, RiskProfileRef};
use shadow_config::schema::ResolvedRuntime;

// ── 入口 ───────────────────────────────────────────

pub async fn run(config: &mut Config, agent_name: Option<&str>) -> Result<()> {
    let theme = ColorfulTheme::default();
    let name = match agent_name {
        Some(n) => n.to_string(),
        None => prompt_agent_name(config, &theme)?,
    };

    // 确保 agent 存在
    if !config.agents.contains_key(&name) {
        let alias = name.clone();
        config.agents.insert(
            alias.clone(),
            AliasedAgentConfig {
                enabled: true,
                workspace: Default::default(),
                memory: Default::default(),
                model_provider: ModelProviderRef::default(),
                risk_profile: RiskProfileRef::default(),
                runtime_profile: Default::default(),
                resolved: ResolvedRuntime {
                    max_tool_iterations: 20,
                },
            },
        );
        println!("  ✓ 已创建 agent: {name}");
    }

    // 菜单循环
    loop {
        let choice = main_menu(config, &name, &theme)?;
        match choice {
            MenuChoice::ModelProvider => {
                configure_model_provider(config, &name, &theme).await?;
            }
            MenuChoice::Channel => {
                configure_channel(config, &theme).await?;
            }
            MenuChoice::Memory => {
                configure_memory(config, &name, &theme).await?;
            }
            MenuChoice::RiskProfile => {
                configure_risk_profile(config, &name, &theme).await?;
            }
            MenuChoice::Save => {
                config.save().await?;
                println!("  ✓ 已保存到 {}", config.config_path.display());
                break;
            }
            MenuChoice::Quit => {
                println!("  未保存退出");
                break;
            }
        }
    }
    Ok(())
}

// ── Agent 名称 ──────────────────────────────────────

fn prompt_agent_name(config: &Config, theme: &ColorfulTheme) -> Result<String> {
    let names: Vec<&String> = config.agents.keys().collect();
    let mut items: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    items.push("＋ 创建新 agent".to_string());

    let selection = Select::with_theme(theme)
        .with_prompt("选择 agent")
        .items(&items)
        .default(0)
        .interact()?;

    if selection < names.len() {
        Ok(names[selection].clone())
    } else {
        let name: String = Input::with_theme(theme)
            .with_prompt("输入 agent 名称")
            .interact_text()?;
        Ok(name.trim().to_string())
    }
}

// ── 主菜单 ──────────────────────────────────────────

enum MenuChoice {
    ModelProvider,
    Channel,
    Memory,
    RiskProfile,
    Save,
    Quit,
}

fn main_menu(config: &Config, agent_name: &str, theme: &ColorfulTheme) -> Result<MenuChoice> {
    let agent = config.agents.get(agent_name);

    let mp = agent
        .map(|a| a.model_provider.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("(未配置)");

    let chan_count = config.channels.lark.len();

    let mem = agent
        .map(|a| format!("{:?}", a.memory.backend).to_lowercase())
        .unwrap_or("(未配置)".into());

    let rp = agent
        .map(|a| a.risk_profile.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("(default)");

    let items = vec![
        format!("model_provider  ({mp})"),
        format!("channel         ({chan_count} 个)"),
        format!("memory          ({mem})"),
        format!("risk_profile    ({rp})"),
        "保存并退出".to_string(),
        "不保存退出".to_string(),
    ];

    let selection = Select::with_theme(theme)
        .with_prompt(format!("Agent: {agent_name}"))
        .items(&items)
        .default(0)
        .interact()?;

    Ok(match selection {
        0 => MenuChoice::ModelProvider,
        1 => MenuChoice::Channel,
        2 => MenuChoice::Memory,
        3 => MenuChoice::RiskProfile,
        4 => MenuChoice::Save,
        _ => MenuChoice::Quit,
    })
}

// ── model_provider 配置 ─────────────────────────────

async fn configure_model_provider(
    config: &mut Config,
    agent_name: &str,
    theme: &ColorfulTheme,
) -> Result<()> {
    let entries: Vec<(String, String)> = config
        .providers
        .models
        .iter_entries()
        .map(|(ty, alias, _)| (ty.to_string(), alias.to_string()))
        .collect();

    let mut items: Vec<String> = entries
        .iter()
        .map(|(ty, alias)| format!("{ty}.{alias}"))
        .collect();

    items.push("＋ 新建".to_string());

    let selection = Select::with_theme(theme)
        .with_prompt("选择 model_provider")
        .items(&items)
        .default(0)
        .interact()?;

    let ref_str = if selection == entries.len() {
        // 新建
        let alias: String = Input::with_theme(theme)
            .with_prompt("别名")
            .with_initial_text("default")
            .interact_text()?;

        let api_key: String = Input::with_theme(theme)
            .with_prompt("api_key")
            .allow_empty(true)
            .interact_text()?;

        let model: String = Input::with_theme(theme)
            .with_prompt("model (如 gpt-4o-mini)")
            .allow_empty(true)
            .interact_text()?;

        let uri: String = Input::with_theme(theme)
            .with_prompt("base_url (可留空)")
            .allow_empty(true)
            .interact_text()?;

        let base = ModelProviderConfig {
            api_key: if api_key.trim().is_empty() {
                None
            } else {
                Some(api_key.trim().to_string())
            },
            model: if model.trim().is_empty() {
                None
            } else {
                Some(model.trim().to_string())
            },
            uri: if uri.trim().is_empty() {
                None
            } else {
                Some(uri.trim().to_string())
            },
            kind: Some("custom".to_string()),
            ..Default::default()
        };

        let entry = CustomModelProviderConfig { base };
        config.providers.models.custom.insert(alias.clone(), entry);

        format!("custom.{alias}")
    } else {
        let (ty, alias) = &entries[selection];
        format!("{ty}.{alias}")
    };

    // 绑定到 agent
    if let Some(agent) = config.agents.get_mut(agent_name) {
        agent.model_provider = ModelProviderRef::new(ref_str.clone());
        println!("  ✓ 已绑定 {ref_str} → {agent_name}");
    }

    Ok(())
}

// ── channel 配置 (增删查) ───────────────────────────

async fn configure_channel(config: &mut Config, theme: &ColorfulTheme) -> Result<()> {
    loop {
        let names: Vec<String> = config.channels.lark.keys().cloned().collect();

        let mut items: Vec<String> = Vec::new();
        for (i, n) in names.iter().enumerate() {
            let c = &config.channels.lark[n];
            let platform = if c.use_feishu { "feishu" } else { "lark" };
            items.push(format!("{n}  ({platform}, {})", c.app_id));
        }
        let _ = names.len(); // silence unused warning
        items.push("＋ 添加".to_string());
        if !names.is_empty() {
            items.push("✕ 删除".to_string());
        }
        items.push("↩ 返回".to_string());

        let selection = Select::with_theme(theme)
            .with_prompt("channel 管理")
            .items(&items)
            .default(0)
            .interact()?;

        // 计算 offset: 添加 / 删除 / 返回 的位置
        let add_idx = names.len();
        let del_idx = if names.is_empty() {
            None
        } else {
            Some(names.len() + 1)
        };
        let back_idx = if names.is_empty() {
            names.len() + 1
        } else {
            names.len() + 2
        };

        if selection == back_idx {
            break;
        } else if Some(selection) == del_idx {
            // 删除
            let del_items: Vec<String> = names
                .iter()
                .enumerate()
                .map(|(i, n)| format!("[{}] {n}", i + 1))
                .collect();
            let del_sel = Select::with_theme(theme)
                .with_prompt("选择要删除的 channel")
                .items(&del_items)
                .default(0)
                .interact()?;
            if del_sel < names.len() {
                let name = &names[del_sel];
                config.channels.lark.remove(name);
                println!("  ✓ 已删除: {name}");
            }
        } else if selection == add_idx {
            // 添加 -- 平台 → 方式 (扫码创建 / 手动输入)
            let platform_items = vec!["feishu (飞书, 国内)", "lark (国际版)"];
            let platform_sel = Select::with_theme(theme)
                .with_prompt("选择平台")
                .items(&platform_items)
                .default(0)
                .interact()?;
            let domain = if platform_sel == 0 { "feishu" } else { "lark" };
            let use_feishu = platform_sel == 0;

            let name: String = Input::with_theme(theme)
                .with_prompt("channel 名称")
                .interact_text()?;
            let name = name.trim().to_string();
            if name.is_empty() {
                continue;
            }

            // 选择配置方式
            let method_items = vec!["扫码自动创建机器人 (推荐)", "手动输入 App ID 和 App Secret"];
            let method_sel = Select::with_theme(theme)
                .with_prompt("配置方式")
                .items(&method_items)
                .default(0)
                .interact()?;

            let (app_id, app_secret) = if method_sel == 0 {
                // 扫码自动创建
                match shadow_channels::lark::qr_register(domain).await {
                    Ok(Some(result)) => {
                        println!("  ✓ 机器人创建成功!");
                        if let Some(name) = result.open_id.as_ref() {
                            println!("    open_id: {name}");
                        }
                        // 扫码创建的域名可能与初始不同 (feishu ↔ lark 自动检测)
                        (result.app_id, result.app_secret)
                    }
                    Ok(None) => {
                        println!("  ⚠ 扫码未完成, 请改用手动输入");
                        let id: String = Input::with_theme(theme)
                            .with_prompt("app_id")
                            .interact_text()?;
                        let secret: String = Input::with_theme(theme)
                            .with_prompt("app_secret")
                            .interact_text()?;
                        (id.trim().to_string(), secret.trim().to_string())
                    }
                    Err(e) => {
                        println!("  ⚠ 扫码失败: {e}");
                        let id: String = Input::with_theme(theme)
                            .with_prompt("app_id")
                            .interact_text()?;
                        let secret: String = Input::with_theme(theme)
                            .with_prompt("app_secret")
                            .interact_text()?;
                        (id.trim().to_string(), secret.trim().to_string())
                    }
                }
            } else {
                // 手动输入
                let id: String = Input::with_theme(theme)
                    .with_prompt("app_id")
                    .interact_text()?;
                let secret: String = Input::with_theme(theme)
                    .with_prompt("app_secret")
                    .interact_text()?;
                (id.trim().to_string(), secret.trim().to_string())
            };

            if app_id.is_empty() || app_secret.is_empty() {
                println!("  ⚠ app_id/app_secret 不能为空, 已跳过");
                continue;
            }

            let lark = LarkConfig {
                enabled: true,
                app_id: app_id.clone(),
                app_secret: app_secret.clone(),
                use_feishu,
                ..Default::default()
            };

            config.channels.lark.insert(name.clone(), lark);
            println!("  ✓ 已添加 channel: {name}");

            // 验证机器人
            match shadow_channels::lark::probe_bot(&app_id, &app_secret, domain).await {
                Ok(Some(bot_info)) => {
                    let bot_name = bot_info
                        .get("bot_name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("(未命名)");
                    println!("  ✓ 机器人验证成功: {bot_name}");
                }
                _ => {
                    println!("  ⚠ 无法验证机器人 (凭证已保存, 可能需要等待应用审核)");
                }
            }
        }
    }

    Ok(())
}

// ── memory 配置 ─────────────────────────────────────

async fn configure_memory(
    config: &mut Config,
    agent_name: &str,
    theme: &ColorfulTheme,
) -> Result<()> {
    let items = vec![
        "none      (禁用记忆)",
        "sqlite    (本地 SQLite, 推荐)",
        "markdown  (Markdown 文件)",
        "postgres  (PostgreSQL)",
    ];

    let selection = Select::with_theme(theme)
        .with_prompt("选择 memory 后端")
        .items(&items)
        .default(1)
        .interact()?;

    let backend = match selection {
        0 => MemoryBackendKind::None,
        1 => MemoryBackendKind::Sqlite,
        2 => MemoryBackendKind::Markdown,
        _ => MemoryBackendKind::Postgres,
    };

    if let Some(agent) = config.agents.get_mut(agent_name) {
        agent.memory.backend = backend;
    }
    println!("  ✓ memory 已设置");

    Ok(())
}

// ── risk_profile 配置 ───────────────────────────────

async fn configure_risk_profile(
    config: &mut Config,
    agent_name: &str,
    theme: &ColorfulTheme,
) -> Result<()> {
    let current = config
        .agents
        .get(agent_name)
        .map(|a| a.risk_profile.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("(default)");

    let names: Vec<String> = config.risk_profiles.keys().cloned().collect();

    let mut items: Vec<String> = names.iter().cloned().collect();
    items.push("＋ 新建".to_string());
    items.push("✎ 直接编辑 autonomy level".to_string());
    items.push("↩ 返回".to_string());

    let selection = Select::with_theme(theme)
        .with_prompt(format!("risk_profile (当前: {current})"))
        .items(&items)
        .default(0)
        .interact()?;

    let new_idx = names.len();
    let edit_idx = names.len() + 1;
    let back_idx = names.len() + 2;

    if selection == back_idx {
        return Ok(());
    } else if selection == new_idx {
        // 新建
        let name: String = Input::with_theme(theme)
            .with_prompt("名称")
            .with_initial_text(agent_name)
            .interact_text()?;
        let name = if name.trim().is_empty() {
            agent_name.to_string()
        } else {
            name.trim().to_string()
        };

        let level = pick_autonomy_level(theme)?;

        config
            .risk_profiles
            .entry(name.clone())
            .or_insert_with(Default::default)
            .level = level;

        if let Some(agent) = config.agents.get_mut(agent_name) {
            agent.risk_profile = RiskProfileRef::new(&name);
            println!("  ✓ 已绑定 risk_profile: {name}");
        }
    } else if selection == edit_idx {
        // 直接编辑
        let level = pick_autonomy_level(theme)?;

        let rp_name = config
            .agents
            .get(agent_name)
            .map(|a| a.risk_profile.as_str().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "default".to_string());

        config
            .risk_profiles
            .entry(rp_name.clone())
            .or_insert_with(Default::default)
            .level = level;
        println!("  ✓ autonomy level 已设置 ({rp_name})");
    } else if selection < names.len() {
        // 选已有
        let n = &names[selection];
        if let Some(agent) = config.agents.get_mut(agent_name) {
            agent.risk_profile = RiskProfileRef::new(n);
            println!("  ✓ 已绑定 risk_profile: {n}");
        }
    }

    Ok(())
}

fn pick_autonomy_level(theme: &ColorfulTheme) -> Result<AutonomyLevel> {
    let items = vec![
        "read_only    (只读, 默认)",
        "supervised   (需确认)",
        "full         (全自动)",
    ];
    let selection = Select::with_theme(theme)
        .with_prompt("autonomy level")
        .items(&items)
        .default(0)
        .interact()?;

    Ok(match selection {
        1 => AutonomyLevel::Supervised,
        2 => AutonomyLevel::Full,
        _ => AutonomyLevel::ReadOnly,
    })
}

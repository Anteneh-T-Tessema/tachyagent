//! Messaging channel integrations — Slack, Discord, Telegram.
//!
//! Each channel is a lightweight bridge that:
//! 1. Receives messages from the platform (via webhook or polling)
//! 2. Forwards them to `POST /api/agents/run`
//! 3. Polls for the result
//! 4. Posts the response back to the platform
//!
//! Configuration in `.tachy/channels.yaml`:
//! ```yaml
//! channels:
//!   - type: slack
//!     bot_token: $SLACK_BOT_TOKEN
//!     app_token: $SLACK_APP_TOKEN
//!     channel: "#ai-agent"
//!     template: chat
//!
//!   - type: discord
//!     bot_token: $DISCORD_BOT_TOKEN
//!     channel_id: "123456789"
//!     template: chat
//!
//!   - type: telegram
//!     bot_token: $TELEGRAM_BOT_TOKEN
//!     allowed_users:
//!       - "username1"
//!     template: chat
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    pub r#type: ChannelType,
    #[serde(default)]
    pub bot_token: String,
    #[serde(default)]
    pub app_token: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub template: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub allowed_users: Vec<String>,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    Slack,
    Discord,
    Telegram,
    Webhook,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelsFile {
    #[serde(default)]
    pub channels: Vec<ChannelConfig>,
}

/// Load channel configurations from `.tachy/channels.yaml`.
pub fn load_channels(tachy_dir: &Path) -> Vec<ChannelConfig> {
    let yaml_path = tachy_dir.join("channels.yaml");
    let yml_path = tachy_dir.join("channels.yml");

    let path = if yaml_path.exists() { yaml_path }
        else if yml_path.exists() { yml_path }
        else { return Vec::new() };

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    parse_channels_yaml(&content)
}

/// Post a message to a Slack channel using the Web API.
pub fn slack_post_message(bot_token: &str, channel: &str, text: &str) -> Result<(), String> {
    let token = expand_env(bot_token);
    let body = serde_json::json!({
        "channel": channel,
        "text": text,
        "unfurl_links": false,
    });

    let output = std::process::Command::new("curl")
        .args([
            "-s", "-X", "POST",
            "https://slack.com/api/chat.postMessage",
            "-H", &format!("Authorization: Bearer {token}"),
            "-H", "Content-Type: application/json",
            "-d", &body.to_string(),
        ])
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

/// Send a message to a Discord channel using the Bot API.
pub fn discord_send_message(bot_token: &str, channel_id: &str, text: &str) -> Result<(), String> {
    let token = expand_env(bot_token);
    // Discord has a 2000 char limit per message
    let truncated = if text.len() > 1900 {
        format!("{}…", &text[..1900])
    } else {
        text.to_string()
    };

    let body = serde_json::json!({ "content": truncated });

    let output = std::process::Command::new("curl")
        .args([
            "-s", "-X", "POST",
            &format!("https://discord.com/api/v10/channels/{channel_id}/messages"),
            "-H", &format!("Authorization: Bot {token}"),
            "-H", "Content-Type: application/json",
            "-d", &body.to_string(),
        ])
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    if output.status.success() { Ok(()) }
    else { Err(String::from_utf8_lossy(&output.stderr).to_string()) }
}

/// Send a message to a Telegram chat using the Bot API.
pub fn telegram_send_message(bot_token: &str, chat_id: &str, text: &str) -> Result<(), String> {
    let token = expand_env(bot_token);
    // Telegram has a 4096 char limit
    let truncated = if text.len() > 4000 {
        format!("{}…", &text[..4000])
    } else {
        text.to_string()
    };

    let body = serde_json::json!({
        "chat_id": chat_id,
        "text": truncated,
        "parse_mode": "Markdown",
    });

    let output = std::process::Command::new("curl")
        .args([
            "-s", "-X", "POST",
            &format!("https://api.telegram.org/bot{token}/sendMessage"),
            "-H", "Content-Type: application/json",
            "-d", &body.to_string(),
        ])
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    if output.status.success() { Ok(()) }
    else { Err(String::from_utf8_lossy(&output.stderr).to_string()) }
}

/// Expand $ENV_VAR references in a string.
fn expand_env(s: &str) -> String {
    if s.starts_with('$') {
        std::env::var(&s[1..]).unwrap_or_else(|_| s.to_string())
    } else {
        s.to_string()
    }
}

/// Simple YAML parser for channels.yaml.
fn parse_channels_yaml(content: &str) -> Vec<ChannelConfig> {
    let mut channels = Vec::new();
    let mut current: Option<BTreeMap<String, serde_json::Value>> = None;
    let mut current_users: Option<Vec<String>> = None;
    let mut in_users = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "channels:" {
            continue;
        }

        let _indent = line.len() - line.trim_start().len();

        if trimmed.starts_with("- type:") {
            // Save previous
            if let Some(mut map) = current.take() {
                if let Some(users) = current_users.take() {
                    map.insert("allowed_users".to_string(), serde_json::json!(users));
                }
                if let Ok(ch) = serde_json::from_value::<ChannelConfig>(serde_json::Value::Object(
                    map.into_iter().collect()
                )) {
                    channels.push(ch);
                }
            }
            let val = trimmed.strip_prefix("- type:").unwrap().trim().trim_matches('"');
            let mut map = BTreeMap::new();
            map.insert("type".to_string(), serde_json::json!(val));
            map.insert("enabled".to_string(), serde_json::json!(true));
            current = Some(map);
            current_users = None;
            in_users = false;
            continue;
        }

        if let Some(map) = current.as_mut() {
            if trimmed == "allowed_users:" {
                in_users = true;
                current_users = Some(Vec::new());
                continue;
            }
            if in_users {
                if trimmed.starts_with("- ") {
                    let user = trimmed.strip_prefix("- ").unwrap().trim().trim_matches('"');
                    if let Some(users) = current_users.as_mut() {
                        users.push(user.to_string());
                    }
                    continue;
                } else {
                    in_users = false;
                }
            }
            if let Some((key, val)) = trimmed.split_once(':') {
                let key = key.trim().trim_start_matches("- ");
                let val = val.trim().trim_matches('"').trim_matches('\'');
                if !key.is_empty() && !val.is_empty() {
                    if val == "true" { map.insert(key.to_string(), serde_json::json!(true)); }
                    else if val == "false" { map.insert(key.to_string(), serde_json::json!(false)); }
                    else { map.insert(key.to_string(), serde_json::json!(val)); }
                }
            }
        }
    }

    // Save last
    if let Some(mut map) = current.take() {
        if let Some(users) = current_users.take() {
            map.insert("allowed_users".to_string(), serde_json::json!(users));
        }
        if let Ok(ch) = serde_json::from_value::<ChannelConfig>(serde_json::Value::Object(
            map.into_iter().collect()
        )) {
            channels.push(ch);
        }
    }

    channels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_channel_config() {
        let yaml = "channels:\n  - type: slack\n    bot_token: $SLACK_TOKEN\n    channel: general\n    template: chat\n    enabled: true\n  - type: telegram\n    bot_token: $TG_TOKEN\n    allowed_users:\n      - alice\n      - bob\n    template: chat\n";
        let channels = parse_channels_yaml(yaml);
        assert_eq!(channels.len(), 2);
        assert_eq!(channels[0].r#type, ChannelType::Slack);
        assert_eq!(channels[0].channel, "general");
        assert_eq!(channels[1].r#type, ChannelType::Telegram);
        assert_eq!(channels[1].allowed_users.len(), 2);
    }

    #[test]
    fn expands_env_vars() {
        std::env::set_var("TACHY_TEST_TOKEN", "secret123");
        assert_eq!(expand_env("$TACHY_TEST_TOKEN"), "secret123");
        assert_eq!(expand_env("plain_text"), "plain_text");
        std::env::remove_var("TACHY_TEST_TOKEN");
    }
}

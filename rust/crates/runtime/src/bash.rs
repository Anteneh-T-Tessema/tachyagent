use std::io;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Command as TokioCommand;
use tokio::runtime::Builder;
use tokio::time::timeout;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BashCommandInput {
    pub command: String,
    pub timeout: Option<u64>,
    pub description: Option<String>,
    #[serde(rename = "run_in_background")]
    pub run_in_background: Option<bool>,
    #[serde(rename = "dangerouslyDisableSandbox")]
    pub dangerously_disable_sandbox: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BashCommandOutput {
    pub stdout: String,
    pub stderr: String,
    #[serde(rename = "rawOutputPath")]
    pub raw_output_path: Option<String>,
    pub interrupted: bool,
    #[serde(rename = "isImage")]
    pub is_image: Option<bool>,
    #[serde(rename = "backgroundTaskId")]
    pub background_task_id: Option<String>,
    #[serde(rename = "backgroundedByUser")]
    pub backgrounded_by_user: Option<bool>,
    #[serde(rename = "assistantAutoBackgrounded")]
    pub assistant_auto_backgrounded: Option<bool>,
    #[serde(rename = "dangerouslyDisableSandbox")]
    pub dangerously_disable_sandbox: Option<bool>,
    #[serde(rename = "returnCodeInterpretation")]
    pub return_code_interpretation: Option<String>,
    #[serde(rename = "noOutputExpected")]
    pub no_output_expected: Option<bool>,
    #[serde(rename = "structuredContent")]
    pub structured_content: Option<Vec<serde_json::Value>>,
    #[serde(rename = "persistedOutputPath")]
    pub persisted_output_path: Option<String>,
    #[serde(rename = "persistedOutputSize")]
    pub persisted_output_size: Option<u64>,
}

pub fn execute_bash(input: BashCommandInput) -> io::Result<BashCommandOutput> {
    if input.run_in_background.unwrap_or(false) {
        let child = if cfg!(target_os = "windows") {
            Command::new("cmd")
                .arg("/C")
                .arg(&input.command)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?
        } else {
            Command::new("sh")
                .arg("-lc")
                .arg(&input.command)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?
        };

        return Ok(BashCommandOutput {
            stdout: String::new(),
            stderr: String::new(),
            raw_output_path: None,
            interrupted: false,
            is_image: None,
            background_task_id: Some(child.id().to_string()),
            backgrounded_by_user: Some(false),
            assistant_auto_backgrounded: Some(false),
            dangerously_disable_sandbox: input.dangerously_disable_sandbox,
            return_code_interpretation: None,
            no_output_expected: Some(true),
            structured_content: None,
            persisted_output_path: None,
            persisted_output_size: None,
        });
    }

    // Use separate thread if already inside a tokio runtime (e.g., daemon)
    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::scope(|s| {
            s.spawn(|| {
                let rt = Builder::new_current_thread().enable_all().build()?;
                rt.block_on(execute_bash_async(input))
            })
            .join()
            .map_err(|_| io::Error::other("bash thread panicked"))?
        })
    } else {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        runtime.block_on(execute_bash_async(input))
    }
}

async fn execute_bash_async(input: BashCommandInput) -> io::Result<BashCommandOutput> {
    let mut command = if cfg!(target_os = "windows") {
        let mut cmd = TokioCommand::new("cmd");
        cmd.arg("/C").arg(&input.command);
        cmd
    } else {
        let mut cmd = TokioCommand::new("sh");
        cmd.arg("-lc").arg(&input.command);
        cmd
    };

    // Default timeout from env (seconds) → milliseconds. Minimum 5s, maximum 5min.
    // Set TACHY_TOOL_TIMEOUT_SECS=0 to use per-call timeout values only.
    let default_ms: u64 = std::env::var("TACHY_TOOL_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(30)
        .saturating_mul(1_000);
    let effective_timeout = input.timeout
        .unwrap_or(default_ms)
        .max(5_000)          // floor: never less than 5s
        .min(300_000);       // ceiling: never more than 5min

    let output_result = match timeout(Duration::from_millis(effective_timeout), command.output()).await {
        Ok(result) => (result?, false),
        Err(_) => {
            return Ok(BashCommandOutput {
                stdout: String::new(),
                stderr: format!("Command exceeded timeout of {effective_timeout} ms"),
                raw_output_path: None,
                interrupted: true,
                is_image: None,
                background_task_id: None,
                backgrounded_by_user: None,
                assistant_auto_backgrounded: None,
                dangerously_disable_sandbox: input.dangerously_disable_sandbox,
                return_code_interpretation: Some(String::from("timeout")),
                no_output_expected: Some(true),
                structured_content: None,
                persisted_output_path: None,
                persisted_output_size: None,
            });
        }
    };

    let (output, interrupted) = output_result;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let no_output_expected = Some(stdout.trim().is_empty() && stderr.trim().is_empty());
    let return_code_interpretation = output.status.code().and_then(|code| {
        if code == 0 {
            None
        } else {
            Some(format!("exit_code:{code}"))
        }
    });

    Ok(BashCommandOutput {
        stdout,
        stderr,
        raw_output_path: None,
        interrupted,
        is_image: None,
        background_task_id: None,
        backgrounded_by_user: None,
        assistant_auto_backgrounded: None,
        dangerously_disable_sandbox: input.dangerously_disable_sandbox,
        return_code_interpretation,
        no_output_expected,
        structured_content: None,
        persisted_output_path: None,
        persisted_output_size: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{execute_bash, BashCommandInput};

    #[test]
    fn executes_simple_command() {
        let output = execute_bash(BashCommandInput {
            command: String::from("printf 'hello'"),
            timeout: Some(1_000),
            description: None,
            run_in_background: Some(false),
            dangerously_disable_sandbox: Some(false),
        })
        .expect("bash command should execute");

        assert_eq!(output.stdout, "hello");
        assert!(!output.interrupted);
    }

    #[test]
    fn timeout_enforced_when_none_provided() {
        // No timeout in input → default kicks in (30s), command finishes well within that.
        let output = execute_bash(BashCommandInput {
            command: String::from("printf 'ok'"),
            timeout: None,
            description: None,
            run_in_background: Some(false),
            dangerously_disable_sandbox: Some(false),
        })
        .expect("should succeed with default timeout");
        assert_eq!(output.stdout, "ok");
        assert!(!output.interrupted);
    }

    #[test]
    fn timeout_kills_slow_command() {
        // 5_000ms minimum floor; test a command with an explicit short timeout.
        let output = execute_bash(BashCommandInput {
            command: String::from("sleep 10"),
            timeout: Some(5_000), // minimum floor, will be applied as-is
            description: None,
            run_in_background: Some(false),
            dangerously_disable_sandbox: Some(false),
        })
        .expect("should return a timeout result");
        assert!(output.interrupted, "sleep 10 should be interrupted");
        assert!(output.stderr.contains("timeout") || output.stderr.contains("exceeded"),
            "stderr should mention timeout: {}", output.stderr);
    }
}

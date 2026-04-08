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

    // Enforce minimum 5 second timeout — local models often send tiny values
    let effective_timeout = input.timeout.map(|t| t.max(5_000));

    let output_result = if let Some(timeout_ms) = effective_timeout {
        match timeout(Duration::from_millis(timeout_ms), command.output()).await {
            Ok(result) => (result?, false),
            Err(_) => {
                return Ok(BashCommandOutput {
                    stdout: String::new(),
                    stderr: format!("Command exceeded timeout of {timeout_ms} ms"),
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
        }
    } else {
        (command.output().await?, false)
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
}

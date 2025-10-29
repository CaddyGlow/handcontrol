use anyhow::{anyhow, Context, Result};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use crate::config::parser::CommandConfig;

/// Result of command execution
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub exit_code: i32,
    pub timed_out: bool,
}

/// Determines the appropriate shell for the current platform
fn get_shell() -> (&'static str, &'static str) {
    if cfg!(target_os = "windows") {
        ("cmd.exe", "/C")
    } else {
        ("/bin/sh", "-c")
    }
}

/// Executes a command and returns stdout/stderr output lines via callback
pub async fn execute_command<F>(
    command_config: &CommandConfig,
    shell_command: String,
    mut output_callback: F,
) -> Result<ExecutionResult>
where
    F: FnMut(CommandOutput) + Send,
{
    let (shell, shell_arg) = get_shell();

    info!(
        "Executing command: id={}, timeout={}s",
        command_config.id, command_config.timeout_seconds
    );
    debug!("Shell command: {}", shell_command);

    // Spawn process
    let mut child = Command::new(shell)
        .arg(shell_arg)
        .arg(&shell_command)
        .envs(&command_config.env)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn command process")?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Failed to capture stdout"))?;

    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("Failed to capture stderr"))?;

    // Create buffered readers for stdout and stderr
    let stdout_reader = BufReader::new(stdout);
    let stderr_reader = BufReader::new(stderr);

    let mut stdout_lines = stdout_reader.lines();
    let mut stderr_lines = stderr_reader.lines();

    // Set up timeout
    let timeout_duration = Duration::from_secs(command_config.timeout_seconds);

    // Read output until process completes or timeout
    let result = timeout(timeout_duration, async {
        loop {
            tokio::select! {
                // Read from stdout
                line = stdout_lines.next_line() => {
                    match line {
                        Ok(Some(text)) => {
                            output_callback(CommandOutput::Stdout(text));
                        }
                        Ok(None) => {
                            // EOF on stdout, but stderr might still have data
                        }
                        Err(e) => {
                            warn!("Error reading stdout: {}", e);
                        }
                    }
                }
                // Read from stderr
                line = stderr_lines.next_line() => {
                    match line {
                        Ok(Some(text)) => {
                            output_callback(CommandOutput::Stderr(text));
                        }
                        Ok(None) => {
                            // EOF on stderr, but stdout might still have data
                        }
                        Err(e) => {
                            warn!("Error reading stderr: {}", e);
                        }
                    }
                }
                // Wait for process to exit
                status = child.wait() => {
                    let exit_status = status.context("Failed to wait for child process")?;
                    let exit_code = exit_status.code().unwrap_or(-1);

                    info!(
                        "Command completed: id={}, exit_code={}",
                        command_config.id, exit_code
                    );

                    return Ok::<ExecutionResult, anyhow::Error>(ExecutionResult {
                        exit_code,
                        timed_out: false,
                    });
                }
            }
        }
    })
    .await;

    match result {
        Ok(exec_result) => exec_result,
        Err(_) => {
            // Timeout occurred
            warn!(
                "Command timed out after {}s: id={}",
                command_config.timeout_seconds, command_config.id
            );

            // Kill the process
            if let Err(e) = child.kill().await {
                warn!("Failed to kill timed-out process: {}", e);
            }

            Ok(ExecutionResult {
                exit_code: -1,
                timed_out: true,
            })
        }
    }
}

/// Output from command execution
#[derive(Debug, Clone)]
pub enum CommandOutput {
    Stdout(String),
    Stderr(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn make_test_command(id: &str, shell: &str, timeout: u64) -> CommandConfig {
        CommandConfig {
            id: id.to_string(),
            name: format!("Test {}", id),
            description: None,
            icon: None,
            shell: shell.to_string(),
            tags: vec![],
            timeout_seconds: timeout,
            env: HashMap::new(),
            parameters: vec![],
        }
    }

    #[tokio::test]
    async fn test_execute_simple_command() {
        let command = make_test_command("echo-test", "echo hello", 5);
        let output = Arc::new(Mutex::new(Vec::new()));
        let output_clone = output.clone();

        let result = execute_command(&command, "echo hello".to_string(), move |out| {
            output_clone.lock().unwrap().push(out);
        })
        .await
        .unwrap();

        assert_eq!(result.exit_code, 0);
        assert!(!result.timed_out);

        let captured = output.lock().unwrap();
        assert!(!captured.is_empty());
    }

    #[tokio::test]
    async fn test_execute_command_with_stderr() {
        // Command that writes to stderr
        let shell_cmd = if cfg!(target_os = "windows") {
            "echo error 1>&2"
        } else {
            "echo error >&2"
        };

        let command = make_test_command("stderr-test", shell_cmd, 5);
        let output = Arc::new(Mutex::new(Vec::new()));
        let output_clone = output.clone();

        let result = execute_command(&command, shell_cmd.to_string(), move |out| {
            output_clone.lock().unwrap().push(out);
        })
        .await
        .unwrap();

        assert_eq!(result.exit_code, 0);
        assert!(!result.timed_out);

        let captured = output.lock().unwrap();
        let has_stderr = captured
            .iter()
            .any(|out| matches!(out, CommandOutput::Stderr(_)));
        assert!(has_stderr);
    }

    #[tokio::test]
    async fn test_execute_command_nonzero_exit() {
        let shell_cmd = if cfg!(target_os = "windows") {
            "exit 42"
        } else {
            "exit 42"
        };

        let command = make_test_command("exit-test", shell_cmd, 5);
        let output = Arc::new(Mutex::new(Vec::new()));

        let result = execute_command(&command, shell_cmd.to_string(), move |out| {
            output.lock().unwrap().push(out);
        })
        .await
        .unwrap();

        assert_eq!(result.exit_code, 42);
        assert!(!result.timed_out);
    }

    #[tokio::test]
    async fn test_execute_command_timeout() {
        // Command that sleeps longer than timeout
        let shell_cmd = if cfg!(target_os = "windows") {
            "timeout /t 10 /nobreak"
        } else {
            "sleep 10"
        };

        let command = make_test_command("timeout-test", shell_cmd, 1);
        let output = Arc::new(Mutex::new(Vec::new()));

        let result = execute_command(&command, shell_cmd.to_string(), move |out| {
            output.lock().unwrap().push(out);
        })
        .await
        .unwrap();

        assert!(result.timed_out);
        assert_eq!(result.exit_code, -1);
    }

    #[tokio::test]
    async fn test_execute_command_with_env() {
        let shell_cmd = if cfg!(target_os = "windows") {
            "echo %TEST_VAR%"
        } else {
            "echo $TEST_VAR"
        };

        let mut command = make_test_command("env-test", shell_cmd, 5);
        command.env.insert("TEST_VAR".to_string(), "test_value".to_string());

        let output = Arc::new(Mutex::new(Vec::new()));
        let output_clone = output.clone();

        let result = execute_command(&command, shell_cmd.to_string(), move |out| {
            output_clone.lock().unwrap().push(out);
        })
        .await
        .unwrap();

        assert_eq!(result.exit_code, 0);

        let captured = output.lock().unwrap();
        let has_test_value = captured.iter().any(|out| {
            if let CommandOutput::Stdout(text) = out {
                text.contains("test_value")
            } else {
                false
            }
        });
        assert!(has_test_value);
    }

    #[test]
    fn test_get_shell() {
        let (shell, arg) = get_shell();

        if cfg!(target_os = "windows") {
            assert_eq!(shell, "cmd.exe");
            assert_eq!(arg, "/C");
        } else {
            assert_eq!(shell, "/bin/sh");
            assert_eq!(arg, "-c");
        }
    }
}

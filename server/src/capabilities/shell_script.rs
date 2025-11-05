use super::{Capability, CapabilityMetadata, CapabilityOpenContext, SessionMode};
use crate::capabilities::parameters;
use crate::config::parser::{CapabilityConfig, CapabilityDefinition, ShellScriptDefinition};
use crate::sessions::{SessionClientEvent, SessionEndpoints, SessionServerEvent};
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use std::collections::HashMap;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::{Duration, timeout};
use tracing::{debug, info, warn};

pub struct ShellScriptCapability {
    config: CapabilityConfig,
    metadata: CapabilityMetadata,
}

impl ShellScriptCapability {
    pub fn new(config: CapabilityConfig, metadata: CapabilityMetadata) -> Self {
        Self { config, metadata }
    }

    fn definition(&self) -> &ShellScriptDefinition {
        match &self.config.definition {
            CapabilityDefinition::ShellScript(def) => def,
            _ => unreachable!("ShellScriptCapability constructed with non-shell definition"),
        }
    }
}

#[async_trait]
impl Capability for ShellScriptCapability {
    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }

    fn definition(&self) -> &CapabilityDefinition {
        &self.config.definition
    }

    async fn open_session(
        &self,
        ctx: CapabilityOpenContext,
        endpoints: SessionEndpoints,
    ) -> Result<()> {
        if self.metadata.session_mode != SessionMode::OneShot {
            return Err(anyhow!(
                "Capability '{}' misconfigured: shell script must be one-shot session",
                self.metadata.id
            ));
        }

        let validated = parameters::validate_parameters(&self.config, &ctx.parameters)?;
        let command = parameters::substitute_parameters(&self.config, &validated)?;
        let definition = self.definition().clone();
        let capability_id = self.metadata.id.clone();
        let show_output = definition.show_output;

        let mut inbound = endpoints.inbound;
        let outbound = endpoints.outbound;
        let close_capability_id = capability_id.clone();

        tokio::spawn(async move {
            while let Some(event) = inbound.recv().await {
                if let SessionClientEvent::Close { reason } = event {
                    debug!(
                        capability_id = %close_capability_id,
                        ?reason,
                        "Shell script session received close request"
                    );
                    break;
                }
            }
        });

        tokio::spawn(async move {
            if let Err(err) = run_shell_command(
                &capability_id,
                command,
                definition.env,
                definition.timeout_seconds,
                show_output,
                outbound.clone(),
            )
            .await
            {
                warn!(
                    capability_id = %capability_id,
                    error = %err,
                    "Shell script capability failed"
                );
                let _ = outbound
                    .send(SessionServerEvent::Error {
                        message: format!("Command execution failed: {}", err),
                        code: None,
                    })
                    .await;
            }

            let _ = outbound
                .send(SessionServerEvent::Closed { reason: None })
                .await;
        });

        Ok(())
    }
}

async fn run_shell_command(
    capability_id: &str,
    command: String,
    env: HashMap<String, String>,
    timeout_seconds: u64,
    show_output: bool,
    outbound: tokio::sync::mpsc::Sender<SessionServerEvent>,
) -> Result<()> {
    outbound
        .send(SessionServerEvent::Ready { message: None })
        .await
        .ok();

    let (shell, shell_arg) = if cfg!(target_os = "windows") {
        ("cmd.exe", "/C")
    } else {
        ("/bin/sh", "-c")
    };

    info!(
        capability_id = %capability_id,
        timeout_seconds,
        "Executing shell command"
    );
    debug!(
        capability_id = %capability_id,
        command = %command,
        "Resolved shell command"
    );

    let mut child = Command::new(shell)
        .arg(shell_arg)
        .arg(&command)
        .envs(env)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn shell process")?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Failed to capture stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("Failed to capture stderr"))?;

    let mut stdout_lines = BufReader::new(stdout).lines();
    let mut stderr_lines = BufReader::new(stderr).lines();

    let timeout_duration = Duration::from_secs(timeout_seconds.max(1));

    let mut outbound_clone = outbound.clone();

    let result = timeout(timeout_duration, async {
        loop {
            tokio::select! {
                line = stdout_lines.next_line() => {
                    match line {
                        Ok(Some(text)) => {
                            if show_output {
                                let _ = outbound_clone.send(SessionServerEvent::Output {
                                    data: bytes::Bytes::from(text),
                                    stderr: false,
                                    binary: false,
                                    timestamp_ms: Some(current_timestamp_ms()),
                                }).await;
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            warn!(
                                capability_id = %capability_id,
                                error = %e,
                                "Error reading stdout"
                            );
                        }
                    }
                }
                line = stderr_lines.next_line() => {
                    match line {
                        Ok(Some(text)) => {
                            if show_output {
                                let _ = outbound_clone.send(SessionServerEvent::Output {
                                    data: bytes::Bytes::from(text),
                                    stderr: true,
                                    binary: false,
                                    timestamp_ms: Some(current_timestamp_ms()),
                                }).await;
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            warn!(
                                capability_id = %capability_id,
                                error = %e,
                                "Error reading stderr"
                            );
                        }
                    }
                }
                status = child.wait() => {
                    let exit_status = status.context("Failed to await process")?;
                    let exit_code = exit_status.code().unwrap_or(-1);

                    info!(
                        capability_id = %capability_id,
                        exit_code,
                        "Shell command completed"
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
        Ok(Ok(exec)) => {
            send_exit_event(&mut outbound_clone, exec.exit_code, exec.timed_out).await;
        }
        Ok(Err(err)) => {
            warn!(
                capability_id = %capability_id,
                error = %err,
                "Shell command failed during execution"
            );
            let _ = outbound_clone
                .send(SessionServerEvent::Error {
                    message: format!("Command execution failed: {}", err),
                    code: None,
                })
                .await;
        }
        Err(_) => {
            warn!(
                capability_id = %capability_id,
                timeout_seconds,
                "Shell command timed out"
            );
            if let Err(e) = child.kill().await {
                warn!(
                    capability_id = %capability_id,
                    error = %e,
                    "Failed to kill timed-out process"
                );
            }
            send_exit_event(&mut outbound_clone, -1, true).await;
        }
    }

    Ok(())
}

struct ExecutionResult {
    exit_code: i32,
    timed_out: bool,
}

async fn send_exit_event(
    outbound: &mut tokio::sync::mpsc::Sender<SessionServerEvent>,
    exit_code: i32,
    timed_out: bool,
) {
    let _ = outbound
        .send(SessionServerEvent::Exit {
            exit_code,
            timed_out,
            message: None,
        })
        .await;
}

fn current_timestamp_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

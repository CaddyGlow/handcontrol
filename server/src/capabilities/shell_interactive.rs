use super::{Capability, CapabilityMetadata, CapabilityOpenContext, SessionMode};
use crate::config::parser::{CapabilityConfig, CapabilityDefinition, ShellInteractiveDefinition};
use crate::sessions::{SessionClientEvent, SessionEndpoints, SessionServerEvent};
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use futures_util::future;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tracing::{debug, info, warn};

pub struct ShellInteractiveCapability {
    config: CapabilityConfig,
    metadata: CapabilityMetadata,
}

impl ShellInteractiveCapability {
    pub fn new(config: CapabilityConfig, metadata: CapabilityMetadata) -> Self {
        Self { config, metadata }
    }

    fn definition(&self) -> &ShellInteractiveDefinition {
        match &self.config.definition {
            CapabilityDefinition::ShellInteractive(def) => def,
            _ => unreachable!(
                "ShellInteractiveCapability constructed with non-interactive definition"
            ),
        }
    }
}

#[async_trait]
impl Capability for ShellInteractiveCapability {
    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }

    fn definition(&self) -> &CapabilityDefinition {
        &self.config.definition
    }

    async fn open_session(
        &self,
        _ctx: CapabilityOpenContext,
        endpoints: SessionEndpoints,
    ) -> Result<()> {
        if self.metadata.session_mode != SessionMode::Realtime {
            return Err(anyhow!(
                "Capability '{}' misconfigured: interactive shell must be realtime session",
                self.metadata.id
            ));
        }

        let definition = self.definition().clone();
        let capability_id = self.metadata.id.clone();
        let shell_path = definition.shell.clone();
        let working_dir = definition.working_directory.clone();
        let env = definition.env.clone();

        let max_duration = definition.max_duration_seconds;
        let idle_timeout = definition.idle_timeout_seconds;

        tokio::spawn(async move {
            if let Err(err) = run_interactive_shell(
                &capability_id,
                shell_path,
                working_dir,
                env,
                idle_timeout,
                max_duration,
                endpoints,
            )
            .await
            {
                warn!(
                    capability_id = %capability_id,
                    error = %err,
                    "Interactive shell session failed"
                );
            }
        });

        Ok(())
    }
}

async fn run_interactive_shell(
    capability_id: &str,
    shell_path: String,
    working_dir: Option<String>,
    env: std::collections::HashMap<String, String>,
    idle_timeout: Option<u64>,
    max_duration: Option<u64>,
    endpoints: SessionEndpoints,
) -> Result<()> {
    let mut command = Command::new(&shell_path);
    command.stdin(std::process::Stdio::piped());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    if let Some(dir) = working_dir {
        command.current_dir(dir);
    }
    if !env.is_empty() {
        command.envs(env);
    }

    info!(
        capability_id = %capability_id,
        shell = %shell_path,
        "Launching interactive shell"
    );

    let mut child = command
        .spawn()
        .context("Failed to spawn interactive shell")?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("Failed to capture stdin for interactive shell"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Failed to capture stdout for interactive shell"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("Failed to capture stderr for interactive shell"))?;

    let outbound = endpoints.outbound.clone();
    outbound
        .send(SessionServerEvent::Ready {
            message: Some(format!("Shell started: {}", shell_path)),
        })
        .await
        .ok();

    let stdout_handle = tokio::spawn(pipe_output(
        capability_id.to_string(),
        stdout,
        false,
        outbound.clone(),
    ));
    let stderr_handle = tokio::spawn(pipe_output(
        capability_id.to_string(),
        stderr,
        true,
        outbound.clone(),
    ));

    let mut stdout_handle = Some(stdout_handle);
    let mut stderr_handle = Some(stderr_handle);

    let mut inbound = endpoints.inbound;
    let start_time = tokio::time::Instant::now();
    let mut last_activity = start_time;
    let mut exit_tick = tokio::time::interval(tokio::time::Duration::from_millis(100));
    exit_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        let idle_deadline =
            idle_timeout.map(|secs| last_activity + tokio::time::Duration::from_secs(secs));
        let max_deadline =
            max_duration.map(|secs| start_time + tokio::time::Duration::from_secs(secs));
        let mut session_finished = false;

        tokio::select! {
            event = inbound.recv() => {
                match event {
                    Some(SessionClientEvent::Input { data, .. }) => {
                        if let Err(e) = stdin.write_all(&data).await {
                            warn!(
                                capability_id = %capability_id,
                                error = %e,
                                "Failed to write to interactive shell stdin"
                            );
                            let _ = child.kill().await;
                            let _ = outbound.send(SessionServerEvent::Exit {
                                exit_code: -1,
                                timed_out: false,
                                message: Some(format!("Failed to write to stdin: {}", e)),
                            }).await;
                            session_finished = true;
                        } else if let Err(e) = stdin.flush().await {
                            warn!(
                                capability_id = %capability_id,
                                error = %e,
                                "Failed to flush interactive shell stdin"
                            );
                            let _ = child.kill().await;
                            let _ = outbound.send(SessionServerEvent::Exit {
                                exit_code: -1,
                                timed_out: false,
                                message: Some(format!("Failed to flush stdin: {}", e)),
                            }).await;
                            session_finished = true;
                        } else {
                            last_activity = tokio::time::Instant::now();
                        }
                    }
                    Some(SessionClientEvent::Resize { .. }) => {
                        debug!(
                            capability_id = %capability_id,
                            "Received terminal resize event (PTY not yet supported)"
                        );
                    }
                    Some(SessionClientEvent::Heartbeat { timestamp_ms }) => {
                        let _ = outbound.send(SessionServerEvent::HeartbeatAck {
                            timestamp_ms,
                            latency_hint_ms: Some(0),
                        }).await;
                    }
                    Some(SessionClientEvent::Close { reason }) => {
                        info!(
                            capability_id = %capability_id,
                            ?reason,
                            "Interactive shell requested to close"
                        );
                        let _ = child.kill().await;
                        let _ = outbound.send(SessionServerEvent::Exit {
                            exit_code: -1,
                            timed_out: false,
                            message: reason,
                        }).await;
                        session_finished = true;
                    }
                    None => {
                        debug!(
                            capability_id = %capability_id,
                            "Session inbound channel closed"
                        );
                        let _ = child.kill().await;
                        let _ = outbound.send(SessionServerEvent::Exit {
                            exit_code: -1,
                            timed_out: false,
                            message: Some("Client disconnected".to_string()),
                        }).await;
                        session_finished = true;
                    }
                }
            }
            _ = exit_tick.tick() => {}
            _ = async {
                if let Some(deadline) = idle_deadline {
                    tokio::time::sleep_until(deadline).await;
                } else {
                    future::pending::<()>().await;
                }
            } => {
                if idle_deadline.is_some() {
                    warn!(
                        capability_id = %capability_id,
                        "Interactive shell idle timeout reached"
                    );
                    let _ = child.kill().await;
                    let _ = outbound
                        .send(SessionServerEvent::Exit {
                            exit_code: -1,
                            timed_out: true,
                            message: Some("Idle timeout reached".to_string()),
                        })
                        .await;
                    session_finished = true;
                }
            }
            _ = async {
                if let Some(deadline) = max_deadline {
                    tokio::time::sleep_until(deadline).await;
                } else {
                    future::pending::<()>().await;
                }
            } => {
                if max_deadline.is_some() {
                    warn!(
                        capability_id = %capability_id,
                        "Interactive shell maximum duration reached"
                    );
                    let _ = child.kill().await;
                    let _ = outbound
                        .send(SessionServerEvent::Exit {
                            exit_code: -1,
                            timed_out: true,
                            message: Some("Session duration limit exceeded".to_string()),
                        })
                        .await;
                    session_finished = true;
                }
            }
        }

        if session_finished {
            break;
        }

        if let Some(status) = child.try_wait().context("Failed to poll shell process")? {
            let exit_code = status.code().unwrap_or(-1);
            let _ = outbound
                .send(SessionServerEvent::Exit {
                    exit_code,
                    timed_out: false,
                    message: None,
                })
                .await;
            break;
        }
    }

    abort_handle(&mut stdout_handle).await;
    abort_handle(&mut stderr_handle).await;

    let _ = outbound
        .send(SessionServerEvent::Closed {
            reason: Some("Session terminated".to_string()),
        })
        .await;

    Ok(())
}

async fn pipe_output<R>(
    capability_id: String,
    mut reader: R,
    stderr: bool,
    outbound: tokio::sync::mpsc::Sender<SessionServerEvent>,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let mut buf = vec![0u8; 2048];
    loop {
        let bytes_read = reader.read(&mut buf).await?;
        if bytes_read == 0 {
            break;
        }
        let data = bytes::Bytes::copy_from_slice(&buf[..bytes_read]);
        if let Err(e) = outbound
            .send(SessionServerEvent::Output {
                data,
                stderr,
                binary: true,
                timestamp_ms: Some(current_timestamp_ms()),
            })
            .await
        {
            debug!(
                capability_id = %capability_id,
                error = %e,
                "Client disconnected while streaming output"
            );
            break;
        }
    }
    Ok(())
}

async fn abort_handle(handle: &mut Option<tokio::task::JoinHandle<Result<()>>>) {
    if let Some(task) = handle.take() {
        if !task.is_finished() {
            task.abort();
        }
        if let Err(err) = task.await {
            if !err.is_cancelled() {
                warn!("Output stream task ended with error: {}", err);
            }
        }
    }
}

fn current_timestamp_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

use super::{Capability, CapabilityMetadata, CapabilityOpenContext, SessionMode};
use crate::config::parser::{CapabilityConfig, CapabilityDefinition, ShellInteractiveDefinition};
use crate::sessions::{SessionClientEvent, SessionEndpoints, SessionServerEvent};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures_util::future;
#[cfg(not(unix))]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, info, warn};

#[cfg(not(unix))]
use tokio::process::Command;

#[cfg(not(unix))]
use anyhow::Context;

#[cfg(unix)]
use {
    libc,
    std::{
        ffi::CString,
        os::unix::io::{AsRawFd, RawFd},
        sync::Arc,
    },
    tokio::io::unix::AsyncFd,
    tokio::task,
};

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

#[cfg(unix)]
struct PtyMaster {
    fd: RawFd,
}

#[cfg(unix)]
impl AsRawFd for PtyMaster {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

#[cfg(unix)]
impl Drop for PtyMaster {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

#[cfg(unix)]
async fn run_interactive_shell(
    capability_id: &str,
    shell_path: String,
    working_dir: Option<String>,
    env: std::collections::HashMap<String, String>,
    idle_timeout: Option<u64>,
    max_duration: Option<u64>,
    endpoints: SessionEndpoints,
) -> Result<()> {
    use std::io;

    let mut env_pairs: Vec<(CString, CString)> = env
        .into_iter()
        .map(|(k, v)| Ok((CString::new(k)?, CString::new(v)?)))
        .collect::<Result<Vec<_>, anyhow::Error>>()?;

    if !env_pairs
        .iter()
        .any(|(k, _)| k.as_c_str().to_bytes() == b"TERM")
    {
        env_pairs.push((CString::new("TERM")?, CString::new("xterm-256color")?));
    }

    let working_dir_cstr = if let Some(dir) = working_dir {
        Some(CString::new(dir)?)
    } else {
        None
    };

    let mut argv = Vec::new();
    argv.push(CString::new(shell_path.clone())?);
    argv.push(CString::new("-i")?);

    let (master_fd, child_pid) = spawn_shell_with_pty(
        capability_id,
        &shell_path,
        &argv,
        working_dir_cstr.as_ref(),
        &env_pairs,
    )?;

    let master = Arc::new(master_fd);
    let outbound = endpoints.outbound.clone();

    let mut idle_timeout_secs = idle_timeout;
    let mut max_duration_secs = max_duration;

    outbound
        .send(SessionServerEvent::Ready {
            message: Some(format!("Shell started: {}", shell_path)),
        })
        .await
        .ok();

    let reader_master = Arc::clone(&master);
    let reader_outbound = outbound.clone();
    let capability_for_reader = capability_id.to_string();

    let reader_task = tokio::spawn(async move {
        let mut buffer = vec![0u8; 4096];
        loop {
            match pty_read(&reader_master, &mut buffer).await {
                Ok(0) => break,
                Ok(n) => {
                    let data = bytes::Bytes::copy_from_slice(&buffer[..n]);
                    if reader_outbound
                        .send(SessionServerEvent::Output {
                            data,
                            stderr: false,
                            binary: true,
                            timestamp_ms: Some(current_timestamp_ms()),
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(err) => {
                    if err.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    debug!(
                        capability_id = %capability_for_reader,
                        error = %err,
                        "PTY read error"
                    );
                    break;
                }
            }
        }
    });

    let (exit_tx, mut exit_rx) = tokio::sync::oneshot::channel::<ChildExit>();
    task::spawn(async move {
        let result = wait_for_child(child_pid).await;
        let _ = exit_tx.send(result);
    });

    let mut inbound = endpoints.inbound;
    let start_time = tokio::time::Instant::now();
    let mut last_activity = start_time;
    let mut exit_tick = tokio::time::interval(tokio::time::Duration::from_millis(100));
    exit_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut exit_metadata: Option<ExitMetadata> = None;

    loop {
        let idle_deadline =
            idle_timeout_secs.map(|secs| last_activity + tokio::time::Duration::from_secs(secs));
        let max_deadline =
            max_duration_secs.map(|secs| start_time + tokio::time::Duration::from_secs(secs));

        tokio::select! {
            exit = &mut exit_rx => {
                let exit = exit.unwrap_or_else(|_| ChildExit::default());
                let metadata = exit_metadata.unwrap_or_else(|| ExitMetadata {
                    timed_out: false,
                    message: exit.message.clone(),
                });

                outbound
                    .send(SessionServerEvent::Exit {
                        exit_code: exit.code,
                        timed_out: metadata.timed_out,
                        message: metadata.message.clone(),
                    })
                    .await
                    .ok();

                outbound
                    .send(SessionServerEvent::Closed {
                        reason: Some("Session terminated".to_string()),
                    })
                    .await
                    .ok();

                break;
            }
            event = inbound.recv() => {
                match event {
                    Some(SessionClientEvent::Input { data, .. }) => {
                        if let Err(err) = pty_write(&master, &data).await {
                            warn!(
                                capability_id = %capability_id,
                                error = %err,
                                "Failed to write to PTY"
                            );
                        } else {
                            last_activity = tokio::time::Instant::now();
                        }
                    }
                    Some(SessionClientEvent::Resize { cols, rows }) => {
                        if let Err(err) = resize_pty(&master, cols, rows) {
                            debug!(
                                capability_id = %capability_id,
                                error = %err,
                                "Failed to resize PTY"
                            );
                        }
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
                        exit_metadata = Some(ExitMetadata {
                            timed_out: false,
                            message: reason.clone(),
                        });
                        terminate_child(child_pid);
                    }
                    Some(SessionClientEvent::Resume { .. }) => {
                        debug!(
                            capability_id = %capability_id,
                            "Received resume event at capability layer; ignoring"
                        );
                    }
                    None => {
                        debug!(
                            capability_id = %capability_id,
                            "Session inbound channel closed"
                        );
                        exit_metadata = Some(ExitMetadata {
                            timed_out: false,
                            message: Some("Client disconnected".to_string()),
                        });
                        terminate_child(child_pid);
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
                    idle_timeout_secs = None;
                    exit_metadata = Some(ExitMetadata {
                        timed_out: true,
                        message: Some("Idle timeout reached".to_string()),
                    });
                    terminate_child(child_pid);
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
                    max_duration_secs = None;
                    exit_metadata = Some(ExitMetadata {
                        timed_out: true,
                        message: Some("Session duration limit exceeded".to_string()),
                    });
                    terminate_child(child_pid);
                }
            }
        }
    }

    drop(master);
    let _ = reader_task.await;

    Ok(())
}

#[cfg(unix)]
#[derive(Clone)]
struct ExitMetadata {
    timed_out: bool,
    message: Option<String>,
}

#[cfg(unix)]
#[derive(Clone)]
struct ChildExit {
    code: i32,
    message: Option<String>,
}

#[cfg(unix)]
impl Default for ChildExit {
    fn default() -> Self {
        Self {
            code: -1,
            message: Some("Process exited unexpectedly".to_string()),
        }
    }
}

#[cfg(unix)]
fn spawn_shell_with_pty(
    capability_id: &str,
    shell_display: &str,
    argv: &[CString],
    working_dir: Option<&CString>,
    env: &[(CString, CString)],
) -> Result<(AsyncFd<PtyMaster>, libc::pid_t)> {
    use std::io;

    let mut raw_args: Vec<*const libc::c_char> = argv.iter().map(|c| c.as_ptr()).collect();
    raw_args.push(std::ptr::null());

    let mut master_fd: libc::c_int = 0;
    let mut winsize = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    let pid = unsafe {
        libc::forkpty(
            &mut master_fd,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut winsize,
        )
    };

    if pid < 0 {
        let err = io::Error::last_os_error();
        return Err(anyhow!("forkpty failed: {}", err));
    }

    if pid == 0 {
        if let Some(dir) = working_dir {
            let _ = unsafe { libc::chdir(dir.as_ptr()) };
        }

        for (key, value) in env {
            unsafe {
                libc::setenv(key.as_ptr(), value.as_ptr(), 1);
            }
        }

        let result = unsafe { libc::execvp(argv[0].as_ptr(), raw_args.as_ptr()) };
        let err = io::Error::last_os_error();
        let _ = result;
        let shell_name = argv
            .get(0)
            .map(|c| c.as_c_str().to_string_lossy().into_owned())
            .unwrap_or_else(|| shell_display.to_string());
        eprintln!(
            "Interactive shell capability '{}' failed to exec '{}': {}",
            capability_id, shell_name, err
        );
        unsafe {
            libc::_exit(1);
        }
    }

    unsafe {
        let flags = libc::fcntl(master_fd, libc::F_GETFL);
        if flags != -1 {
            libc::fcntl(master_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }

    let master = AsyncFd::new(PtyMaster { fd: master_fd })
        .map_err(|e| anyhow!("Failed to wrap PTY descriptor: {}", e))?;

    Ok((master, pid))
}

#[cfg(unix)]
async fn pty_read(master: &AsyncFd<PtyMaster>, buf: &mut [u8]) -> std::io::Result<usize> {
    loop {
        let mut guard = master.readable().await?;
        match guard.try_io(|inner| {
            let fd = inner.as_raw_fd();
            let result = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
            if result < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(result as usize)
            }
        }) {
            Ok(res) => return res,
            Err(_would_block) => continue,
        }
    }
}

#[cfg(unix)]
async fn pty_write(master: &AsyncFd<PtyMaster>, data: &[u8]) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < data.len() {
        let mut guard = master.writable().await?;
        match guard.try_io(|inner| {
            let fd = inner.as_raw_fd();
            let slice = &data[offset..];
            let result = unsafe { libc::write(fd, slice.as_ptr() as *const _, slice.len()) };
            if result < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(result as usize)
            }
        }) {
            Ok(bytes_written_result) => {
                let written = bytes_written_result?;
                offset += written;
            }
            Err(_would_block) => continue,
        }
    }
    Ok(())
}

#[cfg(unix)]
fn resize_pty(master: &AsyncFd<PtyMaster>, cols: u32, rows: u32) -> std::io::Result<()> {
    let winsize = libc::winsize {
        ws_row: rows as u16,
        ws_col: cols as u16,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    let fd = master.get_ref().as_raw_fd();
    let result = unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &winsize) };
    if result == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn terminate_child(pid: libc::pid_t) {
    if pid > 0 {
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
    }
}

#[cfg(unix)]
async fn wait_for_child(pid: libc::pid_t) -> ChildExit {
    use std::io;

    task::spawn_blocking(move || {
        let mut status: libc::c_int = 0;
        loop {
            let result = unsafe { libc::waitpid(pid, &mut status, 0) };
            if result == -1 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return ChildExit {
                    code: -1,
                    message: Some(format!("waitpid failed: {}", err)),
                };
            }

            if libc::WIFEXITED(status) {
                let code = libc::WEXITSTATUS(status) as i32;
                return ChildExit {
                    code,
                    message: None,
                };
            } else if libc::WIFSIGNALED(status) {
                let signal = libc::WTERMSIG(status) as i32;
                return ChildExit {
                    code: -signal,
                    message: Some(format!("Process terminated by signal {}", signal)),
                };
            } else {
                return ChildExit::default();
            }
        }
    })
    .await
    .unwrap_or_else(|_| ChildExit {
        code: -1,
        message: Some("waitpid task cancelled".to_string()),
    })
}

#[cfg(not(unix))]
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
    #[cfg(unix)]
    {
        command.arg("-i");
    }
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
    let mut idle_timeout_secs = idle_timeout;
    let mut max_duration_secs = max_duration;

    loop {
        let idle_deadline =
            idle_timeout_secs.map(|secs| last_activity + tokio::time::Duration::from_secs(secs));
        let max_deadline =
            max_duration_secs.map(|secs| start_time + tokio::time::Duration::from_secs(secs));
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
                    idle_timeout_secs = None;
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
                    max_duration_secs = None;
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

#[cfg(not(unix))]
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

#[cfg(not(unix))]
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

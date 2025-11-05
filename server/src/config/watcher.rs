use anyhow::{Context, Result};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use super::broadcaster::{ConfigBroadcaster, ConfigUpdateNotification};
use super::parser::{Config, load_config};

/// Watches config file for changes and triggers hot-reload
pub struct ConfigWatcher {
    config_path: PathBuf,
    config: Arc<RwLock<Config>>,
    config_version: Arc<AtomicU64>,
    broadcaster: Arc<ConfigBroadcaster>,
    last_update: Arc<AtomicU64>,
}

impl ConfigWatcher {
    pub fn new(
        config_path: PathBuf,
        config: Arc<RwLock<Config>>,
        config_version: Arc<AtomicU64>,
        broadcaster: Arc<ConfigBroadcaster>,
        last_update: Arc<AtomicU64>,
    ) -> Self {
        Self {
            config_path,
            config,
            config_version,
            broadcaster,
            last_update,
        }
    }

    /// Start watching the config file for changes
    pub async fn start(self) -> Result<()> {
        info!(
            "Starting config file watcher: {}",
            self.config_path.display()
        );

        let (tx, mut rx) = mpsc::channel(100);

        // Create file watcher
        let mut watcher: RecommendedWatcher = Watcher::new(
            move |res: notify::Result<Event>| {
                if let Ok(event) = res {
                    let _ = tx.blocking_send(event);
                }
            },
            notify::Config::default(),
        )
        .context("Failed to create file watcher")?;

        // Watch the config file
        watcher
            .watch(&self.config_path, RecursiveMode::NonRecursive)
            .with_context(|| {
                format!(
                    "Failed to watch config file: {}",
                    self.config_path.display()
                )
            })?;

        info!("Config watcher initialized successfully");

        // Debounce timer - wait this long after last event before reloading
        let mut debounce_timer = tokio::time::interval(Duration::from_millis(500));
        debounce_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut pending_reload = false;

        loop {
            tokio::select! {
                Some(event) = rx.recv() => {
                    // Check if this is a modify event
                    if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                        pending_reload = true;
                    }
                }
                _ = debounce_timer.tick() => {
                    if pending_reload {
                        if let Err(e) = self.reload_config().await {
                            error!("Config reload failed: {}", e);
                        }
                        pending_reload = false;
                    }
                }
            }
        }
    }

    /// Reload the config file and broadcast update notification
    async fn reload_config(&self) -> Result<()> {
        info!("Config file changed, reloading...");

        // Parse new config
        let new_config = match load_config(&self.config_path) {
            Ok(cfg) => cfg,
            Err(e) => {
                error!("Config reload failed: {}. Keeping old config.", e);
                return Ok(()); // Don't propagate error - keep running with old config
            }
        };

        // Validate that only commands changed (enforce scope)
        {
            let old_config = self.config.read().unwrap();
            if !config_reload_allowed(&old_config, &new_config) {
                warn!(
                    "Config contains non-capability changes. These changes require server restart. \
                     Only capability modifications will be hot-reloaded in future versions."
                );
                // For now, we still allow the reload but warn the user
                // In the future, we could reject this reload
            }
        }

        // Atomic swap of config
        {
            let mut config_guard = self.config.write().unwrap();
            *config_guard = new_config;
        }

        // Increment version and get the new version
        let new_version = self.config_version.fetch_add(1, Ordering::SeqCst) + 1;

        // Update last_update timestamp
        let timestamp_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        self.last_update
            .store(timestamp_ms as u64, Ordering::SeqCst);

        info!("Config reloaded successfully. New version: {}", new_version);

        // Broadcast update notification
        self.broadcaster.broadcast(ConfigUpdateNotification {
            version: new_version,
            timestamp_ms,
        });

        Ok(())
    }
}

/// Check if config reload is allowed (only capability changes)
fn config_reload_allowed(old: &Config, new: &Config) -> bool {
    // For now, we're lenient - we allow any changes but warn about non-capability changes
    // In a stricter implementation, we would reject changes to server/security settings

    // Check if server settings changed
    let server_changed = old.server.port != new.server.port
        || old.server.bind_address != new.server.bind_address
        || old.server.mdns_service_name != new.server.mdns_service_name
        || old.server.mdns_instance_name != new.server.mdns_instance_name;

    // Check if security settings changed
    let security_changed = old.security.cert_path != new.security.cert_path
        || old.security.key_path != new.security.key_path
        || old.security.authorized_clients_dir != new.security.authorized_clients_dir
        || old.security.enrollment_token_ttl != new.security.enrollment_token_ttl
        || old.security.enrollment.qr_code_enabled != new.security.enrollment.qr_code_enabled
        || old.security.enrollment.approval_enabled != new.security.enrollment.approval_enabled
        || old.security.enrollment.approval_timeout_seconds
            != new.security.enrollment.approval_timeout_seconds
        || old.security.enrollment.approval_notification
            != new.security.enrollment.approval_notification;

    // Return true if only capabilities changed (server and security are unchanged)
    !server_changed && !security_changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_reload_allowed_commands_only() {
        let old_config = r#"
            [server]
            port = 50051
            bind_address = "::"

            [security]

            [[command]]
            id = "test"
            name = "Test"
            shell = "echo test"
        "#;

        let new_config = r#"
            [server]
            port = 50051
            bind_address = "::"

            [security]

            [[capabilities]]
            id = "test"
            name = "Test"
            kind = "shell_script"
            command = "echo test"

            [[capabilities]]
            id = "test2"
            name = "Test 2"
            kind = "shell_script"
            command = "echo test2"
        "#;

        let old = super::super::parser::load_config_from_str(old_config).unwrap();
        let new = super::super::parser::load_config_from_str(new_config).unwrap();

        assert!(config_reload_allowed(&old, &new));
    }

    #[test]
    fn test_config_reload_disallowed_server_change() {
        let old_config = r#"
            [server]
            port = 50051
            bind_address = "::"

            [security]
        "#;

        let new_config = r#"
            [server]
            port = 8080
            bind_address = "::"

            [security]
        "#;

        let old = super::super::parser::load_config_from_str(old_config).unwrap();
        let new = super::super::parser::load_config_from_str(new_config).unwrap();

        assert!(!config_reload_allowed(&old, &new));
    }

    #[test]
    fn test_config_reload_disallowed_security_change() {
        let old_config = r#"
            [server]
            port = 50051

            [security]
            enrollment_token_ttl = 300
        "#;

        let new_config = r#"
            [server]
            port = 50051

            [security]
            enrollment_token_ttl = 600
        "#;

        let old = super::super::parser::load_config_from_str(old_config).unwrap();
        let new = super::super::parser::load_config_from_str(new_config).unwrap();

        assert!(!config_reload_allowed(&old, &new));
    }
}

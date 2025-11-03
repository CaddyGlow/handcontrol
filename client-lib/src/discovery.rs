use crate::config::DiscoveryConfig;
use anyhow::{Context, Result};
use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent};
use serde::Serialize;
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};
use tracing::debug;
use uuid::Uuid;

const SERVICE_TYPE: &str = "_handcontrol._tcp.local.";

/// Result of an mDNS discovery pass.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredServer {
    pub instance_name: String,
    pub fullname: String,
    pub hostname: String,
    pub addresses: Vec<String>,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "std::collections::HashMap::is_empty", default)]
    pub txt_properties: HashMap<String, String>,
}

/// Discover HandControl servers announced via mDNS.
pub fn discover_servers(config: &DiscoveryConfig) -> Result<Vec<DiscoveredServer>> {
    let timeout_secs = config.timeout_seconds.max(1);
    let timeout = Duration::from_secs(timeout_secs);
    let deadline = Instant::now() + timeout;

    let mdns = ServiceDaemon::new().context("Failed to initialize mDNS daemon")?;
    let receiver = mdns
        .browse(SERVICE_TYPE)
        .context("Failed to start mDNS browse for HandControl service")?;

    let mut discovered: HashMap<String, DiscoveredServer> = HashMap::new();

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(remaining) {
            Ok(event) => handle_event(event, config, &mut discovered),
            Err(_) => break,
        }
    }

    if let Err(err) = mdns.shutdown() {
        debug!("Failed to shutdown mDNS daemon cleanly: {err}");
    }

    let mut entries: Vec<_> = discovered.into_values().collect();
    entries.sort_by(|a, b| a.instance_name.cmp(&b.instance_name));
    Ok(entries)
}

fn handle_event(
    event: ServiceEvent,
    config: &DiscoveryConfig,
    discovered: &mut HashMap<String, DiscoveredServer>,
) {
    match event {
        ServiceEvent::ServiceFound(_, _) => {
            // Resolution happens automatically; nothing to do.
        }
        ServiceEvent::ServiceResolved(resolved) => {
            if !resolved.is_valid() {
                return;
            }

            if let Some(server) = build_discovered_server(*resolved, config) {
                let key = server
                    .server_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| server.fullname.clone());
                discovered.insert(key, server);
            }
        }
        ServiceEvent::ServiceRemoved(_, fullname) => {
            discovered.retain(|_, entry| entry.fullname != fullname);
        }
        ServiceEvent::SearchStopped(_) | ServiceEvent::SearchStarted(_) => {}
        _ => {}
    }
}

fn build_discovered_server(
    resolved: ResolvedService,
    config: &DiscoveryConfig,
) -> Option<DiscoveredServer> {
    let fullname = resolved.get_fullname().to_string();
    let instance_name = extract_instance_name(&fullname);
    let hostname = resolved.get_hostname().trim_end_matches('.').to_string();

    let addresses = collect_addresses(&resolved, config);
    if addresses.is_empty() {
        return None;
    }

    let txt_properties = resolved.get_properties().clone().into_property_map_str();
    let server_id = txt_properties
        .get("server_id")
        .and_then(|raw| Uuid::parse_str(raw).ok());
    let cert_fingerprint = txt_properties.get("cert_fingerprint").cloned();

    let server = DiscoveredServer {
        instance_name,
        fullname,
        hostname,
        addresses,
        port: resolved.get_port(),
        server_id,
        cert_fingerprint,
        txt_properties,
    };

    Some(server)
}

fn collect_addresses(resolved: &ResolvedService, config: &DiscoveryConfig) -> Vec<String> {
    let mut addresses: Vec<IpAddr> = resolved
        .get_addresses()
        .iter()
        .map(|scoped| scoped.to_ip_addr())
        .filter(|ip| {
            if !config.include_link_local && is_link_local(ip) {
                return false;
            }
            !ip.is_loopback()
        })
        .collect();

    addresses.sort_by_key(|ip| {
        let preference = match (config.prefer_ipv6, ip) {
            (true, IpAddr::V6(_)) => 0,
            (true, IpAddr::V4(_)) => 1,
            (false, IpAddr::V4(_)) => 0,
            (false, IpAddr::V6(_)) => 1,
        };
        (preference, ip.to_string())
    });

    addresses.into_iter().map(|ip| ip.to_string()).collect()
}

fn is_link_local(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => v4.octets()[0] == 169 && v4.octets()[1] == 254,
        IpAddr::V6(v6) => v6.segments()[0] & 0xffc0 == 0xfe80,
    }
}

fn extract_instance_name(fullname: &str) -> String {
    fullname
        .strip_suffix(SERVICE_TYPE)
        .unwrap_or(fullname)
        .trim_end_matches('.')
        .to_string()
}

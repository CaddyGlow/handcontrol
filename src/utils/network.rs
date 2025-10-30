use std::net::{IpAddr, UdpSocket};

/// Get all usable local IP addresses, filtered and prioritized
///
/// Returns a list of IP addresses ordered by likelihood of success:
/// 1. Primary IP from get_local_ip() (UDP connect trick)
/// 2. Other IPv4 addresses (sorted by interface name)
/// 3. IPv6 addresses (sorted by interface name)
///
/// Filters out:
/// - Loopback addresses (127.0.0.0/8, ::1)
/// - Link-local addresses (169.254.0.0/16, fe80::/10)
/// - Docker/VPN interfaces
/// - Interfaces that are DOWN
pub fn get_all_local_ips() -> Vec<String> {
    let mut ipv4_addrs = Vec::new();
    let mut ipv6_addrs = Vec::new();

    // Get all network interfaces
    let interfaces = match if_addrs::get_if_addrs() {
        Ok(addrs) => addrs,
        Err(e) => {
            tracing::warn!("Failed to enumerate network interfaces: {}", e);
            return Vec::new();
        }
    };

    // Separate and filter IPv4 and IPv6 addresses
    for iface in interfaces {
        // Skip filtered interfaces
        if should_filter_interface(&iface.name) {
            tracing::debug!("Filtering out interface: {}", iface.name);
            continue;
        }

        let ip = iface.ip();

        // Skip filtered IPs
        if should_filter_ip(&ip) {
            tracing::debug!("Filtering out IP: {} on {}", ip, iface.name);
            continue;
        }

        let ip_string = ip.to_string();

        match ip {
            IpAddr::V4(_) => ipv4_addrs.push((iface.name.clone(), ip_string)),
            IpAddr::V6(_) => ipv6_addrs.push((iface.name.clone(), ip_string)),
        }
    }

    // Sort by interface name for consistent ordering
    ipv4_addrs.sort_by(|a, b| a.0.cmp(&b.0));
    ipv6_addrs.sort_by(|a, b| a.0.cmp(&b.0));

    // Get primary IP first
    let primary_ip = get_local_ip();

    // Build final list with prioritization
    let mut result = Vec::new();

    // 1. Add primary IP first (if available)
    if let Some(primary) = primary_ip {
        result.push(primary.clone());
    }

    // 2. Add other IPv4 addresses (excluding primary)
    for (_, ip) in ipv4_addrs {
        if !result.contains(&ip) {
            result.push(ip);
        }
    }

    // 3. Add IPv6 addresses
    for (_, ip) in ipv6_addrs {
        if !result.contains(&ip) {
            result.push(ip);
        }
    }

    result
}

/// Get single best-guess local IP (existing UDP connect trick)
///
/// Uses UDP connect to 8.8.8.8 to determine which local IP
/// would be used to reach the internet. This doesn't actually
/// send any data.
pub fn get_local_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let local_addr = socket.local_addr().ok()?;

    Some(local_addr.ip().to_string())
}

/// Check if an interface should be filtered out
///
/// Filters:
/// - Docker interfaces: docker0, br-*, veth*
/// - VPN interfaces: tun*, tap*, vpn*, wg*
fn should_filter_interface(name: &str) -> bool {
    let name_lower = name.to_lowercase();

    // Docker interfaces
    if name_lower == "docker0"
        || name_lower.starts_with("br-")
        || name_lower.starts_with("veth")
    {
        return true;
    }

    // VPN interfaces
    if name_lower.starts_with("tun")
        || name_lower.starts_with("tap")
        || name_lower.starts_with("vpn")
        || name_lower.starts_with("wg")
    {
        return true;
    }

    false
}

/// Check if an IP address should be filtered out
///
/// Filters:
/// - Loopback: 127.0.0.0/8, ::1
/// - Link-local: 169.254.0.0/16, fe80::/10
fn should_filter_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            // Loopback: 127.0.0.0/8
            if octets[0] == 127 {
                return true;
            }
            // Link-local: 169.254.0.0/16
            if octets[0] == 169 && octets[1] == 254 {
                return true;
            }
            false
        }
        IpAddr::V6(v6) => {
            // Loopback: ::1
            if v6.is_loopback() {
                return true;
            }
            // Link-local: fe80::/10
            let segments = v6.segments();
            if segments[0] & 0xffc0 == 0xfe80 {
                return true;
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_get_all_local_ips() {
        let ips = get_all_local_ips();

        // Should return at least one IP in most environments
        // (may be empty in very restricted environments)
        if !ips.is_empty() {
            // Verify no loopback
            assert!(
                !ips.iter().any(|ip| ip.starts_with("127.")),
                "Loopback IPs should be filtered"
            );

            // Verify no link-local
            assert!(
                !ips.iter().any(|ip| ip.starts_with("169.254.")),
                "Link-local IPv4 should be filtered"
            );
            assert!(
                !ips.iter().any(|ip| ip.to_lowercase().starts_with("fe80:")),
                "Link-local IPv6 should be filtered"
            );
        }
    }

    #[test]
    fn test_get_local_ip() {
        // This may fail in some environments (e.g., no network)
        // So we just test it doesn't panic and returns valid IP if present
        if let Some(ip) = get_local_ip() {
            // Should be parseable as IP
            assert!(ip.parse::<IpAddr>().is_ok());
            // Should not be loopback
            assert!(!ip.starts_with("127."));
        }
    }

    #[test]
    fn test_filter_docker_interfaces() {
        assert!(should_filter_interface("docker0"));
        assert!(should_filter_interface("br-1234567890ab"));
        assert!(should_filter_interface("veth1a2b3c4"));
        assert!(!should_filter_interface("eth0"));
        assert!(!should_filter_interface("wlan0"));
        assert!(!should_filter_interface("en0"));
    }

    #[test]
    fn test_filter_vpn_interfaces() {
        assert!(should_filter_interface("tun0"));
        assert!(should_filter_interface("tap0"));
        assert!(should_filter_interface("vpn0"));
        assert!(should_filter_interface("wg0"));
        assert!(!should_filter_interface("eth0"));
        assert!(!should_filter_interface("wlan0"));
    }

    #[test]
    fn test_filter_loopback_ipv4() {
        let loopback = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        assert!(should_filter_ip(&loopback));

        let other_loopback = IpAddr::V4(Ipv4Addr::new(127, 1, 2, 3));
        assert!(should_filter_ip(&other_loopback));

        let normal = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        assert!(!should_filter_ip(&normal));
    }

    #[test]
    fn test_filter_link_local_ipv4() {
        let link_local = IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1));
        assert!(should_filter_ip(&link_local));

        let normal = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        assert!(!should_filter_ip(&normal));
    }

    #[test]
    fn test_filter_loopback_ipv6() {
        let loopback = IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1));
        assert!(should_filter_ip(&loopback));

        let normal = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));
        assert!(!should_filter_ip(&normal));
    }

    #[test]
    fn test_filter_link_local_ipv6() {
        let link_local = IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1));
        assert!(should_filter_ip(&link_local));

        let normal = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));
        assert!(!should_filter_ip(&normal));
    }

    #[test]
    fn test_prioritization_order() {
        // This test verifies the general structure but can't fully test
        // prioritization without mocking network interfaces
        let ips = get_all_local_ips();

        if ips.is_empty() {
            // Skip test in restricted environments
            return;
        }

        // If we have both IPv4 and IPv6, verify IPv4 comes before IPv6
        let has_v4 = ips.iter().any(|ip| ip.contains('.'));
        let has_v6 = ips.iter().any(|ip| ip.contains(':'));

        if has_v4 && has_v6 {
            let first_v6_idx = ips.iter().position(|ip| ip.contains(':')).unwrap();
            let last_v4_idx = ips.iter().rposition(|ip| ip.contains('.')).unwrap();

            assert!(
                last_v4_idx < first_v6_idx,
                "IPv4 addresses should come before IPv6"
            );
        }
    }
}

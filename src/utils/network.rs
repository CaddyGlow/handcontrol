use std::net::{IpAddr, Ipv6Addr, UdpSocket};

/// IPv6 address scope classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ipv6AddressScope {
    LinkLocal,      // fe80::/10
    UniqueLocal,    // fc00::/7 (ULA)
    Global,         // 2000::/3
    Multicast,      // ff00::/8
    Other,
}

impl Ipv6AddressScope {
    pub fn from_address(addr: &Ipv6Addr) -> Self {
        let segments = addr.segments();

        // Link-local: fe80::/10
        if (segments[0] & 0xffc0) == 0xfe80 {
            return Self::LinkLocal;
        }

        // Unique Local Address: fc00::/7
        if (segments[0] & 0xfe00) == 0xfc00 {
            return Self::UniqueLocal;
        }

        // Global unicast: 2000::/3
        if (segments[0] & 0xe000) == 0x2000 {
            return Self::Global;
        }

        // Multicast: ff00::/8
        if (segments[0] & 0xff00) == 0xff00 {
            return Self::Multicast;
        }

        Self::Other
    }
}

/// Check if IPv6 address appears to be temporary (privacy extensions)
///
/// This is a heuristic check. For accurate detection, OS-specific APIs would be needed.
pub fn is_ipv6_temporary(addr: &Ipv6Addr) -> bool {
    // Temporary addresses (RFC 4941) have random interface identifiers
    // EUI-64 addresses have 0xfffe in the middle (segments[5])
    let segments = addr.segments();
    let iid = [segments[4], segments[5], segments[6], segments[7]];

    // If not EUI-64, likely temporary
    iid[1] != 0xfffe
}

/// Address priority for sorting
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct AddressPriority {
    score: u8,
    is_temporary: bool,
}

impl AddressPriority {
    fn calculate(addr: &IpAddr) -> Self {
        let (score, is_temporary) = match addr {
            IpAddr::V4(ipv4) => {
                if is_public_ipv4(ipv4) {
                    (80, false)
                } else {
                    (40, false) // Private IPv4
                }
            }
            IpAddr::V6(ipv6) => {
                let scope = Ipv6AddressScope::from_address(ipv6);
                let is_temp = is_ipv6_temporary(ipv6);

                let base_score = match scope {
                    Ipv6AddressScope::Global => 100,
                    Ipv6AddressScope::UniqueLocal => 60,
                    Ipv6AddressScope::LinkLocal => 20,
                    _ => 0,
                };

                (base_score, is_temp)
            }
        };

        Self { score, is_temporary }
    }
}

/// Check if IPv4 address is public
fn is_public_ipv4(addr: &std::net::Ipv4Addr) -> bool {
    !addr.is_private()
        && !addr.is_loopback()
        && !addr.is_link_local()
        && !addr.is_broadcast()
        && !addr.is_documentation()
}

/// Get all usable local IP addresses, filtered and prioritized
///
/// Returns a list of IP addresses ordered by priority:
/// 1. Global IPv6 (2000::/3) - Score: 100
/// 2. Public IPv4 - Score: 80
/// 3. ULA IPv6 (fc00::/7) - Score: 60
/// 4. Private IPv4 (RFC1918) - Score: 40
/// 5. Link-local IPv6 (fe80::/10) - Score: 20
///
/// Within each priority tier, stable addresses are preferred over temporary.
///
/// Filters out:
/// - Loopback addresses (127.0.0.0/8, ::1)
/// - Link-local IPv4 (169.254.0.0/16)
/// - Docker/VPN interfaces
/// - Multicast and other special addresses
pub fn get_all_local_ips() -> Vec<String> {
    get_all_local_ips_with_options(&NetworkOptions::default())
}

/// Network enumeration options
#[derive(Debug, Clone)]
pub struct NetworkOptions {
    pub include_link_local_ipv6: bool,
    pub prefer_stable_addresses: bool,
    pub max_addresses: Option<usize>,
}

impl Default for NetworkOptions {
    fn default() -> Self {
        Self {
            include_link_local_ipv6: false,
            prefer_stable_addresses: true,
            max_addresses: Some(5),
        }
    }
}

/// Get all usable local IP addresses with custom options
pub fn get_all_local_ips_with_options(options: &NetworkOptions) -> Vec<String> {
    let mut addresses = Vec::new();

    // Get all network interfaces
    let interfaces = match if_addrs::get_if_addrs() {
        Ok(addrs) => addrs,
        Err(e) => {
            tracing::warn!("Failed to enumerate network interfaces: {}", e);
            return Vec::new();
        }
    };

    // Collect addresses with priority scores
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

        // Skip link-local IPv6 if not included
        if !options.include_link_local_ipv6 {
            if let IpAddr::V6(v6) = ip {
                if Ipv6AddressScope::from_address(&v6) == Ipv6AddressScope::LinkLocal {
                    tracing::debug!("Filtering out link-local IPv6: {}", ip);
                    continue;
                }
            }
        }

        let priority = AddressPriority::calculate(&ip);
        addresses.push((ip, priority));
    }

    // Sort by priority (higher score first, stable addresses before temporary)
    addresses.sort_by(|a, b| {
        // Reverse score for descending order
        b.1.score
            .cmp(&a.1.score)
            // For same score, prefer stable over temporary
            .then_with(|| a.1.is_temporary.cmp(&b.1.is_temporary))
            // For same priority and stability, use string representation for consistency
            .then_with(|| a.0.to_string().cmp(&b.0.to_string()))
    });

    // Apply max addresses limit
    let addresses: Vec<IpAddr> = addresses.into_iter().map(|(ip, _)| ip).collect();
    let addresses = if let Some(max) = options.max_addresses {
        addresses.into_iter().take(max).collect()
    } else {
        addresses
    };

    // Convert to strings
    addresses.into_iter().map(|ip| ip.to_string()).collect()
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
        // This test verifies the general structure
        let ips = get_all_local_ips();

        if ips.is_empty() {
            // Skip test in restricted environments
            return;
        }

        // Verify max addresses limit is applied
        assert!(ips.len() <= 5, "Should not exceed max addresses limit");
    }

    #[test]
    fn test_ipv6_scope_detection() {
        // Global
        let global = "2001:db8::1".parse::<Ipv6Addr>().unwrap();
        assert_eq!(
            Ipv6AddressScope::from_address(&global),
            Ipv6AddressScope::Global
        );

        // Link-local
        let link_local = "fe80::1".parse::<Ipv6Addr>().unwrap();
        assert_eq!(
            Ipv6AddressScope::from_address(&link_local),
            Ipv6AddressScope::LinkLocal
        );

        // ULA
        let ula = "fc00::1".parse::<Ipv6Addr>().unwrap();
        assert_eq!(
            Ipv6AddressScope::from_address(&ula),
            Ipv6AddressScope::UniqueLocal
        );

        // Multicast
        let multicast = "ff02::1".parse::<Ipv6Addr>().unwrap();
        assert_eq!(
            Ipv6AddressScope::from_address(&multicast),
            Ipv6AddressScope::Multicast
        );
    }

    #[test]
    fn test_address_priority_calculation() {
        // Global IPv6 should have highest priority
        let global_v6 = "2001:db8::1".parse::<IpAddr>().unwrap();
        let priority_v6 = AddressPriority::calculate(&global_v6);
        assert_eq!(priority_v6.score, 100);

        // Public IPv4
        let public_v4 = "8.8.8.8".parse::<IpAddr>().unwrap();
        let priority_pub = AddressPriority::calculate(&public_v4);
        assert_eq!(priority_pub.score, 80);

        // ULA IPv6
        let ula_v6 = "fc00::1".parse::<IpAddr>().unwrap();
        let priority_ula = AddressPriority::calculate(&ula_v6);
        assert_eq!(priority_ula.score, 60);

        // Private IPv4
        let private_v4 = "192.168.1.1".parse::<IpAddr>().unwrap();
        let priority_priv = AddressPriority::calculate(&private_v4);
        assert_eq!(priority_priv.score, 40);

        // Link-local IPv6
        let link_local = "fe80::1".parse::<IpAddr>().unwrap();
        let priority_ll = AddressPriority::calculate(&link_local);
        assert_eq!(priority_ll.score, 20);
    }

    #[test]
    fn test_network_options() {
        let options = NetworkOptions {
            include_link_local_ipv6: true,
            prefer_stable_addresses: true,
            max_addresses: Some(10),
        };

        let ips = get_all_local_ips_with_options(&options);

        // Should not exceed max
        assert!(ips.len() <= 10);
    }

    #[test]
    fn test_is_public_ipv4() {
        // Public IPs
        assert!(is_public_ipv4(&"8.8.8.8".parse().unwrap()));
        assert!(is_public_ipv4(&"1.1.1.1".parse().unwrap()));

        // Private IPs
        assert!(!is_public_ipv4(&"192.168.1.1".parse().unwrap()));
        assert!(!is_public_ipv4(&"10.0.0.1".parse().unwrap()));
        assert!(!is_public_ipv4(&"172.16.0.1".parse().unwrap()));

        // Loopback
        assert!(!is_public_ipv4(&"127.0.0.1".parse().unwrap()));

        // Link-local
        assert!(!is_public_ipv4(&"169.254.1.1".parse().unwrap()));
    }
}

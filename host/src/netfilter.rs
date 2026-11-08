//! Peer address allow rules: loopback, Tailscale, and (with --allow-lan) private LAN ranges.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// 100.64.0.0/10 (CGNAT range used by Tailscale).
pub fn is_tailscale_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (o[1] & 0xC0) == 64
}

/// Tailscale IPv6 ULA prefix fd7a:115c:a1e0::/48.
pub fn is_tailscale_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0
}

pub fn is_private_lan_v4(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_link_local()
}

pub fn is_private_lan_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80
}

/// Whether a connection from `ip` may be served.
pub fn is_allowed(ip: IpAddr, allow_lan: bool) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => {
            v4.is_loopback() || is_tailscale_v4(v4) || (allow_lan && is_private_lan_v4(v4))
        }
        IpAddr::V6(v6) => {
            v6.is_loopback() || is_tailscale_v6(v6) || (allow_lan && is_private_lan_v6(v6))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn loopback_always_allowed() {
        assert!(is_allowed(ip("127.0.0.1"), false));
        assert!(is_allowed(ip("127.5.5.5"), false));
        assert!(is_allowed(ip("::1"), false));
        assert!(is_allowed(ip("::ffff:127.0.0.1"), false));
    }

    #[test]
    fn tailscale_range_boundaries() {
        assert!(is_allowed(ip("100.64.0.0"), false));
        assert!(is_allowed(ip("100.101.102.103"), false));
        assert!(is_allowed(ip("100.127.255.255"), false));
        assert!(!is_allowed(ip("100.63.255.255"), false));
        assert!(!is_allowed(ip("100.128.0.0"), false));
        assert!(is_allowed(ip("fd7a:115c:a1e0::1"), false));
        assert!(!is_allowed(ip("fd7a:115c:a1e1::1"), false));
    }

    #[test]
    fn lan_only_with_flag() {
        for a in ["192.168.1.20", "10.0.0.5", "172.16.0.1", "172.31.255.255", "169.254.1.1"] {
            assert!(!is_allowed(ip(a), false), "{a}");
            assert!(is_allowed(ip(a), true), "{a}");
        }
        assert!(!is_allowed(ip("172.32.0.1"), true));
        assert!(is_allowed(ip("fe80::1"), true));
        assert!(is_allowed(ip("fc00::1"), true));
        assert!(!is_allowed(ip("fe80::1"), false));
    }

    #[test]
    fn public_never_allowed() {
        for a in ["8.8.8.8", "1.1.1.1", "203.0.113.9", "2001:db8::1"] {
            assert!(!is_allowed(ip(a), false), "{a}");
            assert!(!is_allowed(ip(a), true), "{a}");
        }
    }
}

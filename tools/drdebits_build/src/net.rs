//! What the network commands share: URL parts as `urllib.parse` reads them,
//! `ipaddress`'s idea of a global address, and percent-quoting.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// `urllib.parse.urlsplit(url)`'s scheme, hostname and port text.
pub struct Split {
    pub scheme: String,
    pub hostname: Option<String>,
    pub port: Option<String>,
}

pub fn urlsplit(url: &str) -> Option<Split> {
    let (scheme, rest) = match url.find(':') {
        Some(i)
            if i > 0
                && url.as_bytes()[0].is_ascii_alphabetic()
                && url[..i]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) =>
        {
            (url[..i].to_ascii_lowercase(), &url[i + 1..])
        }
        _ => (String::new(), url),
    };
    let Some(after) = rest.strip_prefix("//") else {
        return Some(Split {
            scheme,
            hostname: None,
            port: None,
        });
    };
    let end = after.find(['/', '?', '#']).unwrap_or(after.len());
    let netloc = &after[..end];
    let host_port = netloc.rsplit_once('@').map_or(netloc, |(_, h)| h);
    let (host, port) = if let Some(bracketed) = host_port.strip_prefix('[') {
        // An unclosed bracket makes urlsplit raise "Invalid IPv6 URL".
        let close = bracketed.find(']')?;
        let rest = &bracketed[close + 1..];
        (&bracketed[..close], rest.strip_prefix(':'))
    } else {
        match host_port.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (host_port, None),
        }
    };
    let hostname = (!host.is_empty()).then(|| host.to_lowercase());
    Some(Split {
        scheme,
        hostname,
        port: port.map(str::to_string),
    })
}

/// A port `urllib.parse` refuses with ValueError.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadPort;

/// `SplitResult.port`: None for no port, Err where Python raises ValueError.
pub fn port_number(port: &Option<String>) -> Result<Option<u16>, BadPort> {
    match port {
        None => Ok(None),
        Some(p) if p.is_empty() => Ok(None),
        Some(p) => {
            if !p.chars().all(|c| c.is_ascii_digit()) {
                return Err(BadPort);
            }
            p.parse::<u32>()
                .ok()
                .filter(|n| *n <= 65535)
                .map(|n| Some(n as u16))
                .ok_or(BadPort)
        }
    }
}

fn v4_in(ip: Ipv4Addr, net: [u8; 4], prefix: u32) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    u32::from(ip) & mask == u32::from(Ipv4Addr::from(net)) & mask
}

fn v6_in(ip: Ipv6Addr, net: &str, prefix: u32) -> bool {
    let net: Ipv6Addr = net.parse().expect("a valid network");
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - prefix)
    };
    u128::from(ip) & mask == u128::from(net) & mask
}

fn v4_private(ip: Ipv4Addr) -> bool {
    const NETS: [([u8; 4], u32); 14] = [
        ([0, 0, 0, 0], 8),
        ([10, 0, 0, 0], 8),
        ([127, 0, 0, 0], 8),
        ([169, 254, 0, 0], 16),
        ([172, 16, 0, 0], 12),
        ([192, 0, 0, 0], 24),
        ([192, 0, 0, 170], 31),
        ([192, 0, 2, 0], 24),
        ([192, 168, 0, 0], 16),
        ([198, 18, 0, 0], 15),
        ([198, 51, 100, 0], 24),
        ([203, 0, 113, 0], 24),
        ([240, 0, 0, 0], 4),
        ([255, 255, 255, 255], 32),
    ];
    const EXCEPTIONS: [([u8; 4], u32); 2] = [([192, 0, 0, 9], 32), ([192, 0, 0, 10], 32)];
    NETS.iter().any(|(n, p)| v4_in(ip, *n, *p))
        && EXCEPTIONS.iter().all(|(n, p)| !v4_in(ip, *n, *p))
}

/// `ipaddress.ip_address(ip).is_global`, as Python 3.15 defines it.
pub fn is_global(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => !v4_in(v4, [100, 64, 0, 0], 10) && !v4_private(v4),
        IpAddr::V6(v6) => {
            if u128::from(v6) >> 32 == 0xFFFF {
                let mapped = Ipv4Addr::from((u128::from(v6) & 0xFFFF_FFFF) as u32);
                return is_global(IpAddr::V4(mapped));
            }
            const NETS: [(&str, u32); 11] = [
                ("::1", 128),
                ("::", 128),
                ("::ffff:0:0", 96),
                ("64:ff9b:1::", 48),
                ("100::", 64),
                ("2001::", 23),
                ("2001:db8::", 32),
                ("2002::", 16),
                ("3fff::", 20),
                ("fc00::", 7),
                ("fe80::", 10),
            ];
            const EXCEPTIONS: [(&str, u32); 6] = [
                ("2001:1::1", 128),
                ("2001:1::2", 128),
                ("2001:3::", 32),
                ("2001:4:112::", 48),
                ("2001:20::", 28),
                ("2001:30::", 28),
            ];
            let private = NETS.iter().any(|(n, p)| v6_in(v6, n, *p))
                && EXCEPTIONS.iter().all(|(n, p)| !v6_in(v6, n, *p));
            !private
        }
    }
}

/// `urllib.parse.quote(text)`: everything but letters, digits, `_.-~` and `/` is escaped.
pub fn quote(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"_.-~/".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_addresses_follow_python() {
        for (ip, global) in [
            ("8.8.8.8", true),
            ("10.1.2.3", false),
            ("100.64.0.1", false),
            ("192.0.0.9", true),
            ("224.0.0.1", true),
            ("::1", false),
            ("::ffff:10.0.0.1", false),
            ("2001:4:112::1", true),
            ("2606:4700::1111", true),
        ] {
            assert_eq!(is_global(ip.parse().unwrap()), global, "{ip}");
        }
    }

    #[test]
    fn urlsplit_reads_hosts_and_ports() {
        let s = urlsplit("HTTPS://User@Example.COM:8443/x").unwrap();
        assert_eq!(
            (s.scheme.as_str(), s.hostname.as_deref()),
            ("https", Some("example.com"))
        );
        assert_eq!(port_number(&s.port), Ok(Some(8443)));
        assert_eq!(port_number(&Some("99999".into())), Err(BadPort));
        assert_eq!(
            quote("titleId eq 'C1' and x:y"),
            "titleId%20eq%20%27C1%27%20and%20x%3Ay"
        );
    }
}

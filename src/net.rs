use std::net::{IpAddr, Ipv4Addr};

pub fn is_lan_ipv4(address: Ipv4Addr) -> bool {
    !address.is_loopback()
        && !address.is_link_local()
        && !address.is_unspecified()
        && !address.is_broadcast()
        && !address.is_multicast()
}

pub fn lan_ipv4_addresses() -> Vec<Ipv4Addr> {
    let mut addresses = if_addrs::get_if_addrs()
        .map(|interfaces| {
            interfaces
                .into_iter()
                .filter_map(|interface| match interface.ip() {
                    IpAddr::V4(address) if is_lan_ipv4(address) => Some(address),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    addresses.sort();
    addresses.dedup();
    addresses
}

pub fn primary_ipv4() -> Option<Ipv4Addr> {
    lan_ipv4_addresses().into_iter().next()
}

pub fn access_urls(port: u16) -> Vec<String> {
    let mut addresses = lan_ipv4_addresses();
    if addresses.is_empty() {
        addresses.push(Ipv4Addr::LOCALHOST);
    }
    addresses
        .into_iter()
        .map(|address| format!("http://{address}:{port}/"))
        .collect()
}

pub fn primary_url(port: u16) -> String {
    primary_ipv4()
        .map(|address| format!("http://{address}:{port}/"))
        .unwrap_or_else(|| format!("http://127.0.0.1:{port}/"))
}

pub fn admin_url(port: u16) -> String {
    format!("{}admin", primary_url(port))
}

pub fn format_bytes(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GIB {
        format!("{:.1} GiB", bytes / GIB)
    } else {
        format!("{:.0} MiB", bytes / MIB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_non_lan_addresses() {
        assert!(is_lan_ipv4(Ipv4Addr::new(192, 168, 1, 2)));
        assert!(is_lan_ipv4(Ipv4Addr::new(10, 0, 0, 1)));
        assert!(!is_lan_ipv4(Ipv4Addr::LOCALHOST));
        assert!(!is_lan_ipv4(Ipv4Addr::new(169, 254, 1, 1)));
        assert!(!is_lan_ipv4(Ipv4Addr::UNSPECIFIED));
    }

    #[test]
    fn formats_human_sizes() {
        assert_eq!(format_bytes(1024 * 1024), "1 MiB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.0 GiB");
    }
}

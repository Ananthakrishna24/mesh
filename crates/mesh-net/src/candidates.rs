use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use mesh_core::{
    CandidateKind, EndpointCandidate, filter_advertised_candidates, now_unix_ms,
    sort_candidates_for_dial,
};

use crate::mapping::MappingResult;

pub fn collect_local_candidates(listen_addr: SocketAddr) -> Vec<EndpointCandidate> {
    collect_local_candidates_at(listen_addr, now_unix_ms())
}

pub fn collect_local_candidates_at(
    listen_addr: SocketAddr,
    observed_at_unix_ms: i64,
) -> Vec<EndpointCandidate> {
    let port = listen_addr.port();
    let supports_ipv4 = listen_addr.is_ipv4() || listen_addr.ip().is_unspecified();
    let supports_ipv6 = listen_addr.is_ipv6();
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();

    if supports_ipv4 {
        push_unique(
            &mut candidates,
            &mut seen,
            EndpointCandidate::new_at(
                CandidateKind::LocalNetwork,
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
                observed_at_unix_ms,
            ),
        );
    }
    if supports_ipv6 {
        push_unique(
            &mut candidates,
            &mut seen,
            EndpointCandidate::new_at(
                CandidateKind::LocalNetwork,
                SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port),
                observed_at_unix_ms,
            ),
        );
    }

    for ip in interface_addresses() {
        let ip = normalize_bind_ip(ip);
        if (ip.is_ipv4() && !supports_ipv4) || (ip.is_ipv6() && !supports_ipv6) {
            continue;
        }
        if matches!(ip, IpAddr::V4(v4) if v4.is_loopback())
            || matches!(ip, IpAddr::V6(v6) if v6.is_loopback())
        {
            continue;
        }
        let kind = match ip {
            IpAddr::V6(v6) if is_global_v6(&v6) => CandidateKind::GlobalIpv6,
            IpAddr::V4(v4) if is_public_v4(v4) => CandidateKind::PublicIpv4,
            _ => CandidateKind::LocalNetwork,
        };
        push_unique(
            &mut candidates,
            &mut seen,
            EndpointCandidate::new_at(kind, SocketAddr::new(ip, port), observed_at_unix_ms),
        );
    }

    sort_candidates_for_dial(&mut candidates);
    candidates
}

pub fn with_router_mapping(
    mut candidates: Vec<EndpointCandidate>,
    mapping: &MappingResult,
) -> Vec<EndpointCandidate> {
    let mut seen = candidates
        .iter()
        .map(|candidate| candidate.address)
        .collect::<BTreeSet<_>>();
    push_unique(&mut candidates, &mut seen, mapping.candidate());
    sort_candidates_for_dial(&mut candidates);
    candidates
}

pub fn with_manual_candidate(
    mut candidates: Vec<EndpointCandidate>,
    address: SocketAddr,
) -> Vec<EndpointCandidate> {
    let address = mesh_core::normalize_candidate_address(address);
    if address.port() == 0 {
        return candidates;
    }
    let mut seen = candidates
        .iter()
        .map(|candidate| candidate.address)
        .collect::<BTreeSet<_>>();
    push_unique(
        &mut candidates,
        &mut seen,
        EndpointCandidate::new(CandidateKind::Manual, address),
    );
    sort_candidates_for_dial(&mut candidates);
    candidates
}

pub fn with_peer_observed(
    mut candidates: Vec<EndpointCandidate>,
    address: SocketAddr,
    source_node_id: mesh_core::NodeId,
) -> Vec<EndpointCandidate> {
    let address = mesh_core::normalize_candidate_address(address);
    if address.port() == 0 {
        return candidates;
    }
    let mut seen = candidates
        .iter()
        .map(|candidate| candidate.address)
        .collect::<BTreeSet<_>>();
    push_unique(
        &mut candidates,
        &mut seen,
        EndpointCandidate::new(CandidateKind::PeerObserved, address).with_source(source_node_id),
    );
    sort_candidates_for_dial(&mut candidates);
    candidates
}

pub fn advertised_candidates(candidates: &[EndpointCandidate]) -> Vec<EndpointCandidate> {
    filter_advertised_candidates(candidates, now_unix_ms())
}

fn normalize_bind_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(v4) if v4.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(v6) if v6.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        other => other,
    }
}

fn push_unique(
    candidates: &mut Vec<EndpointCandidate>,
    seen: &mut BTreeSet<SocketAddr>,
    candidate: EndpointCandidate,
) {
    let address = mesh_core::normalize_candidate_address(candidate.address);
    if seen.insert(address) {
        let mut candidate = candidate;
        candidate.address = address;
        candidates.push(candidate);
    }
}

fn interface_addresses() -> Vec<IpAddr> {
    let mut out = BTreeSet::new();
    for probe in ["8.8.8.8:80", "[2001:4860:4860::8888]:80"] {
        if let Ok(socket) = std::net::UdpSocket::bind(bind_for(probe)) {
            if socket.connect(probe).is_ok() {
                if let Ok(local) = socket.local_addr() {
                    out.insert(normalize_bind_ip(local.ip()));
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "lo" || name.starts_with("docker") || name.starts_with("veth") {
                continue;
            }
            if let Ok(socket) = std::net::UdpSocket::bind((format!("{name}"), 0)) {
                let _ = socket;
            }
        }
    }

    // Also probe common LAN gateways so multi-homed hosts surface private IPv4.
    for remote in [
        Ipv4Addr::new(1, 1, 1, 1),
        Ipv4Addr::new(8, 8, 8, 8),
        Ipv4Addr::new(9, 9, 9, 9),
    ] {
        if let Ok(socket) = std::net::UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))) {
            if socket.connect(SocketAddr::from((remote, 80))).is_ok() {
                if let Ok(local) = socket.local_addr() {
                    out.insert(normalize_bind_ip(local.ip()));
                }
            }
        }
    }

    out.into_iter().collect()
}

fn bind_for(probe: &str) -> &'static str {
    if probe.starts_with('[') {
        "[::]:0"
    } else {
        "0.0.0.0:0"
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || is_cgnat(ip)
        || is_benchmark(ip))
}

fn is_cgnat(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (octets[1] & 0b1100_0000) == 64
}

fn is_benchmark(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 198 && (octets[1] == 18 || octets[1] == 19)
}

fn is_global_v6(ip: &Ipv6Addr) -> bool {
    let segments = ip.segments();
    let is_ula = (segments[0] & 0xfe00) == 0xfc00;
    let is_link_local = (segments[0] & 0xffc0) == 0xfe80;
    !(ip.is_loopback() || ip.is_unspecified() || is_ula || is_link_local || ip.is_multicast())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_match_bound_address_families() {
        let ipv4 = collect_local_candidates(SocketAddr::from(([0, 0, 0, 0], 4_444)));
        assert!(ipv4.iter().all(|candidate| candidate.address.is_ipv4()));
        assert!(
            ipv4.windows(2)
                .all(|pair| { pair[0].kind.priority() >= pair[1].kind.priority() })
        );

        let dual_stack = collect_local_candidates(SocketAddr::from((Ipv6Addr::UNSPECIFIED, 4_444)));
        assert!(dual_stack.iter().any(|candidate| {
            candidate.address == SocketAddr::from((Ipv4Addr::LOCALHOST, 4_444))
        }));
        assert!(dual_stack.iter().any(|candidate| {
            candidate.address == SocketAddr::from((Ipv6Addr::LOCALHOST, 4_444))
        }));
    }
}

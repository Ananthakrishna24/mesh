use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use mesh_core::{CandidateKind, EndpointCandidate};

pub fn collect_local_candidates(listen_addr: SocketAddr) -> Vec<EndpointCandidate> {
    let port = listen_addr.port();
    let mut candidates = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    push_unique(
        &mut candidates,
        &mut seen,
        EndpointCandidate::new(
            CandidateKind::LocalNetwork,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        ),
    );

    if let Ok(interfaces) = local_ip_addresses() {
        for ip in interfaces {
            let ip = normalize_bind_ip(ip);
            let kind = match ip {
                IpAddr::V6(v6) if is_global_v6(&v6) => CandidateKind::GlobalIpv6,
                IpAddr::V4(v4) if is_public_v4(v4) => CandidateKind::PublicIpv4,
                _ => CandidateKind::LocalNetwork,
            };
            push_unique(
                &mut candidates,
                &mut seen,
                EndpointCandidate::new(kind, SocketAddr::new(ip, port)),
            );
        }
    }

    candidates.sort_by(|left, right| right.priority.cmp(&left.priority));
    candidates
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
    seen: &mut std::collections::BTreeSet<SocketAddr>,
    candidate: EndpointCandidate,
) {
    if seen.insert(candidate.address) {
        candidates.push(candidate);
    }
}

fn local_ip_addresses() -> std::io::Result<Vec<IpAddr>> {
    let mut out = Vec::new();
    for probe in ["8.8.8.8:80", "[2001:4860:4860::8888]:80"] {
        if let Ok(socket) = std::net::UdpSocket::bind(bind_for(probe)) {
            if socket.connect(probe).is_ok() {
                if let Ok(local) = socket.local_addr() {
                    out.push(normalize_bind_ip(local.ip()));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
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
        || ip.is_unspecified())
}

fn is_global_v6(ip: &Ipv6Addr) -> bool {
    let segments = ip.segments();
    let is_ula = (segments[0] & 0xfe00) == 0xfc00;
    let is_link_local = (segments[0] & 0xffc0) == 0xfe80;
    !(ip.is_loopback() || ip.is_unspecified() || is_ula || is_link_local || ip.is_multicast())
}

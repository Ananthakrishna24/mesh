use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;
use std::time::{Duration, Instant};

use crab_nat::{
    GatewayAddress, InternetProtocol, PortMapping, PortMappingOptions, PortMappingType,
    TimeoutConfig, natpmp,
};
use igd_next::PortMappingProtocol;
use igd_next::aio::tokio as igd_tokio;
use mesh_core::{CandidateKind, EndpointCandidate, now_unix_ms};
use tokio::time::timeout;
use tracing::{info, warn};

pub const MAPPING_LIFETIME_SECS: u32 = 7_200;
pub const PROTOCOL_DEADLINE: Duration = Duration::from_secs(2);
pub const MAPPING_BUDGET: Duration = Duration::from_secs(6);
#[allow(dead_code)]
pub const RENEW_RETRY_DELAY: Duration = Duration::from_secs(30);


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingProtocol {
    Pcp,
    NatPmp,
    Upnp,
}

impl MappingProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pcp => "pcp",
            Self::NatPmp => "nat-pmp",
            Self::Upnp => "upnp",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MappingResult {
    pub external: SocketAddr,
    pub internal_port: u16,
    pub lifetime_secs: u32,
    pub protocol: MappingProtocol,
    pub local_lan: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub expires_at_unix_ms: i64,
    pub observed_at_unix_ms: i64,
}

impl MappingResult {
    pub fn candidate(&self) -> EndpointCandidate {
        EndpointCandidate::new_at(
            CandidateKind::RouterMapping,
            self.external,
            self.observed_at_unix_ms,
        )
        .with_expiry(Some(self.expires_at_unix_ms))
    }
}

#[derive(Debug)]
enum ActiveMapping {
    Crab(PortMapping),
    Upnp {
        gateway: igd_next::aio::Gateway<igd_tokio::Tokio>,
        external_port: u16,
        local_addr: SocketAddr,
        lifetime_secs: u32,
    },
}

#[derive(Debug)]
pub struct RouterMappingHandle {
    active: ActiveMapping,
    result: MappingResult,
}

impl RouterMappingHandle {
    pub fn result(&self) -> &MappingResult {
        &self.result
    }

    pub fn candidate(&self) -> EndpointCandidate {
        self.result.candidate()
    }

    pub fn renew_after(&self) -> Duration {
        let half = Duration::from_secs(u64::from(self.result.lifetime_secs.max(1)) / 2);
        half.max(Duration::from_secs(30))
    }

    pub async fn renew(&mut self) -> Result<MappingResult, String> {
        match &mut self.active {
            ActiveMapping::Crab(mapping) => {
                mapping
                    .renew()
                    .await
                    .map_err(|error| format!("mapping renew failed: {error}"))?;
                let observed_at_unix_ms = now_unix_ms();
                let lifetime_secs = mapping.lifetime().max(1);
                let external_ip = external_ip_from_mapping(mapping).await?;
                let external = SocketAddr::new(external_ip, mapping.external_port().get());
                self.result = MappingResult {
                    external,
                    internal_port: mapping.internal_port().get(),
                    lifetime_secs,
                    protocol: protocol_from_mapping(mapping),
                    local_lan: self.result.local_lan,
                    gateway: self.result.gateway,
                    expires_at_unix_ms: observed_at_unix_ms
                        + i64::from(lifetime_secs) * 1_000,
                    observed_at_unix_ms,
                };
                Ok(self.result.clone())
            }
            ActiveMapping::Upnp {
                gateway,
                external_port,
                local_addr,
                lifetime_secs,
            } => {
                gateway
                    .add_port(
                        PortMappingProtocol::UDP,
                        *external_port,
                        *local_addr,
                        *lifetime_secs,
                        "mesh",
                    )
                    .await
                    .map_err(|error| format!("upnp renew failed: {error}"))?;
                let observed_at_unix_ms = now_unix_ms();
                self.result.observed_at_unix_ms = observed_at_unix_ms;
                self.result.expires_at_unix_ms =
                    observed_at_unix_ms + i64::from(*lifetime_secs) * 1_000;
                Ok(self.result.clone())
            }
        }
    }

    pub async fn delete(self) {
        match self.active {
            ActiveMapping::Crab(mapping) => {
                if let Err((error, _)) = mapping.try_drop().await {
                    warn!(%error, "failed to delete crab_nat mapping");
                }
            }
            ActiveMapping::Upnp {
                gateway,
                external_port,
                ..
            } => {
                if let Err(error) = timeout(
                    Duration::from_secs(2),
                    gateway.remove_port(PortMappingProtocol::UDP, external_port),
                )
                .await
                {
                    warn!(?error, "failed to delete upnp mapping");
                }
            }
        }
    }
}

pub async fn attempt_router_mapping(internal_port: u16) -> Result<RouterMappingHandle, String> {
    if internal_port == 0 {
        return Err("internal port must be nonzero".to_owned());
    }
    let budget = timeout(MAPPING_BUDGET, attempt_router_mapping_inner(internal_port));
    match budget.await {
        Ok(result) => result,
        Err(_) => Err("router mapping budget exhausted".to_owned()),
    }
}

async fn attempt_router_mapping_inner(internal_port: u16) -> Result<RouterMappingHandle, String> {
    let (gateway, local_lan) = discover_ipv4_gateway_and_local()
        .ok_or_else(|| "no default IPv4 gateway discovered".to_owned())?;
    info!(%gateway, %local_lan, %internal_port, "attempting router mapping");

    if let Ok(handle) = try_pcp(gateway, local_lan, internal_port).await {
        return Ok(handle);
    }
    if let Ok(handle) = try_natpmp(gateway, local_lan, internal_port).await {
        return Ok(handle);
    }
    try_upnp(local_lan, internal_port).await
}

async fn try_pcp(
    gateway: Ipv4Addr,
    local_lan: Ipv4Addr,
    internal_port: u16,
) -> Result<RouterMappingHandle, String> {
    let internal = NonZeroU16::new(internal_port).ok_or_else(|| "port zero".to_owned())?;
    let options = PortMappingOptions {
        external_port: Some(internal),
        lifetime_seconds: Some(MAPPING_LIFETIME_SECS),
        timeout_config: Some(short_timeout_config()),
    };
    let mapping = timeout(
        PROTOCOL_DEADLINE,
        crab_nat::pcp::port_mapping(
            crab_nat::pcp::BaseMapRequest::new(
                GatewayAddress::from(IpAddr::V4(gateway)),
                IpAddr::V4(local_lan),
                InternetProtocol::Udp,
                internal,
            ),
            None,
            None,
            options,
        ),
    )
    .await
    .map_err(|_| "pcp timed out".to_owned())?
    .map_err(|error| format!("pcp failed: {error}"))?;

    finish_crab_mapping(mapping, local_lan, gateway, MappingProtocol::Pcp).await
}

async fn try_natpmp(
    gateway: Ipv4Addr,
    local_lan: Ipv4Addr,
    internal_port: u16,
) -> Result<RouterMappingHandle, String> {
    let internal = NonZeroU16::new(internal_port).ok_or_else(|| "port zero".to_owned())?;
    let options = PortMappingOptions {
        external_port: Some(internal),
        lifetime_seconds: Some(MAPPING_LIFETIME_SECS),
        timeout_config: Some(short_timeout_config()),
    };
    let mapping = timeout(
        PROTOCOL_DEADLINE,
        natpmp::port_mapping(
            GatewayAddress::from(IpAddr::V4(gateway)),
            InternetProtocol::Udp,
            internal,
            options,
        ),
    )
    .await
    .map_err(|_| "nat-pmp timed out".to_owned())?
    .map_err(|error| format!("nat-pmp failed: {error}"))?;

    finish_crab_mapping(mapping, local_lan, gateway, MappingProtocol::NatPmp).await
}

async fn try_upnp(local_lan: Ipv4Addr, internal_port: u16) -> Result<RouterMappingHandle, String> {
    let search = igd_next::SearchOptions {
        timeout: Some(PROTOCOL_DEADLINE),
        single_search_timeout: Some(Duration::from_secs(1)),
        ..Default::default()
    };
    let gateway = timeout(PROTOCOL_DEADLINE, igd_tokio::search_gateway(search))
        .await
        .map_err(|_| "upnp search timed out".to_owned())?
        .map_err(|error| format!("upnp search failed: {error}"))?;

    let local_addr = SocketAddr::new(IpAddr::V4(local_lan), internal_port);
    let external_port = match timeout(
        PROTOCOL_DEADLINE,
        gateway.add_port(
            PortMappingProtocol::UDP,
            internal_port,
            local_addr,
            MAPPING_LIFETIME_SECS,
            "mesh",
        ),
    )
    .await
    {
        Ok(Ok(())) => internal_port,
        Ok(Err(_)) | Err(_) => timeout(
            PROTOCOL_DEADLINE,
            gateway.add_any_port(
                PortMappingProtocol::UDP,
                local_addr,
                MAPPING_LIFETIME_SECS,
                "mesh",
            ),
        )
        .await
        .map_err(|_| "upnp add_any_port timed out".to_owned())?
        .map_err(|error| format!("upnp mapping failed: {error}"))?,
    };

    let external_ip = timeout(PROTOCOL_DEADLINE, gateway.get_external_ip())
        .await
        .map_err(|_| "upnp external ip timed out".to_owned())?
        .map_err(|error| format!("upnp external ip failed: {error}"))?;

    let observed_at_unix_ms = now_unix_ms();
    let result = MappingResult {
        external: SocketAddr::new(external_ip, external_port),
        internal_port,
        lifetime_secs: MAPPING_LIFETIME_SECS,
        protocol: MappingProtocol::Upnp,
        local_lan,
        gateway: match gateway.addr {
            SocketAddr::V4(v4) => *v4.ip(),
            SocketAddr::V6(_) => local_lan,
        },
        expires_at_unix_ms: observed_at_unix_ms + i64::from(MAPPING_LIFETIME_SECS) * 1_000,
        observed_at_unix_ms,
    };
    info!(
        external = %result.external,
        protocol = result.protocol.as_str(),
        "router mapping created"
    );
    Ok(RouterMappingHandle {
        active: ActiveMapping::Upnp {
            gateway,
            external_port,
            local_addr,
            lifetime_secs: MAPPING_LIFETIME_SECS,
        },
        result,
    })
}

async fn finish_crab_mapping(
    mapping: PortMapping,
    local_lan: Ipv4Addr,
    gateway: Ipv4Addr,
    protocol: MappingProtocol,
) -> Result<RouterMappingHandle, String> {
    let external_ip = external_ip_from_mapping(&mapping).await?;
    let observed_at_unix_ms = now_unix_ms();
    let lifetime_secs = mapping.lifetime().max(1);
    let result = MappingResult {
        external: SocketAddr::new(external_ip, mapping.external_port().get()),
        internal_port: mapping.internal_port().get(),
        lifetime_secs,
        protocol,
        local_lan,
        gateway,
        expires_at_unix_ms: observed_at_unix_ms + i64::from(lifetime_secs) * 1_000,
        observed_at_unix_ms,
    };
    info!(
        external = %result.external,
        protocol = result.protocol.as_str(),
        "router mapping created"
    );
    Ok(RouterMappingHandle {
        active: ActiveMapping::Crab(mapping),
        result,
    })
}

async fn external_ip_from_mapping(mapping: &PortMapping) -> Result<IpAddr, String> {
    match mapping.mapping_type() {
        PortMappingType::Pcp { external_ip, .. } => Ok(external_ip),
        PortMappingType::NatPmp => {
            let ip = timeout(
                PROTOCOL_DEADLINE,
                natpmp::external_address(mapping.gateway(), Some(short_timeout_config())),
            )
            .await
            .map_err(|_| "nat-pmp external address timed out".to_owned())?
            .map_err(|error| format!("nat-pmp external address failed: {error}"))?;
            Ok(IpAddr::V4(ip))
        }
    }
}

fn protocol_from_mapping(mapping: &PortMapping) -> MappingProtocol {
    match mapping.mapping_type() {
        PortMappingType::Pcp { .. } => MappingProtocol::Pcp,
        PortMappingType::NatPmp => MappingProtocol::NatPmp,
    }
}

fn short_timeout_config() -> TimeoutConfig {
    TimeoutConfig {
        initial_timeout: Duration::from_millis(250),
        max_retries: 2,
        max_retry_timeout: Some(Duration::from_millis(750)),
    }
}

pub fn discover_ipv4_gateway_and_local() -> Option<(Ipv4Addr, Ipv4Addr)> {
    #[cfg(target_os = "linux")]
    {
        return linux_default_gateway_and_local();
    }
    #[cfg(not(target_os = "linux"))]
    {
        fallback_gateway_and_local()
    }
}

#[cfg(target_os = "linux")]
fn linux_default_gateway_and_local() -> Option<(Ipv4Addr, Ipv4Addr)> {
    let table = std::fs::read_to_string("/proc/net/route").ok()?;
    let mut best: Option<(u32, Ipv4Addr)> = None;
    for line in table.lines().skip(1) {
        let mut parts = line.split_whitespace();
        let _iface = parts.next()?;
        let destination = parts.next()?;
        let gateway_hex = parts.next()?;
        let flags = u16::from_str_radix(parts.next()?, 16).ok()?;
        let _refcnt = parts.next()?;
        let _use = parts.next()?;
        let metric = parts.next()?.parse::<u32>().ok()?;
        if destination != "00000000" {
            continue;
        }
        // RTF_UP | RTF_GATEWAY
        if flags & 0x0003 != 0x0003 {
            continue;
        }
        let gateway = parse_proc_ipv4(gateway_hex)?;
        if gateway.is_unspecified() {
            continue;
        }
        match best {
            None => best = Some((metric, gateway)),
            Some((best_metric, _)) if metric < best_metric => best = Some((metric, gateway)),
            _ => {}
        }
    }
    let gateway = best?.1;
    let local = local_ipv4_toward(gateway)?;
    Some((gateway, local))
}

#[cfg(not(target_os = "linux"))]
fn fallback_gateway_and_local() -> Option<(Ipv4Addr, Ipv4Addr)> {
    let local = local_ipv4_toward(Ipv4Addr::new(8, 8, 8, 8))?;
    let octets = local.octets();
    let gateway = Ipv4Addr::new(octets[0], octets[1], octets[2], 1);
    Some((gateway, local))
}

fn local_ipv4_toward(remote: Ipv4Addr) -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).ok()?;
    socket.connect(SocketAddr::from((remote, 80))).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_unspecified() => Some(v4),
        _ => None,
    }
}

fn parse_proc_ipv4(value: &str) -> Option<Ipv4Addr> {
    let raw = u32::from_str_radix(value, 16).ok()?;
    Some(Ipv4Addr::from(raw.to_ne_bytes()))
}

#[allow(dead_code)]
pub fn mapping_deadline_remaining(started: Instant, budget: Duration) -> Option<Duration> {
    budget.checked_sub(started.elapsed())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_gateway_does_not_panic() {
        let _ = discover_ipv4_gateway_and_local();
    }

    #[tokio::test]
    async fn mapping_attempt_fails_closed_without_router() {
        let result = attempt_router_mapping(41_000).await;
        // On machines without IGD/PMP this is Err; success is also fine.
        if let Ok(handle) = result {
            let candidate = handle.candidate();
            assert_eq!(candidate.kind, CandidateKind::RouterMapping);
            assert_ne!(candidate.address.port(), 0);
            handle.delete().await;
        }
    }
}

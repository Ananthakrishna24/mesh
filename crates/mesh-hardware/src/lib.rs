use std::time::{Duration, Instant};

use mesh_core::{CapabilityReport, ComputeProxy, GpuBackendKind, GpuDeviceInfo, now_unix_ms};
use sysinfo::{Disks, System};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HardwareError {
    Unavailable(String),
}

impl std::fmt::Display for HardwareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for HardwareError {}

pub type HardwareResult<T> = Result<T, HardwareError>;

pub fn discover_capabilities() -> CapabilityReport {
    let mut system = System::new();
    system.refresh_memory();
    system.refresh_cpu_all();

    let cpu_model = system
        .cpus()
        .first()
        .map(|cpu| cpu.brand().trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Unknown CPU".to_owned());
    let cpu_logical_cores = system.cpus().len().max(1) as u32;
    let memory_total_bytes = system.total_memory();
    let memory_available_bytes = system.available_memory();

    let disks = Disks::new_with_refreshed_list();
    let (disk_total_bytes, disk_available_bytes) =
        disks
            .list()
            .iter()
            .fold((0u64, 0u64), |(total, available), disk| {
                (
                    total.saturating_add(disk.total_space()),
                    available.saturating_add(disk.available_space()),
                )
            });

    let (gpus, gpu_status) = discover_gpus();
    let compute = measure_cpu_fp32_proxy();
    let status = if gpus.is_empty() {
        gpu_status
    } else {
        format!("Discovered {} GPU(s).", gpus.len())
    };

    CapabilityReport {
        collected_at_unix_ms: now_unix_ms(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        cpu_model,
        cpu_logical_cores,
        memory_total_bytes,
        memory_available_bytes,
        disk_total_bytes,
        disk_available_bytes,
        gpus,
        compute,
        status,
    }
}

fn measure_cpu_fp32_proxy() -> ComputeProxy {
    let duration = Duration::from_millis(200);
    let start = Instant::now();
    let mut ops: u64 = 0;
    let mut value = 1.000_001_f32;
    while start.elapsed() < duration {
        for _ in 0..16_384 {
            value = value.mul_add(1.000_000_1, 0.000_000_1);
            ops += 2;
        }
    }
    let elapsed = start.elapsed().as_secs_f64().max(1e-6);
    let gflops = (ops as f64 / elapsed) / 1_000_000_000.0;
    let _keep = value;
    ComputeProxy {
        cpu_fp32_gflops: gflops,
        measured_at_unix_ms: now_unix_ms(),
    }
}

fn discover_gpus() -> (Vec<GpuDeviceInfo>, String) {
    #[cfg(all(feature = "cuda", any(target_os = "linux", target_os = "windows")))]
    {
        match discover_nvidia_gpus() {
            Ok(gpus) if !gpus.is_empty() => return (gpus, "NVIDIA GPUs discovered.".to_owned()),
            Ok(_) => {}
            Err(error) => return (Vec::new(), error.to_string()),
        }
    }

    #[cfg(all(feature = "metal", target_os = "macos"))]
    {
        match discover_metal_gpus() {
            Ok(gpus) if !gpus.is_empty() => {
                return (gpus, "Apple Metal GPU discovered.".to_owned());
            }
            Ok(_) => {}
            Err(error) => return (Vec::new(), error.to_string()),
        }
    }

    (
        Vec::new(),
        "No supported GPU backend initialized on this host.".to_owned(),
    )
}

#[cfg(all(feature = "cuda", any(target_os = "linux", target_os = "windows")))]
fn discover_nvidia_gpus() -> HardwareResult<Vec<GpuDeviceInfo>> {
    let nvml = nvml_wrapper::Nvml::init()
        .map_err(|error| HardwareError::Unavailable(format!("NVML unavailable: {error}")))?;
    let count = nvml.device_count().map_err(|error| {
        HardwareError::Unavailable(format!("NVML device count failed: {error}"))
    })?;
    let driver_version = nvml.sys_driver_version().ok();
    let mut gpus = Vec::with_capacity(count as usize);
    for index in 0..count {
        let device = nvml.device_by_index(index).map_err(|error| {
            HardwareError::Unavailable(format!("NVML device {index} unavailable: {error}"))
        })?;
        let name = device
            .name()
            .unwrap_or_else(|_| format!("NVIDIA GPU {index}"));
        let uuid = device.uuid().unwrap_or_else(|_| format!("nvml-{index}"));
        let memory = device.memory_info().ok();
        let total_memory_bytes = memory.as_ref().map(|info| info.total).unwrap_or(0);
        let available_memory_bytes = memory.as_ref().map(|info| info.free);
        gpus.push(GpuDeviceInfo {
            backend: GpuBackendKind::Cuda,
            stable_id: uuid,
            name,
            total_memory_bytes,
            available_memory_bytes,
            driver_version: driver_version.clone(),
            runtime_version: None,
        });
    }
    Ok(gpus)
}

#[cfg(all(feature = "metal", target_os = "macos"))]
fn discover_metal_gpus() -> HardwareResult<Vec<GpuDeviceInfo>> {
    use objc2_metal::{MTLCreateSystemDefaultDevice, MTLDevice};

    let device = MTLCreateSystemDefaultDevice()
        .ok_or_else(|| HardwareError::Unavailable("Metal default device unavailable".to_owned()))?;
    let name = device.name().to_string();
    let total = device.recommendedMaxWorkingSetSize();
    let free = total.saturating_sub(device.currentAllocatedSize());
    Ok(vec![GpuDeviceInfo {
        backend: GpuBackendKind::Metal,
        stable_id: format!("metal-{}", name.replace(' ', "-").to_lowercase()),
        name,
        total_memory_bytes: total,
        available_memory_bytes: Some(free),
        driver_version: None,
        runtime_version: None,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_local_host_capabilities() {
        let report = discover_capabilities();
        assert!(!report.cpu_model.is_empty());
        assert!(report.cpu_logical_cores >= 1);
        assert!(report.memory_total_bytes > 0);
        assert!(report.compute.cpu_fp32_gflops > 0.0);
        assert!(!report.os.is_empty());
        assert!(!report.arch.is_empty());
    }
}

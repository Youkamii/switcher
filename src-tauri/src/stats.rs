//! 시스템 상태 샘플링 (CPU·메모리·디스크·네트워크) — 위젯 SYSTEM 섹션의 데이터원.
//!
//! System·Networks를 호출 사이에 유지해야 하는 이유:
//! - CPU 사용률은 두 샘플 사이의 델타라 매번 새로 만들면 항상 0이 나온다.
//! - 네트워크 `received()`는 "직전 refresh 이후 바이트"라 경과 시간으로 나눠야
//!   초당 속도가 된다.

use serde::Serialize;
use std::sync::Mutex;
use std::time::Instant;
use sysinfo::{Disks, Networks, System};

#[derive(Serialize, Clone, Copy, Debug, Default)]
pub struct SysStats {
    /// 전체 CPU 사용률 0~100 (첫 샘플은 비교 기준이 없어 0이 나올 수 있다)
    pub cpu: f32,
    /// 메모리 바이트
    pub mem_used: u64,
    pub mem_total: u64,
    /// 전체 디스크 합계 바이트
    pub disk_used: u64,
    pub disk_total: u64,
    /// 네트워크 초당 바이트 (전 인터페이스 합)
    pub net_rx: u64,
    pub net_tx: u64,
}

struct Sampler {
    sys: System,
    disks: Disks,
    networks: Networks,
    last: Instant,
}

/// SYSTEM 섹션이 1초 주기로만 부르므로 잠금 경합은 사실상 없다
static SAMPLER: Mutex<Option<Sampler>> = Mutex::new(None);

pub fn sample() -> SysStats {
    let mut guard = match SAMPLER.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let sampler = guard.get_or_insert_with(|| Sampler {
        sys: System::new(),
        disks: Disks::new_with_refreshed_list(),
        networks: Networks::new_with_refreshed_list(),
        last: Instant::now(),
    });
    sampler.sys.refresh_cpu_usage();
    sampler.sys.refresh_memory();
    sampler.disks.refresh(true);
    sampler.networks.refresh(true);
    let elapsed = sampler.last.elapsed().as_secs_f64().max(0.05);
    sampler.last = Instant::now();

    let (mut disk_used, mut disk_total) = (0u64, 0u64);
    for disk in sampler.disks.list() {
        // macOS: APFS는 같은 컨테이너의 볼륨("/"·"/System/Volumes/Data" 등)을
        // 별개 디스크로 나열해 단순 합산이 실물의 배수가 된다 — 루트 볼륨만
        // 센다 (red-review 지적, 맥 실기기 미검증)
        #[cfg(target_os = "macos")]
        if disk.mount_point() != std::path::Path::new("/") {
            continue;
        }
        disk_total += disk.total_space();
        disk_used += disk.total_space().saturating_sub(disk.available_space());
    }
    let (mut rx, mut tx) = (0u64, 0u64);
    for (_, data) in sampler.networks.list() {
        rx += data.received();
        tx += data.transmitted();
    }
    SysStats {
        cpu: sampler.sys.global_cpu_usage().clamp(0.0, 100.0),
        mem_used: sampler.sys.used_memory(),
        mem_total: sampler.sys.total_memory(),
        disk_used,
        disk_total,
        net_rx: (rx as f64 / elapsed) as u64,
        net_tx: (tx as f64 / elapsed) as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 로컬 전용 눈 검증 (`cargo test -- --ignored real_` 관례): 실기기 수치를
    /// 출력해 df(디스크)·sysctl hw.memsize(메모리)와 사람이 대조한다 —
    /// macOS APFS 루트만 세는 필터가 실물과 맞는지 확인용
    #[test]
    #[ignore]
    fn real_stats_probe() {
        let _ = sample();
        std::thread::sleep(std::time::Duration::from_millis(1000));
        let s = sample();
        let gb = |b: u64| b as f64 / 1024f64.powi(3);
        println!("cpu   = {:.1}%", s.cpu);
        println!("mem   = {:.1}G / {:.1}G", gb(s.mem_used), gb(s.mem_total));
        println!("disk  = {:.1}G / {:.1}G", gb(s.disk_used), gb(s.disk_total));
        println!("net   = rx {}B/s, tx {}B/s", s.net_rx, s.net_tx);
    }

    #[test]
    fn totals_are_sane() {
        let first = sample();
        assert!(first.mem_total > 0);
        assert!(first.mem_used <= first.mem_total);
        assert!(first.disk_total > 0);
        assert!(first.disk_used <= first.disk_total);
        // 두 번째 샘플부터 CPU가 실제 델타 — 범위만 확인한다
        std::thread::sleep(std::time::Duration::from_millis(250));
        let second = sample();
        assert!((0.0..=100.0).contains(&second.cpu));
    }
}

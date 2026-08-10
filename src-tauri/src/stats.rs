//! 시스템 상태 샘플링 (CPU·메모리·디스크 I/O·네트워크) — 위젯 SYSTEM 섹션의 데이터원.
//!
//! System·Networks를 호출 사이에 유지해야 하는 이유:
//! - CPU 사용률은 두 샘플 사이의 델타라 매번 새로 만들면 항상 0이 나온다.
//! - 네트워크 `received()`는 "직전 refresh 이후 바이트"라 경과 시간으로 나눠야
//!   초당 속도가 된다.
//!
//! 디스크는 플랫폼이 갈린다 (2026-08-10 사용자 보고: "작업 관리자는 0%인데
//! 바가 80% 차 있다"):
//! - Windows: 성능 카운터 PhysicalDisk(_Total) — **작업 관리자와 같은 원천**.
//!   프로세스 IO 카운터 합산은 파일 캐시·파이프·소켓까지 세는 논리 I/O라
//!   물리 디스크가 놀아도 수십 MB/s가 잡히는 괴리가 있었다.
//! - macOS: 전 프로세스 disk_usage 합산 유지 (proc_pid_rusage는 실제 디스크
//!   바이트 기반이라 괴리가 작다 — 맥 실기기 미검증, 다음 맥 세션 몫).

use serde::Serialize;
#[cfg(target_os = "macos")]
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Instant;
#[cfg(target_os = "macos")]
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate};
use sysinfo::{Networks, System};

#[derive(Serialize, Clone, Copy, Debug, Default)]
pub struct SysStats {
    /// 전체 CPU 사용률 0~100 (첫 샘플은 비교 기준이 없어 0이 나올 수 있다)
    pub cpu: f32,
    /// 메모리 바이트
    pub mem_used: u64,
    pub mem_total: u64,
    /// 디스크 초당 바이트 — 용량이 아니라 **활동량**이다 (사용자 지시)
    pub disk_read: u64,
    pub disk_write: u64,
    /// 디스크 활성 시간 % (Windows: PhysicalDisk % Disk Time — 작업 관리자의
    /// 그 수치). 맥은 None — 프론트가 세션 피크 대비 폴백으로 바를 그린다
    pub disk_pct: Option<f32>,
    /// 네트워크 초당 바이트 (전 인터페이스 합)
    pub net_rx: u64,
    pub net_tx: u64,
}

/// Windows 물리 디스크 성능 카운터 (PDH) — 시스템 pdh.dll, 추가 의존성 없음.
/// 영어 카운터 경로(AddEnglishCounter)라 한국어 Windows에서도 동작한다.
#[cfg(windows)]
mod disk_perf {
    /// PDH_FMT_COUNTERVALUE — CStatus(u32) + (정렬 패딩) + union(f64)
    #[repr(C)]
    struct FmtValue {
        status: u32,
        value: f64,
    }
    #[link(name = "pdh")]
    extern "system" {
        fn PdhOpenQueryW(source: *const u16, data: usize, query: *mut isize) -> i32;
        fn PdhAddEnglishCounterW(
            query: isize,
            path: *const u16,
            data: usize,
            counter: *mut isize,
        ) -> i32;
        fn PdhCollectQueryData(query: isize) -> i32;
        fn PdhGetFormattedCounterValue(
            counter: isize,
            format: u32,
            ctype: *mut u32,
            value: *mut FmtValue,
        ) -> i32;
    }
    const PDH_FMT_DOUBLE: u32 = 0x0000_0200;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 핸들은 isize로 들고 있는다 (포인터 크기 정수 — ABI 동일, Send 자동)
    pub struct DiskPerf {
        query: isize,
        read: isize,
        write: isize,
        time: isize,
    }

    impl DiskPerf {
        /// 실패하면 None — 호출부는 디스크 행을 0으로 두고 나머지는 계속 산다
        pub fn open() -> Option<DiskPerf> {
            unsafe {
                let mut query = 0isize;
                if PdhOpenQueryW(std::ptr::null(), 0, &mut query) != 0 {
                    return None;
                }
                let add = |path: &str| -> Option<isize> {
                    let mut counter = 0isize;
                    let w = wide(path);
                    (PdhAddEnglishCounterW(query, w.as_ptr(), 0, &mut counter) == 0)
                        .then_some(counter)
                };
                let read = add(r"\PhysicalDisk(_Total)\Disk Read Bytes/sec")?;
                let write = add(r"\PhysicalDisk(_Total)\Disk Write Bytes/sec")?;
                let time = add(r"\PhysicalDisk(_Total)\% Disk Time")?;
                // rate 카운터는 표본 두 개가 있어야 값이 나온다 — 기준 표본을 깔아둔다
                let _ = PdhCollectQueryData(query);
                Some(DiskPerf { query, read, write, time })
            }
        }

        /// (read B/s, write B/s, 활성 %) — 수집·포맷 실패는 0으로 (첫 표본 포함)
        pub fn collect(&self) -> (u64, u64, f32) {
            unsafe {
                if PdhCollectQueryData(self.query) != 0 {
                    return (0, 0, 0.0);
                }
                let get = |counter: isize| -> f64 {
                    let mut value = FmtValue { status: 0, value: 0.0 };
                    let mut ctype = 0u32;
                    if PdhGetFormattedCounterValue(counter, PDH_FMT_DOUBLE, &mut ctype, &mut value)
                        == 0
                    {
                        value.value.max(0.0)
                    } else {
                        0.0
                    }
                };
                (
                    get(self.read) as u64,
                    get(self.write) as u64,
                    // % Disk Time은 큐 깊이 기반이라 100을 넘을 수 있다 — 표시용 클램프
                    get(self.time).min(100.0) as f32,
                )
            }
        }
    }
}

struct Sampler {
    sys: System,
    networks: Networks,
    /// 직전 샘플에 존재한 pid — sysinfo(0.39 실측)는 처음 본 프로세스의 구간
    /// I/O(read_bytes)로 old=0 기준의 **프로세스 전체 누적**을 돌려주므로, 신규
    /// pid는 한 샘플 건너뛴다. 안 거르면 첫 샘플이 부팅 후 총 I/O로 폭발한다.
    #[cfg(target_os = "macos")]
    known_pids: HashSet<Pid>,
    #[cfg(windows)]
    disk: Option<disk_perf::DiskPerf>,
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
        networks: Networks::new_with_refreshed_list(),
        #[cfg(target_os = "macos")]
        known_pids: HashSet::new(),
        #[cfg(windows)]
        disk: disk_perf::DiskPerf::open(),
        last: Instant::now(),
    });
    sampler.sys.refresh_cpu_usage();
    sampler.sys.refresh_memory();
    // 맥만 프로세스 디스크 카운터를 갱신한다 — Windows는 PDH가 맡는다
    #[cfg(target_os = "macos")]
    sampler.sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_disk_usage(),
    );
    sampler.networks.refresh(true);
    let elapsed = sampler.last.elapsed().as_secs_f64().max(0.05);
    sampler.last = Instant::now();

    #[cfg(windows)]
    let (disk_read, disk_write, disk_pct) = {
        let (read, write, pct) = sampler
            .disk
            .as_ref()
            .map(|d| d.collect())
            .unwrap_or((0, 0, 0.0));
        (read, write, Some(pct)) // PDH가 이미 초당 값 — elapsed로 나누지 않는다
    };
    #[cfg(target_os = "macos")]
    let (disk_read, disk_write, disk_pct) = {
        let (mut read, mut write) = (0u64, 0u64);
        let mut seen = HashSet::with_capacity(sampler.sys.processes().len());
        for (pid, process) in sampler.sys.processes() {
            seen.insert(*pid);
            // 신규 pid의 구간값은 누적 오염이라 버린다 (Sampler.known_pids 주석)
            if sampler.known_pids.contains(pid) {
                let usage = process.disk_usage();
                read += usage.read_bytes;
                write += usage.written_bytes;
            }
        }
        sampler.known_pids = seen;
        (
            (read as f64 / elapsed) as u64,
            (write as f64 / elapsed) as u64,
            None::<f32>,
        )
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let (disk_read, disk_write, disk_pct) = (0u64, 0u64, None::<f32>);

    let (mut rx, mut tx) = (0u64, 0u64);
    for (_, data) in sampler.networks.list() {
        rx += data.received();
        tx += data.transmitted();
    }
    SysStats {
        cpu: sampler.sys.global_cpu_usage().clamp(0.0, 100.0),
        mem_used: sampler.sys.used_memory(),
        mem_total: sampler.sys.total_memory(),
        disk_read,
        disk_write,
        disk_pct,
        net_rx: (rx as f64 / elapsed) as u64,
        net_tx: (tx as f64 / elapsed) as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 로컬 전용 눈 검증 (`cargo test -- --ignored real_` 관례): 실기기 수치를
    /// 출력해 작업 관리자·Activity Monitor와 사람이 대조한다 — 유휴에서 활성 %가
    /// 작업 관리자처럼 0 근처인지, R/W가 실물 규모인지 확인용
    #[test]
    #[ignore]
    fn real_stats_probe() {
        let first = sample();
        std::thread::sleep(std::time::Duration::from_millis(1000));
        let s = sample();
        let gb = |b: u64| b as f64 / 1024f64.powi(3);
        let mb = |b: u64| b as f64 / 1024f64.powi(2);
        println!("cpu   = {:.1}%", s.cpu);
        println!("mem   = {:.1}G / {:.1}G", gb(s.mem_used), gb(s.mem_total));
        println!(
            "disk  = 1st R {:.2}M/s W {:.2}M/s pct {:?} → 2nd R {:.2}M/s W {:.2}M/s pct {:?}",
            mb(first.disk_read),
            mb(first.disk_write),
            first.disk_pct,
            mb(s.disk_read),
            mb(s.disk_write),
            s.disk_pct
        );
        println!("net   = rx {}B/s, tx {}B/s", s.net_rx, s.net_tx);
    }

    #[test]
    fn totals_are_sane() {
        // 디스크 새너티: 실물 상한(NVMe ~14GB/s)보다 훨씬 큰 100GB/s를 경계로 —
        // 맥 경로의 known_pids 필터가 빠지면 첫 샘플이 부팅 후 누적으로 폭발하고,
        // Windows PDH도 값이 깨지면 여기 걸린다. SAMPLER는 전역이라 어떤 테스트
        // 순서에서도 성립해야 하는 조건이다.
        const IO_SANITY: u64 = 100 * 1024 * 1024 * 1024;
        let first = sample();
        assert!(first.mem_total > 0);
        assert!(first.mem_used <= first.mem_total);
        assert!(first.disk_read < IO_SANITY, "read={}", first.disk_read);
        assert!(first.disk_write < IO_SANITY, "write={}", first.disk_write);
        // 두 번째 샘플부터 CPU가 실제 델타 — 범위만 확인한다
        std::thread::sleep(std::time::Duration::from_millis(250));
        let second = sample();
        assert!((0.0..=100.0).contains(&second.cpu));
        assert!(second.disk_read < IO_SANITY, "read={}", second.disk_read);
        assert!(second.disk_write < IO_SANITY, "write={}", second.disk_write);
        if let Some(pct) = second.disk_pct {
            assert!((0.0..=100.0).contains(&pct), "pct={pct}");
        }
    }
}

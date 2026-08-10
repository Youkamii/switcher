//! 시스템 상태 샘플링 (CPU·메모리·디스크 I/O·네트워크) — 위젯 SYSTEM 섹션의 데이터원.
//!
//! System·Networks를 호출 사이에 유지해야 하는 이유:
//! - CPU 사용률은 두 샘플 사이의 델타라 매번 새로 만들면 항상 0이 나온다.
//! - 네트워크 `received()`·프로세스 `disk_usage()`는 "직전 refresh 이후 바이트"라
//!   경과 시간으로 나눠야 초당 속도가 된다.

use serde::Serialize;
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Instant;
use sysinfo::{Networks, Pid, ProcessRefreshKind, ProcessesToUpdate, System};

#[derive(Serialize, Clone, Copy, Debug, Default)]
pub struct SysStats {
    /// 전체 CPU 사용률 0~100 (첫 샘플은 비교 기준이 없어 0이 나올 수 있다)
    pub cpu: f32,
    /// 메모리 바이트
    pub mem_used: u64,
    pub mem_total: u64,
    /// 디스크 초당 바이트 — 용량이 아니라 **활동량**이다 (사용자 지시: 차지한
    /// 공간이 아니라 지금 얼마나 읽고 쓰는지). 전 프로세스 disk_usage 합이라
    /// 접근 권한 없는 시스템 프로세스·구간 중에 죽은 프로세스는 빠지는 근사치 —
    /// "지금 디스크가 바쁜가"를 보는 위젯 용도에는 충분하다.
    pub disk_read: u64,
    pub disk_write: u64,
    /// 네트워크 초당 바이트 (전 인터페이스 합)
    pub net_rx: u64,
    pub net_tx: u64,
}

struct Sampler {
    sys: System,
    networks: Networks,
    /// 직전 샘플에 존재한 pid — sysinfo(0.39 실측)는 처음 본 프로세스의 구간
    /// I/O(read_bytes)로 old=0 기준의 **프로세스 전체 누적**을 돌려주므로, 신규
    /// pid는 한 샘플 건너뛴다. 안 거르면 첫 샘플이 부팅 후 총 I/O를 0.05초로
    /// 나눈 값(수 TB/s)으로 폭발하고, 새로 뜬 프로세스마다 스파이크가 낀다.
    known_pids: HashSet<Pid>,
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
        known_pids: HashSet::new(),
        last: Instant::now(),
    });
    sampler.sys.refresh_cpu_usage();
    sampler.sys.refresh_memory();
    // disk_usage만 갱신 — 전 프로세스 순회지만 I/O 카운터 하나라 매초 감당된다.
    // remove_dead(true)로 죽은 프로세스를 정리해 known_pids와 함께 새지 않는다
    sampler.sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_disk_usage(),
    );
    sampler.networks.refresh(true);
    let elapsed = sampler.last.elapsed().as_secs_f64().max(0.05);
    sampler.last = Instant::now();

    let (mut disk_read, mut disk_write) = (0u64, 0u64);
    let mut seen = HashSet::with_capacity(sampler.sys.processes().len());
    for (pid, process) in sampler.sys.processes() {
        seen.insert(*pid);
        // 신규 pid의 구간값은 누적 오염이라 버린다 (Sampler.known_pids 주석)
        if sampler.known_pids.contains(pid) {
            let usage = process.disk_usage();
            disk_read += usage.read_bytes;
            disk_write += usage.written_bytes;
        }
    }
    sampler.known_pids = seen;
    let (mut rx, mut tx) = (0u64, 0u64);
    for (_, data) in sampler.networks.list() {
        rx += data.received();
        tx += data.transmitted();
    }
    SysStats {
        cpu: sampler.sys.global_cpu_usage().clamp(0.0, 100.0),
        mem_used: sampler.sys.used_memory(),
        mem_total: sampler.sys.total_memory(),
        disk_read: (disk_read as f64 / elapsed) as u64,
        disk_write: (disk_write as f64 / elapsed) as u64,
        net_rx: (rx as f64 / elapsed) as u64,
        net_tx: (tx as f64 / elapsed) as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 로컬 전용 눈 검증 (`cargo test -- --ignored real_` 관례): 실기기 수치를
    /// 출력해 작업 관리자·Activity Monitor와 사람이 대조한다 — 프로세스 합산
    /// 디스크 I/O가 실물 규모와 맞는지, 첫 샘플 누적 폭발이 없는지 확인용
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
            "disk  = 1st R {:.2}M/s W {:.2}M/s → 2nd R {:.2}M/s W {:.2}M/s",
            mb(first.disk_read),
            mb(first.disk_write),
            mb(s.disk_read),
            mb(s.disk_write)
        );
        println!("net   = rx {}B/s, tx {}B/s", s.net_rx, s.net_tx);
    }

    #[test]
    fn totals_are_sane() {
        // 디스크 I/O 새너티: known_pids 필터가 빠지면 첫 샘플이 "부팅 후 누적
        // ÷ 0.05초"(수 TB/s)로 폭발한다 — 실물 상한(NVMe ~14GB/s)보다 훨씬 큰
        // 100GB/s를 경계로 잡아 회귀를 잡는다. SAMPLER는 전역이라 어떤 테스트
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
    }
}

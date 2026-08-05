//! 화면별 밝기 조절 (Windows) — DDC/CI 표준 경로.
//!
//! 그래픽카드 벤더 SDK 없이 dxva2.dll의 모니터 구성 API를 직접 바인딩한다.
//! SetMonitorBrightness는 모니터 OSD에서 밝기를 바꾸는 것과 같은 하드웨어
//! 명령이라 백라이트가 실제로 조절된다 (Monitorian·Twinkle Tray와 같은 방식).
//!
//! 미지원 사유는 다양하다(모니터 OSD에서 DDC/CI 꺼짐, 독·KVM·어댑터가 신호를
//! 안 통과시킴, 무선 디스플레이) — 그 모니터는 brightness=None으로 보고하고
//! 프론트가 안내 문구를 띄운다. 노트북 내장 패널(WMI)은 후속 (#31 비고).
//!
//! 핸들 수명: 물리 모니터 핸들은 호출마다 새로 얻고 반드시 Destroy로 돌려준다 —
//! 핫플러그로 스테일 핸들이 남는 것보다 열거 비용이 싸다.
#![cfg(windows)]

use serde::Serialize;

#[derive(Serialize)]
pub struct DisplayInfo {
    /// 열거 순서 기반 식별자 — 목록과 설정 호출 사이에만 유효
    pub id: usize,
    pub name: String,
    /// 0~100 정규화된 현재 밝기. None이면 이 모니터는 DDC/CI 밝기 미지원
    pub brightness: Option<u32>,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PhysicalMonitor {
    handle: isize,
    description: [u16; 128],
}

#[link(name = "dxva2")]
extern "system" {
    fn GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor: isize, count: *mut u32) -> i32;
    fn GetPhysicalMonitorsFromHMONITOR(
        hmonitor: isize,
        count: u32,
        monitors: *mut PhysicalMonitor,
    ) -> i32;
    fn GetMonitorBrightness(handle: isize, min: *mut u32, cur: *mut u32, max: *mut u32) -> i32;
    fn SetMonitorBrightness(handle: isize, value: u32) -> i32;
    fn DestroyPhysicalMonitor(handle: isize) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn EnumDisplayMonitors(
        hdc: isize,
        clip: *const core::ffi::c_void,
        callback: extern "system" fn(isize, isize, *mut core::ffi::c_void, isize) -> i32,
        data: isize,
    ) -> i32;
}

extern "system" fn collect_monitor(
    hmonitor: isize,
    _hdc: isize,
    _rect: *mut core::ffi::c_void,
    data: isize,
) -> i32 {
    let list = unsafe { &mut *(data as *mut Vec<isize>) };
    list.push(hmonitor);
    1 // 계속 열거
}

fn hmonitors() -> Vec<isize> {
    let mut list: Vec<isize> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            0,
            std::ptr::null(),
            collect_monitor,
            &mut list as *mut Vec<isize> as isize,
        );
    }
    list
}

/// HMONITOR 하나에 딸린 물리 모니터들 (복제 모드에서는 여러 개일 수 있다)
fn physical_monitors(hmonitor: isize) -> Vec<PhysicalMonitor> {
    let mut count: u32 = 0;
    let ok = unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &mut count) };
    if ok == 0 || count == 0 {
        return Vec::new();
    }
    let mut monitors = vec![
        PhysicalMonitor {
            handle: 0,
            description: [0u16; 128],
        };
        count as usize
    ];
    let ok = unsafe { GetPhysicalMonitorsFromHMONITOR(hmonitor, count, monitors.as_mut_ptr()) };
    if ok == 0 {
        return Vec::new();
    }
    monitors
}

fn monitor_name(pm: &PhysicalMonitor, index: usize) -> String {
    let end = pm
        .description
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(pm.description.len());
    let name = String::from_utf16_lossy(&pm.description[..end]);
    let name = name.trim();
    if name.is_empty() {
        format!("Display {}", index + 1)
    } else {
        // "Generic PnP Monitor" 같은 범용 이름이 흔하다 — 번호를 붙여 구분한다
        format!("{} — {}", index + 1, name)
    }
}

/// (min, cur, max) → 0~100. max<=min이면 미지원으로 본다
fn normalize(min: u32, cur: u32, max: u32) -> Option<u32> {
    if max <= min {
        return None;
    }
    Some(((cur.saturating_sub(min)) * 100 / (max - min)).min(100))
}

/// 0~100 → 모니터 고유 범위
fn denormalize(min: u32, max: u32, percent: u32) -> u32 {
    min + (max - min) * percent.min(100) / 100
}

fn read_brightness(handle: isize) -> Option<(u32, u32, u32)> {
    let (mut min, mut cur, mut max) = (0u32, 0u32, 0u32);
    let ok = unsafe { GetMonitorBrightness(handle, &mut min, &mut cur, &mut max) };
    (ok != 0).then_some((min, cur, max))
}

/// 모든 모니터와 현재 밝기 — 미지원 모니터는 brightness=None
pub fn list() -> Vec<DisplayInfo> {
    let mut out: Vec<DisplayInfo> = Vec::new();
    for hmonitor in hmonitors() {
        for pm in physical_monitors(hmonitor) {
            let index = out.len();
            let brightness =
                read_brightness(pm.handle).and_then(|(min, cur, max)| normalize(min, cur, max));
            out.push(DisplayInfo {
                id: index,
                name: monitor_name(&pm, index),
                brightness,
            });
            unsafe { DestroyPhysicalMonitor(pm.handle) };
        }
    }
    out
}

/// id번째 모니터의 밝기를 percent(0~100)로 — 실제 백라이트 명령
pub fn set_brightness(id: usize, percent: u32) -> Result<(), String> {
    let mut index = 0usize;
    for hmonitor in hmonitors() {
        for pm in physical_monitors(hmonitor) {
            let this = index;
            index += 1;
            if this != id {
                unsafe { DestroyPhysicalMonitor(pm.handle) };
                continue;
            }
            let result = (|| {
                let Some((min, _cur, max)) = read_brightness(pm.handle) else {
                    return Err(
                        "이 모니터는 밝기 조절을 지원하지 않습니다 (DDC/CI 확인)".to_string()
                    );
                };
                if max <= min {
                    return Err("모니터가 밝기 범위를 보고하지 않습니다".to_string());
                }
                let ok = unsafe { SetMonitorBrightness(pm.handle, denormalize(min, max, percent)) };
                if ok == 0 {
                    return Err("밝기 설정 실패 — 모니터가 응답하지 않습니다".to_string());
                }
                Ok(())
            })();
            unsafe { DestroyPhysicalMonitor(pm.handle) };
            return result;
        }
    }
    Err("모니터를 찾을 수 없습니다 — 연결이 바뀌었으면 새로고침하세요".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_covers_odd_ranges() {
        assert_eq!(normalize(0, 50, 100), Some(50));
        assert_eq!(normalize(20, 20, 80), Some(0)); // min 오프셋 모니터
        assert_eq!(normalize(20, 80, 80), Some(100));
        assert_eq!(normalize(0, 0, 0), None); // 범위 없음 = 미지원
        assert_eq!(normalize(50, 60, 40), None); // 이상 응답 방어
        assert_eq!(denormalize(20, 80, 0), 20);
        assert_eq!(denormalize(20, 80, 100), 80);
        assert_eq!(denormalize(20, 80, 50), 50);
        assert_eq!(denormalize(0, 100, 250), 100); // percent 상한
    }

    /// 실기기 전용: 모니터 열거·밝기 읽기 + 같은 값 되쓰기 왕복 (화면 변화 없음)
    /// (`cargo test -- --ignored real_`)
    #[test]
    #[ignore]
    fn real_display_roundtrip_same_value() {
        let monitors = list();
        assert!(!monitors.is_empty(), "모니터가 하나도 열거되지 않았다");
        for m in &monitors {
            println!("[{}] {} → 밝기 {:?}", m.id, m.name, m.brightness);
        }
        // DDC/CI 지원 모니터가 있으면 현재 값 그대로 되써서 쓰기 경로를 검증한다
        if let Some(m) = monitors.iter().find(|m| m.brightness.is_some()) {
            set_brightness(m.id, m.brightness.unwrap()).expect("같은 값 되쓰기 실패");
        }
    }
}

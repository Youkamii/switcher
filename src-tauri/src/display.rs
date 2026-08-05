//! 화면별 밝기 조절 (Windows: DDC/CI, macOS: DisplayServices).
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
//!
//! macOS: 내장 디스플레이는 DisplayServices 비공개 프레임워크로 조절한다 —
//! MonitorControl·brightness CLI가 쓰는 검증된 경로. 비공개 API라 링크하지 않고
//! dlopen으로 열며, 없으면(미래 macOS 변경 등) 밝기 미지원으로만 강등된다.
//! 외장 모니터 DDC(IOAVService 역공학 경로)는 검증할 장비가 없어 미구현 —
//! brightness=None으로 보고되고 프론트가 미지원 안내를 띄운다.

use serde::Serialize;

#[derive(Serialize)]
pub struct DisplayInfo {
    /// 열거 순서 기반 식별자 — 목록과 설정 호출 사이에만 유효
    pub id: usize,
    pub name: String,
    /// 0~100 정규화된 현재 밝기. None이면 이 모니터는 DDC/CI 밝기 미지원
    pub brightness: Option<u32>,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
struct PhysicalMonitor {
    handle: isize,
    description: [u16; 128],
}

#[cfg(windows)]
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

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn EnumDisplayMonitors(
        hdc: isize,
        clip: *const core::ffi::c_void,
        callback: extern "system" fn(isize, isize, *mut core::ffi::c_void, isize) -> i32,
        data: isize,
    ) -> i32;
}

#[cfg(windows)]
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

#[cfg(windows)]
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
#[cfg(windows)]
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

#[cfg(windows)]
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

/// (min, cur, max) → 0~100. max<=min이면 미지원으로 본다.
/// 값은 외부 장치(DDC/CI)가 보고한 것 — u64 중간 계산으로 오버플로를 차단한다
#[cfg_attr(not(windows), allow(dead_code))]
fn normalize(min: u32, cur: u32, max: u32) -> Option<u32> {
    if max <= min {
        return None;
    }
    let pct = cur.saturating_sub(min) as u64 * 100 / (max - min) as u64;
    Some((pct as u32).min(100))
}

/// 0~100 → 모니터 고유 범위 (u64 중간 계산 — 비정상 max 보고 방어)
#[cfg_attr(not(windows), allow(dead_code))]
fn denormalize(min: u32, max: u32, percent: u32) -> u32 {
    min + ((max - min) as u64 * percent.min(100) as u64 / 100) as u32
}

#[cfg(windows)]
fn read_brightness(handle: isize) -> Option<(u32, u32, u32)> {
    let (mut min, mut cur, mut max) = (0u32, 0u32, 0u32);
    let ok = unsafe { GetMonitorBrightness(handle, &mut min, &mut cur, &mut max) };
    (ok != 0).then_some((min, cur, max))
}

/// 모든 모니터와 현재 밝기 — 미지원 모니터는 brightness=None
#[cfg(windows)]
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

/// id번째 모니터의 밝기를 percent(0~100)로 — 실제 백라이트 명령.
/// expected_name 대조: id는 열거 순서라 목록 이후 모니터가 꽂히거나 빠지면 다른
/// 모니터를 가리킬 수 있다 — 이름이 어긋나면 쓰지 않고 에러로 알린다 (red-review).
/// 대상 처리 후에도 열거를 끝까지 돌며 모든 핸들을 Destroy한다 — 조기 return은
/// 같은 그룹의 나머지 핸들을 누수시킨다 (red-review)
#[cfg(windows)]
pub fn set_brightness(id: usize, percent: u32, expected_name: &str) -> Result<(), String> {
    let mut index = 0usize;
    let mut outcome: Option<Result<(), String>> = None;
    for hmonitor in hmonitors() {
        for pm in physical_monitors(hmonitor) {
            let this = index;
            index += 1;
            if this == id && outcome.is_none() {
                if monitor_name(&pm, this) != expected_name {
                    outcome = Some(Err(
                        "모니터 구성이 바뀌었습니다 — 새로고침 후 다시 조절하세요".to_string(),
                    ));
                    unsafe { DestroyPhysicalMonitor(pm.handle) };
                    continue;
                }
                outcome = Some((|| {
                    let Some((min, _cur, max)) = read_brightness(pm.handle) else {
                        return Err(
                            "이 모니터는 밝기 조절을 지원하지 않습니다 (DDC/CI 확인)".to_string(),
                        );
                    };
                    if max <= min {
                        return Err("모니터가 밝기 범위를 보고하지 않습니다".to_string());
                    }
                    let raw = denormalize(min, max, percent);
                    let ok = unsafe { SetMonitorBrightness(pm.handle, raw) };
                    if ok == 0 {
                        return Err("밝기 설정 실패 — 모니터가 응답하지 않습니다".to_string());
                    }
                    Ok(())
                })());
            }
            unsafe { DestroyPhysicalMonitor(pm.handle) };
        }
    }
    outcome
        .unwrap_or_else(|| Err("모니터를 찾을 수 없습니다 — 연결이 바뀌었으면 새로고침하세요".to_string()))
}

// ── macOS ───────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGGetOnlineDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGDisplayIsBuiltin(display: u32) -> u32;
}

/// DisplayServices 비공개 프레임워크 — dlopen으로 열어 없어도 앱은 뜬다
#[cfg(target_os = "macos")]
mod display_services {
    use std::sync::OnceLock;

    pub struct Api {
        pub get: unsafe extern "C" fn(u32, *mut f32) -> i32,
        pub set: unsafe extern "C" fn(u32, f32) -> i32,
        pub can_change: unsafe extern "C" fn(u32) -> bool,
    }

    pub fn api() -> Option<&'static Api> {
        static API: OnceLock<Option<Api>> = OnceLock::new();
        API.get_or_init(|| unsafe {
            let handle = libc::dlopen(
                c"/System/Library/PrivateFrameworks/DisplayServices.framework/DisplayServices"
                    .as_ptr(),
                libc::RTLD_LAZY,
            );
            if handle.is_null() {
                return None;
            }
            let get = libc::dlsym(handle, c"DisplayServicesGetBrightness".as_ptr());
            let set = libc::dlsym(handle, c"DisplayServicesSetBrightness".as_ptr());
            let can = libc::dlsym(handle, c"DisplayServicesCanChangeBrightness".as_ptr());
            if get.is_null() || set.is_null() || can.is_null() {
                return None;
            }
            Some(Api {
                get: std::mem::transmute(get),
                set: std::mem::transmute(set),
                can_change: std::mem::transmute(can),
            })
        })
        .as_ref()
    }
}

/// 켜져 있는 디스플레이 ID 목록 (CoreGraphics 공개 API — 스레드 안전)
#[cfg(target_os = "macos")]
fn online_displays() -> Vec<u32> {
    let mut ids = [0u32; 16];
    let mut count = 0u32;
    let ok = unsafe { CGGetOnlineDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
    if ok != 0 {
        return Vec::new();
    }
    ids[..count as usize].to_vec()
}

/// 모든 디스플레이와 현재 밝기 — 조절 불가(외장 DDC 미구현 등)는 brightness=None
#[cfg(target_os = "macos")]
pub fn list() -> Vec<DisplayInfo> {
    let api = display_services::api();
    online_displays()
        .into_iter()
        .enumerate()
        .map(|(index, did)| {
            let name = if unsafe { CGDisplayIsBuiltin(did) } != 0 {
                "Built-in Display".to_string()
            } else {
                format!("Display {}", index + 1)
            };
            let brightness = api.and_then(|api| {
                if !unsafe { (api.can_change)(did) } {
                    return None;
                }
                let mut value = 0f32;
                if unsafe { (api.get)(did, &mut value) } != 0 {
                    return None;
                }
                Some((value * 100.0).round().clamp(0.0, 100.0) as u32)
            });
            // id는 CGDirectDisplayID 그대로 — 열거 순서가 아니라 시스템 ID라
            // 목록 이후 구성이 바뀌어도 다른 디스플레이를 가리키지 않는다
            DisplayInfo {
                id: did as usize,
                name,
                brightness,
            }
        })
        .collect()
}

/// id 디스플레이의 밝기를 percent(0~100)로 — 실제 백라이트 명령.
/// expected_name 대조는 윈도우와 같은 계약 (id가 시스템 ID라 사고 여지는 작지만 유지)
#[cfg(target_os = "macos")]
pub fn set_brightness(id: usize, percent: u32, expected_name: &str) -> Result<(), String> {
    let Some(api) = display_services::api() else {
        return Err("밝기 API(DisplayServices)를 사용할 수 없습니다".to_string());
    };
    let Some(target) = list().into_iter().find(|d| d.id == id) else {
        return Err("디스플레이를 찾을 수 없습니다 — 연결이 바뀌었으면 새로고침하세요".to_string());
    };
    if target.name != expected_name {
        return Err("모니터 구성이 바뀌었습니다 — 새로고침 후 다시 조절하세요".to_string());
    }
    let did = id as u32;
    if !unsafe { (api.can_change)(did) } {
        return Err("이 디스플레이는 밝기 조절을 지원하지 않습니다".to_string());
    }
    let value = percent.min(100) as f32 / 100.0;
    if unsafe { (api.set)(did, value) } != 0 {
        return Err("밝기 설정 실패 — 디스플레이가 응답하지 않습니다".to_string());
    }
    Ok(())
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
    #[cfg(any(windows, target_os = "macos"))]
    fn real_display_roundtrip_same_value() {
        let monitors = list();
        assert!(!monitors.is_empty(), "모니터가 하나도 열거되지 않았다");
        for m in &monitors {
            println!("[{}] {} → 밝기 {:?}", m.id, m.name, m.brightness);
        }
        // DDC/CI 지원 모니터가 있으면 현재 값 그대로 되써서 쓰기 경로를 검증한다
        if let Some(m) = monitors.iter().find(|m| m.brightness.is_some()) {
            set_brightness(m.id, m.brightness.unwrap(), &m.name).expect("같은 값 되쓰기 실패");
        }
    }
}

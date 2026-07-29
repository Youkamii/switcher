mod accounts;
mod login;
mod usage;

use accounts::{Env, Provider, Snapshot, SwitchResult};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// 고정(위젯) 모드의 클릭 투과.
/// 카드 위에서도 위젯은 마우스를 받지 않는다 — 단일 클릭·드래그가 전부 뒤 창으로 간다.
/// 대신 Rust가 커서 위치와 마우스 버튼을 직접 감시해, 전환 카드 위의 더블클릭 패턴을
/// 감지하면 전환을 실행하고 웹뷰에는 호버·완료 신호만 보낸다.
/// 타이틀바 버튼·이동 핸들(action 없는 영역)만 예외로 마우스를 받는다.
static CLICK_THROUGH_MODE: AtomicBool = AtomicBool::new(false);
static HIT_REGIONS: Mutex<Vec<HitRegion>> = Mutex::new(Vec::new());

#[derive(serde::Deserialize, Clone)]
struct HitRegion {
    /// 창 기준 논리 좌표 [x, y, w, h]
    rect: [f64; 4],
    /// Some((provider, name))이면 더블클릭으로 이 프로필로 전환하는 카드.
    /// None이면 마우스를 실제로 받아야 하는 UI(버튼·핸들).
    action: Option<(String, String)>,
}

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn GetAsyncKeyState(v_key: i32) -> i16;
    fn GetDoubleClickTime() -> u32;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    /// 전역 마우스 버튼 상태 (state_id 0 = combined session, button 0 = 왼쪽).
    /// 이벤트 탭이 아닌 상태 조회라 입력 모니터링 권한 없이 동작한다 (실기기 확인).
    fn CGEventSourceButtonState(state_id: i32, button: u32) -> bool;
}

/// 시스템 더블클릭 간격(ms) — 맥은 setup에서 NSEvent 값으로 채운다 (500ms는 폴백)
#[cfg(target_os = "macos")]
static DOUBLE_CLICK_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(500);

// 위젯 창에 갈아 끼울 NSPanel 서브클래스.
// macOS(26 실측): 일반 NSWindow는 CanJoinAllSpaces·FullScreenAuxiliary를 줘도
// 다른 Space(특히 전체화면)에 올라가지 못한다 — 비활성 패널(NSPanel)만 허용된다.
// 클래스를 바꾸면 tao 서브클래스가 덮어쓰던 canBecomeKeyWindow(테두리 없는 창은
// 기본 NO)가 사라지므로 여기서 복원한다 — 없으면 입력칸이 포커스를 못 받는다.
#[cfg(target_os = "macos")]
objc2::define_class!(
    #[unsafe(super(objc2_app_kit::NSPanel))]
    #[thread_kind = objc2::MainThreadOnly]
    #[name = "SwitcherPanel"]
    struct SwitcherPanel;

    impl SwitcherPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }
    }
);

/// 왼쪽 버튼이 눌려 있는가 — 투과 중엔 웹뷰가 클릭을 못 받으므로 시스템에 직접 묻는다
#[cfg(any(windows, target_os = "macos"))]
fn primary_button_down() -> bool {
    #[cfg(windows)]
    return (unsafe { GetAsyncKeyState(0x01) } as u16 & 0x8000) != 0;
    #[cfg(target_os = "macos")]
    return unsafe { CGEventSourceButtonState(0, 0) };
}

/// 시스템 더블클릭 판정 간격
#[cfg(any(windows, target_os = "macos"))]
fn double_click_window() -> std::time::Duration {
    #[cfg(windows)]
    return std::time::Duration::from_millis(unsafe { GetDoubleClickTime() } as u64);
    #[cfg(target_os = "macos")]
    return std::time::Duration::from_millis(DOUBLE_CLICK_MS.load(Ordering::Relaxed));
}

#[tauri::command]
fn list_profiles(provider: String) -> Result<Snapshot, String> {
    accounts::list(&Env::real()?, Provider::parse(&provider)?)
}

#[tauri::command]
fn save_profile(provider: String, name: String) -> Result<(), String> {
    accounts::save_current(&Env::real()?, Provider::parse(&provider)?, &name)
}

#[tauri::command]
fn switch_profile(provider: String, name: String) -> Result<SwitchResult, String> {
    accounts::switch(&Env::real()?, Provider::parse(&provider)?, &name)
}

#[tauri::command]
fn delete_profile(provider: String, name: String) -> Result<(), String> {
    accounts::delete(&Env::real()?, Provider::parse(&provider)?, &name)
}

#[tauri::command]
async fn fetch_usage(provider: String, profile: Option<String>) -> Result<usage::Usage, String> {
    usage::fetch(&Env::real()?, Provider::parse(&provider)?, profile.as_deref()).await
}

/// 로그인을 시작하고 사용자가 원하는 브라우저에 붙여넣을 주소를 돌려준다.
/// 활성 계정은 어느 단계에서도 건드리지 않는다.
#[tauri::command]
async fn start_login(provider: String) -> Result<login::LoginPrompt, String> {
    let provider = Provider::parse(&provider)?;
    tauri::async_runtime::spawn_blocking(move || login::start(&Env::real()?, provider))
        .await
        .map_err(|e| format!("로그인 시작 실패: {e}"))?
}

/// 브라우저에서 받은 코드를 넘겨 로그인을 끝낸다 (클로드)
#[tauri::command]
async fn submit_login_code(code: String) -> Result<login::LoginOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || login::submit_code(&Env::real()?, &code))
        .await
        .map_err(|e| format!("로그인 완료 실패: {e}"))?
}

/// 브라우저 쪽에서 로그인이 끝나기를 기다린다 (코덱스)
#[tauri::command]
async fn await_device_login() -> Result<login::LoginOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || login::wait_device(&Env::real()?))
        .await
        .map_err(|e| format!("로그인 대기 실패: {e}"))?
}

#[tauri::command]
fn cancel_login() {
    login::cancel();
}

/// 데모·스크린샷용: SWITCHER_VIEW=normal|locked|compact 로 초기 보기 모드를 강제한다
#[tauri::command]
fn initial_view_mode() -> Option<String> {
    std::env::var("SWITCHER_VIEW").ok()
}

/// 데모(GIF)용: SWITCHER_DEMO=1 이면 전환 완료 안내를 끄고 반투명하게 시작한다
#[tauri::command]
fn demo_mode() -> bool {
    std::env::var("SWITCHER_DEMO").is_ok()
}

/// 프론트가 렌더 후 카드·버튼의 화면 좌표를 보고한다
#[tauri::command]
fn set_hit_regions(regions: Vec<HitRegion>) {
    if let Ok(mut guard) = HIT_REGIONS.lock() {
        *guard = regions;
    }
}

/// 고정 모드 진입/해제 — 해제 시 투과를 즉시 끈다
#[tauri::command]
fn set_click_through(app: tauri::AppHandle, enabled: bool) {
    use tauri::Manager;
    CLICK_THROUGH_MODE.store(enabled, Ordering::Relaxed);
    if !enabled {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.set_ignore_cursor_events(false);
        }
    }
}

/// 위젯을 주 모니터 작업영역(작업표시줄 제외) 우하단에 붙인다
fn position_bottom_right(window: &tauri::WebviewWindow) {
    let Ok(Some(monitor)) = window.primary_monitor() else {
        return;
    };
    let size = window
        .outer_size()
        .unwrap_or(tauri::PhysicalSize::new(360, 540));
    let area = monitor.work_area();
    let margin = 16;
    let x = area.position.x + area.size.width as i32 - size.width as i32 - margin;
    let y = area.position.y + area.size.height as i32 - size.height as i32 - margin;
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}

/// 최소화 파킹(-32000,-32000)이나 해상도 축소·모니터 분리로 화면 밖에 나가 있으면
/// 제자리로 되돌린다. 어떤 모니터와도 겹치지 않을 때만 옮기므로
/// 사용자가 직접 옮긴 정상 위치는 건드리지 않는다.
fn ensure_on_screen(window: &tauri::WebviewWindow) {
    let Ok(pos) = window.outer_position() else {
        position_bottom_right(window);
        return;
    };
    let size = window
        .outer_size()
        .unwrap_or(tauri::PhysicalSize::new(360, 540));
    let on_some_monitor = window
        .available_monitors()
        .map(|monitors| {
            monitors.iter().any(|monitor| {
                let area = monitor.work_area();
                let (ax, ay) = (area.position.x, area.position.y);
                let (aw, ah) = (area.size.width as i32, area.size.height as i32);
                // 창 사각형이 이 모니터 작업영역과 겹치는가
                pos.x < ax + aw
                    && pos.x + size.width as i32 > ax
                    && pos.y < ay + ah
                    && pos.y + size.height as i32 > ay
            })
        })
        .unwrap_or(false);
    if !on_some_monitor {
        position_bottom_right(window);
    }
}

/// 무조건 보이게 한다: 최소화 해제 → 화면 안 복귀 → 표시 → 앞으로
fn show_main_window(app: &tauri::AppHandle) {
    use tauri::Manager;
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        ensure_on_screen(&window);
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn toggle_main_window(app: &tauri::AppHandle) {
    use tauri::Manager;
    if let Some(window) = app.get_webview_window("main") {
        let visible = window.is_visible().unwrap_or(false);
        let minimized = window.is_minimized().unwrap_or(false);
        let focused = window.is_focused().unwrap_or(false);
        // "보이는 상태"라도 다른 창에 묻혀 있으면 숨기지 말고 앞으로 끌어온다
        if visible && !minimized && focused {
            let _ = window.hide();
        } else {
            show_main_window(app);
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    tauri::Builder::default()
        // 단일 인스턴스 — 이미 떠 있는데 exe를 또 실행하면 새 프로세스는 뜨지 않고
        // 기존 창이 앞으로 온다. 두 인스턴스의 토큰 재발급·전환이 경합하는 사고도
        // 함께 차단된다 (첫 플러그인으로 등록해야 다른 초기화보다 먼저 판정한다)
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .setup(|app| {
            use tauri::Manager;
            #[cfg(target_os = "macos")]
            {
                // 위젯은 Dock·Cmd+Tab에 나오지 않는다 — 트레이(메뉴바)로만 상주
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                // 시스템 더블클릭 간격 (setup은 메인 스레드 — AppKit 호출이 안전한 곳)
                let interval = objc2_app_kit::NSEvent::doubleClickInterval();
                if interval > 0.0 && interval < 5.0 {
                    DOUBLE_CLICK_MS.store((interval * 1000.0) as u64, Ordering::Relaxed);
                }
            }
            // 창은 숨김으로 만들어(visible: false) 자리·Space 속성을 정한 뒤 보여준다.
            // 이미 표시된 창은 Space 참여 플래그를 바꿔도 다시 order하기 전까지
            // 반영되지 않는다 (macOS 실측) — 첫 표시 전에 정하면 처음부터 맞는 곳에 뜬다.
            if let Some(window) = app.get_webview_window("main") {
                position_bottom_right(&window);
                // 맥의 Space 개념 대응: 위젯은 어느 Space로 옮겨가도, 전체화면 앱
                // 위에서도 보여야 한다 (윈도우판 alwaysOnTop의 의미를 그대로 옮김)
                #[cfg(target_os = "macos")]
                {
                    // 공식 API로 모든 Space 참여를 켠 뒤 (tao가 상태를 추적한다)
                    let _ = window.set_visible_on_all_workspaces(true);
                    if let Ok(ptr) = window.ns_window() {
                        use objc2::ClassType;
                        use objc2_app_kit::{
                            NSPanel, NSWindowCollectionBehavior, NSWindowStyleMask,
                        };
                        extern "C" {
                            fn object_setClass(
                                obj: *mut objc2::runtime::AnyObject,
                                cls: *const objc2::runtime::AnyClass,
                            ) -> *const objc2::runtime::AnyClass;
                        }
                        // 창을 비활성 패널로 전환 — Space 합류의 필요조건 (실측)
                        unsafe {
                            object_setClass(
                                ptr as *mut objc2::runtime::AnyObject,
                                SwitcherPanel::class(),
                            );
                        }
                        let panel = unsafe { &*(ptr as *const NSPanel) };
                        panel.setStyleMask(
                            panel.styleMask() | NSWindowStyleMask::NonactivatingPanel,
                        );
                        // 입력칸을 누를 때만 키보드를 받는다 — 위젯이 작업 포커스를 뺏지 않는다
                        panel.setBecomesKeyOnlyIfNeeded(true);
                        // 패널 기본값(비활성화 시 숨김)이 끼어들지 않게 명시적으로 끈다
                        panel.setHidesOnDeactivate(false);
                        panel.setCollectionBehavior(
                            panel.collectionBehavior()
                                | NSWindowCollectionBehavior::CanJoinAllSpaces
                                | NSWindowCollectionBehavior::FullScreenAuxiliary,
                        );
                    }
                }
                let _ = window.show();
            }
            // macOS(26 실측) 한계 보완: CanJoinAllSpaces를 켜도 Space를 옮기면 창이
            // 따라오지 않고, 다시 order해야 새 Space에 붙는다. 활성 Space에서 벗어난 게
            // 감지되면 스스로 앞에 다시 서는 워치독 (숨김 상태는 건드리지 않는다).
            #[cfg(target_os = "macos")]
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_millis(1000));
                    let on_main = handle.clone();
                    let _ = handle.run_on_main_thread(move || {
                        use tauri::Manager;
                        let Some(window) = on_main.get_webview_window("main") else {
                            return;
                        };
                        if !window.is_visible().unwrap_or(false) {
                            return;
                        }
                        if let Ok(ptr) = window.ns_window() {
                            let ns_window = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
                            if !ns_window.isOnActiveSpace() {
                                ns_window.orderFrontRegardless();
                            }
                        }
                    });
                });
            }
            // 지난 실행이 중단·크래시로 남긴 임시 로그인 폴더(토큰 포함 가능)를 청소한다.
            // 다른 인스턴스의 진행 중 로그인은 나이 필터가 보호한다.
            std::thread::spawn(|| {
                if let Ok(env) = Env::real() {
                    login::sweep_stale(&env);
                }
            });

            // 시작 시 토큰 일괄 갱신 (무조건 1회) — 밤새 꺼져 있던 컴퓨터에서도
            // 위젯이 뜨자마자 비활성 프로필의 사용량이 되살아난다
            tauri::async_runtime::spawn(async {
                if let Ok(env) = Env::real() {
                    usage::refresh_all_claude_profiles(&env).await;
                }
            });

            // 클릭 투과 폴링 (고정 모드, 25ms 주기):
            // - UI 영역(버튼·핸들) 위 → 마우스를 받는다
            // - 그 외 전부(카드 포함) → 뒤 창으로 통과. 단일 클릭·드래그를 절대 먹지 않는다.
            // - 전환 카드 위 더블클릭은 시스템 버튼 상태 폴링으로 직접 감지해 전환을
            //   실행한다 (부작용: 그 더블클릭은 뒤 창에도 전달된다 — 단일 클릭을 먹는 것보다 낫다)
            #[cfg(any(windows, target_os = "macos"))]
            {
                use tauri::Emitter;
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    let mut ignoring = false;
                    let mut prev_down = false;
                    let mut hover_idx: i64 = -1;
                    let mut last_click: Option<(std::time::Instant, usize)> = None;
                    loop {
                        std::thread::sleep(std::time::Duration::from_millis(25));
                        let Some(window) = handle.get_webview_window("main") else {
                            continue;
                        };
                        if !CLICK_THROUGH_MODE.load(Ordering::Relaxed) {
                            if ignoring {
                                let _ = window.set_ignore_cursor_events(false);
                                ignoring = false;
                            }
                            continue;
                        }
                        if !window.is_visible().unwrap_or(false) {
                            continue;
                        }
                        let (Ok(cursor), Ok(pos)) =
                            (handle.cursor_position(), window.outer_position())
                        else {
                            continue;
                        };
                        let scale = window.scale_factor().unwrap_or(1.0);
                        let rel_x = (cursor.x - pos.x as f64) / scale;
                        let rel_y = (cursor.y - pos.y as f64) / scale;
                        let regions: Vec<HitRegion> = HIT_REGIONS
                            .lock()
                            .map(|guard| guard.clone())
                            .unwrap_or_default();
                        let over = regions.iter().position(|region| {
                            let r = region.rect;
                            rel_x >= r[0]
                                && rel_x <= r[0] + r[2]
                                && rel_y >= r[1]
                                && rel_y <= r[1] + r[3]
                        });

                        // 마우스를 실제로 받는 곳은 action 없는 UI 영역뿐
                        let over_ui = over
                            .map(|i| regions[i].action.is_none())
                            .unwrap_or(false);
                        let want_ignore = !over_ui;
                        if want_ignore != ignoring {
                            let _ = window.set_ignore_cursor_events(want_ignore);
                            ignoring = want_ignore;
                        }

                        // 전환 카드 호버 표시 (웹뷰는 투과 중이라 자체 hover가 없다)
                        let over_card = over
                            .filter(|i| regions[*i].action.is_some())
                            .map(|i| i as i64)
                            .unwrap_or(-1);
                        if over_card != hover_idx {
                            hover_idx = over_card;
                            let _ = handle.emit("card-hover", hover_idx);
                        }

                        // 더블클릭 감지 — 같은 카드 위에서 시스템 더블클릭 시간 내 두 번 눌림
                        let down = primary_button_down();
                        let down_edge = down && !prev_down;
                        prev_down = down;
                        if !down_edge {
                            continue;
                        }
                        let Some(idx) =
                            over.filter(|i| regions[*i].action.is_some())
                        else {
                            last_click = None;
                            continue;
                        };
                        let now = std::time::Instant::now();
                        let dclk = double_click_window();
                        let is_double = matches!(
                            last_click,
                            Some((t, prev)) if prev == idx && now.duration_since(t) <= dclk
                        );
                        if !is_double {
                            last_click = Some((now, idx));
                            continue;
                        }
                        last_click = None;
                        let Some((provider, name)) = regions[idx].action.clone() else {
                            continue;
                        };
                        let result = (|| {
                            let env = Env::real()?;
                            accounts::switch(&env, Provider::parse(&provider)?, &name)
                        })();
                        let payload = match result {
                            Ok(_) => serde_json::json!({ "ok": true, "provider": provider, "name": name }),
                            Err(e) => serde_json::json!({ "ok": false, "error": e }),
                        };
                        let _ = handle.emit("account-switched", payload);
                    }
                });
            }
            let show = MenuItem::with_id(app, "show", "열기", true, None::<&str>)?;
            let hide = MenuItem::with_id(app, "hide", "숨기기", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &hide, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("switcher")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "hide" => {
                        use tauri::Manager;
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.hide();
                        }
                    }
                    "quit" => {
                        // 진행 중인 로그인 프로세스·토큰 임시 폴더를 정리하고 종료
                        login::cancel();
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_main_window(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        // 창 닫기 = 트레이로 숨김 (완전 종료는 트레이 메뉴의 "종료")
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_profiles,
            save_profile,
            switch_profile,
            delete_profile,
            fetch_usage,
            start_login,
            submit_login_code,
            await_device_login,
            cancel_login,
            set_hit_regions,
            set_click_through,
            initial_view_mode,
            demo_mode
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

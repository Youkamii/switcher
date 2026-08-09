mod accounts;
mod display;
mod github;
mod login;
mod memo;
mod settings;
mod stats;
mod tfsd;
mod update;
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
    /// 블랙 모니터 오버레이를 최상위 밴드의 맨 위로 재상승시키는 데 사용
    fn SetWindowPos(
        hwnd: isize,
        insert_after: isize,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> i32;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    /// 전역 마우스 버튼 상태 (state_id 0 = combined session, button 0 = 왼쪽).
    /// 이벤트 탭이 아닌 상태 조회라 입력 모니터링 권한 없이 동작한다 (실기기 확인).
    fn CGEventSourceButtonState(state_id: i32, button: u32) -> bool;
    /// 전역 키 눌림 상태 (key 53 = ESC). ButtonState와 같은 상태 조회 계열 —
    /// 블랙 모니터의 웹뷰가 죽었을 때를 위한 백업 해제 수단으로만 쓴다.
    fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;
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
fn switch_profile(
    app: tauri::AppHandle,
    provider: String,
    name: String,
) -> Result<SwitchResult, String> {
    let result = accounts::switch(&Env::real()?, Provider::parse(&provider)?, &name)?;
    // 수동 전환 = 운전대를 잡은 것 — TFSD 자율주행을 해제한다 (#36 후속)
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || disengage_tfsd(&handle));
    Ok(result)
}

/// 수동 전환이 감지되면 TFSD를 끈다 — 무인 프로세스가 사람의 선택을 되엎지 않게.
/// 트레이 메뉴 갱신 때문에 메인 스레드에서 불러야 한다.
fn disengage_tfsd(app: &tauri::AppHandle) {
    use tauri::Emitter;
    let Ok(env) = Env::real() else { return };
    if !settings::load_flag(&env.store, settings::KEY_TFSD, false) {
        return; // 애초에 꺼져 있으면 알림도 없다
    }
    if let Err(e) = settings::save_flag(&env.store, settings::KEY_TFSD, false) {
        eprintln!("TFSD 해제 저장 실패: {e}");
    }
    refresh_tray_menu(app, &settings::load_language(&env.store));
    let _ = app.emit("visibility-changed", ());
    let _ = app.emit("tfsd-disengaged", ());
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

/// 블랙 모니터 활성 여부 — 최상위 재확인 감시 스레드의 수명을 제어한다
static BLACK_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 블랙 모니터 켜기: 모니터마다 최상위 검은 오버레이 창을 띄운다.
/// - Windows: topmost + 2초마다 z-순서 재상승 — 나중에 뜨는 다른 최상위 창
///   (switcher 위젯 포함)도 오버레이 밑에 머물게 한다.
///   UAC 같은 시스템 보안 화면은 OS가 보호하므로 덮을 수 없다 (의도된 한계).
/// - macOS: 위젯과 같은 이유(일반 NSWindow는 전체화면 Space 불가)로 NSPanel로
///   전환하고 스크린세이버 레벨(1000)로 올린다 — 메뉴바·위젯(floating)보다 위.
///   Space를 옮기면 재-order해야 붙는 한계(실측)는 감시 스레드가 보완한다.
#[cfg(any(windows, target_os = "macos"))]
#[tauri::command]
async fn black_on(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    if BLACK_ACTIVE.swap(true, Ordering::Relaxed) {
        return Ok(()); // 이미 켜져 있다
    }
    let result = (|| -> Result<(), String> {
        let monitors = app
            .available_monitors()
            .map_err(|e| format!("모니터 조회 실패: {e}"))?;
        if monitors.is_empty() {
            return Err("모니터를 찾을 수 없습니다".to_string());
        }
        for (index, monitor) in monitors.iter().enumerate() {
            let label = format!("black-{index}");
            if app.get_webview_window(&label).is_some() {
                continue;
            }
            let window = tauri::WebviewWindowBuilder::new(
                &app,
                &label,
                tauri::WebviewUrl::App("black.html".into()),
            )
            .title("black")
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .shadow(false)
            .visible(false)
            .focused(false)
            .build()
            .map_err(|e| format!("오버레이 창 생성 실패: {e}"))?;
            // 혼합 DPI 환경에서도 정확히 그 모니터를 덮도록 물리 좌표로 배치한다
            let _ = window.set_position(tauri::PhysicalPosition::new(
                monitor.position().x,
                monitor.position().y,
            ));
            let _ = window.set_size(tauri::PhysicalSize::new(
                monitor.size().width,
                monitor.size().height,
            ));
            #[cfg(windows)]
            let _ = window.show();
        }
        // ESC 해제를 받을 수 있게 첫 오버레이에 키보드 포커스
        #[cfg(windows)]
        if let Some(first) = app.get_webview_window("black-0") {
            let _ = first.set_focus();
        }
        // 맥: 첫 표시 전에 패널 전환·레벨·Space 참여를 정해야 처음부터 맞는 곳에
        // 뜬다 (main 창과 같은 실측). NSWindow 조작이라 메인 스레드에서 처리.
        #[cfg(target_os = "macos")]
        {
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                use tauri::Manager;
                for (label, window) in handle.webview_windows() {
                    if !label.starts_with("black-") {
                        continue;
                    }
                    let _ = window.set_visible_on_all_workspaces(true);
                    if let Ok(ptr) = window.ns_window() {
                        use objc2_app_kit::NSWindowCollectionBehavior;
                        let ns_window = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
                        // 위젯과 달리 패널 스왑은 하지 않는다 — 런타임 생성 창을
                        // 스왑하면 닫을 때 ObjC 예외로 프로세스가 abort한다 (실측).
                        // 대가로 전체화면 앱 Space는 못 덮는다 (맥 한계, README 명시).
                        // 스크린세이버 레벨(1000) — 메뉴바·Dock·위젯(floating) 전부 덮는다
                        ns_window.setLevel(1000);
                        ns_window.setCollectionBehavior(
                            ns_window.collectionBehavior()
                                | NSWindowCollectionBehavior::CanJoinAllSpaces
                                | NSWindowCollectionBehavior::FullScreenAuxiliary,
                        );
                        // 맥은 mouseMoved를 기본으로 창에 주지 않는다 — 켜야
                        // 안개 구멍·흔들기 해제(pointermove)가 동작한다
                        ns_window.setAcceptsMouseMovedEvents(true);
                    }
                    let _ = window.show();
                }
                // 위젯(비활성 패널)의 버튼으로 켜면 앱이 비활성 상태다 — 활성화해야
                // macOS가 mouseMoved·키 입력을 오버레이로 준다 (실측: 비활성이면
                // 구멍·흔들기·ESC 전부 무반응). 블랙 모니터는 명시적 전체 덮기
                // 동작이므로 이때만큼은 포커스를 가져와도 된다.
                if let Some(mtm) = objc2::MainThreadMarker::new() {
                    let ns_app = objc2_app_kit::NSApplication::sharedApplication(mtm);
                    #[allow(deprecated)]
                    ns_app.activateIgnoringOtherApps(true);
                }
                // ESC 해제를 받을 수 있게 첫 오버레이에 키보드 포커스
                if let Some(first) = handle.get_webview_window("black-0") {
                    let _ = first.set_focus();
                }
            });
        }
        Ok(())
    })();
    if result.is_err() {
        // 일부 모니터만 덮인 채 남지 않게, 만들어진 오버레이를 전부 걷어낸다
        close_black_overlays(&app);
        return result;
    }
    // 창을 만드는 사이 꺼짐 요청(black_off)이 끼었으면 방금 만든 것까지 걷어낸다 —
    // 감시 스레드 없는 잔존 오버레이를 남기지 않기 위함 (red-review)
    if !BLACK_ACTIVE.load(Ordering::Relaxed) {
        close_black_overlays(&app);
        return Ok(());
    }
    // 감시 스레드: ① 150ms마다 네이티브 ESC 폴링 — 오버레이 웹뷰가 죽어도
    // 갇히지 않는 최후의 해제 수단 ② ~2초마다 창을 다시 위로 —
    // Windows는 z-순서 재상승, 맥은 Space 이탈 감지 후 재-order.
    // 블랙 모니터가 꺼지면 스스로 끝난다.
    let handle = app.clone();
    std::thread::spawn(move || {
        use tauri::Manager;
        let mut tick: u32 = 0;
        while BLACK_ACTIVE.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(150));
            #[cfg(windows)]
            let esc = {
                const VK_ESCAPE: i32 = 0x1B;
                (unsafe { GetAsyncKeyState(VK_ESCAPE) } as u16 & 0x8000) != 0
            };
            // 웹뷰 keydown이 1차 해제 수단(오버레이가 키 포커스)이고 이 폴링은
            // 백업이다. KeyState가 권한 문제로 항상 false여도 ESC 해제 자체는
            // 웹뷰 경로로 동작한다 (key 53 = kVK_Escape).
            #[cfg(target_os = "macos")]
            let esc = unsafe { CGEventSourceKeyState(0, 53) };
            if esc {
                close_black_overlays(&handle);
                break;
            }
            tick += 1;
            if tick % 13 != 0 {
                continue;
            }
            #[cfg(windows)]
            {
                const SWP_NOSIZE: u32 = 0x0001;
                const SWP_NOMOVE: u32 = 0x0002;
                const SWP_NOACTIVATE: u32 = 0x0010;
                const HWND_TOPMOST: isize = -1;
                for (label, window) in handle.webview_windows() {
                    if !label.starts_with("black-") {
                        continue;
                    }
                    if let Ok(hwnd) = window.hwnd() {
                        unsafe {
                            SetWindowPos(
                                hwnd.0 as isize,
                                HWND_TOPMOST,
                                0,
                                0,
                                0,
                                0,
                                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                            );
                        }
                    }
                }
            }
            // 맥: Space를 옮기면 CanJoinAllSpaces여도 다시 order해야 붙는다
            // (main 창 워치독과 같은 실측 보완)
            #[cfg(target_os = "macos")]
            {
                let on_main = handle.clone();
                let _ = handle.run_on_main_thread(move || {
                    for (label, window) in on_main.webview_windows() {
                        if !label.starts_with("black-") {
                            continue;
                        }
                        if let Ok(ptr) = window.ns_window() {
                            let ns_window = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
                            if !ns_window.isOnActiveSpace() {
                                ns_window.orderFrontRegardless();
                            }
                        }
                    }
                });
            }
        }
    });
    Ok(())
}

#[cfg(not(any(windows, target_os = "macos")))]
#[tauri::command]
async fn black_on(_app: tauri::AppHandle) -> Result<(), String> {
    Err("이 플랫폼에서는 블랙 모니터를 지원하지 않습니다".to_string())
}

/// 모든 오버레이 닫기 — black_off 커맨드·네이티브 ESC 감시·부분 실패 롤백이 공유한다.
/// close()가 아니라 destroy()다: close는 CloseRequested를 거치므로 가로채기에 취약하고,
/// 오버레이는 어떤 경우에도 확실히 사라져야 한다.
fn close_black_overlays(app: &tauri::AppHandle) {
    use tauri::Manager;
    BLACK_ACTIVE.store(false, Ordering::Relaxed);
    for (label, window) in app.webview_windows() {
        if label.starts_with("black-") {
            let _ = window.destroy();
        }
    }
}

/// 블랙 모니터 끄기 — (흔들기·ESC·어느 모니터에서든)
#[tauri::command]
fn black_off(app: tauri::AppHandle) {
    close_black_overlays(&app);
}

/// gh CLI에 로그인된 GitHub 계정 목록 (토큰은 만지지 않는다 — 이름·활성 여부만)
#[tauri::command]
async fn github_list() -> github::GithubSnapshot {
    tauri::async_runtime::spawn_blocking(github::list)
        .await
        .unwrap_or(github::GithubSnapshot {
            gh_found: false,
            accounts: Vec::new(),
        })
}

/// GitHub 활성 계정 전환 (gh auth switch + setup-git)
#[tauri::command]
async fn github_switch(name: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || github::switch(&name))
        .await
        .map_err(|e| format!("GitHub 전환 작업 실패: {e}"))?
}

/// GitHub 계정 추가 시작 — 위젯에 띄울 주소·일회용 코드 (PTY로 gh auth login)
#[tauri::command]
async fn github_login_start() -> Result<github::GhLoginPrompt, String> {
    tauri::async_runtime::spawn_blocking(github::login_start)
        .await
        .map_err(|e| format!("GitHub 로그인 시작 실패: {e}"))?
}

/// 브라우저에서 코드 입력이 끝나기를 기다린다 — 성공 시 로그인 이름
#[tauri::command]
async fn github_login_wait() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(github::login_wait)
        .await
        .map_err(|e| format!("GitHub 로그인 대기 실패: {e}"))?
}

#[tauri::command]
fn github_login_cancel() {
    github::login_cancel();
}

/// 표시 기능 플래그 — 프론트가 어떤 섹션·버튼을 그릴지 정한다.
/// tfsd는 섹션이 아니라 상태 표시등(활성 카드의 T 배지)용이다
#[derive(serde::Serialize)]
struct Visibility {
    claude: bool,
    codex: bool,
    github: bool,
    black: bool,
    display: bool,
    tfsd: bool,
}

#[tauri::command]
fn get_visibility() -> Visibility {
    let store = Env::real().map(|env| env.store).ok();
    let flag = |key: &str| {
        store
            .as_deref()
            .map(|s| settings::load_flag(s, key, true))
            .unwrap_or(true)
    };
    let tfsd = store
        .as_deref()
        .map(|s| settings::load_flag(s, settings::KEY_TFSD, false))
        .unwrap_or(false);
    Visibility {
        claude: flag(settings::KEY_SHOW_CLAUDE),
        codex: flag(settings::KEY_SHOW_CODEX),
        github: flag(settings::KEY_SHOW_GITHUB),
        black: flag(settings::KEY_SHOW_BLACK),
        display: flag(settings::KEY_SHOW_DISPLAY),
        tfsd,
    }
}

/// 모니터 목록·현재 밝기 (Windows: DDC/CI, macOS: DisplayServices.
/// 그 외 플랫폼은 빈 목록 → 섹션 생략)
#[tauri::command]
async fn display_list() -> Vec<display::DisplayInfo> {
    #[cfg(any(windows, target_os = "macos"))]
    {
        tauri::async_runtime::spawn_blocking(display::list)
            .await
            .unwrap_or_default()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Vec::new()
    }
}

/// 밝기 설정 — 실제 백라이트 명령이라 수십~수백 ms 걸릴 수 있어 blocking 풀에서.
/// name은 오매핑 방어용 — 목록 이후 모니터 구성이 바뀌면 쓰지 않고 에러
#[tauri::command]
async fn display_set_brightness(id: usize, percent: u32, name: String) -> Result<(), String> {
    #[cfg(any(windows, target_os = "macos"))]
    {
        tauri::async_runtime::spawn_blocking(move || display::set_brightness(id, percent, &name))
            .await
            .map_err(|e| format!("밝기 설정 작업 실패: {e}"))?
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (id, percent, name);
        Err("이 플랫폼에서는 밝기 조절을 지원하지 않습니다".to_string())
    }
}

/// 저장된 UI 언어 — 프론트가 시작할 때 읽는다 (설정을 못 읽으면 한국어)
#[tauri::command]
fn get_language() -> String {
    Env::real()
        .map(|env| settings::load_language(&env.store))
        .unwrap_or_else(|_| "ko".to_string())
}

/// 트레이 메뉴를 주어진 언어로 구성한다. 언어가 바뀌면 통째로 다시 만들어 갈아 끼운다.
/// Windows: 설정 → 언어 서브메뉴(체크 표시). macOS: 언어 변경은 아직 개발 진행중 —
/// 비활성 항목으로만 알린다 (메뉴바 앱 관례·키체인 경로 검증이 남아 있다).
fn build_tray_menu(
    app: &tauri::AppHandle,
    lang: &str,
) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, MenuItem, Submenu};
    let [open_l, hide_l, settings_l, language_l, auto_update_l, auto_start_l, black_l, visible_l, display_l, tfsd_l, check_update_l, quit_l] =
        settings::tray_labels(lang);
    let show = MenuItem::with_id(app, "show", open_l, true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", hide_l, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", quit_l, true, None::<&str>)?;
    let black = MenuItem::with_id(app, "black-on", black_l, true, None::<&str>)?;
    // 수동 업데이트 확인 — 라벨 뒤에 최근 결과 서픽스가 붙는다 (#44)
    let check_update = MenuItem::with_id(
        app,
        "check-update",
        format!("{check_update_l}{}", update_status_suffix()),
        true,
        None::<&str>,
    )?;
    let settings_menu = {
        use tauri::menu::CheckMenuItem;
        let mut lang_items: Vec<CheckMenuItem<tauri::Wry>> = Vec::new();
        for (code, name) in settings::LANGS {
            lang_items.push(CheckMenuItem::with_id(
                app,
                format!("lang:{code}"),
                name,
                true,
                code == lang,
                None::<&str>,
            )?);
        }
        let lang_refs: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = lang_items
            .iter()
            .map(|item| item as &dyn tauri::menu::IsMenuItem<tauri::Wry>)
            .collect();
        let language = Submenu::with_id_and_items(app, "language", language_l, true, &lang_refs)?;
        let store = Env::real().map(|env| env.store).ok();
        let flag = |key: &str, default: bool| {
            store
                .as_deref()
                .map(|s| settings::load_flag(s, key, default))
                .unwrap_or(default)
        };
        let auto_update = CheckMenuItem::with_id(
            app,
            "toggle-auto-update",
            auto_update_l,
            true,
            flag(settings::KEY_AUTO_UPDATE, true),
            None::<&str>,
        )?;
        let auto_start = CheckMenuItem::with_id(
            app,
            "toggle-auto-start",
            auto_start_l,
            true,
            flag(settings::KEY_AUTO_START, true),
            None::<&str>,
        )?;
        // TFSD — 사용량 기반 자동 계정 전환 (옵트인이라 기본 꺼짐)
        let tfsd = CheckMenuItem::with_id(
            app,
            "toggle-tfsd",
            tfsd_l,
            true,
            flag(settings::KEY_TFSD, false),
            None::<&str>,
        )?;
        // 표시 기능 — 안 쓰는 섹션·기능을 위젯에서 숨긴다 (제품명은 번역하지 않는다)
        let vis_claude = CheckMenuItem::with_id(
            app,
            "vis:claude",
            "Claude",
            true,
            flag(settings::KEY_SHOW_CLAUDE, true),
            None::<&str>,
        )?;
        let vis_codex = CheckMenuItem::with_id(
            app,
            "vis:codex",
            "Codex",
            true,
            flag(settings::KEY_SHOW_CODEX, true),
            None::<&str>,
        )?;
        let vis_github = CheckMenuItem::with_id(
            app,
            "vis:github",
            "GitHub",
            true,
            flag(settings::KEY_SHOW_GITHUB, true),
            None::<&str>,
        )?;
        let vis_black = CheckMenuItem::with_id(
            app,
            "vis:black",
            black_l,
            true,
            flag(settings::KEY_SHOW_BLACK, true),
            None::<&str>,
        )?;
        let vis_display = CheckMenuItem::with_id(
            app,
            "vis:display",
            display_l,
            true,
            flag(settings::KEY_SHOW_DISPLAY, true),
            None::<&str>,
        )?;
        let visible = Submenu::with_id_and_items(
            app,
            "visible",
            visible_l,
            true,
            &[&vis_claude, &vis_codex, &vis_github, &vis_black, &vis_display],
        )?;
        Submenu::with_id_and_items(
            app,
            "settings",
            settings_l,
            true,
            &[&language, &visible, &auto_update, &auto_start, &tfsd],
        )?
    };
    // 블랙 모니터 진입점은 표시 기능에서 껐으면 트레이에서도 숨긴다
    let black_visible = Env::real()
        .map(|env| settings::load_flag(&env.store, settings::KEY_SHOW_BLACK, true))
        .unwrap_or(true);
    let mut items: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = vec![&show, &hide];
    if black_visible {
        items.push(&black);
    }
    items.push(&settings_menu);
    items.push(&check_update);
    items.push(&quit);
    Menu::with_items(app, &items)
}

/// 수동 업데이트 확인의 최근 결과 — 트레이 라벨 뒤에 붙는 서픽스.
/// 언어 중립 기호만 쓴다: " …" 확인 중 / " — ✓" 최신 / " — v{n} ↻" 재시작 시 적용 / " — ✗" 실패
static UPDATE_STATUS: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
static UPDATE_CHECKING: AtomicBool = AtomicBool::new(false);

fn update_status_suffix() -> String {
    UPDATE_STATUS.lock().map(|s| s.clone()).unwrap_or_default()
}

/// 트레이의 "업데이트 확인" — 자동 업데이트와 같은 경로(check_and_apply)를 즉시 돈다.
/// 결과는 팝업 없이 메뉴 라벨 서픽스로만 알린다 (이 앱은 조용한 게 미덕, #44)
#[cfg(any(windows, target_os = "macos"))]
fn check_update_now(app: &tauri::AppHandle) {
    // dev 빌드는 자기 target 산출물을 덮으므로 자동 경로처럼 건너뛴다
    if cfg!(debug_assertions) {
        return;
    }
    if UPDATE_CHECKING.swap(true, Ordering::SeqCst) {
        return; // 이미 확인 중 — 중복 다운로드 방지
    }
    if let Ok(mut status) = UPDATE_STATUS.lock() {
        *status = " …".to_string();
    }
    if let Ok(env) = Env::real() {
        refresh_tray_menu(app, &settings::load_language(&env.store));
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        use tauri::Emitter;
        let suffix = match update::check_and_apply().await {
            Ok(Some(version)) => {
                // 프론트 콘솔에도 남긴다 (updateReady — 다음 실행부터 적용)
                let _ = handle.emit("update-ready", version.clone());
                format!(" — v{version} ↻")
            }
            Ok(None) => " — ✓".to_string(),
            Err(e) => {
                eprintln!("업데이트 확인 실패: {e}");
                " — ✗".to_string()
            }
        };
        if let Ok(mut status) = UPDATE_STATUS.lock() {
            *status = suffix;
        }
        UPDATE_CHECKING.store(false, Ordering::SeqCst);
        let lang = Env::real()
            .map(|env| settings::load_language(&env.store))
            .unwrap_or_else(|_| "ko".to_string());
        let handle_for_menu = handle.clone();
        let _ = handle.run_on_main_thread(move || refresh_tray_menu(&handle_for_menu, &lang));
    });
}

/// 설정 체크 토글 공통: 플래그 반전 저장 → 부수 효과 적용 → 메뉴 재구성(체크 갱신)
fn toggle_flag(app: &tauri::AppHandle, key: &'static str, default: bool) {
    let Ok(env) = Env::real() else { return };
    let now = !settings::load_flag(&env.store, key, default);
    if let Err(e) = settings::save_flag(&env.store, key, now) {
        eprintln!("설정 저장 실패: {e}");
    }
    #[cfg(any(windows, target_os = "macos"))]
    if key == settings::KEY_AUTO_START {
        if let Err(e) = autostart::set_enabled(now) {
            eprintln!("{e}");
        }
    }
    refresh_tray_menu(app, &settings::load_language(&env.store));
}

/// 트레이 메뉴를 주어진 언어로 다시 만들어 갈아 끼운다 (언어·체크 상태 변경 공통)
fn refresh_tray_menu(app: &tauri::AppHandle, lang: &str) {
    if let Ok(menu) = build_tray_menu(app, lang) {
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

/// 바탕화면 바로가기 (Windows) — 첫 실행 때 한 번만 만든다
#[cfg(windows)]
mod shortcut {
    use std::path::{Path, PathBuf};

    /// 바탕화면 실제 경로 — OneDrive 리디렉션까지 반영된 User Shell Folders 기준
    fn desktop_dir() -> Result<PathBuf, String> {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let raw: Option<String> = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders")
            .ok()
            .and_then(|key| key.get_value("Desktop").ok());
        if let Some(raw) = raw {
            let expanded = expand_env(&raw);
            if !expanded.is_empty() {
                return Ok(PathBuf::from(expanded));
            }
        }
        std::env::var_os("USERPROFILE")
            .map(|p| PathBuf::from(p).join("Desktop"))
            .ok_or_else(|| "바탕화면 경로를 찾을 수 없습니다".to_string())
    }

    /// REG_EXPAND_SZ의 %VAR% 치환 — winreg는 확장하지 않은 원문을 돌려준다
    fn expand_env(s: &str) -> String {
        let mut out = String::new();
        let mut rest = s;
        while let Some(start) = rest.find('%') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            match after.find('%') {
                Some(end) => {
                    let var = &after[..end];
                    match std::env::var(var) {
                        Ok(value) => out.push_str(&value),
                        Err(_) => {
                            out.push('%');
                            out.push_str(var);
                            out.push('%');
                        }
                    }
                    rest = &after[end + 1..];
                }
                None => {
                    out.push_str(&rest[start..]);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// dir 안에 switcher.lnk 생성. 이미 있으면 그대로 둔다.
    fn create_in(dir: &Path, target: &Path) -> Result<(), String> {
        let lnk = dir.join("switcher.lnk");
        if lnk.exists() {
            return Ok(());
        }
        let link =
            mslnk::ShellLink::new(target).map_err(|e| format!("바로가기 생성 실패: {e}"))?;
        link.create_lnk(&lnk)
            .map_err(|e| format!("바로가기 저장 실패: {e}"))
    }

    /// 바탕화면에 switcher 바로가기 생성 (현재 실행 파일 대상)
    pub fn create_on_desktop() -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| format!("실행 경로 확인 실패: {e}"))?;
        create_in(&desktop_dir()?, &exe)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn expands_env_vars() {
            std::env::set_var("SWITCHER_TEST_VAR", "C:\\probe");
            assert_eq!(expand_env("%SWITCHER_TEST_VAR%\\Desktop"), "C:\\probe\\Desktop");
            assert_eq!(expand_env("no-vars"), "no-vars");
            // 정의되지 않은 변수는 원문 그대로 남긴다
            assert_eq!(expand_env("%UNSET_VAR_XYZ%\\x"), "%UNSET_VAR_XYZ%\\x");
        }

        #[test]
        fn creates_lnk_once() {
            let dir =
                std::env::temp_dir().join(format!("switcher-lnk-test-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let exe = std::env::current_exe().unwrap();
            create_in(&dir, &exe).unwrap();
            assert!(dir.join("switcher.lnk").exists());
            // 이미 있으면 조용히 성공한다
            create_in(&dir, &exe).unwrap();
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// 부팅 시 자동 실행 — HKCU Run 키 (Windows, 관리자 권한 불필요)
#[cfg(windows)]
mod autostart {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    pub fn set_enabled(enabled: bool) -> Result<(), String> {
        set_named("switcher", enabled)
    }

    fn set_named(name: &str, enabled: bool) -> Result<(), String> {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(RUN_KEY)
            .map_err(|e| format!("자동 실행 레지스트리 열기 실패: {e}"))?;
        if enabled {
            let exe =
                std::env::current_exe().map_err(|e| format!("실행 경로 확인 실패: {e}"))?;
            key.set_value(name, &format!("\"{}\"", exe.display()))
                .map_err(|e| format!("자동 실행 등록 실패: {e}"))
        } else {
            match key.delete_value(name) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("자동 실행 해제 실패: {e}")),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// 실제 HKCU Run 키에 시험 이름을 왕복시킨다 — 실사용 항목("switcher")은 건드리지 않는다
        #[test]
        fn roundtrip_named_value() {
            let name = format!("switcher-selftest-{}", std::process::id());
            set_named(&name, true).unwrap();
            let key = RegKey::predef(HKEY_CURRENT_USER)
                .open_subkey(RUN_KEY)
                .unwrap();
            let value: String = key.get_value(&name).unwrap();
            assert!(value.to_lowercase().contains(".exe"));
            set_named(&name, false).unwrap();
            assert!(key.get_value::<String, _>(&name).is_err());
            // 이미 없는 값을 다시 꺼도 조용히 성공해야 한다
            set_named(&name, false).unwrap();
        }
    }
}

/// 부팅 시 자동 실행 — 로그인 항목 (macOS 13+, SMAppService).
/// 시스템 설정 → 일반 → 로그인 항목에 표시되고, 사용자가 거기서 꺼도 된다.
#[cfg(target_os = "macos")]
mod autostart {
    pub fn set_enabled(enabled: bool) -> Result<(), String> {
        use objc2_service_management::{SMAppService, SMAppServiceStatus};
        // macOS 12 이하엔 SMAppService가 없다 — 클래스 존재로 판별
        if objc2::runtime::AnyClass::get(c"SMAppService").is_none() {
            return Err("자동 실행은 macOS 13 이상에서 지원됩니다".to_string());
        }
        // 번들(.app) 밖에서 돌면(개발 실행 등) 디버그 바이너리가 로그인 항목에
        // 등록되는 사고를 막는다
        if enabled
            && !std::env::current_exe()
                .map(|p| p.to_string_lossy().contains(".app/Contents/MacOS"))
                .unwrap_or(false)
        {
            return Err("자동 실행 등록은 앱 번들(switcher.app)에서만 가능합니다".to_string());
        }
        let service = unsafe { SMAppService::mainAppService() };
        let status = unsafe { service.status() };
        if enabled {
            if status == SMAppServiceStatus::Enabled {
                return Ok(()); // 이미 등록됨
            }
            unsafe { service.registerAndReturnError() }
                .map_err(|e| format!("자동 실행 등록 실패: {e}"))
        } else {
            if status != SMAppServiceStatus::Enabled {
                return Ok(()); // 이미 없음 — 조용히 성공 (윈도우와 같은 계약)
            }
            unsafe { service.unregisterAndReturnError() }
                .map_err(|e| format!("자동 실행 해제 실패: {e}"))
        }
    }
}

/// 언어 변경 적용: 저장 → 트레이 메뉴 재구성(체크 이동) → 웹뷰에 알림.
/// 저장이 실패해도 이번 세션에는 적용한다 — 다음 시작 때만 원래 언어로 돌아간다.
fn apply_language(app: &tauri::AppHandle, lang: &str) {
    use tauri::Emitter;
    if !settings::is_supported(lang) {
        return;
    }
    match Env::real() {
        Ok(env) => {
            if let Err(e) = settings::save_language(&env.store, lang) {
                eprintln!("언어 저장 실패: {e}");
            }
        }
        Err(e) => eprintln!("언어 저장 실패: {e}"),
    }
    refresh_tray_menu(app, lang);
    let _ = app.emit("language-changed", lang);
}

/// 메모장 내용 읽기 — 파일이 없거나 깨져 있으면 빈 탭 5개
#[tauri::command]
fn memo_load() -> memo::MemoData {
    Env::real()
        .map(|env| memo::load(&env.store))
        .unwrap_or_default()
}

/// 메모장 저장 (본문·활성 탭·투명도 전체를 통째로)
#[tauri::command]
fn memo_save(data: memo::MemoData) -> Result<(), String> {
    let env = Env::real()?;
    memo::save(&env.store, data)
}

/// 부속 창(메모·모니터) 공통 토글 — 없으면 만들고, 보이면 숨기고, 숨어 있으면 앞으로.
/// 위젯의 부속 창이라 위젯과 같은 최상위·작업표시줄 없는 창으로 띄운다.
/// 닫기(✕·ESC)는 프론트가 hide만 하므로 창은 한 번 만들면 재사용된다.
/// macOS: 런타임 생성 창은 패널 스왑 금지(black 창과 같은 실측 — 닫을 때 abort)
/// — 전체화면 Space에는 못 올라가는 한계를 감수한다.
fn toggle_aux_window(
    app: &tauri::AppHandle,
    label: &str,
    page: &str,
    width: f64,
    height: f64,
) -> Result<(), String> {
    use tauri::Manager;
    if let Some(window) = app.get_webview_window(label) {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            ensure_on_screen(&window);
            let _ = window.show();
            let _ = window.set_focus();
        }
        return Ok(());
    }
    let build_result =
        tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App(page.into()))
            .title(label)
            .inner_size(width, height)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(true)
            .visible(false)
            .build();
    let window = match build_result {
        Ok(window) => window,
        // async 커맨드라 빠른 더블클릭이 겹치면 두 번째 build가 중복 라벨로
        // 실패할 수 있다 — 창이 이미 생겼으면 첫 호출이 표시까지 책임진다
        Err(_) if app.get_webview_window(label).is_some() => return Ok(()),
        Err(e) => return Err(format!("{label} 창 생성 실패: {e}")),
    };
    // 위젯 바로 왼쪽에 나란히 — 위젯은 관례상 우하단에 있다.
    // 왼쪽이 화면 밖이면 ensure_on_screen이 우하단(위젯 근처)으로 되돌린다.
    if let Some(main) = app.get_webview_window("main") {
        if let (Ok(pos), Ok(size)) = (main.outer_position(), window.outer_size()) {
            let x = pos.x - size.width as i32 - 12;
            let _ = window.set_position(tauri::PhysicalPosition::new(x, pos.y));
        }
    }
    ensure_on_screen(&window);
    let _ = window.show();
    let _ = window.set_focus();
    Ok(())
}

/// 메모창 토글 (Type2 타이틀바 📝)
#[tauri::command]
async fn memo_toggle(app: tauri::AppHandle) -> Result<(), String> {
    toggle_aux_window(&app, "memo", "memo.html", 280.0, 340.0)
}

/// 시스템 상태 샘플 — 위젯의 SYSTEM 섹션이 1초 주기로 부른다
#[tauri::command]
fn stats_read() -> stats::SysStats {
    stats::sample()
}

/// 데모·검증용 (SWITCHER_OPEN): 프론트가 읽는 시작 옵션 — "monitor"가 있으면
/// SYSTEM 섹션을 켠 채 시작한다 (메모창 "memo"는 setup에서 Rust가 직접 연다).
/// SYSTEM은 모든 보기 타입에서 그려진다 (미니멀 포함 — 사용자 요청)
#[tauri::command]
fn initial_open() -> String {
    std::env::var("SWITCHER_OPEN").unwrap_or_default()
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

            // 부팅 시 자동 실행 (기본 켜짐): 켜져 있으면 시작마다 재등록해
            // exe가 이동·업데이트돼도 자가 치유된다 (맥은 번들 실행일 때만).
            // 꺼짐이면 건드리지 않는다 (해제는 토글에서만).
            #[cfg(any(windows, target_os = "macos"))]
            if Env::real()
                .map(|env| settings::load_flag(&env.store, settings::KEY_AUTO_START, true))
                .unwrap_or(true)
            {
                if let Err(e) = autostart::set_enabled(true) {
                    eprintln!("{e}");
                }
            }

            // 첫 실행 시 바탕화면 바로가기 1회 생성 — 사용자가 지우면 다시 만들지 않는다
            #[cfg(windows)]
            if let Ok(env) = Env::real() {
                if !settings::load_flag(&env.store, settings::KEY_SHORTCUT_DONE, false) {
                    match shortcut::create_on_desktop() {
                        Ok(()) => {
                            let _ =
                                settings::save_flag(&env.store, settings::KEY_SHORTCUT_DONE, true);
                        }
                        Err(e) => eprintln!("{e}"),
                    }
                }
            }

            // 실행 시 자동 업데이트 (릴리스 빌드만): 새 버전이면 실행 파일(윈도우 exe /
            // 맥 .app 번들)을 제자리 교체하고 다음 실행부터 반영된다. dev 빌드가
            // target 산출물을 덮지 않게 debug_assertions에서는 확인 자체를 건너뛴다.
            #[cfg(any(windows, target_os = "macos"))]
            {
                update::sweep_old_exe();
                #[cfg(not(debug_assertions))]
                {
                    let auto_update_on = Env::real()
                        .map(|env| {
                            settings::load_flag(&env.store, settings::KEY_AUTO_UPDATE, true)
                        })
                        .unwrap_or(true);
                    if auto_update_on {
                        let handle = app.handle().clone();
                        tauri::async_runtime::spawn(async move {
                            use tauri::Emitter;
                            match update::check_and_apply().await {
                                Ok(Some(version)) => {
                                    let _ = handle.emit("update-ready", version);
                                }
                                Ok(None) => {}
                                Err(e) => eprintln!("자동 업데이트 실패: {e}"),
                            }
                        });
                    }
                }
            }

            // 시작 시 토큰 일괄 갱신 (무조건 1회) — 밤새 꺼져 있던 컴퓨터에서도
            // 위젯이 뜨자마자 비활성 프로필의 사용량이 되살아난다
            tauri::async_runtime::spawn(async {
                if let Ok(env) = Env::real() {
                    usage::refresh_all_claude_profiles(&env).await;
                }
            });

            // TFSD 자동 전환 감시 — 설정이 꺼져 있으면 틱마다 조용히 지나간다
            tfsd::spawn(app.handle().clone());

            // 데모·자가검증용: SWITCHER_OPEN=memo 이면 시작 직후 메모창을 연다
            // (SWITCHER_VIEW·SWITCHER_DEMO와 같은 스크린샷/검증 훅 계열)
            if let Ok(open) = std::env::var("SWITCHER_OPEN") {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                    if open.contains("memo") {
                        let _ = toggle_aux_window(&handle, "memo", "memo.html", 280.0, 340.0);
                    }
                });
            }

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
                        // GitHub 카드는 gh 통로로 — 프로세스 2회(switch+setup-git)라 수 초
                        // 걸릴 수 있어 폴링 스레드를 막지 않게 별도 스레드에서 돌린다
                        if provider == "github" {
                            let emit_handle = handle.clone();
                            std::thread::spawn(move || {
                                let payload = match github::switch(&name) {
                                    Ok(()) => serde_json::json!({ "ok": true, "provider": provider, "name": name }),
                                    Err(e) => serde_json::json!({ "ok": false, "error": e }),
                                };
                                let _ = emit_handle.emit("account-switched", payload);
                            });
                            continue;
                        }
                        let result: Result<(), String> = (|| {
                            let env = Env::real()?;
                            accounts::switch(&env, Provider::parse(&provider)?, &name).map(|_| ())
                        })();
                        // 더블클릭 수동 전환도 운전대 잡기 — TFSD 해제 (메인 스레드에서)
                        if result.is_ok() {
                            let disengage_handle = handle.clone();
                            let _ = handle.run_on_main_thread(move || {
                                disengage_tfsd(&disengage_handle);
                            });
                        }
                        let payload = match result {
                            Ok(_) => serde_json::json!({ "ok": true, "provider": provider, "name": name }),
                            Err(e) => serde_json::json!({ "ok": false, "error": e }),
                        };
                        let _ = handle.emit("account-switched", payload);
                    }
                });
            }
            // 트레이 메뉴는 저장된 UI 언어로 그린다 (설정을 못 읽으면 한국어)
            let lang = Env::real()
                .map(|env| settings::load_language(&env.store))
                .unwrap_or_else(|_| "ko".to_string());
            let menu = build_tray_menu(app.handle(), &lang)?;
            TrayIconBuilder::with_id("main")
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
                    id if id.starts_with("lang:") => {
                        apply_language(app, id.trim_start_matches("lang:"));
                    }
                    "toggle-auto-update" => {
                        toggle_flag(app, settings::KEY_AUTO_UPDATE, true);
                    }
                    "toggle-auto-start" => {
                        toggle_flag(app, settings::KEY_AUTO_START, true);
                    }
                    "toggle-tfsd" => {
                        use tauri::Emitter;
                        toggle_flag(app, settings::KEY_TFSD, false);
                        // 활성 카드의 T 배지가 즉시 나타나고 사라지게
                        let _ = app.emit("visibility-changed", ());
                    }
                    id if id.starts_with("vis:") => {
                        use tauri::Emitter;
                        let key = match id.trim_start_matches("vis:") {
                            "claude" => Some(settings::KEY_SHOW_CLAUDE),
                            "codex" => Some(settings::KEY_SHOW_CODEX),
                            "github" => Some(settings::KEY_SHOW_GITHUB),
                            "black" => Some(settings::KEY_SHOW_BLACK),
                            "display" => Some(settings::KEY_SHOW_DISPLAY),
                            _ => None,
                        };
                        if let Some(key) = key {
                            toggle_flag(app, key, true);
                            let _ = app.emit("visibility-changed", ());
                        }
                    }
                    "black-on" => {
                        let handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = black_on(handle).await {
                                eprintln!("블랙 모니터 켜기 실패: {e}");
                            }
                        });
                    }
                    #[cfg(any(windows, target_os = "macos"))]
                    "check-update" => check_update_now(app),
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
        // 창 닫기 = 트레이로 숨김 (완전 종료는 트레이 메뉴의 "종료").
        // main 창에만 적용한다 — 블랙 오버레이(black-*)까지 가로채면 close()가
        // 숨김으로 변해 "끈 줄 알았는데 숨어만 있는" 영구 고착이 된다 (red-review critical)
        .on_window_event(|window, event| {
            // 메모창도 닫기=숨기기 — Alt+F4가 창을 파괴하면
            // "창 재사용 + 숨김 전 저장" 계약이 깨진다 (red-review).
            // black-* 오버레이는 black_off가 실제로 닫아야 하므로 제외.
            if !matches!(window.label(), "main" | "memo") {
                return;
            }
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
            memo_load,
            memo_save,
            memo_toggle,
            stats_read,
            initial_open,
            initial_view_mode,
            demo_mode,
            get_language,
            get_visibility,
            github_list,
            github_switch,
            github_login_start,
            github_login_wait,
            github_login_cancel,
            black_on,
            black_off,
            display_list,
            display_set_brightness
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

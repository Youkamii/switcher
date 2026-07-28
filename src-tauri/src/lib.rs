mod accounts;
mod login;
mod usage;

use accounts::{Env, Provider, Snapshot, SwitchResult, MUTATION_LOCK};

#[tauri::command]
fn list_profiles(provider: String) -> Result<Snapshot, String> {
    accounts::list(&Env::real()?, Provider::parse(&provider)?)
}

#[tauri::command]
fn save_profile(provider: String, name: String) -> Result<(), String> {
    let _guard = MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    accounts::save_current(&Env::real()?, Provider::parse(&provider)?, &name)
}

#[tauri::command]
fn switch_profile(provider: String, name: String) -> Result<SwitchResult, String> {
    let _guard = MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    accounts::switch(&Env::real()?, Provider::parse(&provider)?, &name)
}

#[tauri::command]
fn delete_profile(provider: String, name: String) -> Result<(), String> {
    let _guard = MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
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

/// 최소화 파킹(-32000,-32000) 등으로 화면 밖에 나가 있으면 제자리로 되돌린다.
/// 사용자가 직접 옮긴 정상 위치는 건드리지 않는다.
fn ensure_on_screen(window: &tauri::WebviewWindow) {
    match window.outer_position() {
        Ok(pos) if pos.x > -10000 && pos.y > -10000 => {}
        _ => position_bottom_right(window),
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
        .setup(|app| {
            use tauri::Manager;
            // 첫 실행 위치: 작업영역 우하단 (위젯 기본 자리)
            if let Some(window) = app.get_webview_window("main") {
                position_bottom_right(&window);
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
                    "quit" => app.exit(0),
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
            cancel_login
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

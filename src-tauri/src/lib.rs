mod accounts;
mod usage;

use accounts::{Env, Provider, Snapshot, SwitchResult};

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

fn toggle_main_window(app: &tauri::AppHandle) {
    use tauri::Manager;
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use tauri::Manager;

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let show = MenuItem::with_id(app, "show", "열기/숨기기", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("switcher")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => toggle_main_window(app),
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
            fetch_usage
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
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

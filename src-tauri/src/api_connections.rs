//! Explicit API connections selected by the user. No automatic paid fallback.
//! Metadata and launchers contain no key; macOS Keychain / Windows user DPAPI own secrets.
use crate::accounts::{atomic_write, validate_name, Env, Provider};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, sync::Mutex};

static LOCK: Mutex<()> = Mutex::new(());
mod activation;
pub mod balance;
#[cfg(all(test, windows))]
mod windows_tests;

#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    #[default]
    Unchecked,
    Ready,
    InvalidKey,
    Forbidden,
    RateLimited,
    Unavailable,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Connection {
    pub provider: String,
    /// Stable storage name, including for keys created before account ownership.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub checked_at: Option<u64>,
    #[serde(default)]
    pub check_state: CheckState,
    #[serde(default)]
    pub active: bool,
    // A rotated key must not inherit an in-flight check of the previous key.
    #[serde(default)]
    revision: String,
}

fn root(env: &Env) -> PathBuf {
    env.store.join("api-connections")
}
fn checked(env: &Env, provider: &str, name: &str) -> Result<(Provider, PathBuf), String> {
    let provider = Provider::parse(provider)?;
    validate_name(name)?;
    Ok((
        provider,
        root(env)
            .join(provider.dir_name())
            .join(name.to_ascii_lowercase()),
    ))
}
fn service(provider: Provider, name: &str) -> String {
    format!(
        "Switcher-API-{}-{}",
        provider.dir_name(),
        name.to_ascii_lowercase()
    )
}
fn valid_key(key: &str) -> bool {
    (16..=4096).contains(&key.len())
        && key.starts_with("sk-")
        && key.bytes().all(|b| b.is_ascii_graphic())
}

trait Secrets {
    fn put(&self, id: &str, key: &[u8]) -> Result<(), String>;
    fn get(&self, id: &str) -> Result<Vec<u8>, String>;
    fn remove(&self, id: &str) -> Result<(), String>;
}
#[cfg_attr(not(windows), allow(dead_code))]
struct OsSecrets<'a>(&'a Env);
#[cfg(target_os = "macos")]
impl Secrets for OsSecrets<'_> {
    fn put(&self, id: &str, key: &[u8]) -> Result<(), String> {
        crate::accounts::keychain::write_item(id, &crate::accounts::keychain::username(), key)
            .map_err(|_| "API 키를 키체인에 저장하지 못했습니다".into())
    }
    fn get(&self, id: &str) -> Result<Vec<u8>, String> {
        crate::accounts::keychain::read_item(id)
            .map_err(|_| "API 키체인을 읽지 못했습니다")?
            .ok_or_else(|| "저장된 API 키가 없습니다 — 다시 등록하세요".into())
    }
    fn remove(&self, id: &str) -> Result<(), String> {
        crate::accounts::keychain::delete_item(id).map_err(|_| "API 키체인 삭제 실패".into())
    }
}
#[cfg(windows)]
fn protect(data: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    unsafe {
        let ok = if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return Err("Windows 사용자 자격증명 보호 작업 실패".into());
        }
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData.cast());
        Ok(bytes)
    }
}
#[cfg(windows)]
impl Secrets for OsSecrets<'_> {
    fn put(&self, id: &str, key: &[u8]) -> Result<(), String> {
        atomic_write(
            &root(self.0).join(format!("{id}.dpapi")),
            &protect(key, true)?,
        )
    }
    fn get(&self, id: &str) -> Result<Vec<u8>, String> {
        protect(
            &fs::read(root(self.0).join(format!("{id}.dpapi")))
                .map_err(|_| "저장된 API 키를 읽지 못했습니다")?,
            false,
        )
    }
    fn remove(&self, id: &str) -> Result<(), String> {
        match fs::remove_file(root(self.0).join(format!("{id}.dpapi"))) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("API 키 삭제 실패".into()),
        }
    }
}
#[cfg(not(any(windows, target_os = "macos")))]
impl Secrets for OsSecrets<'_> {
    fn put(&self, _: &str, _: &[u8]) -> Result<(), String> {
        Err("API 키 저장은 macOS·Windows에서 지원합니다".into())
    }
    fn get(&self, _: &str) -> Result<Vec<u8>, String> {
        Err("지원하지 않는 비밀 저장소".into())
    }
    fn remove(&self, _: &str) -> Result<(), String> {
        Err("지원하지 않는 비밀 저장소".into())
    }
}

fn read_connection(
    dir: &std::path::Path,
    provider: Provider,
    name: &str,
) -> Result<Connection, String> {
    let bytes =
        fs::read(dir.join("connection.json")).map_err(|_| "API 연결 정보를 읽지 못했습니다")?;
    let mut connection: Connection =
        serde_json::from_slice(&bytes).map_err(|_| "API 연결 정보 형식이 올바르지 않습니다")?;
    connection.provider = provider.dir_name().into();
    connection.name = name.to_ascii_lowercase();
    Ok(connection)
}

pub fn list(env: &Env, provider: &str) -> Result<Vec<Connection>, String> {
    let _guard = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
    let provider = Provider::parse(provider)?;
    let mut result = stored_connections(env, provider)?;
    for connection in &mut result {
        connection.active = activation::is_active(env, provider, connection).unwrap_or(false);
    }
    Ok(result)
}

fn stored_connections(env: &Env, provider: Provider) -> Result<Vec<Connection>, String> {
    let dir = root(env).join(provider.dir_name());
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    for entry in fs::read_dir(dir).map_err(|_| "API 연결 목록 읽기 실패")? {
        let entry = entry.map_err(|_| "API 연결 읽기 실패")?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if validate_name(&name).is_ok() && entry.path().join("connection.json").is_file() {
            result.push(read_connection(&entry.path(), provider, &name)?);
        }
    }
    result.sort_by(|a, b| a.display_name().cmp(b.display_name()).then(a.name.cmp(&b.name)));
    Ok(result)
}

impl Connection {
    fn display_name(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.name)
    }
}

#[cfg(test)]
fn save_with(
    env: &Env,
    secrets: &impl Secrets,
    provider: &str,
    name: &str,
    key: &str,
) -> Result<(), String> {
    let (provider, _) = checked(env, provider, name)?;
    store_key(env, secrets, provider, name, key, None, None).map(|_| ())
}

fn store_key(
    env: &Env,
    secrets: &impl Secrets,
    provider: Provider,
    name: &str,
    key: &str,
    account_id: Option<&str>,
    label: Option<String>,
) -> Result<Connection, String> {
    let (_, dir) = checked(env, provider.dir_name(), name)?;
    if !valid_key(key) {
        return Err("API 키 형식을 확인하세요 (sk-로 시작하는 공백 없는 키)".into());
    }
    fs::create_dir_all(&dir).map_err(|_| "API 연결 폴더 생성 실패")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .map_err(|_| "API 폴더 권한 설정 실패")?;
    }
    let id = service(provider, name);
    // Write unchecked metadata first under LOCK. If secret storage fails, restore it.
    // A crash can leave a connection unchecked, but cannot label a new key as verified.
    let metadata_path = dir.join("connection.json");
    let previous = match fs::read(&metadata_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err("API 연결 정보를 읽지 못했습니다".into()),
    };
    let connection = Connection {
        provider: provider.dir_name().into(),
        name: name.to_ascii_lowercase(),
        account_id: account_id.map(str::to_owned),
        label,
        checked_at: None,
        check_state: CheckState::Unchecked,
        active: false,
        revision: format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "시간 확인 실패")?
                .as_nanos()
        ),
    };
    let data = serde_json::to_vec(&connection).map_err(|_| "API 메타데이터 생성 실패")?;
    atomic_write(&metadata_path, &data)?;
    if let Err(error) = secrets.put(&id, key.as_bytes()) {
        let rollback = if let Some(previous) = previous {
            atomic_write(&metadata_path, &previous)
        } else {
            fs::remove_file(&metadata_path).map_err(|_| "API 연결 정리 실패".to_string())
        };
        if rollback.is_err() {
            return Err("API 키 저장 상태 복구 실패 — 키를 다시 등록하세요".into());
        }
        return Err(error);
    }
    Ok(connection)
}

fn require_account(env: &Env, provider: Provider, account_id: &str) -> Result<(), String> {
    if !account_id.is_empty()
        && crate::accounts::find_profile_by_id(env, provider, account_id)?.is_some()
    {
        Ok(())
    } else {
        Err("API 키를 추가할 계정을 찾을 수 없습니다".into())
    }
}

pub fn save(
    env: &Env,
    provider: &str,
    account_id: &str,
    name: &str,
    key: &str,
    replace: bool,
) -> Result<Connection, String> {
    save_for_account(
        env,
        &OsSecrets(env),
        provider,
        account_id,
        name,
        key,
        replace,
    )
}

fn save_for_account(
    env: &Env,
    secrets: &impl Secrets,
    provider: &str,
    account_id: &str,
    name: &str,
    key: &str,
    replace: bool,
) -> Result<Connection, String> {
    let _mutation = crate::accounts::MUTATION_LOCK
        .lock()
        .map_err(|_| "내부 잠금 오류")?;
    let _guard = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
    let (provider, dir) = checked(env, provider, name)?;
    require_account(env, provider, account_id)?;
    if replace {
        let existing = read_connection(&dir, provider, name)?;
        if existing.account_id.as_deref() != Some(account_id) {
            return Err("이 계정에 연결된 키만 교체할 수 있습니다".into());
        }
        return store_key(
            env,
            secrets,
            provider,
            &existing.name,
            key,
            Some(account_id),
            existing.label,
        );
    }
    let label = name.to_ascii_lowercase();
    let connections = stored_connections(env, provider)?;
    if connections
        .iter()
        .any(|c| c.account_id.as_deref() == Some(account_id) && c.display_name() == label)
    {
        return Err("이 계정에 같은 이름의 API 키가 있습니다".into());
    }
    let mut storage = label.clone();
    // Secret IDs and existing CLI helpers use the storage name. Different
    // accounts can use the same display name without rewriting those secrets.
    for suffix in 2.. {
        if !connections.iter().any(|c| c.name == storage) {
            break;
        }
        storage = format!("{}-{suffix}", &label[..label.len().min(20)]);
    }
    let display = (storage != label).then_some(label);
    store_key(
        env,
        secrets,
        provider,
        &storage,
        key,
        Some(account_id),
        display,
    )
}

pub fn attach(env: &Env, provider: &str, name: &str, account_id: &str) -> Result<(), String> {
    let _mutation = crate::accounts::MUTATION_LOCK
        .lock()
        .map_err(|_| "내부 잠금 오류")?;
    let _guard = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
    let (provider, dir) = checked(env, provider, name)?;
    require_account(env, provider, account_id)?;
    let mut connection = read_connection(&dir, provider, name)?;
    if let Some(previous) = &connection.account_id {
        if previous == account_id {
            return Ok(());
        }
        if crate::accounts::find_profile_by_id(env, provider, previous)?.is_some() {
            return Err("이미 다른 계정에 연결된 API 키입니다".into());
        }
    }
    if stored_connections(env, provider)?.iter().any(|c| {
        c.account_id.as_deref() == Some(account_id) && c.display_name() == connection.display_name()
    }) {
        return Err("이 계정에 같은 이름의 API 키가 있습니다".into());
    }
    connection.account_id = Some(account_id.to_owned());
    let data = serde_json::to_vec(&connection).map_err(|_| "API 연결 정보 저장 실패")?;
    crate::accounts::atomic_write_existing_parent(&dir.join("connection.json"), &data)
}
pub fn delete(env: &Env, provider: &str, name: &str) -> Result<(), String> {
    let _mutation = crate::accounts::MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let _guard = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
    let (provider, dir) = checked(env, provider, name)?;
    if activation::selected_name(env, provider)?.as_deref() == Some(&name.to_ascii_lowercase()) {
        activation::deactivate_locked(env, provider)?;
    }
    OsSecrets(env).remove(&service(provider, name))?;
    // Also retire the reporting credential saved by versions with monthly budgets.
    if dir.join("usage.json").exists() {
        OsSecrets(env).remove(&format!("Switcher-Usage-{}-{}", provider.dir_name(), name.to_ascii_lowercase()))?;
    }
    for file in ["connection.json", "run.sh", "run.ps1", "key.sh", "key.ps1", "usage.json", "console-credit.json"] {
        match fs::remove_file(dir.join(file)) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err("API 연결 파일 삭제 실패 — 다시 시도하세요".into()),
        }
    }
    // CLI history/config lives outside this folder; deleting a connection keeps that history.
    let _ = fs::remove_dir(&dir);
    balance::forget(&dir);
    Ok(())
}

pub fn activate(env: &Env, provider: &str, name: &str) -> Result<Connection, String> {
    let _mutation = crate::accounts::MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let _guard = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
    let (provider, _) = checked(env, provider, name)?;
    activation::activate_locked(env, &OsSecrets(env), provider, name)
}

pub fn deactivate(env: &Env, provider: &str) -> Result<(), String> {
    let _mutation = crate::accounts::MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    deactivate_for_oauth_locked(env, Provider::parse(provider)?)
}

/// Called while MUTATION_LOCK is held; never acquire it under the API lock.
pub(crate) fn deactivate_for_oauth_locked(env: &Env, provider: Provider) -> Result<(), String> {
    let _guard = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
    activation::deactivate_locked(env, provider)
}

pub(crate) fn active_connection(env: &Env, provider: Provider) -> Option<Connection> {
    let name = activation::selected_name(env, provider).ok()??;
    let (_, dir) = checked(env, provider.dir_name(), &name).ok()?;
    let connection = read_connection(&dir, provider, &name).ok()?;
    activation::is_active(env, provider, &connection).ok()?.then_some(connection)
}

pub(crate) fn protected_oauth_id(env: &Env, provider: Provider) -> Option<String> {
    activation::protected_oauth_id(env, provider)
}
fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
const CODEX_CONFIG: &[&str] = &[
    "model_provider=\"switcher_api\"",
    "model_providers.switcher_api.name=\"OpenAI API\"",
    "model_providers.switcher_api.base_url=\"https://api.openai.com/v1\"",
    "model_providers.switcher_api.env_key=\"OPENAI_API_KEY\"",
    "model_providers.switcher_api.wire_api=\"responses\"",
    "model_providers.switcher_api.requires_openai_auth=false",
];
fn launcher(env: &Env, provider: Provider, name: &str, windows: bool) -> String {
    let id = service(provider, name);
    let workspace = env.store.join("api-workspaces").join(format!(
        "{}-{}",
        provider.dir_name(),
        name.to_ascii_lowercase()
    ));
    let env_key = if provider == Provider::Claude {
        "ANTHROPIC_API_KEY"
    } else {
        "OPENAI_API_KEY"
    };
    let config_key = if provider == Provider::Claude {
        "CLAUDE_CONFIG_DIR"
    } else {
        "CODEX_HOME"
    };
    let conflicts = [
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
        "OPENAI_BASE_URL",
    ];
    if windows {
        let blob = root(env).join(format!("{id}.dpapi"));
        let clear = conflicts
            .iter()
            .map(|k| format!("Remove-Item Env:{k} -ErrorAction SilentlyContinue\n"))
            .collect::<String>();
        let options = if provider == Provider::Codex {
            CODEX_CONFIG
                .iter()
                .map(|c| format!("'-c', {}", ps_quote(c)))
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            String::new()
        };
        format!("$ErrorActionPreference = 'Stop'\nAdd-Type -AssemblyName System.Security\n{clear}$env:{config_key} = {}\nNew-Item -ItemType Directory -Force -Path $env:{config_key} | Out-Null\n$bytes = [System.Security.Cryptography.ProtectedData]::Unprotect([IO.File]::ReadAllBytes({}), $null, [System.Security.Cryptography.DataProtectionScope]::CurrentUser)\n$env:{env_key} = [Text.Encoding]::UTF8.GetString($bytes)\n[Array]::Clear($bytes, 0, $bytes.Length)\n$opts = @({options})\ntry {{ & {} @opts @args; exit $LASTEXITCODE }} finally {{ Remove-Item Env:{env_key} -ErrorAction SilentlyContinue }}\n", ps_quote(&workspace.to_string_lossy()), ps_quote(&blob.to_string_lossy()), provider.dir_name())
    } else {
        let options = if provider == Provider::Codex {
            CODEX_CONFIG
                .iter()
                .map(|c| format!(" -c {}", sh_quote(c)))
                .collect::<String>()
        } else {
            String::new()
        };
        format!("#!/bin/bash\nset +x\nset -euo pipefail\numask 077\nunset {}\nexport {config_key}={}\nmkdir -p \"${config_key}\"\n{env_key}=$(/usr/bin/security find-generic-password -s {} -w)\nexport {env_key}\nexec {}{options} \"$@\"\n", conflicts.join(" "), sh_quote(&workspace.to_string_lossy()), sh_quote(&id), provider.dir_name())
    }
}
pub fn launch_command(env: &Env, provider: &str, name: &str) -> Result<String, String> {
    let _guard = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
    let (provider, dir) = checked(env, provider, name)?;
    if !dir.join("connection.json").is_file() {
        return Err("API 연결이 없습니다".into());
    }
    let windows = cfg!(windows);
    let path = dir.join(if windows { "run.ps1" } else { "run.sh" });
    atomic_write(&path, launcher(env, provider, name, windows).as_bytes())?;
    Ok(if windows {
        format!(
            "powershell.exe -NoProfile -File {}",
            ps_quote(&path.to_string_lossy())
        )
    } else {
        format!("/bin/bash {}", sh_quote(&path.to_string_lossy()))
    })
}

fn check_message(state: CheckState) -> &'static str {
    match state {
        CheckState::InvalidKey => "API 키가 거부됐습니다 — 키를 교체하세요",
        CheckState::Forbidden => "모델 조회 권한이 없습니다 — 프로젝트 권한을 확인하세요",
        CheckState::RateLimited => "요청 제한 중입니다 — 잠시 후 다시 확인하세요",
        _ => "연결을 확인하지 못했습니다 — 네트워크와 서비스 상태를 확인하세요",
    }
}
fn check_state(status: u16) -> CheckState {
    match status {
        200..=299 => CheckState::Ready,
        401 => CheckState::InvalidKey,
        403 => CheckState::Forbidden,
        429 => CheckState::RateLimited,
        _ => CheckState::Unavailable,
    }
}
async fn check_at(provider: Provider, key: &str, url: &str) -> CheckState {
    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(client) => client,
        Err(_) => return CheckState::Unavailable,
    };
    let request = match provider {
        Provider::Claude => client
            .get(url)
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01"),
        Provider::Codex => client.get(url).bearer_auth(key),
    };
    match request.send().await {
        Ok(response) => {
            let state = check_state(response.status().as_u16());
            if state != CheckState::Ready {
                return state;
            }
            match response.json::<serde_json::Value>().await {
                Ok(body) if body.get("data").is_some_and(|data| data.is_array()) => {
                    CheckState::Ready
                }
                _ => CheckState::Unavailable,
            }
        }
        Err(_) => CheckState::Unavailable,
    }
}
fn land_check(
    env: &Env,
    provider: Provider,
    snapshot: &Connection,
    state: CheckState,
) -> Result<Connection, String> {
    let _guard = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
    let (_, dir) = checked(env, provider.dir_name(), &snapshot.name)?;
    let mut current = read_connection(&dir, provider, &snapshot.name)?;
    if current.revision != snapshot.revision {
        return Err("확인 중 API 키가 교체됐습니다 — 다시 확인하세요".into());
    }
    current.checked_at = Some(crate::accounts::now());
    current.check_state = state;
    let bytes = serde_json::to_vec(&current).map_err(|_| "API 확인 상태 저장 실패")?;
    atomic_write(&dir.join("connection.json"), &bytes)?;
    current.active = activation::is_active(env, provider, &current).unwrap_or(false);
    Ok(current)
}
pub async fn check(env: &Env, provider: &str, name: &str) -> Result<Connection, String> {
    let (provider, dir) = checked(env, provider, name)?;
    let (snapshot, key) = {
        let _guard = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
        (
            read_connection(&dir, provider, name)?,
            OsSecrets(env).get(&service(provider, name))?,
        )
    };
    let key = String::from_utf8(key).map_err(|_| "API 키 저장 형식 오류")?;
    let url = match provider {
        Provider::Claude => "https://api.anthropic.com/v1/models?limit=1",
        Provider::Codex => "https://api.openai.com/v1/models",
    };
    let state = check_at(provider, &key, url).await;
    let connection = land_check(env, provider, &snapshot, state)?;
    if state == CheckState::Ready {
        Ok(connection)
    } else {
        Err(check_message(state).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashMap};
    #[derive(Default)]
    pub(super) struct Memory(RefCell<HashMap<String, Vec<u8>>>);
    impl Secrets for Memory {
        fn put(&self, id: &str, key: &[u8]) -> Result<(), String> {
            self.0.borrow_mut().insert(id.into(), key.into());
            Ok(())
        }
        fn get(&self, id: &str) -> Result<Vec<u8>, String> {
            self.0.borrow().get(id).cloned().ok_or("missing".into())
        }
        fn remove(&self, id: &str) -> Result<(), String> {
            self.0.borrow_mut().remove(id);
            Ok(())
        }
    }
    pub(super) fn add_account(env: &Env, provider: Provider, name: &str, id: &str) {
        crate::accounts::write_profile_parts(
            env,
            provider,
            name,
            &crate::accounts::LiveIdentity {
                id: id.into(),
                email: None,
            },
            b"{}",
            None,
        )
        .unwrap();
    }

    #[test]
    fn keys_belong_to_saved_accounts_and_names_are_scoped_to_their_owner() {
        let env = crate::accounts::test_support::test_env("api-account-owners");
        let vault = Memory::default();
        add_account(&env, Provider::Claude, "one", "alpha");
        add_account(&env, Provider::Claude, "two", "beta");
        let create =
            |provider, id, key| save_for_account(&env, &vault, provider, id, "work", key, false);
        assert!(create("claude", "missing", "sk-dummy-first-fixture").is_err());
        assert!(create("codex", "alpha", "sk-dummy-first-fixture").is_err());
        let first = create("claude", "alpha", "sk-dummy-first-fixture").unwrap();
        let second = create("claude", "beta", "sk-dummy-second-fixture").unwrap();
        assert_eq!(first.display_name(), second.display_name());
        assert_ne!(first.name, second.name);
        assert!(create("claude", "alpha", "sk-dummy-new-fixture").is_err());
        assert!(save_for_account(
            &env,
            &vault,
            "claude",
            "alpha",
            &second.name,
            "sk-dummy-new-fixture",
            true
        )
        .is_err());
        assert_eq!(
            vault.get(&service(Provider::Claude, &second.name)).unwrap(),
            b"sk-dummy-second-fixture"
        );
        crate::accounts::rename(&env, Provider::Claude, "two", "renamed").unwrap();
        let rotated = save_for_account(
            &env,
            &vault,
            "claude",
            "beta",
            &second.name,
            "sk-dummy-third-fixture",
            true,
        )
        .unwrap();
        assert_eq!(rotated.display_name(), "work");
        assert_eq!(rotated.account_id.as_deref(), Some("beta"));
        assert!(active_connection(&env, Provider::Claude).is_none());
        activation::activate_locked(&env, &vault, Provider::Claude, &rotated.name).unwrap();
        let snapshot = crate::accounts::list(&env, Provider::Claude).unwrap();
        assert_eq!(snapshot.api_active.as_deref(), Some(rotated.name.as_str()));
        assert_eq!(snapshot.api_active_label.as_deref(), Some("work"));
        assert_eq!(snapshot.api_active_account_id.as_deref(), Some("beta"));
        assert!(snapshot.profiles.iter().all(|profile| !profile.active));
    }

    #[test]
    fn attaching_legacy_key_preserves_vault_credit_revision_and_current_selection() {
        let env = crate::accounts::test_support::test_env("api-attach-existing");
        let vault = Memory::default();
        add_account(&env, Provider::Claude, "one", "alpha");
        add_account(&env, Provider::Claude, "two", "beta");
        save_with(&env, &vault, "claude", "old", "sk-dummy-legacy-fixture").unwrap();
        let (_, dir) = checked(&env, "claude", "old").unwrap();
        let before = read_connection(&dir, Provider::Claude, "old").unwrap();
        fs::write(dir.join("console-credit.json"), b"fixture-credit").unwrap();
        activation::activate_locked(&env, &vault, Provider::Claude, "old").unwrap();
        attach(&env, "claude", "old", "beta").unwrap();
        attach(&env, "claude", "old", "beta").unwrap();
        assert!(attach(&env, "claude", "old", "alpha").is_err());
        let after = land_check(&env, Provider::Claude, &before, CheckState::Ready).unwrap();
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.account_id.as_deref(), Some("beta"));
        assert_eq!(
            fs::read(dir.join("console-credit.json")).unwrap(),
            b"fixture-credit"
        );
        assert_eq!(
            vault.get(&service(Provider::Claude, "old")).unwrap(),
            b"sk-dummy-legacy-fixture"
        );
        crate::accounts::delete(&env, Provider::Claude, "two").unwrap();
        attach(&env, "claude", "old", "alpha").unwrap();
        assert_eq!(
            active_connection(&env, Provider::Claude)
                .unwrap()
                .account_id
                .as_deref(),
            Some("alpha")
        );
    }

    #[test]
    #[ignore = "local migration of an explicitly selected saved key and account"]
    fn link_requested_local_api_account() {
        let Ok(name) = std::env::var("SWITCHER_LINK_KEY") else {
            return;
        };
        let email = std::env::var("SWITCHER_LINK_EMAIL").expect("expected account email");
        let provider =
            Provider::parse(&std::env::var("SWITCHER_LINK_PROVIDER").expect("expected provider"))
                .unwrap();
        let env = Env::real().unwrap();
        let matching: Vec<_> = crate::accounts::profile_dirs(&env, provider)
            .unwrap()
            .into_iter()
            .filter_map(|(_, dir)| crate::accounts::read_meta(&dir))
            .filter(|m| m.email.as_deref() == Some(email.as_str()))
            .collect();
        assert_eq!(matching.len(), 1, "expected one exact saved identity");
        let (_, dir) = checked(&env, provider.dir_name(), &name).unwrap();
        let before = read_connection(&dir, provider, &name).unwrap();
        let credit = fs::read(dir.join("console-credit.json")).ok();
        let active = activation::selected_name(&env, provider).unwrap();
        attach(&env, provider.dir_name(), &name, &matching[0].id).unwrap();
        let after = read_connection(&dir, provider, &name).unwrap();
        assert!(after.account_id.as_deref() == Some(matching[0].id.as_str()));
        assert!(after.revision == before.revision && after.check_state == before.check_state);
        assert!(fs::read(dir.join("console-credit.json")).ok() == credit);
        assert!(activation::selected_name(&env, provider).unwrap() == active);
    }
    #[test]
    fn metadata_and_commands_never_contain_key_or_modify_oauth() {
        let env = crate::accounts::test_support::test_env("api-safe-registration");
        let memory = Memory::default();
        let oauth = env.live_credential_path(Provider::Codex);
        fs::create_dir_all(oauth.parent().unwrap()).unwrap();
        fs::write(&oauth, b"untouched-oauth-fixture").unwrap();
        let key = "sk-dummy-api-fixture-123456";
        save_with(&env, &memory, "codex", "work", key).unwrap();
        assert_eq!(
            memory.get("Switcher-API-codex-work").unwrap(),
            key.as_bytes()
        );
        let entries = list(&env, "codex").unwrap();
        assert_eq!(entries.len(), 1);
        let metadata = fs::read_to_string(root(&env).join("codex/work/connection.json")).unwrap();
        assert!(!metadata.contains(key));
        for windows in [false, true] {
            let script = launcher(&env, Provider::Codex, "work", windows);
            assert!(!script.contains(key));
            assert!(script.contains("requires_openai_auth=false"));
            assert!(script.contains("api-workspaces"));
        }
        assert_eq!(fs::read(&oauth).unwrap(), b"untouched-oauth-fixture");
    }
    #[test]
    fn registration_rejects_paths_and_secret_injection() {
        let env = crate::accounts::test_support::test_env("api-validate");
        let memory = Memory::default();
        for name in ["../escape", "a/b", "con", "a'\ncmd"] {
            assert!(save_with(&env, &memory, "claude", name, "sk-dummy-fixture-key").is_err());
        }
        for key in [
            "",
            "sk-short",
            "sk-123456789012345\n",
            "sk-1234567890 12345",
        ] {
            assert!(save_with(&env, &memory, "claude", "ok", key).is_err());
        }
        assert!(memory.0.borrow().is_empty());
    }
    #[test]
    fn scripts_quote_paths_clear_conflicts_and_preserve_cli_arguments() {
        let mut env = crate::accounts::test_support::test_env("api-launcher");
        env.store = env.home.join("space ' dollar $() path");
        let shell = launcher(&env, Provider::Claude, "work", false);
        assert!(shell.contains("'\"'\"'"));
        assert!(shell.contains("CLAUDE_CODE_OAUTH_TOKEN"));
        assert!(shell.contains("exec claude \"$@\""));
        let ps = launcher(&env, Provider::Codex, "work", true);
        assert!(ps.contains("space '' dollar $() path"));
        assert!(ps.contains("@opts @args"));
    }
    #[cfg(unix)]
    #[test]
    fn generated_launcher_runs_cli_with_private_env_and_preserves_arguments() {
        use std::os::unix::fs::PermissionsExt;
        let mut env = crate::accounts::test_support::test_env("api-process-launch");
        env.store = env.home.join("space ' $(touch SHOULD_NOT_EXIST)");
        let bin = env.home.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let cli = bin.join("codex");
        fs::write(
            &cli,
            r#"#!/bin/bash
set -eu
[[ "$OPENAI_API_KEY" == sk-dummy-process-fixture ]]
[[ "$CODEX_HOME" == *api-workspaces/codex-work ]]
[[ -z "${OPENAI_BASE_URL+x}" ]]
[[ "${@: -1}" == 'argument with spaces' ]]
printf 'fixture-ok'
"#,
        )
        .unwrap();
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
        let secret = env.home.join("dummy-secret");
        fs::write(&secret, b"sk-dummy-process-fixture").unwrap();
        let script = launcher(&env, Provider::Codex, "work", false).replace(
            &format!(
                "/usr/bin/security find-generic-password -s {} -w",
                sh_quote("Switcher-API-codex-work")
            ),
            &format!("/bin/cat {}", sh_quote(&secret.to_string_lossy())),
        );
        let run = env.home.join("run.sh");
        fs::write(&run, script).unwrap();
        let output = std::process::Command::new("/bin/bash")
            .arg(run)
            .arg("argument with spaces")
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("OPENAI_BASE_URL", "https://invalid.test")
            .current_dir(&env.home)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"fixture-ok");
        assert!(!env.home.join("SHOULD_NOT_EXIST").exists());
    }

    #[test]
    fn verification_survives_reload_but_not_key_rotation_or_deletion() {
        let env = crate::accounts::test_support::test_env("api-check-state");
        let memory = Memory::default();
        save_with(&env, &memory, "claude", "work", "sk-dummy-check-first").unwrap();
        let initial = list(&env, "claude").unwrap().remove(0);
        assert_eq!(initial.check_state, CheckState::Unchecked);
        let verified = land_check(&env, Provider::Claude, &initial, CheckState::Ready).unwrap();
        assert!(verified.checked_at.is_some());
        assert_eq!(
            list(&env, "claude").unwrap()[0].check_state,
            CheckState::Ready
        );
        save_with(&env, &memory, "claude", "work", "sk-dummy-check-second").unwrap();
        assert!(land_check(&env, Provider::Claude, &initial, CheckState::Ready).is_err());
        let rotated = list(&env, "claude").unwrap().remove(0);
        assert_eq!(rotated.check_state, CheckState::Unchecked);
        assert!(rotated.checked_at.is_none());
        land_check(&env, Provider::Claude, &rotated, CheckState::Forbidden).unwrap();
        assert_eq!(
            list(&env, "claude").unwrap()[0].check_state,
            CheckState::Forbidden
        );
        fs::remove_file(root(&env).join("claude/work/connection.json")).unwrap();
        assert!(land_check(&env, Provider::Claude, &rotated, CheckState::Ready).is_err());
        assert!(list(&env, "claude").unwrap().is_empty());
    }
    #[test]
    fn failed_secret_save_rolls_back_metadata_and_missing_secret_can_be_replaced() {
        struct Denied;
        impl Secrets for Denied {
            fn put(&self, _: &str, _: &[u8]) -> Result<(), String> {
                Err("fixture denied".into())
            }
            fn get(&self, _: &str) -> Result<Vec<u8>, String> {
                panic!("saving must not require reading the previous secret")
            }
            fn remove(&self, _: &str) -> Result<(), String> {
                panic!("failed writes must not delete a previous secret")
            }
        }
        let env = crate::accounts::test_support::test_env("api-save-rollback");
        assert!(save_with(&env, &Denied, "claude", "work", "sk-dummy-first-save").is_err());
        assert!(list(&env, "claude").unwrap().is_empty());
        let memory = Memory::default();
        save_with(&env, &memory, "claude", "work", "sk-dummy-first-save").unwrap();
        let metadata_path = root(&env).join("claude/work/connection.json");
        let before = fs::read(&metadata_path).unwrap();
        assert!(save_with(&env, &Denied, "claude", "work", "sk-dummy-failed-save").is_err());
        assert_eq!(fs::read(&metadata_path).unwrap(), before);
        memory.0.borrow_mut().clear();
        save_with(&env, &memory, "claude", "work", "sk-dummy-recover-key").unwrap();
        assert_eq!(
            memory.get("Switcher-API-claude-work").unwrap(),
            b"sk-dummy-recover-key"
        );
    }

    #[test]
    fn existing_connection_requires_explicit_replacement_before_os_secret_access() {
        let env = crate::accounts::test_support::test_env("api-explicit-replace");
        let memory = Memory::default();
        add_account(&env, Provider::Codex, "one", "alpha");
        save_for_account(&env, &memory, "codex", "alpha", "work", "sk-dummy-existing-key", false).unwrap();
        let before = fs::read(root(&env).join("codex/work/connection.json")).unwrap();
        assert!(save(&env, "codex", "alpha", "WORK", "sk-dummy-replacement", false)
            .unwrap_err()
            .contains("같은 이름"));
        assert_eq!(
            before,
            fs::read(root(&env).join("codex/work/connection.json")).unwrap()
        );
    }
    #[test]
    fn legacy_metadata_loads_without_a_claim_of_verification() {
        let env = crate::accounts::test_support::test_env("api-legacy");
        let dir = root(&env).join("claude/work");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("connection.json"),
            br#"{"provider":"claude","name":"work"}"#,
        )
        .unwrap();
        let entry = list(&env, "claude").unwrap().remove(0);
        assert_eq!(entry.check_state, CheckState::Unchecked);
        assert!(entry.checked_at.is_none());
    }
    #[tokio::test]
    async fn checks_use_provider_headers_and_classify_errors_without_echoing_response() {
        use std::io::{Read, Write};
        for (provider, status, body, expected) in [
            (Provider::Claude, 200, r#"{"data":[]}"#, CheckState::Ready),
            (Provider::Codex, 200, r#"{"data":[]}"#, CheckState::Ready),
            (
                Provider::Codex,
                200,
                "not model JSON",
                CheckState::Unavailable,
            ),
            (
                Provider::Claude,
                401,
                "sk-sensitive-echo-fixture",
                CheckState::InvalidKey,
            ),
            (
                Provider::Claude,
                403,
                "sk-sensitive-echo-fixture",
                CheckState::Forbidden,
            ),
            (
                Provider::Codex,
                429,
                "sk-sensitive-echo-fixture",
                CheckState::RateLimited,
            ),
            (
                Provider::Codex,
                503,
                "sk-sensitive-echo-fixture",
                CheckState::Unavailable,
            ),
            (Provider::Claude, 302, "", CheckState::Unavailable),
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/v1/models", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let mut received = Vec::new();
                let mut buffer = [0; 2048];
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    received.extend_from_slice(&buffer[..count]);
                    if received.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nLocation: http://127.0.0.1:1/never-follow\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                String::from_utf8(received).unwrap().to_lowercase()
            });
            let outcome = check_at(provider, "sk-dummy-header-fixture", &url).await;
            assert_eq!(outcome, expected);
            assert!(!check_message(outcome).contains("sensitive-echo"));
            let request = server.join().unwrap();
            assert!(request.starts_with("get /v1/models "));
            match provider {
                Provider::Claude => {
                    assert!(request.contains("x-api-key: sk-dummy-header-fixture"));
                    assert!(request.contains("anthropic-version: 2023-06-01"));
                }
                Provider::Codex => {
                    assert!(request.contains("authorization: bearer sk-dummy-header-fixture"))
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "local installed CLI smoke test; version only, synthetic key and isolated config"]
    fn installed_clis_accept_generated_launchers_without_generation() {
        use std::os::unix::fs::PermissionsExt;
        let env = crate::accounts::test_support::test_env("api-installed-cli");
        let bin = env.home.join("bin");
        fs::create_dir_all(&bin).unwrap();
        for provider in [Provider::Claude, Provider::Codex] {
            let program = crate::login::resolve_program(provider.dir_name());
            assert!(
                std::path::Path::new(&program).is_file(),
                "installed CLI required"
            );
            let wrapper = bin.join(provider.dir_name());
            fs::write(
                &wrapper,
                format!("#!/bin/bash\nexec {} \"$@\"\n", sh_quote(&program)),
            )
            .unwrap();
            fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
            let script = launcher(&env, provider, "smoke", false).replace(
                &format!(
                    "/usr/bin/security find-generic-password -s {} -w",
                    sh_quote(&service(provider, "smoke"))
                ),
                "printf %s sk-dummy-version-fixture",
            );
            let run = env.home.join(format!("{}.sh", provider.dir_name()));
            fs::write(&run, script).unwrap();
            let output = std::process::Command::new("/bin/bash")
                .arg(run)
                .arg("--version")
                .env(
                    "PATH",
                    format!(
                        "{}:{}",
                        bin.display(),
                        std::env::var("PATH").unwrap_or_default()
                    ),
                )
                .current_dir(&env.home)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "installed CLI must accept generated launch arguments"
            );
            assert!(!output.stdout.is_empty());
            assert!(!env
                .store
                .join("api-workspaces")
                .join(format!("{}-smoke", provider.dir_name()))
                .join("auth.json")
                .exists());
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "local Keychain integration; synthetic key only, cleans up its unique item"]
    fn api_keychain_roundtrip_with_synthetic_key() {
        let env = crate::accounts::test_support::test_env("api-keychain-integration");
        let secrets = OsSecrets(&env);
        let id = format!(
            "Switcher-API-test-{}-{}",
            std::process::id(),
            crate::accounts::now()
        );
        secrets.put(&id, b"sk-dummy-keychain-fixture").unwrap();
        let read = secrets.get(&id);
        let cleanup = secrets.remove(&id);
        assert!(
            cleanup.is_ok(),
            "temporary synthetic Keychain item cleanup must succeed"
        );
        assert_eq!(read.unwrap(), b"sk-dummy-keychain-fixture");
        assert!(secrets.get(&id).is_err());
    }
}

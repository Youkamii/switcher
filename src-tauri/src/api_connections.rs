//! Explicit API connections. Never participate in OAuth switching or TFSD.
//! Metadata and launchers contain no key; macOS Keychain / Windows user DPAPI own secrets.
use crate::accounts::{atomic_write, validate_name, Env, Provider};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, sync::Mutex};

static LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize, Clone)]
pub struct Connection {
    pub provider: String,
    pub name: String,
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

pub fn list(env: &Env, provider: &str) -> Result<Vec<Connection>, String> {
    let _guard = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
    let provider = Provider::parse(provider)?;
    let dir = root(env).join(provider.dir_name());
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    for entry in fs::read_dir(dir).map_err(|_| "API 연결 목록 읽기 실패")? {
        let entry = entry.map_err(|_| "API 연결 읽기 실패")?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if validate_name(&name).is_ok() && entry.path().join("connection.json").is_file() {
            result.push(Connection {
                provider: provider.dir_name().into(),
                name,
            });
        }
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}
fn save_with(
    env: &Env,
    secrets: &impl Secrets,
    provider: &str,
    name: &str,
    key: &str,
) -> Result<(), String> {
    let (provider, dir) = checked(env, provider, name)?;
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
    // Metadata is non-secret. On first registration roll back the secret if metadata fails.
    let existed = dir.join("connection.json").exists();
    secrets.put(&id, key.as_bytes())?;
    let data = serde_json::to_vec(&Connection {
        provider: provider.dir_name().into(),
        name: name.into(),
    })
    .map_err(|_| "API 메타데이터 생성 실패")?;
    if let Err(error) = atomic_write(&dir.join("connection.json"), &data) {
        if !existed {
            let _ = secrets.remove(&id);
        }
        return Err(error);
    }
    Ok(())
}
pub fn save(env: &Env, provider: &str, name: &str, key: &str) -> Result<(), String> {
    let _guard = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
    save_with(env, &OsSecrets(env), provider, name, key)
}
pub fn delete(env: &Env, provider: &str, name: &str) -> Result<(), String> {
    let _guard = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
    let (provider, dir) = checked(env, provider, name)?;
    OsSecrets(env).remove(&service(provider, name))?;
    for file in ["connection.json", "run.sh", "run.ps1"] {
        match fs::remove_file(dir.join(file)) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err("API 연결 파일 삭제 실패 — 다시 시도하세요".into()),
        }
    }
    // CLI history/config lives outside this folder; deleting a connection keeps that history.
    let _ = fs::remove_dir(dir);
    Ok(())
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

pub async fn check(env: &Env, provider: &str, name: &str) -> Result<(), String> {
    let (provider, dir) = checked(env, provider, name)?;
    // The blocking secret operation is called by the command on a blocking worker.
    if !dir.join("connection.json").is_file() {
        return Err("API 연결이 없습니다".into());
    }
    let key = OsSecrets(env).get(&service(provider, name))?;
    let key = String::from_utf8(key).map_err(|_| "API 키 저장 형식 오류")?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|_| "연결 확인 준비 실패")?;
    let request = match provider {
        Provider::Claude => client
            .get("https://api.anthropic.com/v1/models?limit=1")
            .header("x-api-key", &key)
            .header("anthropic-version", "2023-06-01"),
        Provider::Codex => client
            .get("https://api.openai.com/v1/models")
            .bearer_auth(&key),
    };
    let status = request
        .send()
        .await
        .map_err(|_| "네트워크 연결을 확인하세요")?
        .status()
        .as_u16();
    match status {
        200..=299 => Ok(()),
        401 => Err("API 키가 거부됐습니다 — 키를 교체하세요".into()),
        403 => Err("API 키의 모델 조회 권한이 없습니다 — 프로젝트 권한을 확인하세요".into()),
        429 => Err("요청 제한 중입니다 — 잠시 후 다시 확인하세요".into()),
        _ => Err(format!("연결 확인 실패 (HTTP {status})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashMap};
    #[derive(Default)]
    struct Memory(RefCell<HashMap<String, Vec<u8>>>);
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

//! Apply an explicitly selected API connection to the normal CLI configuration.
//! Keep an undo journal before writes and restore only settings we still own.
use super::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use toml_edit::{DocumentMut, Item};

#[cfg(test)]
thread_local! { static FAIL_AUTH_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

#[derive(Clone, Serialize, Deserialize)]
struct Selection {
    name: String,
    revision: String,
    original_config: Option<String>,
    applied_config: String,
    original_auth_present: bool,
    original_oauth_id: Option<String>,
    key_hash: String,
}

fn directory(env: &Env, provider: Provider) -> PathBuf {
    env.store.join("api-active").join(provider.dir_name())
}
fn config_path(env: &Env, provider: Provider) -> PathBuf {
    match provider {
        Provider::Claude => env.home.join(".claude/settings.json"),
        Provider::Codex => env.codex_home.join("config.toml"),
    }
}
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("CLI 설정을 읽지 못했습니다 — 파일 권한을 확인하세요".into()),
    }
}
fn text(bytes: Option<&[u8]>) -> Result<Option<String>, String> {
    bytes
        .map(|bytes| {
            String::from_utf8(bytes.to_vec())
                .map_err(|_| "CLI 설정의 문자 인코딩을 확인하세요".into())
        })
        .transpose()
}
fn read_selection(env: &Env, provider: Provider) -> Result<Option<Selection>, String> {
    read_optional(&directory(env, provider).join("selection.json"))?
        .map(|bytes| {
            serde_json::from_slice(&bytes)
                .map_err(|_| "API 전환 복원 정보를 읽지 못했습니다".into())
        })
        .transpose()
}
fn write_selection(env: &Env, provider: Provider, selection: &Selection) -> Result<(), String> {
    atomic_write(
        &directory(env, provider).join("selection.json"),
        &serde_json::to_vec(selection).map_err(|_| "API 전환 정보 저장 실패")?,
    )
}
fn hash(key: &[u8]) -> String {
    format!("{:x}", Sha256::digest(key))
}
fn json_config(text: Option<&str>) -> Result<Value, String> {
    let value: Value = serde_json::from_str(text.unwrap_or("{}"))
        .map_err(|_| "Claude settings.json 형식을 확인하세요")?;
    if !value.is_object() {
        return Err("Claude settings.json은 JSON 객체여야 합니다".into());
    }
    Ok(value)
}
fn toml_config(text: Option<&str>) -> Result<DocumentMut, String> {
    text.unwrap_or("")
        .parse()
        .map_err(|_| "Codex config.toml 형식을 확인하세요".into())
}
fn provider_item(config: &DocumentMut) -> Option<&Item> {
    config.get("model_providers")?.get("switcher_api")
}
const CLAUDE_ENV: &[(&str, &str)] = &[
    ("ANTHROPIC_API_KEY", ""),
    ("ANTHROPIC_AUTH_TOKEN", ""),
    ("CLAUDE_CODE_OAUTH_TOKEN", ""),
    ("ANTHROPIC_BASE_URL", "https://api.anthropic.com"),
    ("CLAUDE_CODE_USE_BEDROCK", "0"),
    ("CLAUDE_CODE_USE_VERTEX", "0"),
    ("CLAUDE_CODE_USE_FOUNDRY", "0"),
];
fn helper(env: &Env, provider: Provider, name: &str) -> Result<String, String> {
    let (_, dir) = checked(env, provider.dir_name(), name)?;
    let id = service(provider, name);
    let (path, script, command) = if cfg!(windows) {
        let path = dir.join("key.ps1");
        let blob = root(env).join(format!("{id}.dpapi"));
        let script = format!("$ErrorActionPreference = 'Stop'\nAdd-Type -AssemblyName System.Security\n$bytes = [System.Security.Cryptography.ProtectedData]::Unprotect([IO.File]::ReadAllBytes({}), $null, [System.Security.Cryptography.DataProtectionScope]::CurrentUser)\ntry {{ [Console]::Write([Text.Encoding]::UTF8.GetString($bytes)) }} finally {{ [Array]::Clear($bytes, 0, $bytes.Length) }}\n", ps_quote(&blob.to_string_lossy()));
        // cmd.exe expands %VAR% even in quoted paths. Encode the non-secret
        // helper source so path characters never become command syntax.
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(
            script
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        let command = format!("powershell.exe -NoProfile -EncodedCommand {encoded}");
        (path, script, command)
    } else {
        let path = dir.join("key.sh");
        let script = format!("#!/bin/bash\nset +x\nset -euo pipefail\nexec /usr/bin/security find-generic-password -s {} -w\n", sh_quote(&id));
        let command = format!("/bin/bash {}", sh_quote(&path.to_string_lossy()));
        (path, script, command)
    };
    atomic_write(&path, script.as_bytes())?;
    Ok(command)
}

fn applied_config(
    env: &Env,
    provider: Provider,
    name: &str,
    current: Option<&str>,
) -> Result<String, String> {
    match provider {
        Provider::Claude => {
            let mut config = json_config(current)?;
            // apiKeyHelper has precedence over subscription OAuth. Leave OAuth intact.
            config["apiKeyHelper"] = Value::String(helper(env, provider, name)?);
            config["forceLoginMethod"] = json!("console");
            if config.get("env").is_some_and(|env| !env.is_object()) {
                return Err("Claude env 설정 형식을 확인하세요".into());
            }
            if config.get("env").is_none() {
                config["env"] = json!({});
            }
            for (key, value) in CLAUDE_ENV {
                config["env"][key] = json!(value);
            }
            serde_json::to_string_pretty(&config).map_err(|_| "Claude 설정 생성 실패".into())
        }
        Provider::Codex => {
            let mut config = toml_config(current)?;
            match config.get("model_providers") {
                None => {
                    config["model_providers"] = Item::Table(toml_edit::Table::new());
                }
                Some(Item::Value(toml_edit::Value::InlineTable(table))) => {
                    config["model_providers"] = Item::Table(table.clone().into_table());
                }
                Some(Item::Table(_)) => (),
                _ => return Err("Codex model_providers 설정 형식을 확인하세요".into()),
            }
            config["model_provider"] = toml_edit::value("switcher_api");
            if config.get("forced_login_method").is_some() {
                config["forced_login_method"] = toml_edit::value("api");
            }
            let provider_config: DocumentMut = r#"
name = "OpenAI API (Switcher)"
base_url = "https://api.openai.com/v1"
wire_api = "responses"
requires_openai_auth = true
"#
            .parse()
            .unwrap();
            config["model_providers"]["switcher_api"] =
                Item::Table(provider_config.as_table().clone());
            Ok(config.to_string())
        }
    }
}

pub(super) fn selected_name(env: &Env, provider: Provider) -> Result<Option<String>, String> {
    Ok(read_selection(env, provider)?.map(|record| record.name))
}
pub(super) fn protected_oauth_id(env: &Env, provider: Provider) -> Option<String> {
    read_selection(env, provider)
        .ok()
        .flatten()?
        .original_oauth_id
}
/// 다른 PC에서 옮겨 온 로그인으로 복원 지점을 바꾼다 (#189). Codex는 API를 쓰는 동안 그 계정의 구독
/// 로그인이 활성 자리(`auth.json`)가 아니라 여기에 보관돼 있다. 바꾸지 않으면 API 사용을 해제할 때
/// 옛 로그인이 돌아오고, 다음 전환의 백업이 그것으로 방금 받은 로그인을 덮는다.
/// Claude는 API를 쓰는 동안에도 구독 로그인이 활성 자리에 그대로 있어 해당이 없다.
pub(super) fn replace_protected_oauth(
    env: &Env,
    provider: Provider,
    account_id: &str,
    cred: &[u8],
) -> Result<bool, String> {
    if provider != Provider::Codex {
        return Ok(false);
    }
    let Some(record) = read_selection(env, provider)? else {
        return Ok(false);
    };
    if !record.original_auth_present || record.original_oauth_id.as_deref() != Some(account_id) {
        return Ok(false);
    }
    atomic_write(&directory(env, provider).join("auth.before.json"), cred)?;
    Ok(true)
}
pub(super) fn is_active(
    env: &Env,
    provider: Provider,
    connection: &Connection,
) -> Result<bool, String> {
    let Some(record) = read_selection(env, provider)? else {
        return Ok(false);
    };
    if record.name != connection.name
        || (provider == Provider::Codex && record.revision != connection.revision)
    {
        return Ok(false);
    }
    let current = text(read_optional(&config_path(env, provider))?.as_deref())?;
    match provider {
        Provider::Claude => {
            let current = json_config(current.as_deref())?;
            let applied = json_config(Some(&record.applied_config))?;
            Ok(["apiKeyHelper", "forceLoginMethod"]
                .iter()
                .all(|key| current.get(key) == applied.get(key))
                && CLAUDE_ENV.iter().all(|(key, _)| {
                    current.get("env").and_then(|env| env.get(key))
                        == applied.get("env").and_then(|env| env.get(key))
                }))
        }
        Provider::Codex => {
            let current = toml_config(current.as_deref())?;
            let expected = toml_config(Some(&record.applied_config))?;
            Ok(
                current.get("model_provider").and_then(Item::as_str) == Some("switcher_api")
                    && provider_item(&current).map(Item::to_string)
                        == provider_item(&expected).map(Item::to_string)
                    && auth_matches(env, &record)?,
            )
        }
    }
}
fn auth_matches(env: &Env, record: &Selection) -> Result<bool, String> {
    let Some(bytes) = read_optional(&env.live_credential_path(Provider::Codex))? else {
        return Ok(false);
    };
    let auth: Value =
        serde_json::from_slice(&bytes).map_err(|_| "Codex 인증 파일 형식을 확인하세요")?;
    Ok(auth
        .get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .is_some_and(|key| hash(key.as_bytes()) == record.key_hash))
}
fn restore_file(path: &Path, previous: Option<&[u8]>) -> Result<(), String> {
    match previous {
        Some(bytes) => atomic_write(path, bytes),
        None => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("API 전환 복원 파일 정리 실패".into()),
        },
    }
}

pub(super) fn activate_locked(
    env: &Env,
    secrets: &impl Secrets,
    provider: Provider,
    name: &str,
) -> Result<Connection, String> {
    env.check_codex_store(provider)?;
    let (_, dir) = checked(env, provider.dir_name(), name)?;
    let mut connection = read_connection(&dir, provider, name)?;
    let key = secrets.get(&service(provider, name))?;
    let key_str = std::str::from_utf8(&key).map_err(|_| "저장된 API 키를 읽지 못했습니다")?;
    if !valid_key(key_str) {
        return Err("저장된 API 키 형식을 확인하세요".into());
    }
    let previous_record = read_selection(env, provider)?;
    if let Some(record) = previous_record.as_ref() {
        let owned = Connection {
            name: record.name.clone(),
            revision: record.revision.clone(),
            ..connection.clone()
        };
        if !is_active(env, provider, &owned)? {
            // Preserve configuration or login edits made outside Switcher as
            // the new restore point instead of reviving an obsolete snapshot.
            deactivate_locked(env, provider)?;
            return activate_locked(env, secrets, provider, name);
        }
    }
    let config = config_path(env, provider);
    let config_before = read_optional(&config)?;
    let config_text = text(config_before.as_deref())?;
    // Validate before creating a journal or touching live credentials.
    let desired = applied_config(env, provider, name, config_text.as_deref())?;
    let mut record = if let Some(previous) = previous_record.clone() {
        previous
    } else {
        let (profile, backup) = crate::accounts::backup_live_locked(env, provider)?;
        let original_oauth_id = profile
            .as_ref()
            .and_then(|name| crate::accounts::read_meta(&env.profiles_dir(provider).join(name)))
            .map(|meta| meta.id);
        if provider == Provider::Codex {
            if let Some(bytes) = backup.as_deref() {
                atomic_write(&directory(env, provider).join("auth.before.json"), bytes)?;
            }
        }
        Selection {
            name: String::new(),
            revision: String::new(),
            original_config: config_text.clone(),
            applied_config: String::new(),
            original_auth_present: backup.is_some(),
            original_oauth_id,
            key_hash: String::new(),
        }
    };
    record.name = connection.name.clone();
    record.revision = connection.revision.clone();
    record.key_hash = hash(&key);
    record.applied_config = desired.clone();
    write_selection(env, provider, &record)?;
    let apply = || -> Result<(), String> {
        atomic_write(&config, desired.as_bytes())?;
        if provider == Provider::Codex {
            // The standard file adapter requires this active auth copy (0600).
            // Saved connections continue to keep their keys in the OS vault.
            let auth =
                serde_json::to_vec_pretty(&json!({"auth_mode":"apikey","OPENAI_API_KEY":key_str}))
                    .map_err(|_| "Codex API 인증 생성 실패")?;
            #[cfg(test)]
            if FAIL_AUTH_WRITE.with(|fail| fail.replace(false)) {
                return Err("fixture write failure".into());
            }
            crate::accounts::write_live_cred(env, provider, &auth)?;
        }
        Ok(())
    };
    if let Err(error) = apply() {
        // Atomic credential writes either succeed or leave the original file.
        // Restore only our own config write; keep a journal if recovery fails.
        let current = read_optional(&config)?;
        if current.as_deref() == Some(desired.as_bytes()) {
            restore_file(&config, config_before.as_deref())
                .map_err(|_| "API 적용과 설정 복구에 실패했습니다 — API 사용 해제로 복구하세요")?;
        }
        if let Some(previous) = previous_record {
            write_selection(env, provider, &previous)?;
        } else {
            restore_file(&directory(env, provider).join("selection.json"), None)?;
        }
        return Err(error);
    }
    connection.active = true;
    Ok(connection)
}

fn restore_toml_key(
    current: &mut DocumentMut,
    before: &DocumentMut,
    applied: &DocumentMut,
    key: &str,
) {
    if current.get(key).map(Item::to_string) == applied.get(key).map(Item::to_string) {
        if let Some(value) = before.get(key) {
            current[key] = value.clone();
        } else {
            current.as_table_mut().remove(key);
        }
    }
}
pub(super) fn deactivate_locked(env: &Env, provider: Provider) -> Result<(), String> {
    let Some(record) = read_selection(env, provider)? else {
        return Ok(());
    };
    env.check_codex_store(provider)?;
    let path = config_path(env, provider);
    let current_bytes = read_optional(&path)?;
    let current_text = text(current_bytes.as_deref())?;
    let restored = match provider {
        Provider::Claude => {
            let mut current = json_config(current_text.as_deref())?;
            let applied = json_config(Some(&record.applied_config))?;
            let before = json_config(record.original_config.as_deref())?;
            for key in ["apiKeyHelper", "forceLoginMethod"] {
                if current.get(key) == applied.get(key) {
                    if let Some(value) = before.get(key) {
                        current[key] = value.clone();
                    } else {
                        current.as_object_mut().unwrap().remove(key);
                    }
                }
            }
            if let Some(current_env) = current.get_mut("env").and_then(Value::as_object_mut) {
                for (key, _) in CLAUDE_ENV {
                    if current_env.get(*key) == applied.get("env").and_then(|env| env.get(key)) {
                        if let Some(value) = before.get("env").and_then(|env| env.get(key)) {
                            current_env.insert((*key).into(), value.clone());
                        } else {
                            current_env.remove(*key);
                        }
                    }
                }
                if current_env.is_empty() && before.get("env").is_none() {
                    current.as_object_mut().unwrap().remove("env");
                }
            }
            serde_json::to_string_pretty(&current).map_err(|_| "Claude 설정 복구 실패")?
        }
        Provider::Codex => {
            let mut current = toml_config(current_text.as_deref())?;
            let before = toml_config(record.original_config.as_deref())?;
            let applied = toml_config(Some(&record.applied_config))?;
            for key in ["model_provider", "forced_login_method"] {
                restore_toml_key(&mut current, &before, &applied, key);
            }
            if provider_item(&current).map(Item::to_string)
                == provider_item(&applied).map(Item::to_string)
            {
                if let Some(previous) = provider_item(&before) {
                    current["model_providers"]["switcher_api"] = previous.clone();
                } else if let Some(table) = current
                    .get_mut("model_providers")
                    .and_then(Item::as_table_like_mut)
                {
                    table.remove("switcher_api");
                    if table.is_empty() && before.get("model_providers").is_none() {
                        current.as_table_mut().remove("model_providers");
                    }
                }
            }
            // A CLI may have logged into OAuth meanwhile. Never overwrite that login.
            if auth_matches(env, &record)? {
                let backup = read_optional(&directory(env, provider).join("auth.before.json"))?;
                if record.original_auth_present && backup.is_none() {
                    return Err("구독 복원용 인증 파일이 없습니다 — API 선택을 유지했습니다".into());
                }
                restore_file(&env.live_credential_path(provider), backup.as_deref())?;
            }
            current.to_string()
        }
    };
    if current_bytes.as_deref() != Some(restored.as_bytes()) {
        atomic_write(&path, restored.as_bytes())?;
    }
    restore_file(&directory(env, provider).join("selection.json"), None)?;
    // Remove the token backup only after restoration has committed.
    restore_file(&directory(env, provider).join("auth.before.json"), None)?;
    Ok(())
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
            self.0
                .borrow()
                .get(id)
                .cloned()
                .ok_or("missing fixture key".into())
        }
        fn remove(&self, id: &str) -> Result<(), String> {
            self.0.borrow_mut().remove(id);
            Ok(())
        }
    }
    fn oauth(env: &Env, provider: Provider, id: &str) -> Vec<u8> {
        let value = match provider {
            Provider::Claude => {
                json!({"claudeAiOauth":{"accessToken":"fixture-access","refreshToken":format!("fixture-{id}"),"expiresAt":9999999999999_i64}})
            }
            Provider::Codex => {
                json!({"tokens":{"access_token":"fixture-access","refresh_token":format!("fixture-{id}"),"account_id":id}})
            }
        };
        let bytes = serde_json::to_vec(&value).unwrap();
        crate::accounts::write_live_cred(env, provider, &bytes).unwrap();
        if provider == Provider::Claude {
            atomic_write(&env.home.join(".claude.json"), &serde_json::to_vec(&json!({"oauthAccount":{"accountUuid":id,"emailAddress":"fixture@example.test"}})).unwrap()).unwrap();
        }
        bytes
    }
    fn register(env: &Env, secrets: &impl Secrets, provider: Provider, name: &str) {
        save_with(
            env,
            secrets,
            provider.dir_name(),
            name,
            &format!("sk-dummy-activation-{name}-123456"),
        )
        .unwrap();
    }
    #[test]
    fn claude_selects_helper_and_restores_only_its_setting() {
        let env = crate::accounts::test_support::test_env("api-activate-claude");
        let secrets = Memory::default();
        let bytes = oauth(&env, Provider::Claude, "oauth-a");
        atomic_write(
            &config_path(&env, Provider::Claude),
            br#"{"apiKeyHelper":"existing-helper","theme":"dark"}"#,
        )
        .unwrap();
        register(&env, &secrets, Provider::Claude, "work");
        let connection = activate_locked(&env, &secrets, Provider::Claude, "work").unwrap();
        assert!(is_active(&env, Provider::Claude, &connection).unwrap());
        assert_eq!(
            fs::read(env.live_credential_path(Provider::Claude)).unwrap(),
            bytes
        );
        assert_eq!(
            protected_oauth_id(&env, Provider::Claude).as_deref(),
            Some("oauth-a")
        );
        let path = config_path(&env, Provider::Claude);
        let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(config["apiKeyHelper"].as_str().unwrap().contains("key.sh") || cfg!(windows));
        config["theme"] = json!("light");
        atomic_write(&path, &serde_json::to_vec(&config).unwrap()).unwrap();
        deactivate_locked(&env, Provider::Claude).unwrap();
        let restored: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(
            restored,
            json!({"apiKeyHelper":"existing-helper","theme":"light"})
        );
        assert!(!is_active(&env, Provider::Claude, &connection).unwrap());
        assert!(protected_oauth_id(&env, Provider::Claude).is_none());
    }
    #[test]
    fn codex_api_selection_preserves_oauth_and_user_configuration() {
        let env = crate::accounts::test_support::test_env("api-activate-codex");
        let secrets = Memory::default();
        let bytes = oauth(&env, Provider::Codex, "oauth-a");
        let original = "# keep this comment\nmodel_provider = 'custom'\nmodel = 'existing-model'\nforced_login_method = 'chatgpt'\n[model_providers.switcher_api]\nname = 'existing definition'\n";
        let path = config_path(&env, Provider::Codex);
        atomic_write(&path, original.as_bytes()).unwrap();
        register(&env, &secrets, Provider::Codex, "one");
        register(&env, &secrets, Provider::Codex, "two");
        let first = activate_locked(&env, &secrets, Provider::Codex, "one").unwrap();
        assert!(is_active(&env, Provider::Codex, &first).unwrap());
        let mut current = fs::read_to_string(&path).unwrap();
        current.push_str("\n[notifications]\nenabled = true\n");
        atomic_write(&path, current.as_bytes()).unwrap();
        let second = activate_locked(&env, &secrets, Provider::Codex, "two").unwrap();
        assert!(is_active(&env, Provider::Codex, &second).unwrap());
        assert!(!is_active(&env, Provider::Codex, &first).unwrap());
        assert_eq!(
            fs::read(directory(&env, Provider::Codex).join("auth.before.json")).unwrap(),
            bytes
        );
        assert_eq!(
            fs::read(
                env.profiles_dir(Provider::Codex)
                    .join(
                        crate::accounts::find_profile_by_id(&env, Provider::Codex, "oauth-a")
                            .unwrap()
                            .unwrap()
                    )
                    .join("auth.json")
            )
            .unwrap(),
            bytes
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(env.live_credential_path(Provider::Codex))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        deactivate_locked(&env, Provider::Codex).unwrap();
        assert_eq!(
            fs::read(env.live_credential_path(Provider::Codex)).unwrap(),
            bytes
        );
        let restored = fs::read_to_string(path).unwrap();
        assert!(restored.contains("# keep this comment"));
        let restored = toml_config(Some(&restored)).unwrap();
        assert_eq!(restored["model_provider"].as_str(), Some("custom"));
        assert_eq!(restored["forced_login_method"].as_str(), Some("chatgpt"));
        assert_eq!(restored["notifications"]["enabled"].as_bool(), Some(true));
        assert_eq!(
            provider_item(&restored).unwrap()["name"].as_str(),
            Some("existing definition")
        );
    }
    #[test]
    fn selecting_subscription_restores_api_settings_before_switching() {
        let env = crate::accounts::test_support::test_env("api-to-subscription");
        let secrets = Memory::default();
        oauth(&env, Provider::Claude, "oauth-b");
        crate::accounts::save_current(&env, Provider::Claude, "b").unwrap();
        oauth(&env, Provider::Claude, "oauth-a");
        register(&env, &secrets, Provider::Claude, "work");
        activate_locked(&env, &secrets, Provider::Claude, "work").unwrap();
        assert_eq!(selected_name(&env, Provider::Claude).unwrap().as_deref(), Some("work"));
        crate::accounts::switch(&env, Provider::Claude, "b").unwrap();
        assert!(selected_name(&env, Provider::Claude).unwrap().is_none());
        assert_eq!(
            crate::accounts::live_identity(&env, Provider::Claude)
                .unwrap()
                .unwrap()
                .id,
            "oauth-b"
        );
        let config: Value =
            serde_json::from_slice(&fs::read(config_path(&env, Provider::Claude)).unwrap())
                .unwrap();
        assert!(config.get("apiKeyHelper").is_none());
    }
    #[test]
    fn failed_auth_write_restores_configuration_and_previous_selection() {
        let env = crate::accounts::test_support::test_env("api-activate-rollback");
        let secrets = Memory::default();
        let bytes = oauth(&env, Provider::Codex, "oauth-a");
        register(&env, &secrets, Provider::Codex, "one");
        register(&env, &secrets, Provider::Codex, "two");
        FAIL_AUTH_WRITE.with(|fail| fail.set(true));
        assert!(activate_locked(&env, &secrets, Provider::Codex, "one").is_err());
        assert_eq!(
            fs::read(env.live_credential_path(Provider::Codex)).unwrap(),
            bytes
        );
        assert!(!config_path(&env, Provider::Codex).exists());
        assert!(selected_name(&env, Provider::Codex).unwrap().is_none());
        let first = activate_locked(&env, &secrets, Provider::Codex, "one").unwrap();
        FAIL_AUTH_WRITE.with(|fail| fail.set(true));
        assert!(activate_locked(&env, &secrets, Provider::Codex, "two").is_err());
        assert!(is_active(&env, Provider::Codex, &first).unwrap());
    }
    #[test]
    fn external_settings_and_fresh_oauth_are_not_overwritten_on_restore() {
        let env = crate::accounts::test_support::test_env("api-external-edits");
        let secrets = Memory::default();
        for provider in [Provider::Claude, Provider::Codex] {
            oauth(&env, provider, "original");
            register(&env, &secrets, provider, "work");
            activate_locked(&env, &secrets, provider, "work").unwrap();
            let fresh = oauth(&env, provider, "new-login");
            let path = config_path(&env, provider);
            if provider == Provider::Claude {
                atomic_write(&path, br#"{"apiKeyHelper":"new-helper"}"#).unwrap();
            }
            deactivate_locked(&env, provider).unwrap();
            assert_eq!(fs::read(env.live_credential_path(provider)).unwrap(), fresh);
            if provider == Provider::Claude {
                assert_eq!(
                    json_config(
                        text(read_optional(&path).unwrap().as_deref())
                            .unwrap()
                            .as_deref()
                    )
                    .unwrap()["apiKeyHelper"],
                    "new-helper"
                );
            }
        }
    }
    #[test]
    fn missing_credentials_and_malformed_config_cannot_destroy_oauth() {
        let env = crate::accounts::test_support::test_env("api-invalid-config");
        let secrets = Memory::default();
        let bytes = oauth(&env, Provider::Codex, "oauth-a");
        register(&env, &secrets, Provider::Codex, "work");
        atomic_write(
            &config_path(&env, Provider::Codex),
            b"broken = [ secret-fixture",
        )
        .unwrap();
        let error = activate_locked(&env, &secrets, Provider::Codex, "work")
            .err()
            .unwrap();
        assert!(!error.contains("secret-fixture"));
        assert_eq!(
            fs::read(env.live_credential_path(Provider::Codex)).unwrap(),
            bytes
        );
        assert!(selected_name(&env, Provider::Codex).unwrap().is_none());
    }

    /// Local only: isolated homes, one fake Keychain item, and status commands.
    /// No model request, real API key, real OAuth store, or terminal window.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn installed_clis_recognize_api_selection_without_a_launcher() {
        use std::process::{Command, Stdio};
        for provider in [Provider::Claude, Provider::Codex] {
            let env = crate::accounts::test_support::test_env(&format!(
                "api-native-selection-{}",
                provider.dir_name()
            ));
            let name = format!("native-api-{}", std::process::id());
            struct Cleanup(Env, Provider, String);
            impl Drop for Cleanup {
                fn drop(&mut self) {
                    let _ = OsSecrets(&self.0).remove(&service(self.1, &self.2));
                }
            }
            let _cleanup = Cleanup(env.clone(), provider, name.clone());
            save_with(
                &env,
                &OsSecrets(&env),
                provider.dir_name(),
                &name,
                "sk-dummy-native-selection-123456",
            )
            .unwrap();
            activate_locked(&env, &OsSecrets(&env), provider, &name).unwrap();
            let mut command = Command::new(crate::login::resolve_program(provider.dir_name()));
            match provider {
                Provider::Claude => {
                    command
                        .args(["auth", "status", "--json"])
                        .env("CLAUDE_CONFIG_DIR", env.home.join(".claude"));
                }
                Provider::Codex => {
                    command
                        .args(["login", "status"])
                        .env("CODEX_HOME", &env.codex_home);
                }
            }
            for key in [
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
                "CLAUDE_CODE_OAUTH_TOKEN",
                "OPENAI_API_KEY",
                "CODEX_API_KEY",
                "ANTHROPIC_BASE_URL",
                "OPENAI_BASE_URL",
                "CLAUDE_CODE_USE_BEDROCK",
                "CLAUDE_CODE_USE_VERTEX",
                "CLAUDE_CODE_USE_FOUNDRY",
            ] {
                command.env_remove(key);
            }
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut child = command.spawn().expect("installed CLI starts");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while child.try_wait().unwrap().is_none() {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("CLI status timed out");
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "CLI did not recognize selected API credentials"
            );
            match provider {
                Provider::Claude => {
                    let status: Value =
                        serde_json::from_slice(&output.stdout).expect("Claude status is JSON");
                    assert_eq!(status["loggedIn"], true);
                    assert_eq!(status["authMethod"], "api_key_helper");
                    assert_eq!(status["apiKeySource"], "apiKeyHelper");
                }
                Provider::Codex => {
                    let status = format!(
                        "{}{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                    assert!(
                        status.contains("API key"),
                        "Codex must report API authentication"
                    );
                }
            }
            super::super::delete(&env, provider.dir_name(), &name).unwrap();
            assert!(selected_name(&env, provider).unwrap().is_none());
            assert!(OsSecrets(&env).get(&service(provider, &name)).is_err());
        }
    }
}

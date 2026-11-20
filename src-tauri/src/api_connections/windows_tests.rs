//! Windows integration tests use real DPAPI and PowerShell with synthetic keys.
//! All account/config files stay in a temporary home; no model requests are made.
use super::*;
use serde_json::{json, Value};
use std::os::windows::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Fixture(Env);
impl Fixture {
    fn new(tag: &str) -> Self {
        let mut env = crate::accounts::test_support::test_env(tag);
        // Exercise Unicode, quotes, cmd metacharacters, and %VAR% expansion.
        env.store = env.home.join("키 ' & %SWITCHER_PROBE%");
        Self(env)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0.home);
    }
}

fn output(command: &mut Command) -> Output {
    let mut child = command
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Windows test process starts");
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = Command::new("taskkill.exe")
                .args(["/F", "/T", "/PID", &child.id().to_string()])
                .creation_flags(0x08000000)
                .output();
            let _ = child.wait();
            panic!("Windows test process timed out");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.wait_with_output().unwrap()
}

fn oauth(env: &Env, provider: Provider, id: &str) -> Vec<u8> {
    let value = match provider {
        Provider::Claude => {
            atomic_write(
                &env.home.join(".claude.json"),
                &serde_json::to_vec(&json!({"oauthAccount": {
                    "accountUuid": id, "emailAddress": "fixture@example.test"
                }}))
                .unwrap(),
            )
            .unwrap();
            json!({"claudeAiOauth": {
                "accessToken": "fixture-access", "refreshToken": format!("fixture-{id}"),
                "expiresAt": 9999999999999_i64
            }})
        }
        Provider::Codex => json!({"tokens": {
            "access_token": "fixture-access", "refresh_token": format!("fixture-{id}"),
            "account_id": id
        }}),
    };
    let bytes = serde_json::to_vec(&value).unwrap();
    crate::accounts::write_live_cred(env, provider, &bytes).unwrap();
    bytes
}

fn assert_selected_key(env: &Env, provider: Provider, key: &str) {
    let active = active_connection(env, provider).expect("saved key is active");
    assert_eq!((active.name.as_str(), active.account_id.as_deref()), ("work", Some("oauth-b")));
    match provider {
        Provider::Claude => {
            let config: Value =
                serde_json::from_slice(&fs::read(env.home.join(".claude/settings.json")).unwrap())
                    .unwrap();
            let command = config["apiKeyHelper"].as_str().unwrap();
            assert!(!command.contains(key));
            let result = output(
                Command::new("cmd.exe")
                    .args(["/D", "/S", "/C", command])
                    .env("SWITCHER_PROBE", "must-not-expand")
                    .current_dir(&env.home),
            );
            assert!(result.status.success(), "PowerShell helper must succeed");
            assert!(
                result.stdout == key.as_bytes(),
                "helper must read the selected DPAPI key"
            );
            // Startup diagnostics on stderr do not change the helper protocol:
            // its exit status and stdout carry the result. Never log the key.
            assert!(
                !result
                    .stderr
                    .windows(key.len())
                    .any(|part| part == key.as_bytes()),
                "helper must not expose its key in diagnostics"
            );
        }
        Provider::Codex => {
            let auth: Value =
                serde_json::from_slice(&fs::read(env.live_credential_path(provider)).unwrap())
                    .unwrap();
            assert_eq!(auth["auth_mode"], "apikey");
            assert!(auth["OPENAI_API_KEY"].as_str() == Some(key));
        }
    }
}

#[test]
fn windows_dpapi_roundtrip_rotation_corruption_and_delete() {
    let fixture = Fixture::new("win-vault");
    let env = &fixture.0;
    let secrets = OsSecrets(env);
    for provider in [Provider::Claude, Provider::Codex] {
        tests::add_account(env, provider, "b", "oauth-b");
        let id = service(provider, "work");
        let key = "sk-dummy-windows-vault-fixture-123456";
        save(env, provider.dir_name(), "oauth-b", "work", key, false).unwrap();
        let blob = root(env).join(format!("{id}.dpapi"));
        let ciphertext = fs::read(&blob).unwrap();
        assert!(!ciphertext
            .windows(key.len())
            .any(|part| part == key.as_bytes()));
        assert!(secrets.get(&id).unwrap() == key.as_bytes());
        let mut corrupt = ciphertext;
        corrupt[0] ^= 0xff;
        fs::write(&blob, corrupt).unwrap();
        let error = secrets.get(&id).unwrap_err();
        assert!(!error.contains(key));
        let replacement = "sk-dummy-windows-replacement-fixture-123456";
        save(env, provider.dir_name(), "oauth-b", "work", replacement, true).unwrap();
        assert!(secrets.get(&id).unwrap() == replacement.as_bytes());
        delete(env, provider.dir_name(), "work").unwrap();
        assert!(!blob.exists());
        assert!(secrets.get(&id).is_err());
        assert!(list(env, provider.dir_name()).unwrap().is_empty());
    }
}

#[test]
fn windows_api_selection_rotation_stop_subscription_and_delete_restore() {
    for provider in [Provider::Claude, Provider::Codex] {
        let fixture = Fixture::new(&format!("win-flow-{}", provider.dir_name()));
        let env = &fixture.0;
        let target = oauth(env, provider, "oauth-b");
        crate::accounts::save_current(env, provider, "b").unwrap();
        let original = oauth(env, provider, "oauth-a");
        let (config_path, original_config) = match provider {
            Provider::Claude => (
                env.home.join(".claude/settings.json"),
                "{\"theme\":\"dark\",\"apiKeyHelper\":\"original-helper\"}",
            ),
            Provider::Codex => (
                env.codex_home.join("config.toml"),
                "model = 'fixture-model'\nforced_login_method = 'chatgpt'\n",
            ),
        };
        atomic_write(&config_path, original_config.as_bytes()).unwrap();
        let key = "sk-dummy-windows-active-fixture-123456";
        save(env, provider.dir_name(), "oauth-b", "work", key, false).unwrap();
        activate(env, provider.dir_name(), "work").unwrap();
        assert_selected_key(env, provider, key);
        if provider == Provider::Claude {
            assert_eq!(
                fs::read(env.live_credential_path(provider)).unwrap(),
                original
            );
        }
        let replacement = "sk-dummy-windows-rotated-fixture-123456";
        save(env, provider.dir_name(), "oauth-b", "work", replacement, true).unwrap();
        activate(env, provider.dir_name(), "work").unwrap();
        assert_selected_key(env, provider, replacement);
        deactivate(env, provider.dir_name()).unwrap();
        assert!(active_connection(env, provider).is_none());
        assert_eq!(
            fs::read(env.live_credential_path(provider)).unwrap(),
            original
        );
        match provider {
            Provider::Claude => assert_eq!(
                serde_json::from_slice::<Value>(&fs::read(&config_path).unwrap()).unwrap(),
                serde_json::from_str::<Value>(original_config).unwrap()
            ),
            Provider::Codex => assert_eq!(
                toml::from_str::<toml::Value>(&fs::read_to_string(&config_path).unwrap()).unwrap(),
                toml::from_str::<toml::Value>(original_config).unwrap()
            ),
        }
        activate(env, provider.dir_name(), "work").unwrap();
        crate::accounts::switch(env, provider, "b").unwrap();
        assert!(active_connection(env, provider).is_none());
        assert_eq!(
            fs::read(env.live_credential_path(provider)).unwrap(),
            target
        );
        activate(env, provider.dir_name(), "work").unwrap();
        delete(env, provider.dir_name(), "work").unwrap();
        assert!(active_connection(env, provider).is_none());
        assert_eq!(
            fs::read(env.live_credential_path(provider)).unwrap(),
            target
        );
        assert!(OsSecrets(env).get(&service(provider, "work")).is_err());
    }
}

#[test]
#[ignore = "requires installed Claude/Codex CLIs; Windows CI runs with synthetic keys"]
fn windows_installed_clis_recognize_api_selection() {
    for provider in [Provider::Claude, Provider::Codex] {
        let fixture = Fixture::new(&format!("win-cli-{}", provider.dir_name()));
        let env = &fixture.0;
        tests::add_account(env, provider, "b", "oauth-b");
        save(
            env,
            provider.dir_name(),
            "oauth-b",
            "work",
            "sk-dummy-windows-cli-fixture-123456",
            false,
        )
        .unwrap();
        activate(env, provider.dir_name(), "work").unwrap();
        let mut command = Command::new("cmd.exe");
        command.args(["/D", "/S", "/C"]);
        match provider {
            Provider::Claude => {
                command
                    .arg("claude auth status --json")
                    .env("CLAUDE_CONFIG_DIR", env.home.join(".claude"));
            }
            Provider::Codex => {
                command
                    .arg("codex login status")
                    .env("CODEX_HOME", &env.codex_home);
            }
        }
        for name in [
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
            command.env_remove(name);
        }
        let result = output(command.current_dir(&env.home));
        assert!(
            result.status.success(),
            "installed CLI must recognize API credentials"
        );
        match provider {
            Provider::Claude => {
                let status: Value =
                    serde_json::from_slice(&result.stdout).expect("Claude JSON status");
                assert_eq!(status["loggedIn"], true);
                assert_eq!(status["authMethod"], "api_key_helper");
                assert_eq!(status["apiKeySource"], "apiKeyHelper");
            }
            Provider::Codex => {
                assert!(
                    String::from_utf8_lossy(&result.stdout).contains("API key")
                        || String::from_utf8_lossy(&result.stderr).contains("API key"),
                    "Codex must report API authentication"
                );
            }
        }
        delete(env, provider.dir_name(), "work").unwrap();
        assert!(active_connection(env, provider).is_none());
    }
}

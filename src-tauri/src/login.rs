//! 위젯에서 새 계정 추가 — 격리된 설정 폴더에 CLI 로그인을 돌린 뒤 결과만 프로필로 가져온다.
//!
//! 왜 격리하나: 그냥 로그인하면 지금 쓰는 계정의 토큰·계정 정보가 덮어써진다.
//! CLAUDE_CONFIG_DIR / CODEX_HOME을 임시 폴더로 지정하면 새 계정 정보가 그 폴더에만 생기고
//! 활성 계정은 전혀 건드리지 않는다 (실측 확인).
//!
//! 주의: `claude auth login`은 stdin을 리다이렉트하면 즉시 종료된다 — stdin은 건드리지 않는다.

use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::accounts::{
    auto_name, find_profile_by_id, jwt_payload, now, read_json, write_profile_parts, Env,
    LiveIdentity, Provider, MUTATION_LOCK,
};

/// 사용자가 브라우저에서 로그인을 마칠 때까지 기다리는 최대 시간
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const POLL_INTERVAL: Duration = Duration::from_millis(400);

#[derive(Serialize)]
pub struct LoginOutcome {
    pub profile: String,
    pub email: Option<String>,
    /// 이미 저장돼 있던 계정을 다시 로그인한 경우 (새 계정이 아님)
    pub updated_existing: bool,
}

/// 진행 중인 로그인 프로세스 — 취소할 수 있게 붙잡아 둔다
static ACTIVE_LOGIN: Mutex<Option<Child>> = Mutex::new(None);

fn spawn_login(provider: Provider, config_dir: &Path) -> Result<Child, String> {
    let (program, args, env_key) = match provider {
        Provider::Claude => ("claude", ["auth", "login"].as_slice(), "CLAUDE_CONFIG_DIR"),
        Provider::Codex => ("codex", ["login"].as_slice(), "CODEX_HOME"),
    };

    #[cfg(windows)]
    let mut cmd = {
        // npm 전역 설치본은 .cmd 셔임이라 cmd 경유로 실행한다.
        // CREATE_NO_WINDOW로 콘솔 창이 뜨지 않게 한다.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut c = Command::new("cmd");
        c.arg("/c").arg(program).args(args);
        c.creation_flags(CREATE_NO_WINDOW);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new(program);
        c.args(args);
        c
    };

    cmd.env(env_key, config_dir);
    // stdin은 상속 그대로 둔다 (리다이렉트하면 claude auth login이 바로 종료됨)
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().map_err(|e| {
        format!("{program} 실행에 실패했습니다: {e} — CLI가 설치되어 있는지 확인하세요")
    })
}

/// 로그인 완료를 기다린다. 취소되면 Err.
fn wait_for_exit() -> Result<(), String> {
    let started = Instant::now();
    loop {
        {
            let mut guard = ACTIVE_LOGIN.lock().map_err(|_| "내부 잠금 오류")?;
            let Some(child) = guard.as_mut() else {
                // cancel_login이 이미 정리했다
                return Err("로그인을 취소했습니다".into());
            };
            match child.try_wait() {
                Ok(Some(_)) => {
                    *guard = None;
                    return Ok(());
                }
                Ok(None) => {}
                Err(e) => {
                    *guard = None;
                    return Err(format!("로그인 프로세스 상태 확인 실패: {e}"));
                }
            }
        }
        if started.elapsed() > LOGIN_TIMEOUT {
            cancel();
            return Err("시간이 초과됐습니다 — 다시 시도하세요".into());
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// 격리 폴더에 생긴 로그인 결과를 읽어 계정 정보와 토큰을 뽑는다
fn read_login_result(
    provider: Provider,
    config_dir: &Path,
) -> Result<(LiveIdentity, Vec<u8>, Option<Value>), String> {
    match provider {
        Provider::Claude => {
            let cred_path = config_dir.join(".credentials.json");
            if !cred_path.exists() {
                return Err("로그인이 완료되지 않았습니다".into());
            }
            let cred = fs::read(&cred_path).map_err(|e| format!("읽기 실패: {e}"))?;
            let root = read_json(&config_dir.join(".claude.json"))?;
            let acc = root
                .get("oauthAccount")
                .ok_or("로그인 결과에서 계정 정보를 찾지 못했습니다")?;
            let id = acc
                .get("accountUuid")
                .and_then(|v| v.as_str())
                .ok_or("로그인 결과에 계정 식별자가 없습니다")?
                .to_string();
            let email = acc
                .get("emailAddress")
                .and_then(|v| v.as_str())
                .map(String::from);
            Ok((LiveIdentity { id, email }, cred, Some(acc.clone())))
        }
        Provider::Codex => {
            let cred_path = config_dir.join("auth.json");
            if !cred_path.exists() {
                return Err("로그인이 완료되지 않았습니다".into());
            }
            let cred = fs::read(&cred_path).map_err(|e| format!("읽기 실패: {e}"))?;
            let root: Value =
                serde_json::from_slice(&cred).map_err(|e| format!("JSON 파싱 실패: {e}"))?;
            let tokens = root
                .get("tokens")
                .ok_or("로그인 결과에서 계정 정보를 찾지 못했습니다")?;
            let id_token = tokens.get("id_token").and_then(|v| v.as_str());
            let id = tokens
                .get("account_id")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| {
                    id_token
                        .and_then(jwt_payload)
                        .and_then(|p| p.get("sub").and_then(|v| v.as_str()).map(String::from))
                })
                .ok_or("로그인 결과에 계정 식별자가 없습니다")?;
            let email = id_token
                .and_then(jwt_payload)
                .and_then(|p| p.get("email").and_then(|v| v.as_str()).map(String::from));
            Ok((LiveIdentity { id, email }, cred, None))
        }
    }
}

fn temp_config_dir(env: &Env) -> PathBuf {
    env.store.join("_login").join(now().to_string())
}

/// 진행 중인 로그인을 중단한다.
/// Windows에서는 cmd 셔임을 거쳐 실행하므로 부모만 죽이면 실제 CLI가 살아남는다 —
/// 트리째 종료해 콜백 서버가 떠 있는 채로 남지 않게 한다.
pub fn cancel() {
    if let Ok(mut guard) = ACTIVE_LOGIN.lock() {
        if let Some(mut child) = guard.take() {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                let _ = Command::new("taskkill")
                    .args(["/T", "/F", "/PID", &child.id().to_string()])
                    .creation_flags(CREATE_NO_WINDOW)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// 브라우저 로그인을 띄우고, 끝나면 결과를 프로필로 저장한다.
/// 활성 계정 파일은 어느 경로에서도 건드리지 않는다.
pub fn add_account(env: &Env, provider: Provider) -> Result<LoginOutcome, String> {
    {
        let guard = ACTIVE_LOGIN.lock().map_err(|_| "내부 잠금 오류")?;
        if guard.is_some() {
            return Err("이미 로그인이 진행 중입니다".into());
        }
    }

    let config_dir = temp_config_dir(env);
    fs::create_dir_all(&config_dir)
        .map_err(|e| format!("임시 폴더 생성 실패 {}: {e}", config_dir.display()))?;

    let child = spawn_login(provider, &config_dir).inspect_err(|_| {
        let _ = fs::remove_dir_all(&config_dir);
    })?;
    {
        let mut guard = ACTIVE_LOGIN.lock().map_err(|_| "내부 잠금 오류")?;
        *guard = Some(child);
    }

    let result = wait_for_exit().and_then(|()| read_login_result(provider, &config_dir));

    // 토큰이 남은 임시 폴더는 성공·실패와 무관하게 반드시 지운다
    let cleanup = |r: Result<LoginOutcome, String>| {
        let _ = fs::remove_dir_all(&config_dir);
        r
    };

    let (ident, cred, block) = match result {
        Ok(v) => v,
        Err(e) => return cleanup(Err(e)),
    };

    // 프로필을 실제로 건드리는 구간에서만 잠근다 (로그인 대기 중에는 전환·저장이 계속 가능해야 한다)
    let _guard = match MUTATION_LOCK.lock() {
        Ok(g) => g,
        Err(_) => return cleanup(Err("내부 잠금 오류".into())),
    };

    let existing = match find_profile_by_id(env, provider, &ident.id) {
        Ok(v) => v,
        Err(e) => return cleanup(Err(e)),
    };
    let updated_existing = existing.is_some();
    let name = existing.unwrap_or_else(|| auto_name(env, provider, &ident));

    if let Err(e) = write_profile_parts(env, provider, &name, &ident, &cred, block.as_ref()) {
        return cleanup(Err(e));
    }

    cleanup(Ok(LoginOutcome {
        profile: name,
        email: ident.email,
        updated_existing,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_env(tag: &str) -> Env {
        let base = std::env::temp_dir().join(format!("switcher-login-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        Env {
            home: base.clone(),
            store: base.join(".switcher"),
        }
    }

    /// 격리 폴더에 로그인이 끝난 상태를 흉내내고, 임포트가 프로필을 만드는지 본다
    #[test]
    fn imports_claude_login_result() {
        let env = test_env("claude-import");
        let cfg = env.store.join("_login").join("t1");
        fs::create_dir_all(&cfg).unwrap();
        fs::write(
            cfg.join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"new-tok"}}"#,
        )
        .unwrap();
        fs::write(
            cfg.join(".claude.json"),
            r#"{"oauthAccount":{"accountUuid":"uuid-new","emailAddress":"newbie@test.dev"}}"#,
        )
        .unwrap();

        let (ident, cred, block) = read_login_result(Provider::Claude, &cfg).unwrap();
        assert_eq!(ident.id, "uuid-new");
        assert_eq!(ident.email.as_deref(), Some("newbie@test.dev"));
        assert!(block.is_some());

        let name = auto_name(&env, Provider::Claude, &ident);
        write_profile_parts(&env, Provider::Claude, &name, &ident, &cred, block.as_ref()).unwrap();

        assert_eq!(name, "newbie");
        let snap = crate::accounts::list(&env, Provider::Claude).unwrap();
        assert_eq!(snap.profiles.len(), 1);
        assert_eq!(snap.profiles[0].name, "newbie");
        // 계정 정보(oauth_account.json)까지 저장돼야 전환 대상이 될 수 있다
        assert!(env
            .profiles_dir(Provider::Claude)
            .join("newbie")
            .join("oauth_account.json")
            .exists());
    }

    #[test]
    fn imports_codex_login_result() {
        use base64::Engine;
        let env = test_env("codex-import");
        let cfg = env.store.join("_login").join("t1");
        fs::create_dir_all(&cfg).unwrap();
        let enc = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        let jwt = format!(
            "{}.{}.{}",
            enc(r#"{"alg":"none"}"#),
            enc(r#"{"email":"cx@test.dev","sub":"sub-1"}"#),
            enc("sig")
        );
        fs::write(
            cfg.join("auth.json"),
            format!(
                r#"{{"tokens":{{"id_token":"{jwt}","access_token":"a","account_id":"acct-new"}}}}"#
            ),
        )
        .unwrap();

        let (ident, _cred, block) = read_login_result(Provider::Codex, &cfg).unwrap();
        assert_eq!(ident.id, "acct-new");
        assert_eq!(ident.email.as_deref(), Some("cx@test.dev"));
        assert!(block.is_none());
    }

    /// 실환경 검증: 로그인을 실제로 띄웠다가 취소해도 활성 계정 파일이 변하지 않아야 한다.
    /// 브라우저 탭이 한 번 열린다. CI에서는 돌지 않는다: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn real_add_account_never_touches_active() {
        let env = Env::real().unwrap();
        let live_cred = env.live_credential_path(Provider::Claude);
        let claude_json = env.home.join(".claude.json");
        let before_cred = fs::read(&live_cred).unwrap();
        let before_account = read_json(&claude_json).unwrap()["oauthAccount"].clone();
        let profiles_before = crate::accounts::list(&env, Provider::Claude)
            .unwrap()
            .profiles
            .len();

        // 백그라운드에서 로그인을 시작하고 잠시 뒤 취소한다
        let handle = std::thread::spawn(move || {
            let env = Env::real().unwrap();
            add_account(&env, Provider::Claude)
        });
        std::thread::sleep(Duration::from_secs(6));
        cancel();
        let result = handle.join().unwrap();

        assert!(result.is_err(), "취소했는데 성공으로 판정됐다");
        assert_eq!(
            before_cred,
            fs::read(&live_cred).unwrap(),
            "활성 토큰 파일이 변경됐다"
        );
        assert_eq!(
            before_account,
            read_json(&claude_json).unwrap()["oauthAccount"],
            "활성 계정 정보가 변경됐다"
        );
        assert_eq!(
            profiles_before,
            crate::accounts::list(&env, Provider::Claude)
                .unwrap()
                .profiles
                .len(),
            "프로필이 생기거나 사라졌다"
        );
        // 토큰이 남은 임시 폴더는 지워져야 한다
        let login_root = env.store.join("_login");
        if login_root.exists() {
            let leftovers: Vec<_> = fs::read_dir(&login_root).unwrap().flatten().collect();
            assert!(leftovers.is_empty(), "임시 로그인 폴더가 남았다");
        }
    }

    #[test]
    fn incomplete_login_is_reported() {
        let env = test_env("incomplete");
        let cfg = env.store.join("_login").join("t1");
        fs::create_dir_all(&cfg).unwrap();
        // .claude.json만 생기고 토큰은 없는 상태 = 로그인 미완료
        fs::write(cfg.join(".claude.json"), r#"{"numStartups":1}"#).unwrap();
        // LiveIdentity는 이메일을 담으므로 Debug를 붙이지 않는다 — unwrap_err 대신 match
        match read_login_result(Provider::Claude, &cfg) {
            Err(e) => assert!(e.contains("완료되지 않"), "예상과 다른 에러: {e}"),
            Ok(_) => panic!("토큰이 없는데 성공으로 판정됐다"),
        }
    }
}

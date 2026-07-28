//! 위젯에서 새 계정 추가 — 로그인 링크를 위젯에 띄우고, 원하는 브라우저에서 로그인한 뒤
//! 받은 코드를 위젯에 붙여넣으면 계정이 등록된다.
//!
//! 왜 격리하나: 그냥 로그인하면 지금 쓰는 계정의 토큰·계정 정보가 덮어써진다.
//! CLAUDE_CONFIG_DIR / CODEX_HOME을 임시 폴더로 지정하면 새 계정 정보가 그 폴더에만 생기고
//! 활성 계정은 전혀 건드리지 않는다 (실측 확인).
//!
//! 왜 PTY(가짜 콘솔)를 쓰나: 출력을 파이프로 받으면 CLI가 화면을 그리지 않아
//! "Opening browser to sign in…" 한 줄만 나온다. 진짜 콘솔을 붙여야 로그인 링크와
//! 코드 입력 프롬프트가 나온다 (실측 확인). ESC[6n(커서 위치 질의)에 답하지 않으면
//! 그마저도 그려지지 않는다.

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::accounts::{
    auto_name, find_profile_by_id, jwt_payload, now, read_json, write_profile_parts, Env,
    LiveIdentity, Provider, MUTATION_LOCK,
};

/// 로그인 링크가 화면에 뜰 때까지 기다리는 시간
const PROMPT_TIMEOUT: Duration = Duration::from_secs(60);
/// 코드 입력 후 로그인이 끝날 때까지 기다리는 시간
const FINISH_TIMEOUT: Duration = Duration::from_secs(120);
/// 코덱스처럼 브라우저에서 코드를 넣고 CLI가 알아서 끝내는 방식의 대기 시간
const DEVICE_TIMEOUT: Duration = Duration::from_secs(600);
const POLL: Duration = Duration::from_millis(300);

#[derive(Serialize)]
pub struct LoginPrompt {
    /// 사용자가 원하는 브라우저에 붙여넣을 로그인 주소
    pub url: String,
    /// 코덱스처럼 웹페이지에 입력해야 하는 일회용 코드 (없으면 None)
    pub device_code: Option<String>,
    /// true면 브라우저에서 받은 코드를 위젯에 붙여넣어야 한다 (클로드)
    pub needs_code: bool,
}

#[derive(Serialize)]
pub struct LoginOutcome {
    pub profile: String,
    pub email: Option<String>,
    /// 이미 저장돼 있던 계정을 다시 로그인한 경우 (새 계정이 아님)
    pub updated_existing: bool,
}

struct Session {
    provider: Provider,
    config_dir: PathBuf,
    child: Box<dyn Child + Send + Sync>,
    /// PTY 입력 통로. 읽기 스레드(터미널 질의 응답)와 코드 입력이 함께 쓴다.
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// 화면 누적 버퍼 — 읽기 스레드가 계속 채운다 (진단·확장용으로 살려 둔다)
    _output: Arc<Mutex<Vec<u8>>>,
    /// PTY를 살려둬야 자식 프로세스가 끊기지 않는다
    _master: Box<dyn MasterPty + Send>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

/// ANSI 이스케이프 시퀀스를 걷어내 사람이 읽는 글자만 남긴다.
/// OSC 8 하이퍼링크 안에도 주소가 들어 있지만, 화면에 보이는 주소가 따로 있으므로 통째로 버린다.
fn strip_ansi(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != 0x1b {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        i += 1;
        match bytes.get(i) {
            // CSI: 최종 바이트(@~)까지 건너뛴다.
            // 색상(SGR, 최종 바이트 m)은 글자 중간에도 끼므로 그냥 버리고,
            // 커서 이동·지우기는 화면상 위치가 바뀐다는 뜻이라 줄바꿈으로 바꿔 토큰을 끊는다.
            // (TUI는 줄바꿈 대신 커서 이동으로 그리기 때문에 이렇게 해야 글자가 붙지 않는다)
            Some(b'[') => {
                i += 1;
                let mut final_byte = 0u8;
                while i < bytes.len() {
                    if (0x40..=0x7e).contains(&bytes[i]) {
                        final_byte = bytes[i];
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                if final_byte != b'm' {
                    out.push(b'\n');
                }
            }
            // OSC: BEL 또는 ESC \ 까지 건너뛴다 (하이퍼링크 대상 주소는 버리고 화면 글자만 남긴다)
            Some(b']') => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                out.push(b'\n');
            }
            _ => i += 1,
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 화면 글자에서 로그인 주소를 찾는다
fn extract_url(text: &str) -> Option<String> {
    let start = text.find("https://")?;
    let url: String = text[start..]
        .chars()
        .take_while(|c| !c.is_whitespace() && !c.is_control() && *c != '"' && *c != '\\')
        .collect();
    if url.len() > 20 {
        Some(url)
    } else {
        None
    }
}

/// 코덱스가 보여주는 일회용 코드(예: V4GM-HT05H)를 찾는다
fn extract_device_code(text: &str) -> Option<String> {
    for line in text.lines() {
        let token = line.trim();
        let ok = token.len() >= 8
            && token.len() <= 16
            && token.contains('-')
            && token
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-');
        if ok {
            return Some(token.to_string());
        }
    }
    None
}

fn cli_args(provider: Provider) -> (&'static str, &'static [&'static str], &'static str) {
    match provider {
        Provider::Claude => ("claude", ["auth", "login"].as_slice(), "CLAUDE_CONFIG_DIR"),
        // --device-auth: 브라우저를 열지 않고 주소와 일회용 코드를 글자로 보여준다
        Provider::Codex => (
            "codex",
            ["login", "--device-auth"].as_slice(),
            "CODEX_HOME",
        ),
    }
}

fn temp_config_dir(env: &Env) -> PathBuf {
    env.store.join("_login").join(now().to_string())
}

/// 임시 폴더는 토큰이 들어 있을 수 있으므로 반드시 지운다.
/// 방금 종료한 프로세스가 아직 파일을 물고 있을 수 있어 몇 번 재시도한다.
fn remove_dir_retry(dir: &Path) {
    for attempt in 0..5 {
        if !dir.exists() || fs::remove_dir_all(dir).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(150 * (attempt + 1)));
    }
}

/// 예전에 중단된 로그인이 남긴 폴더를 청소한다
fn sweep_stale(env: &Env) {
    let root = env.store.join("_login");
    let Ok(entries) = fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        remove_dir_retry(&entry.path());
    }
}

/// 로그인을 시작하고 화면에 뜬 주소를 돌려준다
pub fn start(env: &Env, provider: Provider) -> Result<LoginPrompt, String> {
    if SESSION.lock().map_err(|_| "내부 잠금 오류")?.is_some() {
        return Err("이미 로그인이 진행 중입니다".into());
    }
    sweep_stale(env);

    let config_dir = temp_config_dir(env);
    fs::create_dir_all(&config_dir)
        .map_err(|e| format!("임시 폴더 생성 실패 {}: {e}", config_dir.display()))?;

    let (program, args, env_key) = cli_args(provider);
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("가상 콘솔 생성 실패: {e}"))?;

    // npm 전역 설치본은 .cmd 셔임이라 cmd 경유로 실행한다
    #[cfg(windows)]
    let mut cmd = {
        let mut c = CommandBuilder::new("cmd");
        c.arg("/c");
        c.arg(program);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = CommandBuilder::new(program);
    for a in args {
        cmd.arg(a);
    }
    cmd.env(env_key, &config_dir);

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| {
            let _ = fs::remove_dir_all(&config_dir);
            format!("{program} 실행에 실패했습니다: {e} — CLI가 설치되어 있는지 확인하세요")
        })?;
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("콘솔 읽기 실패: {e}"))?;
    let writer = Arc::new(Mutex::new(
        pair.master
            .take_writer()
            .map_err(|e| format!("콘솔 쓰기 실패: {e}"))?,
    ));
    let responder = writer.clone();

    let output = Arc::new(Mutex::new(Vec::new()));
    let sink = output.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            let piece = &buf[..n];
            // 커서 위치 질의에 답하지 않으면 CLI가 화면을 그리지 않는다
            if piece.windows(4).any(|w| w == b"\x1b[6n") {
                if let Ok(mut w) = responder.lock() {
                    let _ = w.write_all(b"\x1b[1;1R");
                    let _ = w.flush();
                }
            }
            if let Ok(mut acc) = sink.lock() {
                acc.extend_from_slice(piece);
            }
        }
    });

    {
        let mut guard = SESSION.lock().map_err(|_| "내부 잠금 오류")?;
        *guard = Some(Session {
            provider,
            config_dir: config_dir.clone(),
            child,
            writer,
            _output: output.clone(),
            _master: pair.master,
        });
    }

    // 주소가 화면에 뜰 때까지 기다린다
    let deadline = Instant::now() + PROMPT_TIMEOUT;
    loop {
        let text = {
            let acc = output.lock().map_err(|_| "내부 잠금 오류")?;
            strip_ansi(&acc)
        };
        if let Some(url) = extract_url(&text) {
            let device_code = if provider == Provider::Codex {
                extract_device_code(&text)
            } else {
                None
            };
            return Ok(LoginPrompt {
                url,
                device_code,
                needs_code: provider == Provider::Claude,
            });
        }
        if Instant::now() > deadline {
            cancel();
            return Err("로그인 주소를 받지 못했습니다 — 잠시 후 다시 시도하세요".into());
        }
        std::thread::sleep(POLL);
    }
}

/// 세션의 자식 프로세스가 끝날 때까지 기다린다 (취소 가능하도록 짧게 끊어 확인)
fn wait_for_exit(timeout: Duration) -> Result<(), String> {
    let started = Instant::now();
    loop {
        {
            let mut guard = SESSION.lock().map_err(|_| "내부 잠금 오류")?;
            let Some(session) = guard.as_mut() else {
                return Err("로그인을 취소했습니다".into());
            };
            match session.child.try_wait() {
                Ok(Some(_)) => return Ok(()),
                Ok(None) => {}
                Err(e) => return Err(format!("로그인 상태 확인 실패: {e}")),
            }
        }
        if started.elapsed() > timeout {
            cancel();
            return Err("시간이 초과됐습니다 — 다시 시도하세요".into());
        }
        std::thread::sleep(POLL);
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
                return Err("로그인이 완료되지 않았습니다 — 코드를 다시 확인하세요".into());
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

/// 세션을 정리하고 임시 폴더를 지운다 (토큰이 남지 않게)
fn finish_session() -> Option<(Provider, PathBuf)> {
    let mut guard = SESSION.lock().ok()?;
    let session = guard.take()?;
    Some((session.provider, session.config_dir))
}

fn import(env: &Env, provider: Provider, config_dir: &Path) -> Result<LoginOutcome, String> {
    let result = read_login_result(provider, config_dir);
    let cleanup = |r: Result<LoginOutcome, String>| {
        remove_dir_retry(config_dir);
        r
    };
    let (ident, cred, block) = match result {
        Ok(v) => v,
        Err(e) => return cleanup(Err(e)),
    };

    // 프로필을 실제로 건드리는 구간에서만 잠근다
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

/// 브라우저에서 받은 코드를 CLI에 전달해 로그인을 끝낸다 (클로드)
pub fn submit_code(env: &Env, code: &str) -> Result<LoginOutcome, String> {
    let code = code.trim();
    if code.is_empty() {
        return Err("코드를 입력하세요".into());
    }
    // 콘솔에 그대로 흘러가므로 줄바꿈·제어문자는 막는다
    if code.contains(['\r', '\n']) || code.chars().any(|c| c.is_control()) {
        return Err("코드 형식이 올바르지 않습니다".into());
    }
    {
        let guard = SESSION.lock().map_err(|_| "내부 잠금 오류")?;
        let session = guard.as_ref().ok_or("진행 중인 로그인이 없습니다")?;
        let mut writer = session.writer.lock().map_err(|_| "내부 잠금 오류")?;
        writer
            .write_all(format!("{code}\r").as_bytes())
            .map_err(|e| format!("코드 전달 실패: {e}"))?;
        writer.flush().ok();
    }
    wait_for_exit(FINISH_TIMEOUT)?;
    let (provider, dir) = finish_session().ok_or("로그인 세션이 사라졌습니다")?;
    import(env, provider, &dir)
}

/// 브라우저에서 코드 입력까지 끝나면 CLI가 스스로 완료한다 (코덱스 device-auth)
pub fn wait_device(env: &Env) -> Result<LoginOutcome, String> {
    wait_for_exit(DEVICE_TIMEOUT)?;
    let (provider, dir) = finish_session().ok_or("로그인 세션이 사라졌습니다")?;
    import(env, provider, &dir)
}

/// 진행 중인 로그인을 중단하고 임시 폴더를 지운다.
/// Windows에서는 cmd 셔임을 거치므로 트리째 종료해야 CLI가 살아남지 않는다.
pub fn cancel() {
    let taken = {
        match SESSION.lock() {
            Ok(mut guard) => guard.take(),
            Err(_) => None,
        }
    };
    if let Some(mut session) = taken {
        #[cfg(windows)]
        if let Some(pid) = session.child.process_id() {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let _ = std::process::Command::new("taskkill")
                .args(["/T", "/F", "/PID", &pid.to_string()])
                .creation_flags(CREATE_NO_WINDOW)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        let _ = session.child.kill();
        let _ = session.child.wait();
        remove_dir_retry(&session.config_dir);
    }
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

    #[test]
    fn strips_ansi_and_finds_url() {
        // 실제 클로드 로그인 화면에서 그대로 가져온 형태 (OSC 8 하이퍼링크 포함)
        let raw = b"\x1b[?25h\x1b]0;claude\x07Opening browser to sign in\xe2\x80\xa6\r\nIf the browser didn't open, visit: \x1b]8;id=1;https://claude.com/cai/oauth/authorize?code=true&state=abc\x1b\\https://claude.com/cai/oauth/authorize?code=true&state=abc\x1b]8;;\x1b\\\r\nPaste code here if prompted > ";
        let text = strip_ansi(raw);
        assert!(text.contains("Paste code here"));
        let url = extract_url(&text).unwrap();
        assert_eq!(
            url,
            "https://claude.com/cai/oauth/authorize?code=true&state=abc"
        );
    }

    #[test]
    fn finds_codex_device_code() {
        let text = "Follow these steps\n\n1. Open this link\n   https://auth.openai.com/codex/device\n\n2. Enter this one-time code\n   V4GM-HT05H\n";
        assert_eq!(
            extract_url(text).unwrap(),
            "https://auth.openai.com/codex/device"
        );
        assert_eq!(extract_device_code(text).unwrap(), "V4GM-HT05H");
    }

    #[test]
    fn rejects_control_characters_in_code() {
        let env = test_env("badcode");
        // 세션이 없어도 형식 검사가 먼저 걸려야 한다
        assert!(submit_code(&env, "abc\ndef").is_err());
        assert!(submit_code(&env, "   ").is_err());
    }

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

        let outcome = import(&env, Provider::Claude, &cfg).unwrap();
        assert_eq!(outcome.profile, "newbie");
        assert!(!outcome.updated_existing);
        // 토큰이 든 임시 폴더는 반드시 지워져야 한다
        assert!(!cfg.exists());

        let snap = crate::accounts::list(&env, Provider::Claude).unwrap();
        assert_eq!(snap.profiles.len(), 1);
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

        let outcome = import(&env, Provider::Codex, &cfg).unwrap();
        assert_eq!(outcome.profile, "cx");
        assert_eq!(outcome.email.as_deref(), Some("cx@test.dev"));
    }

    #[test]
    fn incomplete_login_is_reported() {
        let env = test_env("incomplete");
        let cfg = env.store.join("_login").join("t1");
        fs::create_dir_all(&cfg).unwrap();
        fs::write(cfg.join(".claude.json"), r#"{"numStartups":1}"#).unwrap();
        match import(&env, Provider::Claude, &cfg) {
            Err(e) => assert!(e.contains("완료되지 않"), "예상과 다른 에러: {e}"),
            Ok(_) => panic!("토큰이 없는데 성공으로 판정됐다"),
        }
    }

    /// 실환경: 실제로 로그인 주소가 나오는지, 그리고 활성 계정이 안 바뀌는지 확인한다.
    /// `cargo test -- --ignored real_start_login --nocapture`
    #[test]
    #[ignore]
    fn real_start_login_returns_url() {
        cancel(); // 앞선 테스트가 남긴 세션 정리
        let env = Env::real().unwrap();
        let live_cred = env.live_credential_path(Provider::Claude);
        let before = fs::read(&live_cred).unwrap();

        let prompt = start(&env, Provider::Claude).unwrap();
        println!("받은 로그인 주소: {}", prompt.url);
        assert!(prompt.url.contains("oauth"), "주소: {}", prompt.url);
        assert!(prompt.needs_code, "클로드는 코드 입력이 필요하다");

        cancel();
        assert_eq!(before, fs::read(&live_cred).unwrap(), "활성 토큰이 변경됐다");
        let login_root = env.store.join("_login");
        if login_root.exists() {
            let leftovers: Vec<_> = fs::read_dir(&login_root).unwrap().flatten().collect();
            assert!(leftovers.is_empty(), "임시 로그인 폴더가 남았다");
        }
    }

    /// 코덱스 device-auth가 주소와 일회용 코드를 주는지 확인한다.
    #[test]
    #[ignore]
    fn real_start_login_codex_device_code() {
        cancel(); // 앞선 테스트가 남긴 세션 정리
        let env = Env::real().unwrap();
        let prompt = start(&env, Provider::Codex).unwrap();
        println!(
            "코덱스 주소: {} / 코드: {:?}",
            prompt.url, prompt.device_code
        );
        assert!(prompt.url.contains("openai.com"), "주소: {}", prompt.url);
        assert!(prompt.device_code.is_some(), "일회용 코드가 없다");
        assert!(!prompt.needs_code, "코덱스는 위젯 코드 입력이 필요 없다");
        cancel();
    }
}

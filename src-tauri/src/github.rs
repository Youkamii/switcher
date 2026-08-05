//! GitHub 계정 전환 — 전환은 gh CLI와 같은 통로(`gh auth switch`)를 쓴다.
//!
//! 목록 조회는 gh를 실행하지 않고 gh의 계정 대장인 hosts.yml을 직접 읽는다.
//! `gh auth status`는 계정마다 네트워크로 토큰을 검증하므로 렌더 주기마다 돌리기엔
//! 느리고, 오프라인에서는 거짓 "계정 없음"을 만든다 (red-review 정합성 #4).
//! 파일에서는 이름·활성 여부만 취하고, 토큰 값(비보안 저장 모드의 oauth_token)은
//! 어떤 경로로도 읽지 않는다 — 토큰 관리는 전적으로 gh의 몫이다.
//!
//! 전환 직후 `gh auth setup-git`을 함께 실행해 git push/pull(HTTPS)이 활성 계정을
//! 따라가게 한다. 이 실패는 경고로만 남긴다 — git 미설치 환경에서도 전환 자체는
//! 이미 성공했고, 실패로 보고하면 UI 활성 표시가 실상과 어긋난다 (정합성 #5).
//!
//! 한계(README에 명시): SSH 리모트·커밋 author(user.name/email)·타 앱 세션은
//! 이 전환의 영향 밖이다.

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct GithubAccount {
    pub login: String,
    pub active: bool,
}

#[derive(Serialize)]
pub struct GithubSnapshot {
    /// gh 실행 파일을 찾았는가 — false면 프론트가 설치 안내를 띄운다
    pub gh_found: bool,
    pub accounts: Vec<GithubAccount>,
}

/// gh 실행 파일 해석 (Windows). `Command::new("gh")`의 기본 탐색은 앱 폴더
/// (switcher.exe 옆)가 PATH보다 먼저라, Downloads에 놓인 가짜 gh.exe가 잡히는
/// 바이너리 플랜팅(CWE-427)에 열린다 — update.rs가 tar를 System32 절대 경로로만
/// 부르는 것과 같은 이유로, 알려진 설치 경로 → PATH 항목 순회(자기 폴더 제외)로
/// 직접 해석한다.
#[cfg(windows)]
fn find_gh() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(pf).join("GitHub CLI").join("gh.exe"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        candidates.push(local.join("Programs").join("GitHub CLI").join("gh.exe"));
        candidates.push(
            local
                .join("Microsoft")
                .join("WinGet")
                .join("Links")
                .join("gh.exe"),
        );
    }
    if let Some(data) = std::env::var_os("ProgramData") {
        candidates.push(PathBuf::from(data).join("chocolatey").join("bin").join("gh.exe"));
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        candidates.push(PathBuf::from(home).join("scoop").join("shims").join("gh.exe"));
    }
    if let Some(found) = candidates.into_iter().find(|p| p.is_file()) {
        return Some(found);
    }
    // 비표준 설치 폴백: PATH를 직접 순회한다 — CreateProcess 기본 탐색과 달리
    // 앱 폴더·현재 폴더가 끼어들지 않고, 자기 exe 폴더는 명시적으로 건너뛴다
    let own_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    for entry in std::env::split_paths(&std::env::var_os("PATH")?) {
        if entry.as_os_str().is_empty() || own_dir.as_deref() == Some(entry.as_path()) {
            continue;
        }
        let candidate = entry.join("gh.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(not(windows))]
fn find_gh() -> Option<PathBuf> {
    // 맥 GUI 앱은 셸 PATH를 모른다 — 로그인 셸로 해석 (절대 경로가 아니면 미발견)
    let resolved = PathBuf::from(crate::login::resolve_program("gh"));
    resolved.is_absolute().then_some(resolved)
}

/// 해석된 gh 경로 캐시 — 맥의 로그인 셸 스폰과 경로 재탐색을 앱 수명당 1회로 줄인다.
/// (위젯 실행 중에 gh를 새로 설치하면 재시작해야 인식된다 — 수용한 트레이드오프)
fn gh_bin() -> Option<&'static std::path::Path> {
    static GH_BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    GH_BIN.get_or_init(find_gh).as_deref()
}

fn run_gh(args: &[&str]) -> Result<std::process::Output, String> {
    let Some(bin) = gh_bin() else {
        return Err("gh CLI를 찾을 수 없습니다".to_string());
    };
    let mut cmd = std::process::Command::new(bin);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000; // 콘솔 창을 띄우지 않는다
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.args(args)
        .output()
        .map_err(|e| format!("gh 실행 실패: {e}"))
}

/// gh 설정 폴더 (GH_CONFIG_DIR > 플랫폼 기본값 — gh 문서와 동일한 우선순위)
fn gh_config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("GH_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("GitHub CLI"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config").join("gh"))
    }
}

/// gh에 로그인된 github.com 계정 목록 — 파일만 읽는다 (프로세스·네트워크 없음)
pub fn list() -> GithubSnapshot {
    let accounts = gh_config_dir()
        .map(|dir| dir.join("hosts.yml"))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| parse_hosts(&text))
        .unwrap_or_default();
    GithubSnapshot {
        gh_found: gh_bin().is_some(),
        accounts,
    }
}

/// hosts.yml에서 github.com 계정을 뽑는다 (실측 형식, gh 2.40+):
/// ```yaml
/// github.com:
///     users:
///         alice:
///         bob:
///             oauth_token: gho_xxx   # 비보안 저장 모드일 때만 — 읽지 않는다
///     user: alice                    # 활성 계정
/// ```
/// users 블록의 첫 들여쓰기 깊이만 사용자 이름으로 취급한다 — 더 깊은 줄은
/// 사용자 하위 속성(토큰 등)이므로 건너뛴다. 다른 호스트(엔터프라이즈) 블록은
/// 최상위 키가 github.com이 아니므로 통째로 무시된다.
fn parse_hosts(text: &str) -> Vec<GithubAccount> {
    let mut names: Vec<String> = Vec::new();
    let mut active: Option<String> = None;
    let mut in_github = false;
    let mut users_indent: Option<usize> = None;
    let mut name_indent: Option<usize> = None;
    for raw in text.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = raw.len() - raw.trim_start().len();
        if indent == 0 {
            in_github = trimmed == "github.com:";
            users_indent = None;
            name_indent = None;
            continue;
        }
        if !in_github {
            continue;
        }
        if let Some(ui) = users_indent {
            if indent > ui {
                let ni = *name_indent.get_or_insert(indent);
                if indent == ni {
                    if let Some(name) = trimmed.strip_suffix(':') {
                        if !name.is_empty() {
                            names.push(name.to_string());
                        }
                    }
                }
                continue;
            }
            // users 블록이 끝났다 — 이 줄은 아래의 일반 키 처리로 이어간다
            users_indent = None;
            name_indent = None;
        }
        if trimmed == "users:" {
            users_indent = Some(indent);
        } else if let Some(value) = trimmed.strip_prefix("user:") {
            let value = value.trim();
            if !value.is_empty() {
                active = Some(value.to_string());
            }
        }
    }
    // 다중 계정 이전 gh(users 맵 없음)는 user: 하나만 있다 — 그 계정을 목록으로
    if names.is_empty() {
        if let Some(only) = &active {
            names.push(only.clone());
        }
    }
    names
        .into_iter()
        .map(|login| GithubAccount {
            active: active.as_deref() == Some(login.as_str()),
            login,
        })
        .collect()
}

/// 계정 이름 검증 — 셸을 거치지 않아 인젝션 경로는 없지만, 파싱이 이상한 값을
/// 물어오는 사고 방어. GitHub 로그인 규칙: 영숫자·하이픈, EMU(Enterprise Managed
/// Users)는 밑줄 포함(mona_fabrikam 형태 — red-review 정합성 #2). 선행 하이픈은
/// 플래그 오인 방지로 거부.
fn valid_login(login: &str) -> bool {
    !login.is_empty()
        && !login.starts_with('-')
        && login
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 활성 계정 전환 + git 자격 증명 연동.
pub fn switch(login: &str) -> Result<(), String> {
    if !valid_login(login) {
        return Err(format!("잘못된 GitHub 계정 이름: {login}"));
    }
    let out = run_gh(&["auth", "switch", "--hostname", "github.com", "--user", login])?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("GitHub 계정 전환 실패: {}", err.trim()));
    }
    // git push/pull(HTTPS)이 gh의 활성 계정을 따라가게 연결한다.
    // 실패해도 전환은 이미 성공 — 경고만 남긴다 (git 미설치 등, 정합성 #5)
    match run_gh(&["auth", "setup-git", "--hostname", "github.com"]) {
        Ok(setup) if !setup.status.success() => {
            let err = String::from_utf8_lossy(&setup.stderr);
            eprintln!("git 연동(setup-git) 실패: {}", err.trim());
        }
        Err(e) => eprintln!("git 연동(setup-git) 실행 실패: {e}"),
        _ => {}
    }
    Ok(())
}

// ── 인앱 계정 추가 (gh auth login, PTY) ──────────────────────────────────
// 코덱스 장치 코드 로그인과 같은 UX: 위젯에 주소 + 일회용 코드(XXXX-XXXX)를
// 띄우고, 브라우저에서 코드를 넣으면 gh가 알아서 끝낸다. gh는 다중 계정을
// 네이티브 지원하므로 격리 폴더가 필요 없다 — 라이브 설정에 계정이 "추가"되고
// 기존 계정은 유지된다 (주의: gh는 완료 시 새 계정을 활성으로 만든다).

#[derive(Serialize)]
pub struct GhLoginPrompt {
    pub url: String,
    pub device_code: String,
}

struct GhLoginSession {
    child: Box<dyn Child + Send + Sync>,
    /// 코드 표시 후 Enter를 보내는 데 쓴다 (reader 스레드의 커서 질의 응답과 공유)
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    buffer: Arc<Mutex<Vec<u8>>>,
    _master: Box<dyn MasterPty + Send>,
}

static GH_LOGIN: Mutex<Option<GhLoginSession>> = Mutex::new(None);

/// 로그인 링크·코드가 뜰 때까지 / 브라우저 완료까지 기다리는 시간 (login.rs와 동일 기준)
const GH_PROMPT_TIMEOUT: Duration = Duration::from_secs(60);
const GH_FINISH_TIMEOUT: Duration = Duration::from_secs(600);
const GH_POLL: Duration = Duration::from_millis(300);
const GH_OUTPUT_CAP: usize = 256 * 1024;

/// gh의 일회용 코드 추출: "! First copy your one-time code: XXXX-XXXX" (실측).
/// 코덱스와 달리 코드가 문장 안에 있어 login.rs의 단독-줄 추출기로는 못 잡는다.
fn extract_gh_code(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some(rest) = line.split("one-time code:").nth(1) {
            let token = rest.trim().split_whitespace().next().unwrap_or("");
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
    }
    None
}

fn gh_login_take_buffer() -> Result<Vec<u8>, String> {
    // Arc만 복제해 세션 잠금을 즉시 놓는다 — 버퍼 잠금과 겹치지 않게
    let buffer = {
        let guard = GH_LOGIN.lock().map_err(|_| "내부 잠금 오류")?;
        let Some(session) = guard.as_ref() else {
            return Err("로그인을 취소했습니다".to_string());
        };
        session.buffer.clone()
    };
    let data = buffer.lock().map_err(|_| "내부 잠금 오류")?.clone();
    Ok(data)
}

/// gh auth login을 PTY로 시작해 주소와 일회용 코드를 돌려준다
pub fn login_start() -> Result<GhLoginPrompt, String> {
    let Some(bin) = gh_bin() else {
        return Err("gh CLI를 찾을 수 없습니다 — GitHub CLI를 설치하세요".to_string());
    };
    {
        let mut guard = GH_LOGIN.lock().map_err(|_| "내부 잠금 오류")?;
        if guard.is_some() {
            return Err("이미 GitHub 로그인이 진행 중입니다".to_string());
        }
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 500,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("가상 콘솔 생성 실패: {e}"))?;
        // gh는 진짜 exe라 cmd 셔임 경유가 필요 없다. --web + 플래그로 질문을 없애
        // 곧장 일회용 코드 화면으로 간다
        let mut cmd = CommandBuilder::new(bin);
        for arg in [
            "auth",
            "login",
            "--hostname",
            "github.com",
            "--git-protocol",
            "https",
            "--web",
        ] {
            cmd.arg(arg);
        }
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("gh 실행 실패: {e}"))?;
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
        let buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let responder = writer.clone();
        let sink = buffer.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut tail: Vec<u8> = Vec::new();
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let piece = &buf[..n];
                // 커서 위치 질의(ESC[6n)에 답해야 화면이 그려진다 (login.rs와 동일)
                let mut probe = tail.clone();
                probe.extend_from_slice(piece);
                if probe.windows(4).any(|w| w == b"\x1b[6n") {
                    if let Ok(mut w) = responder.lock() {
                        let _ = w.write_all(b"\x1b[1;1R");
                        let _ = w.flush();
                    }
                }
                tail = probe[probe.len().saturating_sub(3)..].to_vec();
                if let Ok(mut acc) = sink.lock() {
                    acc.extend_from_slice(piece);
                    if acc.len() > GH_OUTPUT_CAP {
                        let cut = acc.len() - GH_OUTPUT_CAP;
                        acc.drain(..cut);
                    }
                }
            }
        });
        *guard = Some(GhLoginSession {
            child,
            writer,
            buffer,
            _master: pair.master,
        });
    }

    // 일회용 코드가 화면에 뜰 때까지 (잠금 밖 — 취소 가능해야 하므로)
    let deadline = Instant::now() + GH_PROMPT_TIMEOUT;
    // gh가 중간에 묻는 "Authenticate Git with your GitHub credentials?"에는 Y로
    // 자동 응답한다 (실측: 이 질문에서 멈춘다). 전환마다 setup-git을 돌리는 우리
    // 설계에서 Y는 항상 안전하다. 화면 버퍼가 누적이라 1회만 답하도록 표시해 둔다.
    let mut answered_git_prompt = false;
    loop {
        {
            let mut guard = GH_LOGIN.lock().map_err(|_| "내부 잠금 오류")?;
            let Some(session) = guard.as_mut() else {
                return Err("로그인을 취소했습니다".to_string());
            };
            if let Ok(Some(status)) = session.child.try_wait() {
                let text = crate::login::strip_ansi(
                    &session.buffer.lock().map_err(|_| "내부 잠금 오류")?,
                );
                *guard = None;
                return Err(format!(
                    "gh가 로그인 화면을 띄우기 전에 종료했습니다 ({status:?}): {}",
                    text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("")
                ));
            }
        }
        let raw = gh_login_take_buffer()?;
        let text = crate::login::strip_ansi(&raw);
        if !answered_git_prompt && text.contains("Authenticate Git with your GitHub credentials?")
        {
            answered_git_prompt = true;
            if let Ok(guard) = GH_LOGIN.lock() {
                if let Some(session) = guard.as_ref() {
                    if let Ok(mut w) = session.writer.lock() {
                        let _ = w.write_all(b"y\r");
                        let _ = w.flush();
                    }
                }
            }
        }
        if let Some(code) = extract_gh_code(&text) {
            let url = crate::login::pick_login_url(crate::login::extract_osc8_urls(&raw))
                .or_else(|| crate::login::extract_visible_url(&text))
                .unwrap_or_else(|| "https://github.com/login/device".to_string());
            // "Press Enter to open ..." 프롬프트를 넘긴다 — gh가 기본 브라우저를 한 번
            // 열려고 시도하지만(닫아도 됨), 이후 폴링이 시작된다
            if let Ok(guard) = GH_LOGIN.lock() {
                if let Some(session) = guard.as_ref() {
                    if let Ok(mut w) = session.writer.lock() {
                        let _ = w.write_all(b"\r");
                        let _ = w.flush();
                    }
                }
            }
            return Ok(GhLoginPrompt {
                url,
                device_code: code,
            });
        }
        if Instant::now() > deadline {
            let tail: String = text
                .lines()
                .rev()
                .filter(|l| !l.trim().is_empty())
                .take(3)
                .collect::<Vec<_>>()
                .join(" | ");
            login_cancel();
            return Err(format!(
                "gh 로그인 화면이 60초 안에 뜨지 않았습니다 — 마지막 화면: {tail}"
            ));
        }
        std::thread::sleep(GH_POLL);
    }
}

/// 브라우저 쪽 완료를 기다린다 — 성공하면 로그인된 계정 이름
pub fn login_wait() -> Result<String, String> {
    let deadline = Instant::now() + GH_FINISH_TIMEOUT;
    loop {
        let exited = {
            let mut guard = GH_LOGIN.lock().map_err(|_| "내부 잠금 오류")?;
            let Some(session) = guard.as_mut() else {
                return Err("로그인을 취소했습니다".to_string());
            };
            matches!(session.child.try_wait(), Ok(Some(_)))
        };
        let text = crate::login::strip_ansi(&gh_login_take_buffer()?);
        // 성공 마커: "✓ Logged in as <login>" (gh 실측 출력)
        if let Some(login) = text
            .lines()
            .rev()
            .find_map(|line| line.trim().split("Logged in as ").nth(1))
            .map(|rest| rest.split_whitespace().next().unwrap_or("").to_string())
            .filter(|l| !l.is_empty())
        {
            login_cleanup();
            return Ok(login);
        }
        if exited {
            let last = text
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .to_string();
            login_cleanup();
            return Err(format!("GitHub 로그인이 완료되지 않았습니다: {last}"));
        }
        if Instant::now() > deadline {
            login_cancel();
            return Err("GitHub 로그인 대기 시간(10분)을 넘겼습니다 — 다시 시도하세요".to_string());
        }
        std::thread::sleep(GH_POLL);
    }
}

fn login_cleanup() {
    if let Ok(mut guard) = GH_LOGIN.lock() {
        *guard = None; // Drop이 PTY·자식 핸들을 정리한다
    }
}

/// 진행 중 로그인 취소 — gh 프로세스를 죽이고 세션을 비운다
pub fn login_cancel() {
    if let Ok(mut guard) = GH_LOGIN.lock() {
        if let Some(mut session) = guard.take() {
            let _ = session.child.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_account_keyring() {
        // 이 PC 실측 형식 (keyring 저장 — 토큰 줄 없음)
        let text = "github.com:\n    users:\n        Youkamii:\n    user: Youkamii\n";
        assert_eq!(
            parse_hosts(text),
            vec![GithubAccount {
                login: "Youkamii".into(),
                active: true
            }]
        );
    }

    #[test]
    fn parses_two_accounts_and_ignores_insecure_tokens() {
        // 가짜 토큰 값 — 실제 형식과 달리 숫자를 뺐다 (secrets-guard 오탐 방지)
        let text = "github.com:\n    git_protocol: https\n    users:\n        alice:\n            oauth_token: gho_fakefakefakefakefakefake\n        bob-2:\n    user: bob-2\n";
        let accounts = parse_hosts(text);
        assert_eq!(accounts.len(), 2);
        assert!(!accounts[0].active && accounts[0].login == "alice");
        assert!(accounts[1].active && accounts[1].login == "bob-2");
        // 토큰 값이 어떤 형태로도 흘러나오지 않는다
        assert!(!format!("{accounts:?}").contains("gho_"));
    }

    #[test]
    fn ignores_enterprise_hosts_before_and_after() {
        let text = "ghe.corp.com:\n    users:\n        carol:\n    user: carol\ngithub.com:\n    users:\n        alice:\n        bob:\n    user: alice\nghe2.corp.com:\n    users:\n        dave:\n    user: dave\n";
        let accounts = parse_hosts(text);
        assert_eq!(accounts.len(), 2);
        assert!(accounts[0].active && accounts[0].login == "alice");
        assert!(!accounts[1].active && accounts[1].login == "bob");
    }

    #[test]
    fn handles_legacy_single_user_format() {
        // 다중 계정 도입 전 gh — users 맵 없이 user:만
        let text = "github.com:\n    oauth_token: ****\n    user: old-timer\n    git_protocol: https\n";
        assert_eq!(
            parse_hosts(text),
            vec![GithubAccount {
                login: "old-timer".into(),
                active: true
            }]
        );
    }

    #[test]
    fn empty_or_alien_file_gives_no_accounts() {
        assert!(parse_hosts("").is_empty());
        assert!(parse_hosts("ghe.corp.com:\n    user: x\n").is_empty());
        assert!(parse_hosts("not yaml at all").is_empty());
    }

    #[test]
    fn extracts_gh_one_time_code() {
        let text = "? Authenticate Git with your GitHub credentials? Yes\n! First copy your one-time code: 3FAA-43FF\nPress Enter to open https://github.com/login/device in your browser...\n";
        assert_eq!(extract_gh_code(text), Some("3FAA-43FF".to_string()));
        assert_eq!(extract_gh_code("no code here"), None);
        // 코드 형식이 아닌 것은 거부
        assert_eq!(extract_gh_code("one-time code: hello-world"), None);
    }

    /// 실기기 전용: 인앱 로그인 프롬프트(주소+일회용 코드)가 뜨는지 확인하고
    /// 즉시 취소한다 — 계정 무변경, 발급된 코드는 브라우저 입력 없이는 무효
    #[test]
    #[ignore]
    fn real_github_login_prompt_then_cancel() {
        let prompt = login_start().expect("로그인 프롬프트 실패");
        println!("url={} code={}", prompt.url, prompt.device_code);
        assert!(prompt.url.contains("github.com"));
        assert!(prompt.device_code.contains('-'));
        login_cancel();
    }

    /// 실기기 전용: gh가 설치·로그인된 환경에서 목록이 나오는지
    /// (`cargo test -- --ignored real_`)
    #[test]
    #[ignore]
    fn real_github_list_shows_accounts() {
        let snap = list();
        assert!(snap.gh_found, "gh CLI를 찾지 못했다");
        assert!(!snap.accounts.is_empty(), "로그인된 계정이 없다");
        assert_eq!(
            snap.accounts.iter().filter(|a| a.active).count(),
            1,
            "활성 계정은 정확히 하나여야 한다"
        );
    }

    #[test]
    fn login_validation_rejects_suspicious_and_allows_emu() {
        assert!(!valid_login(""));
        assert!(!valid_login("a b"));
        assert!(!valid_login("x;rm"));
        assert!(!valid_login("--flag"));
        assert!(valid_login("Youkamii"));
        assert!(valid_login("bob-2"));
        // EMU 계정명(밑줄 접미사) 허용
        assert!(valid_login("mona_fabrikam"));
    }
}

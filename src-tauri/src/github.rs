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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
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
    /// 뒤늦게 끝난 이전 waiter가 새 GitHub 로그인 세션을 건드리지 못하게 하는 ID.
    pub session_id: String,
    pub url: String,
    pub device_code: String,
}

struct GhLoginSession {
    generation: u64,
    request_id: String,
    child: Box<dyn Child + Send + Sync>,
    /// 코드 표시 후 Enter를 보내는 데 쓴다 (reader 스레드의 커서 질의 응답과 공유)
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    buffer: Arc<Mutex<Vec<u8>>>,
    reader_done: Arc<AtomicBool>,
    _master: Box<dyn MasterPty + Send>,
}

static GH_LOGIN: Mutex<Option<GhLoginSession>> = Mutex::new(None);
/// 로그인 완료와 취소가 맞붙을 때의 잠금 순서:
/// GH_COMPLETION_LOCK -> GH_LOGIN -> GhLoginSession::buffer.
static GH_COMPLETION_LOCK: Mutex<()> = Mutex::new(());
const MAX_LIVE_START_REQUESTS: usize = 32;
static GH_START_REQUESTS: LazyLock<crate::login::StartRequestRegistry> =
    LazyLock::new(|| crate::login::StartRequestRegistry::new(MAX_LIVE_START_REQUESTS));
static GH_LOGIN_GEN: AtomicU64 = AtomicU64::new(0);

/// 로그인 링크·코드가 뜰 때까지 / 브라우저 완료까지 기다리는 시간 (login.rs와 동일 기준)
const GH_PROMPT_TIMEOUT: Duration = Duration::from_secs(60);
const GH_FINISH_TIMEOUT: Duration = Duration::from_secs(600);
const GH_POLL: Duration = Duration::from_millis(300);
const GH_OUTPUT_CAP: usize = 256 * 1024;
const GH_WAIT_TIMEOUT_MESSAGE: &str =
    "GitHub 로그인 대기 시간(10분)을 넘겼습니다 — 다시 시도하세요";
/// 자식 종료 후 남은 출력이 버퍼에 도착하기를 기다리는 유예 (login.rs EXIT_FLUSH와 동일).
/// 이게 없으면 gh가 "Logged in as"를 찍고 즉시 종료할 때 성공을 실패로 오인한다.
const GH_EXIT_FLUSH: Duration = Duration::from_millis(700);
const GH_TERMINATE_TIMEOUT: Duration = Duration::from_secs(2);
const GH_TERMINATE_POLL: Duration = Duration::from_millis(50);

/// 진행 중 세션의 PTY에 입력을 보낸다 (Enter·자동 응답 공용)
fn ensure_generation(active: u64, expected: u64) -> Result<(), String> {
    if active == expected {
        Ok(())
    } else {
        Err("이전 GitHub 로그인 요청이라 무시했습니다".to_string())
    }
}

fn ensure_request_id(active: &str, expected: &str) -> Result<(), String> {
    if active == expected {
        Ok(())
    } else {
        Err("이전 GitHub 로그인 시작 취소 요청이라 무시했습니다".to_string())
    }
}

fn validate_request_id(request_id: &str) -> Result<(), String> {
    let bytes = request_id.as_bytes();
    let valid = bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        });
    valid
        .then_some(())
        .ok_or_else(|| "잘못된 GitHub 로그인 요청 ID입니다".to_string())
}

const GITHUB_START_KIND: &str = "github";

pub fn reserve_login_start(request_id: &str) -> Result<(), String> {
    validate_request_id(request_id)?;
    GH_START_REQUESTS.reserve(request_id, GITHUB_START_KIND, "GitHub 로그인")
}

pub fn release_login_start(request_id: &str) -> Result<(), String> {
    validate_request_id(request_id)?;
    GH_START_REQUESTS.release(request_id, GITHUB_START_KIND)
}

pub fn block_starts_for_shutdown() {
    GH_START_REQUESTS.block_for_shutdown();
}

pub fn unblock_starts_after_failed_shutdown() {
    GH_START_REQUESTS.unblock_after_failed_shutdown();
}

/// 프롬프트 생성 오류 뒤에도 살아 있는 정확한 GitHub 세션을 찾는다.
/// 성공 마커가 이미 도착한 세션도 돌려줘 프런트의 정확한 waiter가 완료를 회수하게 한다.
pub fn login_session_for_request(request_id: &str) -> Result<Option<String>, String> {
    validate_request_id(request_id)?;
    let _completion = GH_COMPLETION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let guard = GH_LOGIN.lock().map_err(|_| "내부 잠금 오류")?;
    Ok(crate::login::exact_session_id(
        guard
            .as_ref()
            .map(|session| (session.request_id.as_str(), session.generation)),
        request_id,
    ))
}

fn gh_send(generation: u64, bytes: &[u8]) -> Result<(), String> {
    let result = (|| {
        let guard = GH_LOGIN.lock().map_err(|_| "내부 잠금 오류")?;
        let session = guard
            .as_ref()
            .ok_or_else(|| "GitHub 로그인을 취소했습니다".to_string())?;
        ensure_generation(session.generation, generation)?;
        let mut writer = session.writer.lock().map_err(|_| "내부 잠금 오류")?;
        writer
            .write_all(bytes)
            .and_then(|_| writer.flush())
            .map_err(|e| format!("GitHub 로그인 입력 전송 실패: {e}"))
    })();
    match result {
        Ok(()) => Ok(()),
        Err(error) => Err(error_after_cancel(generation, error)),
    }
}

fn error_after_cancel(generation: u64, error: String) -> String {
    match login_cancel(generation) {
        Ok(_) => error,
        Err(cancel_error) => {
            format!("{error} / GitHub 로그인 프로세스 정리 실패: {cancel_error}")
        }
    }
}

fn prompt_timeout_error(generation: u64, tail: &str) -> String {
    error_after_cancel(
        generation,
        format!("gh 로그인 화면이 60초 안에 뜨지 않았습니다 — 마지막 화면: {tail}"),
    )
}

/// wait 경계에서 취소 drain 중 늦은 성공이 보이면 오류로 위장하지 않고 회수한다.
fn wait_result_after_cancel(generation: u64, error: String) -> Result<String, String> {
    match login_cancel(generation) {
        Ok(true) => Err(error),
        Ok(false) => match poll_login(generation, false, false) {
            Ok(LoginPoll::Complete(login)) => Ok(login),
            Ok(_) => Err(error),
            Err(poll_error) if poll_error.contains("취소") => Err(error),
            Err(poll_error) => Err(format!("{error} / GitHub 로그인 완료 확인 실패: {poll_error}")),
        },
        Err(cancel_error) => Err(format!(
            "{error} / GitHub 로그인 프로세스 정리 실패: {cancel_error}"
        )),
    }
}

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

fn gh_login_take_buffer(generation: u64) -> Result<Vec<u8>, String> {
    // Arc만 복제해 세션 잠금을 즉시 놓는다 — 버퍼 잠금과 겹치지 않게
    let buffer = {
        let guard = GH_LOGIN.lock().map_err(|_| "내부 잠금 오류")?;
        let Some(session) = guard.as_ref() else {
            return Err("로그인을 취소했습니다".to_string());
        };
        ensure_generation(session.generation, generation)?;
        session.buffer.clone()
    };
    let data = buffer.lock().map_err(|_| "내부 잠금 오류")?.clone();
    Ok(data)
}

fn extract_gh_login(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .find_map(|line| line.trim().split("Logged in as ").nth(1))
        .map(|rest| rest.split_whitespace().next().unwrap_or("").to_string())
        .filter(|login| !login.is_empty())
}

fn session_login(session: &GhLoginSession) -> Result<Option<String>, String> {
    let raw = session.buffer.lock().map_err(|_| "내부 잠금 오류")?;
    Ok(extract_gh_login(&crate::login::strip_ansi(&raw)))
}

#[derive(Debug, PartialEq, Eq)]
enum LoginPoll {
    Pending,
    SuccessRunning,
    ExitedAwaitingOutput,
    ExitedWithoutSuccess,
    Complete(String),
}

/// 성공 출력, reader 종료, 자식 종료를 취소와 같은 완료 잠금 안에서 함께 판정한다.
fn poll_login(
    generation: u64,
    terminate_running: bool,
    cleanup_failed_exit: bool,
) -> Result<LoginPoll, String> {
    let _completion = GH_COMPLETION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let mut guard = GH_LOGIN.lock().map_err(|_| "내부 잠금 오류")?;
    let session = guard
        .as_mut()
        .ok_or_else(|| "로그인을 취소했습니다".to_string())?;
    ensure_generation(session.generation, generation)?;
    // Release로 기록된 reader_done을 먼저 읽어야 마지막 buffer write가 이 뒤의
    // session_login 읽기에 보인다. 순서를 바꾸면 성공 출력을 놓치고 지울 수 있다.
    let reader_done = session.reader_done.load(Ordering::Acquire);
    let login = session_login(session)?;
    let exited = child_has_exited(session.child.as_mut())?;
    if let Some(login) = login {
        if !exited {
            if !terminate_running {
                return Ok(LoginPoll::SuccessRunning);
            }
            terminate_child_verified(session.child.as_mut())?;
        }
        *guard = None;
        return Ok(LoginPoll::Complete(login));
    }
    if !exited {
        return Ok(LoginPoll::Pending);
    }
    if !cleanup_failed_exit || !reader_done {
        return Ok(LoginPoll::ExitedAwaitingOutput);
    }
    *guard = None;
    Ok(LoginPoll::ExitedWithoutSuccess)
}

#[cfg(test)]
fn finish_login_if_ready(
    generation: u64,
    terminate_running: bool,
) -> Result<Option<String>, String> {
    match poll_login(generation, terminate_running, false)? {
        LoginPoll::Complete(login) => Ok(Some(login)),
        _ => Ok(None),
    }
}

/// gh auth login을 PTY로 시작해 주소와 일회용 코드를 돌려준다
pub fn login_start(request_id: String) -> Result<GhLoginPrompt, String> {
    validate_request_id(&request_id)?;
    let my_gen;
    {
        let _completion = GH_COMPLETION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
        let mut guard = GH_LOGIN.lock().map_err(|_| "내부 잠금 오류")?;
        let mut start_request =
            GH_START_REQUESTS.claim(&request_id, GITHUB_START_KIND, "GitHub 로그인")?;
        let Some(bin) = gh_bin() else {
            return Err("gh CLI를 찾을 수 없습니다 — GitHub CLI를 설치하세요".to_string());
        };
        if let Some(session) = guard.as_mut() {
            // 좀비 세션 리퍼: 웹뷰 리로드 등으로 wait/cancel이 못 불린 채 gh가 이미
            // 종료했으면 걷어내고 새로 시작한다 — 앱 재시작 없이 복구 (red-review)
            if matches!(session.child.try_wait(), Ok(Some(_))) {
                *guard = None;
            } else {
                return Err("이미 GitHub 로그인이 진행 중입니다".to_string());
            }
        }
        my_gen = GH_LOGIN_GEN.fetch_add(1, Ordering::SeqCst) + 1;
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
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("콘솔 읽기 실패: {e}"))?;
        let writer = Arc::new(Mutex::new(
            pair.master
                .take_writer()
                .map_err(|e| format!("콘솔 쓰기 실패: {e}"))?,
        ));
        // reader/writer 준비를 먼저 끝내야 그 사이 오류가 나도 이미 띄운 gh가 남지 않는다.
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("gh 실행 실패: {e}"))?;
        drop(pair.slave);
        let buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let reader_done = Arc::new(AtomicBool::new(false));
        let responder = writer.clone();
        let sink = buffer.clone();
        let reader_finished = reader_done.clone();
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
            reader_finished.store(true, Ordering::Release);
        });
        *guard = Some(GhLoginSession {
            generation: my_gen,
            request_id,
            child,
            writer,
            buffer,
            reader_done,
            _master: pair.master,
        });
        start_request.release();
    }

    // 일회용 코드가 화면에 뜰 때까지 (잠금 밖 — 취소 가능해야 하므로)
    let deadline = Instant::now() + GH_PROMPT_TIMEOUT;
    // gh가 중간에 묻는 "Authenticate Git with your GitHub credentials?"에는 Y로
    // 자동 응답한다 (실측: 이 질문에서 멈춘다). 전환마다 setup-git을 돌리는 우리
    // 설계에서 Y는 항상 안전하다. 화면 버퍼가 누적이라 1회만 답하도록 표시해 둔다.
    let mut answered_git_prompt = false;
    // 자식이 종료해도 즉시 실패로 접지 않는다 — 마지막 출력이 리더 스레드를 거쳐
    // 버퍼에 닿을 유예(GH_EXIT_FLUSH)를 준다 (login.rs와 동일한 경합 방어)
    let mut exited_at: Option<Instant> = None;
    loop {
        {
            let mut guard = GH_LOGIN.lock().map_err(|_| "내부 잠금 오류")?;
            let Some(session) = guard.as_mut() else {
                return Err("로그인을 취소했습니다".to_string());
            };
            ensure_generation(session.generation, my_gen)?;
            if exited_at.is_none() {
                if let Ok(Some(_)) = session.child.try_wait() {
                    exited_at = Some(Instant::now());
                }
            }
        }
        if let Some(at) = exited_at {
            if at.elapsed() >= GH_EXIT_FLUSH {
                let text = crate::login::strip_ansi(&gh_login_take_buffer(my_gen)?);
                login_cleanup(my_gen)?;
                return Err(format!(
                    "gh가 로그인 화면을 띄우기 전에 종료했습니다: {}",
                    text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("")
                ));
            }
        }
        let raw = gh_login_take_buffer(my_gen)?;
        let text = crate::login::strip_ansi(&raw);
        if !answered_git_prompt && text.contains("Authenticate Git with your GitHub credentials?")
        {
            answered_git_prompt = true;
            gh_send(my_gen, b"y\r")?;
        }
        if let Some(code) = extract_gh_code(&text) {
            let url = crate::login::pick_login_url(crate::login::extract_osc8_urls(&raw))
                .or_else(|| crate::login::extract_visible_url(&text))
                .unwrap_or_else(|| "https://github.com/login/device".to_string());
            // "Press Enter to open ..." 프롬프트를 넘긴다 — gh가 기본 브라우저를 한 번
            // 열려고 시도하지만(닫아도 됨), 이후 폴링이 시작된다
            gh_send(my_gen, b"\r")?;
            return Ok(GhLoginPrompt {
                session_id: my_gen.to_string(),
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
            return Err(prompt_timeout_error(my_gen, &tail));
        }
        std::thread::sleep(GH_POLL);
    }
}

/// 브라우저 쪽 완료를 기다린다 — 성공하면 로그인된 계정 이름
pub fn login_wait(generation: u64) -> Result<String, String> {
    let deadline = Instant::now() + GH_FINISH_TIMEOUT;
    // 종료 감지 후에도 GH_EXIT_FLUSH 동안은 성공 마커를 계속 찾는다 —
    // "Logged in as"를 찍자마자 종료하는 gh와의 경합 방어 (login.rs와 동일)
    let mut exited_at: Option<Instant> = None;
    let mut completed_at: Option<Instant> = None;
    let mut answered_git_prompt = false;
    loop {
        let text = crate::login::strip_ansi(&gh_login_take_buffer(generation)?);
        // gh 버전에 따라 git 인증 질문이 코드 표시 뒤에 올 수도 있다 — 여기서도 방어
        if !answered_git_prompt && text.contains("Authenticate Git with your GitHub credentials?")
        {
            answered_git_prompt = true;
            gh_send(generation, b"y\r")?;
        }
        let terminate_running =
            completed_at.is_some_and(|completed| completed.elapsed() >= GH_EXIT_FLUSH);
        let cleanup_failed_exit = exited_at.is_some_and(|exited| exited.elapsed() >= GH_EXIT_FLUSH);
        match poll_login(generation, terminate_running, cleanup_failed_exit)? {
            LoginPoll::Pending => {
                completed_at = None;
                exited_at = None;
                if Instant::now() > deadline {
                    return wait_result_after_cancel(
                        generation,
                        GH_WAIT_TIMEOUT_MESSAGE.to_string(),
                    );
                }
            }
            LoginPoll::SuccessRunning => {
                completed_at.get_or_insert_with(Instant::now);
                exited_at = None;
            }
            LoginPoll::ExitedAwaitingOutput => {
                let exited = exited_at.get_or_insert_with(Instant::now);
                completed_at = None;
                if exited.elapsed() >= GH_EXIT_FLUSH {
                    let last = text
                        .lines()
                        .rev()
                        .find(|line| !line.trim().is_empty())
                        .unwrap_or("");
                    return wait_result_after_cancel(
                        generation,
                        format!("GitHub 로그인 출력 정리가 완료되지 않았습니다: {last}"),
                    );
                }
            }
            LoginPoll::ExitedWithoutSuccess => {
                let last = text
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .to_string();
                return Err(format!("GitHub 로그인이 완료되지 않았습니다: {last}"));
            }
            LoginPoll::Complete(login) => return Ok(login),
        }
        std::thread::sleep(GH_POLL);
    }
}

fn login_cleanup(generation: u64) -> Result<(), String> {
    let mut guard = GH_LOGIN.lock().map_err(|_| "내부 잠금 오류")?;
    let session = guard
        .as_ref()
        .ok_or_else(|| "GitHub 로그인이 이미 끝났거나 취소됐습니다".to_string())?;
    ensure_generation(session.generation, generation)?;
    *guard = None; // Drop이 PTY·자식 핸들을 정리한다
    Ok(())
}

fn child_has_exited(child: &mut (dyn Child + Send + Sync)) -> Result<bool, String> {
    child
        .try_wait()
        .map(|status| status.is_some())
        .map_err(|error| format!("gh 종료 상태 확인 실패: {error}"))
}

#[cfg(windows)]
fn terminate_windows_process_tree(pid: u32) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let taskkill = PathBuf::from(
        std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()),
    )
    .join("System32")
    .join("taskkill.exe");
    let mut process = std::process::Command::new(&taskkill)
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("gh 프로세스 트리 종료 실행 실패: {error}"))?;

    let deadline = Instant::now() + GH_TERMINATE_TIMEOUT;
    loop {
        match process.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(format!("gh 프로세스 트리 종료 실패: {status}"));
            }
            Err(error) => return Err(format!("gh 프로세스 트리 종료 확인 실패: {error}")),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(GH_TERMINATE_POLL),
            Ok(None) => {
                let _ = process.kill();
                let _ = process.wait();
                return Err("gh 프로세스 트리 종료 시간이 초과됐습니다".into());
            }
        }
    }
}

fn request_child_termination(child: &mut (dyn Child + Send + Sync)) -> Result<(), String> {
    #[cfg(windows)]
    if let Some(pid) = child.process_id() {
        return terminate_windows_process_tree(pid);
    }

    #[cfg(unix)]
    if let Some(pid) = child.process_id() {
        let result = unsafe { libc::killpg(pid as libc::pid_t, libc::SIGKILL) };
        if result == 0 {
            return Ok(());
        }
    }

    child
        .kill()
        .map_err(|error| format!("gh 종료 요청 실패: {error}"))
}

fn terminate_child_verified_with<F, K>(
    child: &mut (dyn Child + Send + Sync),
    poll_attempts: usize,
    mut pause: F,
    request_termination: K,
) -> Result<(), String>
where
    F: FnMut(),
    K: FnOnce(&mut (dyn Child + Send + Sync)) -> Result<(), String>,
{
    if child_has_exited(child)? {
        return Ok(());
    }

    if let Err(error) = request_termination(child) {
        // 상태 확인과 종료 요청 사이에 자연 종료했으면 취소는 이미 달성됐다.
        if child_has_exited(child)? {
            return Ok(());
        }
        return Err(error);
    }

    for attempt in 0..=poll_attempts {
        if child_has_exited(child)? {
            return Ok(());
        }
        if attempt < poll_attempts {
            pause();
        }
    }
    Err("gh 프로세스가 제한 시간 안에 종료되지 않았습니다".into())
}

fn terminate_child_verified(child: &mut (dyn Child + Send + Sync)) -> Result<(), String> {
    let poll_attempts = (GH_TERMINATE_TIMEOUT.as_millis() / GH_TERMINATE_POLL.as_millis()) as usize;
    terminate_child_verified_with(
        child,
        poll_attempts,
        || std::thread::sleep(GH_TERMINATE_POLL),
        request_child_termination,
    )
}

fn terminate_active_session_with<F, T>(
    guard: &mut Option<GhLoginSession>,
    drain_attempts: usize,
    mut pause: F,
    terminate: T,
) -> Result<bool, String>
where
    F: FnMut(),
    T: FnOnce(&mut (dyn Child + Send + Sync)) -> Result<(), String>,
{
    let Some(session) = guard.as_mut() else {
        return Ok(false);
    };
    terminate(session.child.as_mut())?;

    for attempt in 0..=drain_attempts {
        if session_login(session)?.is_some() {
            return Ok(false);
        }
        if session.reader_done.load(Ordering::Acquire) {
            // reader_done은 buffer 쓰기 뒤 Release로 기록된다. 신호를 본 다음 다시 읽어
            // 마지막 write와 cancel의 경합에서도 성공 마커를 놓치지 않는다.
            if session_login(session)?.is_some() {
                return Ok(false);
            }
            break;
        }
        if attempt < drain_attempts {
            pause();
        }
    }

    if session_login(session)?.is_some() {
        return Ok(false);
    }
    *guard = None;
    Ok(true)
}

fn terminate_active_session(guard: &mut Option<GhLoginSession>) -> Result<bool, String> {
    let drain_attempts = (GH_EXIT_FLUSH.as_millis() / GH_TERMINATE_POLL.as_millis()) as usize;
    terminate_active_session_with(
        guard,
        drain_attempts,
        || std::thread::sleep(GH_TERMINATE_POLL),
        terminate_child_verified,
    )
}

/// 진행 중 로그인 취소 — gh 프로세스 종료를 확인한 뒤에만 세션을 비운다.
pub fn login_cancel(generation: u64) -> Result<bool, String> {
    let _completion = GH_COMPLETION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let mut guard = GH_LOGIN.lock().map_err(|_| "내부 잠금 오류")?;
    let Some(current) = guard.as_ref() else {
        return Ok(false);
    };
    ensure_generation(current.generation, generation)?;
    terminate_active_session(&mut guard)
}

/// 프런트가 아직 프롬프트와 세션 ID를 받기 전의 명시적인 취소 경로.
pub fn login_cancel_start(request_id: &str) -> Result<bool, String> {
    validate_request_id(request_id)?;
    let _completion = GH_COMPLETION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let mut guard = GH_LOGIN.lock().map_err(|_| "내부 잠금 오류")?;
    let Some(current) = guard.as_ref() else {
        return Ok(GH_START_REQUESTS.cancel(request_id));
    };
    ensure_request_id(&current.request_id, request_id)?;
    terminate_active_session(&mut guard)
}

/// 앱 종료·업데이트 직전 진행 중인 GitHub 로그인을 최대한 정리한다.
pub fn cancel_on_shutdown() -> Result<bool, String> {
    let _completion = GH_COMPLETION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let mut guard = GH_LOGIN.lock().map_err(|_| "내부 잠금 오류")?;
    terminate_active_session(&mut guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_pty::{ChildKiller, ExitStatus};
    use std::sync::atomic::AtomicBool;

    static GH_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Clone, Debug)]
    struct TestChild {
        killed: Arc<AtomicBool>,
        fail_kill: Arc<AtomicBool>,
        exit_immediately: bool,
        ignore_kill: bool,
    }

    impl ChildKiller for TestChild {
        fn kill(&mut self) -> std::io::Result<()> {
            if self.fail_kill.load(Ordering::SeqCst) {
                return Err(std::io::Error::other("injected kill failure"));
            }
            self.killed.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
            Box::new(self.clone())
        }
    }

    impl Child for TestChild {
        fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
            let exited = self.exit_immediately
                || (!self.ignore_kill && self.killed.load(Ordering::SeqCst));
            Ok(exited.then(|| ExitStatus::with_exit_code(1)))
        }

        fn wait(&mut self) -> std::io::Result<ExitStatus> {
            self.killed.store(true, Ordering::SeqCst);
            Ok(ExitStatus::with_exit_code(1))
        }

        fn process_id(&self) -> Option<u32> {
            None
        }

        #[cfg(windows)]
        fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
            None
        }
    }

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected write failure"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn install_test_login(generation: u64, request_id: &str, output: &[u8]) -> Arc<AtomicBool> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 5,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        drop(pair.slave);
        let writer = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
        let killed = Arc::new(AtomicBool::new(false));
        *GH_LOGIN.lock().unwrap() = Some(GhLoginSession {
            generation,
            request_id: request_id.to_string(),
            child: Box::new(TestChild {
                killed: killed.clone(),
                fail_kill: Arc::new(AtomicBool::new(false)),
                exit_immediately: false,
                ignore_kill: false,
            }),
            writer,
            buffer: Arc::new(Mutex::new(output.to_vec())),
            reader_done: Arc::new(AtomicBool::new(true)),
            _master: pair.master,
        });
        killed
    }

    fn test_child(
        fail_kill: Arc<AtomicBool>,
        exit_immediately: bool,
        ignore_kill: bool,
    ) -> TestChild {
        TestChild {
            killed: Arc::new(AtomicBool::new(false)),
            fail_kill,
            exit_immediately,
            ignore_kill,
        }
    }

    fn install_test_login_with_child(
        generation: u64,
        request_id: &str,
        output: &[u8],
        child: TestChild,
    ) {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 5,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        drop(pair.slave);
        let writer = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
        *GH_LOGIN.lock().unwrap() = Some(GhLoginSession {
            generation,
            request_id: request_id.to_string(),
            child: Box::new(child),
            writer,
            buffer: Arc::new(Mutex::new(output.to_vec())),
            reader_done: Arc::new(AtomicBool::new(true)),
            _master: pair.master,
        });
    }

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

    #[test]
    fn rejects_a_stale_github_login_generation() {
        assert!(ensure_generation(42, 42).is_ok());
        assert!(ensure_generation(43, 42).is_err());
        assert!(ensure_request_id("request-new", "request-new").is_ok());
        assert!(ensure_request_id("request-new", "request-old").is_err());
    }

    #[test]
    fn active_github_session_lookup_is_exact_and_keeps_completion_drainable() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let generation = 7_178;
        let request_id = "00000000-0000-4000-8000-000000007178";
        install_test_login_with_child(
            generation,
            request_id,
            b"\x1b[32mLogged in as recoverable-octocat\x1b[0m\r\n",
            test_child(Arc::new(AtomicBool::new(false)), true, false),
        );

        assert_eq!(
            login_session_for_request(request_id).unwrap().as_deref(),
            Some("7178")
        );
        assert_eq!(
            login_session_for_request("00000000-0000-4000-8000-000000007179").unwrap(),
            None
        );
        assert_eq!(
            finish_login_if_ready(generation, false).unwrap().as_deref(),
            Some("recoverable-octocat")
        );
        assert!(GH_LOGIN.lock().unwrap().is_none());
    }

    #[test]
    fn completed_github_login_stays_tracked_until_child_stops() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let generation = 7_101;
        let request_id = "00000000-0000-4000-8000-000000007101";
        let killed = install_test_login(
            generation,
            request_id,
            b"\x1b[32mLogged in as octocat\x1b[0m\r\n",
        );

        assert_eq!(finish_login_if_ready(generation, false).unwrap(), None);
        assert!(GH_LOGIN.lock().unwrap().is_some());
        assert!(!killed.load(Ordering::SeqCst));

        assert_eq!(login_cancel_start(request_id).unwrap(), false);
        assert_eq!(login_cancel(generation).unwrap(), false);
        assert!(GH_LOGIN.lock().unwrap().is_some());
        assert!(killed.load(Ordering::SeqCst));

        assert_eq!(
            finish_login_if_ready(generation, false).unwrap(),
            Some("octocat".to_string())
        );
        assert!(GH_LOGIN.lock().unwrap().is_none());
        assert!(killed.load(Ordering::SeqCst));
    }

    #[test]
    fn completed_github_login_retains_session_when_termination_fails() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let generation = 7_110;
        let request_id = "00000000-0000-4000-8000-000000007110";
        let fail_kill = Arc::new(AtomicBool::new(true));
        let child = test_child(fail_kill.clone(), false, false);
        let killed = child.killed.clone();
        install_test_login_with_child(
            generation,
            request_id,
            b"\x1b[32mLogged in as retry-octocat\x1b[0m\r\n",
            child,
        );

        let error = finish_login_if_ready(generation, true).unwrap_err();

        assert!(error.contains("종료 요청 실패: injected kill failure"));
        assert!(GH_LOGIN.lock().unwrap().is_some());

        fail_kill.store(false, Ordering::SeqCst);
        assert_eq!(login_cancel(generation).unwrap(), false);
        assert!(killed.load(Ordering::SeqCst));
        assert!(GH_LOGIN.lock().unwrap().is_some());
        assert_eq!(
            finish_login_if_ready(generation, false).unwrap(),
            Some("retry-octocat".to_string())
        );
        assert!(GH_LOGIN.lock().unwrap().is_none());
    }

    #[test]
    fn exited_github_login_waits_for_the_reader_before_failure_cleanup() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let generation = 7_112;
        let request_id = "00000000-0000-4000-8000-000000007112";
        let child = test_child(Arc::new(AtomicBool::new(false)), true, false);
        install_test_login_with_child(generation, request_id, b"Finishing\r\n", child);
        let (buffer, reader_done) = {
            let guard = GH_LOGIN.lock().unwrap();
            let session = guard.as_ref().unwrap();
            session.reader_done.store(false, Ordering::Release);
            (session.buffer.clone(), session.reader_done.clone())
        };

        assert_eq!(
            poll_login(generation, false, true).unwrap(),
            LoginPoll::ExitedAwaitingOutput
        );
        assert!(GH_LOGIN.lock().unwrap().is_some());

        buffer
            .lock()
            .unwrap()
            .extend_from_slice(b"\x1b[32mLogged in as flushed-octocat\x1b[0m\r\n");
        reader_done.store(true, Ordering::Release);
        assert_eq!(
            poll_login(generation, false, true).unwrap(),
            LoginPoll::Complete("flushed-octocat".to_string())
        );
        assert!(GH_LOGIN.lock().unwrap().is_none());
    }

    #[test]
    fn exited_github_login_without_success_cleans_only_after_the_grace() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let generation = 7_113;
        let request_id = "00000000-0000-4000-8000-000000007113";
        let child = test_child(Arc::new(AtomicBool::new(false)), true, false);
        install_test_login_with_child(generation, request_id, b"Authentication failed\r\n", child);

        assert_eq!(
            poll_login(generation, false, false).unwrap(),
            LoginPoll::ExitedAwaitingOutput
        );
        assert!(GH_LOGIN.lock().unwrap().is_some());
        assert_eq!(
            poll_login(generation, false, true).unwrap(),
            LoginPoll::ExitedWithoutSuccess
        );
        assert!(GH_LOGIN.lock().unwrap().is_none());
    }

    #[test]
    fn cancellation_winner_removes_session_before_waiter_can_finish() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let generation = 7_102;
        let request_id = "00000000-0000-4000-8000-000000007102";
        let killed = install_test_login(generation, request_id, b"Waiting for authentication\r\n");

        assert_eq!(login_cancel(generation).unwrap(), true);
        assert!(killed.load(Ordering::SeqCst));
        assert!(GH_LOGIN.lock().unwrap().is_none());
        assert!(finish_login_if_ready(generation, false).is_err());
    }

    #[test]
    fn cancellation_drains_late_success_before_removing_session() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let generation = 7_109;
        let request_id = "00000000-0000-4000-8000-000000007109";
        let child = test_child(Arc::new(AtomicBool::new(false)), true, false);
        install_test_login_with_child(
            generation,
            request_id,
            b"Waiting for authentication\r\n",
            child,
        );
        let (buffer, reader_done) = {
            let guard = GH_LOGIN.lock().unwrap();
            let session = guard.as_ref().unwrap();
            session.reader_done.store(false, Ordering::Release);
            (session.buffer.clone(), session.reader_done.clone())
        };
        let mut emitted = false;

        let cancelled = {
            let mut guard = GH_LOGIN.lock().unwrap();
            terminate_active_session_with(
                &mut guard,
                1,
                || {
                    assert!(!emitted);
                    buffer
                        .lock()
                        .unwrap()
                        .extend_from_slice(b"\x1b[32mLogged in as late-octocat\x1b[0m\r\n");
                    reader_done.store(true, Ordering::Release);
                    emitted = true;
                },
                terminate_child_verified,
            )
            .unwrap()
        };

        assert!(!cancelled);
        assert!(emitted);
        assert!(GH_LOGIN.lock().unwrap().is_some());
        assert_eq!(
            finish_login_if_ready(generation, false).unwrap(),
            Some("late-octocat".to_string())
        );
        assert!(GH_LOGIN.lock().unwrap().is_none());
    }

    #[test]
    fn cancelling_an_already_finished_github_login_is_idempotent() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        assert_eq!(login_cancel(u64::MAX).unwrap(), false);
    }

    #[test]
    fn verified_termination_accepts_an_already_exited_child_without_killing() {
        let fail_kill = Arc::new(AtomicBool::new(false));
        let mut child = test_child(fail_kill, true, false);
        let killed = child.killed.clone();

        terminate_child_verified_with(&mut child, 0, || {}, request_child_termination).unwrap();

        assert!(!killed.load(Ordering::SeqCst));
    }

    #[test]
    fn verified_termination_reports_kill_failure() {
        let fail_kill = Arc::new(AtomicBool::new(true));
        let mut child = test_child(fail_kill, false, false);

        let error = terminate_child_verified_with(
            &mut child,
            0,
            || {},
            request_child_termination,
        )
        .unwrap_err();

        assert!(error.contains("종료 요청 실패"));
    }

    #[test]
    fn verified_termination_reports_timeout_without_real_sleep() {
        let fail_kill = Arc::new(AtomicBool::new(false));
        let mut child = test_child(fail_kill, false, true);

        let error = terminate_child_verified_with(
            &mut child,
            2,
            || {},
            request_child_termination,
        )
        .unwrap_err();

        assert!(error.contains("제한 시간"));
        assert!(child.killed.load(Ordering::SeqCst));
    }

    #[test]
    fn failed_cancellation_retains_the_session_for_retry() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let generation = 7_103;
        let request_id = "00000000-0000-4000-8000-000000007103";
        let fail_kill = Arc::new(AtomicBool::new(true));
        let child = test_child(fail_kill.clone(), false, false);
        install_test_login_with_child(
            generation,
            request_id,
            b"Waiting for authentication\r\n",
            child,
        );

        assert!(login_cancel(generation).is_err());
        assert!(GH_LOGIN.lock().unwrap().is_some());

        fail_kill.store(false, Ordering::SeqCst);
        assert_eq!(login_cancel(generation).unwrap(), true);
        assert!(GH_LOGIN.lock().unwrap().is_none());
    }

    #[test]
    fn send_failure_preserves_termination_failure_and_session() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let generation = 7_105;
        let request_id = "00000000-0000-4000-8000-000000007105";
        let fail_kill = Arc::new(AtomicBool::new(true));
        let child = test_child(fail_kill.clone(), false, false);
        install_test_login_with_child(generation, request_id, b"Waiting\r\n", child);
        GH_LOGIN.lock().unwrap().as_mut().unwrap().writer =
            Arc::new(Mutex::new(Box::new(FailingWriter)));

        let error = gh_send(generation, b"y\r").unwrap_err();

        assert!(error.contains("입력 전송 실패: injected write failure"));
        assert!(error.contains("프로세스 정리 실패"));
        assert!(error.contains("종료 요청 실패: injected kill failure"));
        assert!(GH_LOGIN.lock().unwrap().is_some());

        fail_kill.store(false, Ordering::SeqCst);
        assert_eq!(login_cancel(generation).unwrap(), true);
    }

    #[test]
    fn prompt_timeout_preserves_termination_failure_and_session() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let generation = 7_106;
        let request_id = "00000000-0000-4000-8000-000000007106";
        let fail_kill = Arc::new(AtomicBool::new(true));
        let child = test_child(fail_kill.clone(), false, false);
        install_test_login_with_child(generation, request_id, b"Waiting\r\n", child);

        let error = prompt_timeout_error(generation, "Waiting for code");

        assert!(error.contains("60초 안에 뜨지 않았습니다"));
        assert!(error.contains("마지막 화면: Waiting for code"));
        assert!(error.contains("종료 요청 실패: injected kill failure"));
        assert!(GH_LOGIN.lock().unwrap().is_some());

        fail_kill.store(false, Ordering::SeqCst);
        assert_eq!(login_cancel(generation).unwrap(), true);
    }

    #[test]
    fn wait_timeout_preserves_termination_failure_and_session() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let generation = 7_107;
        let request_id = "00000000-0000-4000-8000-000000007107";
        let fail_kill = Arc::new(AtomicBool::new(true));
        let child = test_child(fail_kill.clone(), false, false);
        install_test_login_with_child(generation, request_id, b"Waiting\r\n", child);

        let error = wait_result_after_cancel(generation, GH_WAIT_TIMEOUT_MESSAGE.to_string())
            .unwrap_err();

        assert!(error.contains("대기 시간(10분)을 넘겼습니다"));
        assert!(error.contains("종료 요청 실패: injected kill failure"));
        assert!(GH_LOGIN.lock().unwrap().is_some());

        fail_kill.store(false, Ordering::SeqCst);
        assert_eq!(login_cancel(generation).unwrap(), true);
    }

    #[test]
    fn timeout_error_stays_unchanged_when_cancel_succeeds() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let generation = 7_108;
        let request_id = "00000000-0000-4000-8000-000000007108";
        install_test_login(generation, request_id, b"Waiting\r\n");

        let error = wait_result_after_cancel(generation, GH_WAIT_TIMEOUT_MESSAGE.to_string())
            .unwrap_err();

        assert_eq!(
            error,
            "GitHub 로그인 대기 시간(10분)을 넘겼습니다 — 다시 시도하세요"
        );
        assert!(GH_LOGIN.lock().unwrap().is_none());
    }

    #[test]
    fn wait_boundary_recovers_a_late_success_from_cancel_drain() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let generation = 7_114;
        let request_id = "00000000-0000-4000-8000-000000007114";
        let killed = install_test_login(
            generation,
            request_id,
            b"\x1b[32mLogged in as boundary-octocat\x1b[0m\r\n",
        );

        assert_eq!(
            wait_result_after_cancel(generation, "stale timeout".to_string()).unwrap(),
            "boundary-octocat"
        );
        assert!(killed.load(Ordering::SeqCst));
        assert!(GH_LOGIN.lock().unwrap().is_none());
    }

    #[test]
    fn stalled_reader_is_bounded_and_cleared_after_verified_child_exit() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let generation = 7_115;
        let request_id = "00000000-0000-4000-8000-000000007115";
        let child = test_child(Arc::new(AtomicBool::new(false)), true, false);
        install_test_login_with_child(generation, request_id, b"Finishing\r\n", child);
        GH_LOGIN
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .reader_done
            .store(false, Ordering::Release);

        let error = wait_result_after_cancel(generation, "reader stalled".to_string()).unwrap_err();

        assert_eq!(error, "reader stalled");
        assert!(GH_LOGIN.lock().unwrap().is_none());
    }

    #[test]
    fn shutdown_cancellation_quiesces_an_active_github_login() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let generation = 7_104;
        let request_id = "00000000-0000-4000-8000-000000007104";
        let killed = install_test_login(generation, request_id, b"Waiting\r\n");

        assert_eq!(cancel_on_shutdown().unwrap(), true);
        assert!(killed.load(Ordering::SeqCst));
        assert!(GH_LOGIN.lock().unwrap().is_none());
        assert_eq!(cancel_on_shutdown().unwrap(), false);
    }

    #[test]
    fn shutdown_still_quiesces_a_completed_but_running_github_login() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let generation = 7_111;
        let request_id = "00000000-0000-4000-8000-000000007111";
        let killed = install_test_login(
            generation,
            request_id,
            b"\x1b[32mLogged in as shutdown-octocat\x1b[0m\r\n",
        );

        assert_eq!(cancel_on_shutdown().unwrap(), false);
        assert!(killed.load(Ordering::SeqCst));
        assert!(GH_LOGIN.lock().unwrap().is_some());
        assert_eq!(
            finish_login_if_ready(generation, false).unwrap(),
            Some("shutdown-octocat".to_string())
        );
        assert!(GH_LOGIN.lock().unwrap().is_none());
    }

    #[test]
    fn failed_starts_and_late_cancels_do_not_grow_the_github_registry() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let baseline = GH_START_REQUESTS.len();
        for index in 0..64 {
            let request_id = format!("00000000-0000-4000-8101-{index:012x}");
            reserve_login_start(&request_id).unwrap();
            let lease = GH_START_REQUESTS
                .claim(&request_id, GITHUB_START_KIND, "GitHub 로그인")
                .unwrap();
            drop(lease);
            assert!(!login_cancel_start(&request_id).unwrap());
        }
        assert_eq!(GH_START_REQUESTS.len(), baseline);
    }

    #[test]
    fn github_start_requests_are_isolated_and_unknown_cancel_is_a_noop() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let baseline = GH_START_REQUESTS.len();
        let first = "00000000-0000-4000-8102-000000000001";
        let second = "00000000-0000-4000-8102-000000000002";
        let unknown = "00000000-0000-4000-8102-000000000003";

        assert!(!login_cancel_start(unknown).unwrap());
        assert_eq!(GH_START_REQUESTS.len(), baseline);
        reserve_login_start(first).unwrap();
        reserve_login_start(second).unwrap();
        assert!(login_cancel_start(first).unwrap());

        let second_lease = GH_START_REQUESTS
            .claim(second, GITHUB_START_KIND, "GitHub 로그인")
            .unwrap();
        drop(second_lease);
        assert!(GH_START_REQUESTS
            .claim(first, GITHUB_START_KIND, "GitHub 로그인")
            .err()
            .unwrap()
            .contains("취소"));
        assert_eq!(GH_START_REQUESTS.len(), baseline);
    }

    #[test]
    fn github_start_request_cleanup_survives_panic_and_join_release() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let baseline = GH_START_REQUESTS.len();
        let panics = "00000000-0000-4000-8103-000000000001";
        let never_joined = "00000000-0000-4000-8103-000000000002";

        reserve_login_start(panics).unwrap();
        let unwind = std::panic::catch_unwind(|| {
            let _lease = GH_START_REQUESTS
                .claim(panics, GITHUB_START_KIND, "GitHub 로그인")
                .unwrap();
            panic!("injected GitHub worker panic");
        });
        assert!(unwind.is_err());
        assert!(!login_cancel_start(panics).unwrap());

        reserve_login_start(never_joined).unwrap();
        release_login_start(never_joined).unwrap();
        assert!(!login_cancel_start(never_joined).unwrap());
        assert_eq!(GH_START_REQUESTS.len(), baseline);
    }

    #[test]
    fn pending_github_start_cancellation_rejects_unbounded_ids() {
        assert!(login_cancel_start("not-a-uuid").is_err());
        assert!(reserve_login_start("not-a-uuid").is_err());
    }

    /// 실기기 전용: 인앱 로그인 프롬프트(주소+일회용 코드)가 뜨는지 확인하고
    /// 즉시 취소한다 — 계정 무변경, 발급된 코드는 브라우저 입력 없이는 무효
    #[test]
    #[ignore]
    fn real_github_login_prompt_then_cancel() {
        let _test = GH_TEST_LOCK.lock().unwrap();
        let request_id = "00000000-0000-4000-8000-000000000099";
        reserve_login_start(request_id).unwrap();
        let prompt = login_start(request_id.to_string()).expect("로그인 프롬프트 실패");
        println!("url={} code={}", prompt.url, prompt.device_code);
        assert!(prompt.url.contains("github.com"));
        assert!(prompt.device_code.contains('-'));
        login_cancel(prompt.session_id.parse().unwrap()).unwrap();
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

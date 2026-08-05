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
    auto_name, ensure_name_not_owned_by_other, find_profile_by_id, identity_from_value, now,
    read_json, write_profile_parts, Env, LiveIdentity, Provider, MUTATION_LOCK,
};

/// 로그인 링크가 화면에 뜰 때까지 기다리는 시간
const PROMPT_TIMEOUT: Duration = Duration::from_secs(60);
/// 코드 입력 후 로그인이 끝날 때까지 기다리는 시간
const FINISH_TIMEOUT: Duration = Duration::from_secs(45);
/// 코덱스처럼 브라우저에서 코드를 넣고 CLI가 알아서 끝내는 방식의 대기 시간
const DEVICE_TIMEOUT: Duration = Duration::from_secs(600);
const POLL: Duration = Duration::from_millis(300);
/// 자식 프로세스가 종료한 뒤 남은 출력이 버퍼에 도착하기를 기다리는 유예
const EXIT_FLUSH: Duration = Duration::from_millis(700);
/// 화면 누적 버퍼 상한 (TUI 스피너가 세션 내내 쌓이므로 캡을 둔다)
const OUTPUT_CAP: usize = 256 * 1024;
/// 코드 입력 최대 길이 (콘솔 stdin으로 흘러가므로 과대 입력을 막는다)
const CODE_MAX_LEN: usize = 256;
/// 이보다 오래된 임시 로그인 폴더만 청소한다 — 다른 인스턴스의 진행 중 로그인을 지우지 않기 위함
/// (DEVICE_TIMEOUT보다 길게 잡아, 살아 있는 세션의 폴더일 가능성을 배제)
const SWEEP_MIN_AGE: Duration = Duration::from_secs(15 * 60);

#[derive(Serialize, Debug)]
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
    /// PTY를 살려둬야 자식 프로세스가 끊기지 않는다
    _master: Box<dyn MasterPty + Send>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

/// ANSI 이스케이프 시퀀스를 걷어내 사람이 읽는 글자만 남긴다.
/// 색상(SGR, 최종 바이트 m)은 글자 중간에도 끼므로 조용히 버리고,
/// 커서 이동·지우기는 화면상 위치가 바뀐다는 뜻이라 줄바꿈으로 바꿔 토큰을 끊는다.
/// (TUI는 줄바꿈 대신 커서 이동으로 그리기 때문에 이렇게 해야 글자가 붙지 않는다)
pub(crate) fn strip_ansi(bytes: &[u8]) -> String {
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
            // OSC: BEL 또는 ESC \ 까지 건너뛴다 (하이퍼링크 대상은 extract_osc8_urls가 따로 줍는다)
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

/// OSC 8 하이퍼링크(ESC]8;params;URL)의 대상 주소를 원시 바이트에서 줍는다.
/// 하이퍼링크 대상은 화면 줄바꿈과 무관하게 항상 완전한 URL이므로,
/// 가시 텍스트 추출보다 이쪽을 우선한다 (긴 OAuth 주소 절단 방지).
pub(crate) fn extract_osc8_urls(bytes: &[u8]) -> Vec<String> {
    let mut urls = Vec::new();
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i] == 0x1b && bytes[i + 1] == b']' && bytes[i + 2] == b'8' && bytes[i + 3] == b';'
        {
            // params 건너뛰기: 다음 ';'까지
            let mut j = i + 4;
            while j < bytes.len() && bytes[j] != b';' {
                j += 1;
            }
            j += 1;
            // URL: BEL 또는 ESC\ 전까지
            let start = j;
            while j < bytes.len() {
                if bytes[j] == 0x07 || (bytes[j] == 0x1b && bytes.get(j + 1) == Some(&b'\\')) {
                    break;
                }
                j += 1;
            }
            let url = String::from_utf8_lossy(&bytes[start..j]).into_owned();
            if url.starts_with("https://") && url.len() > 20 {
                urls.push(url);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    urls
}

/// 화면 글자에서 로그인 주소를 찾는다 (OSC 8이 없을 때의 폴백)
pub(crate) fn extract_visible_url(text: &str) -> Option<String> {
    let mut candidates = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find("https://") {
        let tail = &rest[pos..];
        let url: String = tail
            .chars()
            .take_while(|c| !c.is_whitespace() && !c.is_control() && *c != '"' && *c != '\\')
            .collect();
        let consumed = pos + url.len().max(8);
        if url.len() > 20 {
            candidates.push(url);
        }
        rest = &rest[consumed.min(rest.len())..];
    }
    pick_login_url(candidates)
}

/// 후보 중 로그인용으로 보이는 주소를 고른다 (배너·안내 링크 오탐 방지)
pub(crate) fn pick_login_url(candidates: Vec<String>) -> Option<String> {
    candidates
        .iter()
        .find(|u| u.contains("oauth") || u.contains("authorize") || u.contains("/device"))
        .cloned()
        .or_else(|| candidates.into_iter().next())
}

/// 코덱스가 보여주는 일회용 코드(예: V4GM-HT05H)를 찾는다.
/// 대시 구분선("----")이나 날짜("2026-07-28")를 오인하지 않도록
/// 영문과 숫자가 모두 있고 양끝이 영숫자인 것만 인정한다.
pub(crate) fn extract_device_code(text: &str) -> Option<String> {
    for line in text.lines() {
        let token = line.trim();
        let ok = token.len() >= 8
            && token.len() <= 16
            && token.contains('-')
            && token
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
            && token.chars().any(|c| c.is_ascii_uppercase())
            && token.chars().any(|c| c.is_ascii_digit())
            && token.starts_with(|c: char| c.is_ascii_alphanumeric())
            && token.ends_with(|c: char| c.is_ascii_alphanumeric());
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

/// GUI 앱(Finder·Dock 실행)은 셸 PATH를 모른다 — 로그인 셸에 묻고, 실패하면
/// CLI가 흔히 설치되는 경로를 직접 짚는다. 끝내 못 찾으면 이름 그대로 돌려줘
/// spawn이 명확한 미설치 에러를 내게 둔다.
/// 주의: program은 반드시 상수 이름("claude"·"codex"·"gh")만 — 셸 명령 문자열에
/// 그대로 보간되므로 비상수 입력을 넘기면 셸 인젝션이 된다.
#[cfg(not(windows))]
pub(crate) fn resolve_program(program: &str) -> String {
    let shell = std::process::Command::new("/bin/zsh")
        .args(["-lc", &format!("command -v {program}")])
        .output();
    if let Ok(out) = shell {
        if out.status.success() {
            if let Some(line) = String::from_utf8_lossy(&out.stdout).lines().next() {
                let line = line.trim();
                if line.starts_with('/') && Path::new(line).exists() {
                    return line.to_string();
                }
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let candidates = [
            home.join(".local/bin").join(program),
            PathBuf::from("/opt/homebrew/bin").join(program),
            PathBuf::from("/usr/local/bin").join(program),
        ];
        for candidate in candidates {
            if candidate.exists() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    program.to_string()
}

/// 격리 로그인(CLAUDE_CONFIG_DIR)이 키체인에 만드는 항목 이름.
/// 실측(2026-07-29, claude 2.1.220): 맥 CLI는 격리 로그인 토큰을 폴더의 파일이 아니라
/// "Claude Code-credentials-<sha256(경로)[:8]>" 키체인 항목에 쓴다.
/// 경로 문자열을 그대로 해시하므로, 심볼릭 링크로 표기가 갈릴 때를 대비해
/// 원본·정규화(canonicalize) 두 이름을 모두 후보로 삼는다.
#[cfg(target_os = "macos")]
fn isolated_keychain_services(config_dir: &Path) -> Vec<String> {
    use sha2::{Digest, Sha256};
    let mut out: Vec<String> = Vec::new();
    let mut push = |path: &str| {
        let digest = Sha256::digest(path.as_bytes());
        let hex: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
        let service = format!("{}-{hex}", crate::accounts::keychain::CLAUDE_LIVE_SERVICE);
        if !out.contains(&service) {
            out.push(service);
        }
    };
    push(&config_dir.to_string_lossy());
    if let Ok(real) = config_dir.canonicalize() {
        push(&real.to_string_lossy());
    }
    out
}

/// 임시 로그인 폴더와 그 로그인이 남긴 키체인 항목(맥)을 지운다.
/// 키체인을 폴더보다 먼저 지운다 — 폴더가 사라지면 항목 이름(경로 해시)을 되구할 수 없다.
fn cleanup_isolated(config_dir: &Path) {
    #[cfg(target_os = "macos")]
    for service in isolated_keychain_services(config_dir) {
        crate::accounts::keychain::delete_item(&service);
    }
    remove_dir_retry(config_dir);
}

fn temp_config_dir(env: &Env) -> PathBuf {
    // 같은 초에 두 로그인이 시작해도 겹치지 않게 pid+일련번호를 붙인다
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    env.store
        .join("_login")
        .join(format!("{}-{}-{seq}", now(), std::process::id()))
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

/// 중단·크래시가 남긴 임시 로그인 폴더를 청소한다.
/// 다른 인스턴스가 진행 중일 수 있으므로 충분히 오래된 것만 지운다.
pub fn sweep_stale(env: &Env) {
    let root = env.store.join("_login");
    let Ok(entries) = fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        let old_enough = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|age| age > SWEEP_MIN_AGE)
            .unwrap_or(false);
        if old_enough {
            cleanup_isolated(&entry.path());
        }
    }
}

/// CLI 설치·업데이트 명령 (안내문 공용 — 프로그램명 매핑은 cli_args가 단일 출처)
fn install_cmd(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "npm install -g @anthropic-ai/claude-code",
        Provider::Codex => "npm install -g @openai/codex",
    }
}

/// CLI가 로그인 화면을 띄우기 전에 종료했을 때의 안내.
/// 미설치·구버전이 대표 원인이지만, 코덱스는 시작에 서버 왕복이 필요해 네트워크 문제일 수도 있다.
fn early_exit_error(provider: Provider) -> String {
    let (program, _, _) = cli_args(provider);
    format!(
        "{program} CLI가 로그인 화면을 띄우지 못하고 종료됐습니다 — 설치돼 있고 최신인지, 네트워크가 연결돼 있는지 확인하세요 (설치·업데이트: {})",
        install_cmd(provider)
    )
}

/// 로그인을 시작하고 화면에 뜬 주소를 돌려준다
pub fn start(env: &Env, provider: Provider) -> Result<LoginPrompt, String> {
    start_impl(env, provider, cli_args(provider))
}

/// start의 본체 — 실행 명령을 주입받는다.
/// 테스트가 존재하지 않는 명령을 넣어 조기 종료 경로(미설치 CLI)를 검증한다.
fn start_impl(
    env: &Env,
    provider: Provider,
    (program, args, env_key): (&str, &[&str], &str),
) -> Result<LoginPrompt, String> {
    // 세션 검사부터 등록까지 잠금을 쥔 채 진행한다 —
    // 연타로 두 로그인이 동시에 시작해 폴더·세션이 꼬이는 것을 막는다 (red-review 2라운드)
    {
        let mut guard = SESSION.lock().map_err(|_| "내부 잠금 오류")?;
        if guard.is_some() {
            return Err("이미 로그인이 진행 중입니다".into());
        }
        sweep_stale(env);

        let config_dir = temp_config_dir(env);
        fs::create_dir_all(&config_dir)
            .map_err(|e| format!("임시 폴더 생성 실패 {}: {e}", config_dir.display()))?;

        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                // 긴 OAuth 주소가 줄바꿈으로 잘리지 않게 넉넉히
                cols: 500,
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
        // 유닉스는 GUI 앱이 셸 PATH를 모르므로 절대경로로 해석해 실행한다
        #[cfg(not(windows))]
        let mut cmd = CommandBuilder::new(resolve_program(program));
        for a in args {
            cmd.arg(a);
        }
        cmd.env(env_key, &config_dir);

        let child = pair.slave.spawn_command(cmd).map_err(|e| {
            cleanup_isolated(&config_dir);
            format!(
                "{program} 실행에 실패했습니다: {e} — 설치·업데이트: {}",
                install_cmd(provider)
            )
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

        let sink = output_buffer();
        {
            // 새 세션 시작 — 이전 세션의 화면 잔재를 비운다
            if let Ok(mut acc) = sink.lock() {
                acc.clear();
            }
        }
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            // ESC[6n이 읽기 경계에 걸쳐도 놓치지 않게 직전 꼬리를 이어 검사한다
            let mut tail: Vec<u8> = Vec::new();
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let piece = &buf[..n];
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
                    // 스피너 프레임이 무한히 쌓이지 않게 앞부분을 버린다
                    if acc.len() > OUTPUT_CAP {
                        let cut = acc.len() - OUTPUT_CAP;
                        acc.drain(..cut);
                    }
                }
            }
        });

        *guard = Some(Session {
            provider,
            config_dir,
            child,
            writer,
            _master: pair.master,
        });
    }

    // 주소가 화면에 뜰 때까지 기다린다 (잠금 밖 — 취소 가능해야 하므로)
    let deadline = Instant::now() + PROMPT_TIMEOUT;
    // CLI가 주소를 띄우기 전에 죽으면(미설치·구버전) 60초를 채울 이유가 없다 —
    // 종료를 감지하면 남은 출력이 버퍼에 닿을 짧은 유예만 주고 바로 실패를 알린다
    let mut exited_at: Option<Instant> = None;
    loop {
        // 그 사이 취소됐으면 중단 + 자식 프로세스 생존 확인
        {
            let mut guard = SESSION.lock().map_err(|_| "내부 잠금 오류")?;
            let Some(session) = guard.as_mut() else {
                return Err("로그인을 취소했습니다".into());
            };
            if exited_at.is_none() {
                if let Ok(Some(_)) = session.child.try_wait() {
                    exited_at = Some(Instant::now());
                }
            }
        }
        let raw = {
            let acc = output_buffer().lock().map_err(|_| "내부 잠금 오류")?;
            acc.clone()
        };
        // 하이퍼링크 대상(항상 완전한 주소)을 우선, 가시 텍스트는 폴백
        let url = pick_login_url(extract_osc8_urls(&raw))
            .or_else(|| extract_visible_url(&strip_ansi(&raw)));
        if let Some(url) = url {
            match provider {
                Provider::Claude => {
                    return Ok(LoginPrompt {
                        url,
                        device_code: None,
                        needs_code: true,
                    });
                }
                Provider::Codex => {
                    // 코덱스는 일회용 코드까지 화면에 떠야 완성이다 — 둘 다 기다린다
                    if let Some(code) = extract_device_code(&strip_ansi(&raw)) {
                        return Ok(LoginPrompt {
                            url,
                            device_code: Some(code),
                            needs_code: false,
                        });
                    }
                }
            }
        }
        // 자식이 종료했고 플러시 유예도 지났는데 주소가 없다 — 재시도로 해결될 문제가 아니다
        if exited_at.is_some_and(|t| t.elapsed() > EXIT_FLUSH) {
            cancel();
            return Err(early_exit_error(provider));
        }
        if Instant::now() > deadline {
            cancel();
            return Err("로그인 주소를 받지 못했습니다 — 잠시 후 다시 시도하세요".into());
        }
        std::thread::sleep(POLL);
    }
}

/// 화면 누적 버퍼 (세션 하나만 존재하므로 전역 하나로 충분)
fn output_buffer() -> &'static Mutex<Vec<u8>> {
    static BUF: Mutex<Vec<u8>> = Mutex::new(Vec::new());
    &BUF
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
            return Err("시간이 초과됐습니다 — 처음부터 다시 시도하세요".into());
        }
        std::thread::sleep(POLL);
    }
}

/// 격리 폴더에 생긴 로그인 결과를 읽어 계정 정보와 토큰을 뽑는다.
/// 신원 파싱은 활성 파일 판독과 같은 identity_from_value를 쓴다.
fn read_login_result(
    provider: Provider,
    config_dir: &Path,
) -> Result<(LiveIdentity, Vec<u8>, Option<Value>), String> {
    let cred_path = match provider {
        Provider::Claude => config_dir.join(".credentials.json"),
        Provider::Codex => config_dir.join("auth.json"),
    };
    let mut cred: Option<Vec<u8>> = if cred_path.exists() {
        Some(fs::read(&cred_path).map_err(|e| format!("읽기 실패: {e}"))?)
    } else {
        None
    };
    // 맥 클로드는 격리 로그인 토큰이 파일이 아니라 키체인 항목으로 생긴다 (실측)
    #[cfg(target_os = "macos")]
    if cred.is_none() && provider == Provider::Claude {
        for service in isolated_keychain_services(config_dir) {
            if let Some(data) = crate::accounts::keychain::read_item(&service)? {
                cred = Some(data);
                break;
            }
        }
    }
    let Some(cred) = cred else {
        // 코덱스는 장치 코드 인증이 계정에서 꺼져 있으면 승인 단계에서 거부된다 (기본값이 꺼짐)
        return Err(match provider {
            Provider::Codex => "로그인이 완료되지 않았습니다 — ChatGPT 설정 → 보안에서 \
                'Codex 장치 코드 인증'을 켠 뒤 다시 시도하세요"
                .into(),
            Provider::Claude => "로그인이 완료되지 않았습니다 — 처음부터 다시 시도하세요".to_string(),
        });
    };
    match provider {
        Provider::Claude => {
            let root = read_json(&config_dir.join(".claude.json"))?;
            let ident = identity_from_value(Provider::Claude, &root)
                .ok_or("로그인 결과에서 계정 정보를 찾지 못했습니다")?;
            let block = root.get("oauthAccount").cloned();
            Ok((ident, cred, block))
        }
        Provider::Codex => {
            let root: Value =
                serde_json::from_slice(&cred).map_err(|e| format!("JSON 파싱 실패: {e}"))?;
            let ident = identity_from_value(Provider::Codex, &root)
                .ok_or("로그인 결과에서 계정 정보를 찾지 못했습니다")?;
            Ok((ident, cred, None))
        }
    }
}

/// 세션을 정리하고 (provider, 임시 폴더)를 돌려준다
fn finish_session() -> Option<(Provider, PathBuf)> {
    let mut guard = SESSION.lock().ok()?;
    let session = guard.take()?;
    Some((session.provider, session.config_dir))
}

/// 임시 폴더의 로그인 결과를 프로필로 들여온다.
/// 어떤 경로로 끝나든 폴더와 키체인 잔재(맥)는 지운다.
fn import(env: &Env, provider: Provider, config_dir: &Path) -> Result<LoginOutcome, String> {
    let result = import_inner(env, provider, config_dir);
    cleanup_isolated(config_dir);
    result
}

fn import_inner(env: &Env, provider: Provider, config_dir: &Path) -> Result<LoginOutcome, String> {
    let (ident, cred, block) = read_login_result(provider, config_dir)?;

    // 프로필을 실제로 건드리는 구간에서만 잠근다
    let _guard = MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let existing = find_profile_by_id(env, provider, &ident.id)?;
    let updated_existing = existing.is_some();
    let name = existing.unwrap_or_else(|| auto_name(env, provider, &ident));
    // auto_name이 빈 이름을 보장하지만, 불변("다른 계정 토큰을 덮어쓰지 않는다")은 여기서도 지킨다
    ensure_name_not_owned_by_other(env, provider, &name, &ident)?;
    write_profile_parts(env, provider, &name, &ident, &cred, block.as_ref())?;
    Ok(LoginOutcome {
        profile: name,
        email: ident.email,
        updated_existing,
    })
}

/// 로그인 종료를 기다렸다가 결과를 프로필로 들여온다 (submit/wait 공용 꼬리)
fn finish_and_import(env: &Env, timeout: Duration) -> Result<LoginOutcome, String> {
    wait_for_exit(timeout)?;
    let (provider, dir) = finish_session().ok_or("로그인 세션이 사라졌습니다")?;
    import(env, provider, &dir)
}

/// 브라우저에서 받은 코드를 CLI에 전달해 로그인을 끝낸다 (클로드)
pub fn submit_code(env: &Env, code: &str) -> Result<LoginOutcome, String> {
    let code = code.trim();
    if code.is_empty() {
        return Err("코드를 입력하세요".into());
    }
    if code.len() > CODE_MAX_LEN {
        return Err("코드가 너무 깁니다 — 로그인 화면의 코드만 붙여넣으세요".into());
    }
    // 콘솔에 그대로 흘러가므로 줄바꿈·제어문자는 막는다
    if code.contains(['\r', '\n']) || code.chars().any(|c| c.is_control()) {
        return Err("코드 형식이 올바르지 않습니다".into());
    }
    {
        let guard = SESSION.lock().map_err(|_| "내부 잠금 오류")?;
        let session = guard.as_ref().ok_or("진행 중인 로그인이 없습니다")?;
        if session.provider != Provider::Claude {
            return Err("코드 입력은 클로드 로그인에서만 사용합니다".into());
        }
        let mut writer = session.writer.lock().map_err(|_| "내부 잠금 오류")?;
        writer
            .write_all(format!("{code}\r").as_bytes())
            .map_err(|e| format!("코드 전달 실패: {e}"))?;
        writer.flush().ok();
    }
    finish_and_import(env, FINISH_TIMEOUT)
}

/// 브라우저에서 코드 입력까지 끝나면 CLI가 스스로 완료한다 (코덱스 device-auth)
pub fn wait_device(env: &Env) -> Result<LoginOutcome, String> {
    finish_and_import(env, DEVICE_TIMEOUT)
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
        // PTY 자식은 세션 리더(setsid)다 — 그룹째 보내야 CLI 자손이 살아남지 않는다
        #[cfg(unix)]
        if let Some(pid) = session.child.process_id() {
            unsafe {
                libc::killpg(pid as libc::pid_t, libc::SIGKILL);
            }
        }
        let _ = session.child.kill();
        let _ = session.child.wait();
        cleanup_isolated(&session.config_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::test_support::{fake_jwt, test_env};

    #[test]
    fn strips_ansi_and_finds_url() {
        // 실제 클로드 로그인 화면에서 그대로 가져온 형태 (OSC 8 하이퍼링크 포함)
        let raw = b"\x1b[?25h\x1b]0;claude\x07Opening browser to sign in\xe2\x80\xa6\r\nIf the browser didn't open, visit: \x1b]8;id=1;https://claude.com/cai/oauth/authorize?code=true&state=abc\x1b\\https://claude.com/cai/oauth/authorize?code=true&state=abc\x1b]8;;\x1b\\\r\nPaste code here if prompted > ";
        let text = strip_ansi(raw);
        assert!(text.contains("Paste code here"));
        // 하이퍼링크 대상 우선 추출
        let url = pick_login_url(extract_osc8_urls(raw)).unwrap();
        assert_eq!(
            url,
            "https://claude.com/cai/oauth/authorize?code=true&state=abc"
        );
        // 폴백(가시 텍스트)도 같은 결과여야 한다
        assert_eq!(
            extract_visible_url(&text).unwrap(),
            "https://claude.com/cai/oauth/authorize?code=true&state=abc"
        );
    }

    #[test]
    fn osc8_survives_screen_wrapping() {
        // 화면에서는 커서 이동으로 URL이 잘려 보여도 하이퍼링크 대상은 완전하다
        let raw = b"see \x1b]8;;https://claude.com/cai/oauth/authorize?code=true&code_challenge=AAAABBBB&state=xyz\x1b\\https://claude.com/cai/oa\x1b[2;1Hthorize?code=tr\x1b]8;;\x1b\\ done";
        let url = pick_login_url(extract_osc8_urls(raw)).unwrap();
        assert!(url.ends_with("state=xyz"), "잘리면 안 된다: {url}");
    }

    #[test]
    fn login_url_is_preferred_over_banner_link() {
        let candidates = vec![
            "https://example.com/whats-new-in-cli".to_string(),
            "https://claude.com/cai/oauth/authorize?x=1&state=s".to_string(),
        ];
        assert!(pick_login_url(candidates).unwrap().contains("oauth"));
    }

    #[test]
    fn finds_codex_device_code() {
        let text = "Follow these steps\n\n1. Open this link\n   https://auth.openai.com/codex/device\n\n2. Enter this one-time code\n   V4GM-HT05H\n";
        assert_eq!(
            extract_visible_url(text).unwrap(),
            "https://auth.openai.com/codex/device"
        );
        assert_eq!(extract_device_code(text).unwrap(), "V4GM-HT05H");
    }

    #[test]
    fn device_code_rejects_separators_and_dates() {
        // 대시 구분선·날짜·소문자 토큰을 코드로 오인하면 안 된다
        assert!(extract_device_code("------------").is_none());
        assert!(extract_device_code("2026-07-28").is_none());
        assert!(extract_device_code("-V4GM-HT05").is_none());
        assert_eq!(extract_device_code("A1B2-C3D4").as_deref(), Some("A1B2-C3D4"));
    }

    #[test]
    fn rejects_bad_codes_before_touching_session() {
        let env = test_env("badcode");
        assert!(submit_code(&env, "abc\ndef").is_err());
        assert!(submit_code(&env, "   ").is_err());
        assert!(submit_code(&env, &"x".repeat(300)).is_err());
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
        let env = test_env("codex-import");
        let cfg = env.store.join("_login").join("t1");
        fs::create_dir_all(&cfg).unwrap();
        let jwt = fake_jwt(r#"{"email":"cx@test.dev","sub":"sub-1"}"#);
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
    fn import_never_steals_other_accounts_profile_name() {
        // 이메일 앞부분이 같은 제3의 계정이 로그인해도 기존 프로필을 덮어쓰지 않는다
        let env = test_env("no-steal");
        for (uuid, email, token) in [
            ("uuid-1", "alice@a.dev", "t1"),
            ("uuid-2", "alice@b.dev", "t2"),
            ("uuid-3", "alice@c.dev", "t3"),
        ] {
            let cfg = env.store.join("_login").join(format!("t-{uuid}"));
            fs::create_dir_all(&cfg).unwrap();
            fs::write(
                cfg.join(".credentials.json"),
                format!(r#"{{"claudeAiOauth":{{"accessToken":"{token}"}}}}"#),
            )
            .unwrap();
            fs::write(
                cfg.join(".claude.json"),
                format!(
                    r#"{{"oauthAccount":{{"accountUuid":"{uuid}","emailAddress":"{email}"}}}}"#
                ),
            )
            .unwrap();
            import(&env, Provider::Claude, &cfg).unwrap();
        }
        let snap = crate::accounts::list(&env, Provider::Claude).unwrap();
        assert_eq!(snap.profiles.len(), 3, "계정 3개 = 프로필 3개");
        // 첫 계정의 토큰이 그대로 살아 있어야 한다
        let first = fs::read_to_string(
            env.profiles_dir(Provider::Claude)
                .join("alice")
                .join("credentials.json"),
        )
        .unwrap();
        assert!(first.contains("t1"));
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

    #[test]
    fn codex_incomplete_login_hints_device_code_setting() {
        // 장치 코드 인증이 계정에서 꺼져 있으면 승인이 거부돼 auth.json이 안 생긴다 —
        // 그 경우 무엇을 켜야 하는지 알려줘야 한다
        let env = test_env("codex-hint");
        let cfg = env.store.join("_login").join("t1");
        fs::create_dir_all(&cfg).unwrap();
        match import(&env, Provider::Codex, &cfg) {
            Err(e) => assert!(e.contains("장치 코드 인증"), "안내가 없다: {e}"),
            Ok(_) => panic!("토큰이 없는데 성공으로 판정됐다"),
        }
    }

    #[test]
    fn missing_cli_fails_fast_with_install_hint() {
        cancel(); // 다른 테스트가 남긴 세션이 있으면 정리
        let env = test_env("missing-cli");
        let t0 = Instant::now();
        let err = start_impl(
            &env,
            Provider::Claude,
            ("switcher-no-such-cli-xyz", [].as_slice(), "CLAUDE_CONFIG_DIR"),
        )
        .unwrap_err();
        // 예전에는 PROMPT_TIMEOUT(60초)을 꽉 채운 뒤에야 실패했다
        assert!(
            t0.elapsed() < Duration::from_secs(20),
            "조기 종료를 감지하지 못하고 {}초를 기다렸다",
            t0.elapsed().as_secs()
        );
        assert!(err.contains("설치"), "설치 안내가 없다: {err}");
        // 토큰이 담길 수 있는 임시 로그인 폴더는 정리돼야 한다
        let leftover = fs::read_dir(env.store.join("_login"))
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(leftover, 0, "임시 로그인 폴더가 정리돼야 한다");
    }

    /// 격리 로그인 키체인 항목의 이름 규칙(sha256(경로)[:8] 접미사) 회귀 테스트.
    /// 규칙 자체는 실제 CLI가 만든 항목과 대조해 실측 확인했다 (2026-07-29, claude 2.1.220).
    /// 아래 기대값은 이 경로 문자열의 sha256을 독립 계산한 상수다.
    #[cfg(target_os = "macos")]
    #[test]
    fn isolated_service_name_follows_measured_rule() {
        let dir = Path::new("/Users/user/.switcher/_login/1700000000-1234-0");
        let services = isolated_keychain_services(dir);
        assert!(
            services.contains(&"Claude Code-credentials-a8d80090".to_string()),
            "규칙과 다른 이름: {services:?}"
        );
    }

    #[test]
    fn sweep_keeps_fresh_folders() {
        let env = test_env("sweep");
        let fresh = env.store.join("_login").join("fresh");
        fs::create_dir_all(&fresh).unwrap();
        sweep_stale(&env);
        assert!(fresh.exists(), "방금 만든 폴더(다른 인스턴스의 진행 중 로그인일 수 있음)는 남겨야 한다");
    }

    /// 실환경: 격리 로그인이 남긴 임시 폴더(맥은 키체인 항목 포함)를 실제 import
    /// 경로로 프로필에 들여온다. 성공하면 잔재(폴더·키체인 항목)는 제품 규칙대로 지워진다.
    /// 실행: SWITCHER_TEST_IMPORT_DIR=<격리 폴더> cargo test -- --ignored real_import --nocapture
    #[test]
    #[ignore]
    fn real_import_isolated_login_result() {
        let dir = std::env::var("SWITCHER_TEST_IMPORT_DIR")
            .expect("SWITCHER_TEST_IMPORT_DIR에 격리 로그인 임시 폴더 경로를 지정하세요");
        let env = Env::real().unwrap();
        let outcome = import(&env, Provider::Claude, Path::new(&dir)).unwrap();
        println!(
            "임포트 완료: 프로필 '{}' ({:?}), 기존 갱신 = {}",
            outcome.profile, outcome.email, outcome.updated_existing
        );
        assert!(!outcome.profile.is_empty());
    }

    /// 실환경: 실제로 로그인 주소가 나오는지, 그리고 활성 계정이 안 바뀌는지 확인한다.
    /// `cargo test -- --ignored real_start_login --nocapture --test-threads=1`
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
        assert!(
            prompt.url.contains("state=") && prompt.url.contains("code_challenge="),
            "주소가 잘렸다: {}",
            prompt.url
        );
        assert!(prompt.needs_code, "클로드는 코드 입력이 필요하다");

        cancel();
        assert_eq!(before, fs::read(&live_cred).unwrap(), "활성 토큰이 변경됐다");
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

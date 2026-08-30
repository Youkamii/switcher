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
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::accounts::{
    auto_name, claude_apply_oauth_block, deletion_identity_key, deletion_snapshot,
    ensure_name_not_owned_by_other, find_profile_by_id, identity_from_value, live_identity, now,
    profile_deleted_after, read_json, read_meta, refresh_key, write_live_cred, write_profile_parts,
    Env, LiveIdentity, Provider, MUTATION_LOCK,
};

/// 로그인 링크가 화면에 뜰 때까지 기다리는 시간
const PROMPT_TIMEOUT: Duration = Duration::from_secs(60);
/// 코드 입력 후 로그인이 끝날 때까지 기다리는 시간
const FINISH_TIMEOUT: Duration = Duration::from_secs(45);
/// 코덱스처럼 브라우저에서 코드를 넣고 CLI가 알아서 끝내는 방식의 대기 시간
const DEVICE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const POLL: Duration = Duration::from_millis(300);
/// 자식 프로세스가 종료한 뒤 남은 출력이 버퍼에 도착하기를 기다리는 유예
const EXIT_FLUSH: Duration = Duration::from_millis(700);
const TERMINATE_POLL: Duration = Duration::from_millis(50);
#[cfg(windows)]
const TASKKILL_WAIT_ATTEMPTS: usize = 61;
const CHILD_EXIT_WAIT_ATTEMPTS: usize = 41;
/// 화면 누적 버퍼 상한 (TUI 스피너가 세션 내내 쌓이므로 캡을 둔다)
const OUTPUT_CAP: usize = 256 * 1024;
/// 코드 입력 최대 길이 (콘솔 stdin으로 흘러가므로 과대 입력을 막는다)
const CODE_MAX_LEN: usize = 256;
/// 이보다 오래된 임시 로그인 폴더만 청소한다 — 다른 인스턴스의 진행 중 로그인을 지우지 않기 위함
/// (DEVICE_TIMEOUT보다 길게 잡아, 살아 있는 세션의 폴더일 가능성을 배제)
const SWEEP_MIN_AGE: Duration = Duration::from_secs(20 * 60);
/// 폴더 삭제가 중간에 실패해도 다음 시작에서 정확한 경로를 즉시 복원하도록 루트에 남긴다.
const CLEANUP_MARKER_PREFIX: &str = ".cleanup-pending-";

#[derive(Serialize, Debug)]
pub struct LoginPrompt {
    /// 뒤늦게 도착한 이전 패널의 요청이 새 세션을 건드리지 못하게 하는 ID.
    pub session_id: String,
    /// 사용자가 원하는 브라우저에 붙여넣을 로그인 주소
    pub url: String,
    /// 코덱스처럼 웹페이지에 입력해야 하는 일회용 코드 (없으면 None)
    pub device_code: Option<String>,
    /// true면 브라우저에서 받은 코드를 위젯에 붙여넣어야 한다 (클로드)
    pub needs_code: bool,
}

#[derive(Serialize, Debug)]
pub struct LoginOutcome {
    pub profile: String,
    pub email: Option<String>,
    /// 이미 저장돼 있던 계정을 다시 로그인한 경우 (새 계정이 아님)
    pub updated_existing: bool,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct CancelOutcome {
    /// true면 예약된 사전 취소를 기록했거나, 실제 프로세스 종료를 확인했다.
    pub cancelled: bool,
    /// 프로세스 종료 뒤 격리 로그인 흔적 정리만 실패한 경우의 재시도 가능한 경고.
    pub cleanup_error: Option<String>,
}

struct Session {
    generation: u64,
    request_id: String,
    delete_epoch: u64,
    provider: Provider,
    config_dir: PathBuf,
    child: Box<dyn Child + Send + Sync>,
    /// PTY 입력 통로. 읽기 스레드(터미널 질의 응답)와 코드 입력이 함께 쓴다.
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// PTY를 살려둬야 자식 프로세스가 끊기지 않는다
    _master: Box<dyn MasterPty + Send>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);
const MAX_LIVE_START_REQUESTS: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
enum StartRequestState {
    Reserved,
    Starting,
    Cancelled,
}

#[derive(Clone, Copy)]
struct StartRequest {
    kind: &'static str,
    state: StartRequestState,
}

/// 프런트 예약부터 worker의 세션 등록까지 살아 있는 시작 요청만 보관한다.
/// 완료 ID나 임의 취소 ID는 남기지 않아 오래 켜 둬도 상한이 고갈되지 않는다.
pub(crate) struct StartRequestRegistry {
    entries: Mutex<HashMap<String, StartRequest>>,
    max_live: usize,
    shutdown_blocks: AtomicUsize,
}

pub(crate) struct StartRequestLease {
    registry: &'static StartRequestRegistry,
    request_id: String,
    kind: &'static str,
    active: bool,
}

impl StartRequestRegistry {
    pub(crate) fn new(max_live: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            max_live,
            shutdown_blocks: AtomicUsize::new(0),
        }
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<String, StartRequest>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn reserve(
        &'static self,
        request_id: &str,
        kind: &'static str,
        label: &str,
    ) -> Result<(), String> {
        let mut entries = self.entries();
        if self.shutdown_blocks.load(Ordering::SeqCst) > 0 {
            return Err(format!("종료 준비 중에는 {label}을 시작할 수 없습니다"));
        }
        if entries.contains_key(request_id) {
            return Err(format!("{label} 시작 요청이 이미 예약됐습니다"));
        }
        if entries.len() >= self.max_live {
            return Err(format!("대기 중인 {label} 시작 요청이 너무 많습니다"));
        }
        entries.insert(
            request_id.to_string(),
            StartRequest {
                kind,
                state: StartRequestState::Reserved,
            },
        );
        Ok(())
    }

    pub(crate) fn claim(
        &'static self,
        request_id: &str,
        kind: &'static str,
        label: &str,
    ) -> Result<StartRequestLease, String> {
        let mut entries = self.entries();
        if self.shutdown_blocks.load(Ordering::SeqCst) > 0 {
            entries.remove(request_id);
            return Err(format!("종료 준비 중이라 {label}을 취소했습니다"));
        }
        let Some(entry) = entries.get(request_id).copied() else {
            return Err(format!("{label} 시작 요청이 예약되지 않았습니다"));
        };
        if entry.kind != kind {
            return Err(format!("{label} 시작 요청 종류가 일치하지 않습니다"));
        }
        match entry.state {
            StartRequestState::Reserved => {
                entries.get_mut(request_id).unwrap().state = StartRequestState::Starting;
                Ok(StartRequestLease {
                    registry: self,
                    request_id: request_id.to_string(),
                    kind,
                    active: true,
                })
            }
            StartRequestState::Cancelled => {
                entries.remove(request_id);
                Err(format!("{label}을 취소했습니다"))
            }
            StartRequestState::Starting => Err(format!("{label} 시작 요청이 이미 실행 중입니다")),
        }
    }

    pub(crate) fn cancel(&self, request_id: &str) -> bool {
        let mut entries = self.entries();
        let Some(entry) = entries.get_mut(request_id) else {
            return false;
        };
        entry.state = StartRequestState::Cancelled;
        true
    }

    pub(crate) fn release(&self, request_id: &str, kind: &'static str) -> Result<(), String> {
        let mut entries = self.entries();
        if let Some(entry) = entries.get(request_id) {
            if entry.kind != kind {
                return Err("로그인 시작 요청 종류가 일치하지 않습니다".to_string());
            }
            entries.remove(request_id);
        }
        Ok(())
    }

    pub(crate) fn block_for_shutdown(&self) {
        self.shutdown_blocks.fetch_add(1, Ordering::SeqCst);
        let mut entries = self.entries();
        for entry in entries.values_mut() {
            if entry.state == StartRequestState::Reserved {
                entry.state = StartRequestState::Cancelled;
            }
        }
    }

    pub(crate) fn unblock_after_failed_shutdown(&self) {
        let _ = self
            .shutdown_blocks
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                count.checked_sub(1)
            });
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries().len()
    }

    #[cfg(test)]
    pub(crate) fn shutdown_block_count(&self) -> usize {
        self.shutdown_blocks.load(Ordering::SeqCst)
    }
}

impl StartRequestLease {
    pub(crate) fn release(&mut self) {
        if self.active {
            let _ = self.registry.release(&self.request_id, self.kind);
            self.active = false;
        }
    }
}

impl Drop for StartRequestLease {
    fn drop(&mut self) {
        self.release();
    }
}

static START_REQUESTS: LazyLock<StartRequestRegistry> =
    LazyLock::new(|| StartRequestRegistry::new(MAX_LIVE_START_REQUESTS));
/// CLI 종료 뒤 자격증명을 반영하는 마지막 단계와 취소를 한 순서로 만든다.
/// 먼저 잠근 쪽만 세션을 가져가므로 "취소 성공 뒤 가져오기"가 생기지 않는다.
static LOGIN_COMPLETION_LOCK: Mutex<()> = Mutex::new(());

#[derive(Default)]
struct OutputBuffer {
    bytes: Vec<u8>,
    /// 버퍼 상한 때문에 앞에서 버린 누적 바이트 수. 제출 표식은 이 절대 위치를 쓴다.
    dropped: u64,
}

impl OutputBuffer {
    fn reset(&mut self) {
        self.bytes.clear();
        self.dropped = 0;
    }

    fn append(&mut self, piece: &[u8]) {
        self.bytes.extend_from_slice(piece);
        if self.bytes.len() > OUTPUT_CAP {
            let cut = self.bytes.len() - OUTPUT_CAP;
            self.bytes.drain(..cut);
            self.dropped = self.dropped.saturating_add(cut as u64);
        }
    }

    fn snapshot(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    fn position(&self) -> u64 {
        self.dropped.saturating_add(self.bytes.len() as u64)
    }

    fn since(&self, mark: u64) -> Vec<u8> {
        let offset = mark
            .saturating_sub(self.dropped)
            .min(self.bytes.len() as u64) as usize;
        self.bytes[offset..].to_vec()
    }
}

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

/// 코덱스가 보여주는 일회용 코드(예: ABCD-EFGH)를 찾는다.
/// 대시 구분선("----")이나 날짜("2026-07-28")를 오인하지 않도록
/// 대문자가 있고 양끝이 영숫자인 것만 인정한다.
pub(crate) fn extract_device_code(text: &str) -> Option<String> {
    for line in text.lines() {
        let token = line.trim();
        let Some((left, right)) = token.split_once('-') else {
            continue;
        };
        // Codex가 실제로 출력한 코드는 4-4와 4-5 그룹이 모두 있었다.
        // 첫 그룹은 4자로 고정해 ERROR-CODE 같은 일반 문구를 오인하지 않는다.
        let ok = left.len() == 4
            && (4..=5).contains(&right.len())
            && left
                .chars()
                .chain(right.chars())
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            && token.chars().any(|c| c.is_ascii_uppercase());
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
/// 로그인 셸에는 허용한 상수 이름("claude"·"codex"·"gh")만 넘긴다. 다른 값은
/// 셸을 거치지 않고 그대로 반환해 테스트·향후 호출자가 명령 문자열을 주입하지 못한다.
#[cfg(not(windows))]
pub(crate) fn resolve_program(program: &str) -> String {
    // 로그인 셸에는 고정된 CLI 이름만 넘긴다. 테스트용·향후 호출자가 다른 값을
    // 주면 셸 해석 없이 그대로 실행을 시도해 명령 문자열 주입 여지를 만들지 않는다.
    if !matches!(program, "claude" | "codex" | "gh") {
        return program.to_string();
    }
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

fn cleanup_marker_path(config_dir: &Path) -> Result<PathBuf, String> {
    let parent = config_dir
        .parent()
        .ok_or_else(|| format!("정리 대상의 상위 폴더가 없습니다: {}", config_dir.display()))?;
    let name = config_dir
        .file_name()
        .ok_or_else(|| format!("정리 대상 폴더명이 없습니다: {}", config_dir.display()))?;
    Ok(parent.join(format!("{CLEANUP_MARKER_PREFIX}{}", name.to_string_lossy())))
}

fn pending_cleanup_target(marker: &Path) -> Option<PathBuf> {
    let name = marker.file_name()?.to_str()?;
    let target = name.strip_prefix(CLEANUP_MARKER_PREFIX)?;
    let mut components = Path::new(target).components();
    let std::path::Component::Normal(target) = components.next()? else {
        return None;
    };
    if components.next().is_some() {
        return None;
    }
    Some(marker.parent()?.join(target))
}

fn delete_isolated_keychain(config_dir: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let account = crate::accounts::keychain::username();
        for service in isolated_keychain_services(config_dir) {
            crate::accounts::keychain::delete_item(&service, &account)?;
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = config_dir;
    Ok(())
}

fn remove_cleanup_marker(marker: &Path) -> Result<(), String> {
    match fs::remove_file(marker) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "정리 재시도 표식 삭제 실패 ({}): {error}",
            marker.display()
        )),
    }
}

/// 정리 경로를 먼저 보존한 뒤 키체인, 폴더 순서로 지운다.
/// 어느 단계든 실패하면 marker가 남아 다음 시작의 sweep가 즉시 다시 시도한다.
fn cleanup_isolated_with<K, R>(
    config_dir: &Path,
    delete_keychain: K,
    remove_dir: R,
) -> Result<(), String>
where
    K: FnOnce(&Path) -> Result<(), String>,
    R: FnOnce(&Path) -> Result<(), String>,
{
    let marker = cleanup_marker_path(config_dir)?;
    let parent = marker
        .parent()
        .ok_or_else(|| format!("정리 표식의 상위 폴더가 없습니다: {}", marker.display()))?;
    let mut parent_builder = fs::DirBuilder::new();
    parent_builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        parent_builder.mode(0o700);
    }
    parent_builder
        .create(parent)
        .map_err(|e| format!("정리 표식 폴더 생성 실패 ({}): {e}", parent.display()))?;
    fs::write(&marker, b"")
        .map_err(|e| format!("정리 재시도 표식 생성 실패 ({}): {e}", marker.display()))?;
    delete_keychain(config_dir)?;
    remove_dir(config_dir)?;
    remove_cleanup_marker(&marker)
}

/// 임시 로그인 폴더와 그 로그인이 남긴 키체인 항목(맥)을 지운다.
/// 키체인을 폴더보다 먼저 지운다 — 폴더가 사라지면 항목 이름(경로 해시)을 복구할 수 없다.
fn cleanup_isolated(config_dir: &Path) -> Result<(), String> {
    cleanup_isolated_with(config_dir, delete_isolated_keychain, remove_dir_retry)
}

fn combine_cleanup_result<T>(
    result: Result<T, String>,
    cleanup: Result<(), String>,
) -> Result<T, String> {
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) => Err(format!(
            "계정 정보는 저장됐지만 격리 로그인 정리에 실패했습니다: {error}"
        )),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup_error)) => Err(format!(
            "{error}; 격리 로그인 정리에도 실패했습니다: {cleanup_error}"
        )),
    }
}

fn with_isolated_cleanup<T>(result: Result<T, String>, config_dir: &Path) -> Result<T, String> {
    combine_cleanup_result(result, cleanup_isolated(config_dir))
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
fn remove_dir_retry_with<F>(
    dir: &Path,
    attempts: usize,
    base_delay: Duration,
    mut remove: F,
) -> Result<(), String>
where
    F: FnMut(&Path) -> std::io::Result<()>,
{
    let mut last_error = None;
    for attempt in 0..attempts {
        match remove(dir) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => last_error = Some(error),
        }
        if attempt + 1 < attempts && !base_delay.is_zero() {
            std::thread::sleep(base_delay * (attempt as u32 + 1));
        }
    }
    let error = last_error
        .map(|error| error.to_string())
        .unwrap_or_else(|| "삭제를 시도하지 못했습니다".to_string());
    Err(format!(
        "임시 로그인 폴더 삭제 실패 ({}): {error}",
        dir.display()
    ))
}

fn remove_dir_retry(dir: &Path) -> Result<(), String> {
    remove_dir_retry_with(dir, 5, Duration::from_millis(150), |path| {
        fs::remove_dir_all(path)
    })
}

/// 중단·크래시가 남긴 임시 로그인 폴더를 청소한다.
/// 다른 인스턴스가 진행 중일 수 있으므로 충분히 오래된 것만 지운다.
fn sweep_stale_checked(env: &Env) -> Result<(), String> {
    let root = env.store.join("_login");
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("로그인 임시 폴더 확인 실패: {error}")),
    };
    let mut errors = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                errors.push(format!("로그인 임시 항목 확인 실패: {error}"));
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                errors.push(format!("로그인 임시 항목 종류 확인 실패: {error}"));
                continue;
            }
        };
        if file_type.is_file() {
            if let Some(target) = pending_cleanup_target(&path) {
                if let Err(error) = cleanup_isolated(&target) {
                    errors.push(error);
                }
                continue;
            }
        }
        let is_dir = file_type.is_dir();
        let old_enough = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|age| age > SWEEP_MIN_AGE)
            .unwrap_or(false);
        if is_dir && old_enough {
            if let Err(error) = cleanup_isolated(&path) {
                errors.push(error);
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("격리 로그인 잔재 정리 실패: {}", errors.join("; ")))
    }
}

pub fn sweep_stale(env: &Env) {
    let _ = sweep_stale_checked(env);
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
#[cfg(test)]
pub fn start(env: &Env, provider: Provider) -> Result<LoginPrompt, String> {
    start_impl(env, provider, cli_args(provider), String::new())
}

pub fn start_requested(
    env: &Env,
    provider: Provider,
    request_id: String,
) -> Result<LoginPrompt, String> {
    validate_request_id(&request_id)?;
    start_impl(env, provider, cli_args(provider), request_id)
}

pub fn reserve_start(provider: Provider, request_id: &str) -> Result<(), String> {
    validate_request_id(request_id)?;
    START_REQUESTS.reserve(request_id, provider.dir_name(), "로그인")
}

pub fn release_start(provider: Provider, request_id: &str) -> Result<(), String> {
    validate_request_id(request_id)?;
    START_REQUESTS.release(request_id, provider.dir_name())
}

pub fn block_starts_for_shutdown() {
    START_REQUESTS.block_for_shutdown();
}

pub fn unblock_starts_after_failed_shutdown() {
    START_REQUESTS.unblock_after_failed_shutdown();
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
        .ok_or_else(|| "잘못된 로그인 요청 ID입니다".to_string())
}

pub(crate) fn exact_session_id(
    active: Option<(&str, u64)>,
    request_id: &str,
) -> Option<String> {
    active
        .filter(|(active_request_id, _)| *active_request_id == request_id)
        .map(|(_, generation)| generation.to_string())
}

/// start가 오류를 돌려준 뒤에도 종료되지 않은 정확한 요청의 세션이 남았는지 확인한다.
/// 완료·취소와 같은 잠금 순서를 써서 이미 정리된 세션 ID를 돌려주지 않는다.
pub fn session_for_request(request_id: &str) -> Result<Option<String>, String> {
    validate_request_id(request_id)?;
    let _completion = LOGIN_COMPLETION_LOCK
        .lock()
        .map_err(|_| "내부 잠금 오류")?;
    let guard = SESSION.lock().map_err(|_| "내부 잠금 오류")?;
    Ok(exact_session_id(
        guard
            .as_ref()
            .map(|session| (session.request_id.as_str(), session.generation)),
        request_id,
    ))
}

/// start의 본체 — 실행 명령을 주입받는다.
/// 테스트가 존재하지 않는 명령을 넣어 조기 종료 경로(미설치 CLI)를 검증한다.
fn start_impl(
    env: &Env,
    provider: Provider,
    (program, args, env_key): (&str, &[&str], &str),
    request_id: String,
) -> Result<LoginPrompt, String> {
    let my_gen;
    // 이전 로그인의 마지막 가져오기가 끝난 뒤에만 다음 격리 세션을 등록한다.
    // 등록까지만 직렬화하고 프롬프트 대기 중에는 취소가 들어올 수 있게 즉시 놓는다.
    let completion = LOGIN_COMPLETION_LOCK
        .lock()
        .map_err(|_| "내부 잠금 오류")?;
    // CLI/PTY 준비가 길어지는 동안 일어난 삭제도 "로그인 시작 뒤 삭제"로 잡아야 한다.
    // 세션 등록 직전에 찍으면 그 사이 삭제가 tombstone보다 먼저가 되어 되살아난다.
    let delete_epoch = deletion_snapshot();
    // 세션 검사부터 등록까지 잠금을 쥔 채 진행한다 —
    // 연타로 두 로그인이 동시에 시작해 폴더·세션이 꼬이는 것을 막는다 (red-review 2라운드)
    {
        let mut guard = SESSION.lock().map_err(|_| "내부 잠금 오류")?;
        let mut start_request = if request_id.is_empty() {
            None
        } else {
            Some(START_REQUESTS.claim(
                &request_id,
                provider.dir_name(),
                "로그인",
            )?)
        };
        if guard.is_some() {
            return Err("이미 로그인이 진행 중입니다".into());
        }
        sweep_stale_checked(env)?;

        let config_dir = temp_config_dir(env);
        let mut config_builder = fs::DirBuilder::new();
        config_builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            config_builder.mode(0o700);
        }
        if let Err(error) = config_builder.create(&config_dir) {
            return with_isolated_cleanup(
                Err(format!(
                    "임시 폴더 생성 실패 {}: {error}",
                    config_dir.display()
                )),
                &config_dir,
            );
        }

        let pair = match native_pty_system().openpty(PtySize {
                rows: 40,
                // 긴 OAuth 주소가 줄바꿈으로 잘리지 않게 넉넉히
                cols: 500,
                pixel_width: 0,
                pixel_height: 0,
            }) {
            Ok(pair) => pair,
            Err(error) => {
                return with_isolated_cleanup(
                    Err(format!("가상 콘솔 생성 실패: {error}")),
                    &config_dir,
                )
            }
        };

        // 세션에 보존할 PTY 통로를 자식 실행 전에 모두 준비한다. 실행 뒤 준비에 실패하면
        // 아직 SESSION에 넣지 못한 자식과 격리 경로를 복구할 방법이 없어진다.
        let mut reader = match pair.master.try_clone_reader() {
            Ok(reader) => reader,
            Err(error) => {
                return with_isolated_cleanup(Err(format!("콘솔 읽기 실패: {error}")), &config_dir)
            }
        };
        let raw_writer = match pair.master.take_writer() {
            Ok(writer) => writer,
            Err(error) => {
                return with_isolated_cleanup(Err(format!("콘솔 쓰기 실패: {error}")), &config_dir)
            }
        };
        let writer = Arc::new(Mutex::new(raw_writer));
        let responder = writer.clone();

        let sink = output_buffer();
        // 세션 세대 표식 — 취소 직후 빠른 재시작 때 이전 세션의 reader 스레드가
        // 마지막 조각을 새 세션 버퍼에 흘려 넣는 경합 방지 (#18 견고성). 자식을
        // 실행하기 전에 준비해, 실패한 자식과 격리 경로가 SESSION 밖에 남지 않게 한다.
        my_gen = {
            // 버퍼 잠금 안에서 세대를 올리고 비운다 — 이전 reader가 잠금을 쥔 채
            // 붙이는 중이면 그 뒤에 비워지고, 이후 조각은 세대 불일치로 버려진다
            let mut acc = match sink.lock() {
                Ok(acc) => acc,
                Err(_) => {
                    return with_isolated_cleanup(Err("내부 잠금 오류".into()), &config_dir)
                }
            };
            let next = SESSION_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            acc.reset();
            next
        };

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

        let child = match pair.slave.spawn_command(cmd) {
            Ok(child) => child,
            Err(error) => {
                return with_isolated_cleanup(
                    Err(format!(
                        "{program} 실행에 실패했습니다: {error} — 설치·업데이트: {}",
                        install_cmd(provider)
                    )),
                    &config_dir,
                )
            }
        };
        drop(pair.slave);

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
                    // 세대가 바뀌었다 = 이 세션은 취소되고 새 세션이 시작됐다 —
                    // 새 세션 버퍼를 오염시키지 않고 스레드를 접는다
                    if SESSION_GEN.load(std::sync::atomic::Ordering::SeqCst) != my_gen {
                        break;
                    }
                    acc.append(piece);
                }
            }
        });

        *guard = Some(Session {
            generation: my_gen,
            request_id,
            delete_epoch,
            provider,
            config_dir,
            child,
            writer,
            _master: pair.master,
        });
        if let Some(request) = start_request.as_mut() {
            request.release();
        }
    }
    drop(completion);

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
            if session.generation != my_gen {
                return Err("로그인을 취소했습니다".into());
            }
            if exited_at.is_none() {
                if let Ok(Some(_)) = session.child.try_wait() {
                    exited_at = Some(Instant::now());
                }
            }
        }
        let raw = {
            let acc = output_buffer().lock().map_err(|_| "내부 잠금 오류")?;
            acc.snapshot()
        };
        // 하이퍼링크 대상(항상 완전한 주소)을 우선, 가시 텍스트는 폴백
        let url = pick_login_url(extract_osc8_urls(&raw))
            .or_else(|| extract_visible_url(&strip_ansi(&raw)));
        if let Some(url) = url {
            match provider {
                Provider::Claude => {
                    return Ok(LoginPrompt {
                        session_id: my_gen.to_string(),
                        url,
                        device_code: None,
                        needs_code: true,
                    });
                }
                Provider::Codex => {
                    // 코덱스는 일회용 코드까지 화면에 떠야 완성이다 — 둘 다 기다린다
                    if let Some(code) = extract_device_code(&strip_ansi(&raw)) {
                        return Ok(LoginPrompt {
                            session_id: my_gen.to_string(),
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
            return Err(cancel_generation_with_reason(
                my_gen,
                early_exit_error(provider),
            ));
        }
        if Instant::now() > deadline {
            return Err(cancel_generation_with_reason(
                my_gen,
                "로그인 주소를 받지 못했습니다 — 잠시 후 다시 시도하세요".into(),
            ));
        }
        std::thread::sleep(POLL);
    }
}

/// 화면 누적 버퍼 (세션 하나만 존재하므로 전역 하나로 충분)
fn output_buffer() -> &'static Mutex<OutputBuffer> {
    static BUF: Mutex<OutputBuffer> = Mutex::new(OutputBuffer {
        bytes: Vec::new(),
        dropped: 0,
    });
    &BUF
}

/// 세션 세대 — start_impl이 올리고, reader 스레드가 자기 세대인지 확인한다
static SESSION_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// CLI가 코드 거부를 화면에 알렸는지 감지한다 (빠른 실패 표시, #18).
/// "Invalid code. Please make sure the full code was copied."는 실측 문구다
/// (2026-08-11, claude CLI — 거부 후에도 CLI는 종료하지 않고 재입력을 기다린다).
/// 나머지는 문구가 바뀔 때를 대비한 보수적 변형 — 로그인 화면의 다른 텍스트
/// (주소·state 파라미터 등)와 겹치지 않을 만큼 구체적인 구절만 쓴다.
pub(crate) fn detect_code_rejection(screen_since_submit: &str) -> bool {
    let lower = screen_since_submit.to_lowercase();
    ["invalid code", "code expired", "expired code", "authentication failed"]
        .iter()
        .any(|marker| lower.contains(marker))
}

/// 세션의 자식 프로세스가 끝날 때까지 기다린다 (취소 가능하도록 짧게 끊어 확인)
fn wait_for_exit(timeout: Duration, generation: u64) -> Result<(), String> {
    let started = Instant::now();
    loop {
        let probe = {
            let mut guard = SESSION.lock().map_err(|_| "내부 잠금 오류")?;
            let Some(session) = guard.as_mut() else {
                return Err("로그인을 취소했습니다".into());
            };
            if session.generation != generation {
                return Err("로그인을 취소했습니다".into());
            }
            session.child.try_wait()
        };
        match probe {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => {}
            Err(e) => {
                // 상태 확인이 안 되는 세션은 살려둬 봐야 '계정 추가'만 막는다 (#18) — 정리
                return Err(cancel_generation_with_reason(
                    generation,
                    format!("로그인 상태 확인 실패: {e}"),
                ));
            }
        }
        if started.elapsed() > timeout {
            return Err(cancel_generation_with_reason(
                generation,
                "시간이 초과됐습니다 — 처음부터 다시 시도하세요".into(),
            ));
        }
        std::thread::sleep(POLL);
    }
}

/// submit 전용 대기 — 종료(성공 경로) 또는 화면의 코드 거부 문구(빠른 실패)를 기다린다.
/// 거부를 감지하면 세션은 살려둔다: CLI가 재입력을 기다리므로(실측) 같은 패널에서
/// 코드를 다시 붙여넣을 수 있다.
enum SubmitWait {
    Exited,
    Rejected,
}

fn wait_for_exit_or_rejection(
    timeout: Duration,
    mark: u64,
    generation: u64,
) -> Result<SubmitWait, String> {
    let started = Instant::now();
    loop {
        let probe = {
            let mut guard = SESSION.lock().map_err(|_| "내부 잠금 오류")?;
            let Some(session) = guard.as_mut() else {
                return Err("로그인을 취소했습니다".into());
            };
            if session.generation != generation {
                return Err("로그인을 취소했습니다".into());
            }
            session.child.try_wait()
        };
        match probe {
            Ok(Some(_)) => return Ok(SubmitWait::Exited),
            Ok(None) => {}
            Err(e) => {
                return Err(cancel_generation_with_reason(
                    generation,
                    format!("로그인 상태 확인 실패: {e}"),
                ));
            }
        }
        let fresh = {
            let acc = output_buffer().lock().map_err(|_| "내부 잠금 오류")?;
            strip_ansi(&acc.since(mark))
        };
        if detect_code_rejection(&fresh) {
            return Ok(SubmitWait::Rejected);
        }
        if started.elapsed() > timeout {
            return Err(cancel_generation_with_reason(
                generation,
                "시간이 초과됐습니다 — 처음부터 다시 시도하세요".into(),
            ));
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
    // mut은 맥 전용(키체인 폴백의 재할당) — 다른 플랫폼에선 경고만 남아 잠재운다
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut cred: Option<Vec<u8>> = if cred_path.exists() {
        Some(fs::read(&cred_path).map_err(|e| format!("읽기 실패: {e}"))?)
    } else {
        None
    };
    // 맥 클로드는 격리 로그인 토큰이 파일이 아니라 키체인 항목으로 생긴다 (실측)
    #[cfg(target_os = "macos")]
    if cred.is_none() && provider == Provider::Claude {
        let account = crate::accounts::keychain::username();
        for service in isolated_keychain_services(config_dir) {
            if let Some(data) = crate::accounts::keychain::read_item(&service, &account)? {
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

/// 지정한 세션을 정리하고 (provider, 임시 폴더)를 돌려준다.
/// 취소 직후 새 로그인이 시작됐으면 이전 waiter가 새 세션을 가져가지 않는다.
fn finish_session(generation: u64) -> Option<(Provider, PathBuf, u64)> {
    let mut guard = SESSION.lock().ok()?;
    if guard.as_ref()?.generation != generation {
        return None;
    }
    let session = guard.take()?;
    Some((session.provider, session.config_dir, session.delete_epoch))
}

/// 임시 폴더의 로그인 결과를 프로필로 들여온다.
/// 어떤 경로로 끝나든 폴더와 키체인 잔재(맥)는 지운다.
#[cfg(test)]
fn import(env: &Env, provider: Provider, config_dir: &Path) -> Result<LoginOutcome, String> {
    import_started(env, provider, config_dir, deletion_snapshot())
}

fn import_started(
    env: &Env,
    provider: Provider,
    config_dir: &Path,
    delete_epoch: u64,
) -> Result<LoginOutcome, String> {
    let result = import_inner(env, provider, config_dir, delete_epoch);
    with_isolated_cleanup(result, config_dir)
}

fn import_inner(
    env: &Env,
    provider: Provider,
    config_dir: &Path,
    delete_epoch: u64,
) -> Result<LoginOutcome, String> {
    let (ident, cred, block) = read_login_result(provider, config_dir)?;

    // 프로필을 실제로 건드리는 구간에서만 잠근다
    let _guard = MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let existing = find_profile_by_id(env, provider, &ident.id)?;
    let updated_existing = existing.is_some();
    let name = existing.unwrap_or_else(|| auto_name(env, provider, &ident));
    let key = refresh_key(env, provider, &name);
    if profile_deleted_after(&key, delete_epoch)
        || profile_deleted_after(
            &deletion_identity_key(env, provider, &ident.id),
            delete_epoch,
        )
    {
        return Err(format!(
            "로그인 중 프로필 '{name}'이 삭제되어 결과를 다시 만들지 않았습니다 — 계정 추가를 다시 시도하세요"
        ));
    }
    // auto_name이 빈 이름을 보장하지만, 불변("다른 계정 토큰을 덮어쓰지 않는다")은 여기서도 지킨다
    ensure_name_not_owned_by_other(env, provider, &name, &ident)?;
    let updates_active = live_identity(env, provider)?
        .is_some_and(|live| live.id == ident.id);
    if updates_active {
        // 같은 활성 계정의 재로그인 결과를 프로필에만 쓰면 다음 전환의 활성 백업이
        // 폐기된 옛 토큰으로 새 토큰을 덮는다. 같은 계정일 때만 활성도 먼저 갱신한다.
        write_live_cred(env, provider, &cred)?;
    }
    write_profile_parts(env, provider, &name, &ident, &cred, block.as_ref())?;
    if updates_active && provider == Provider::Claude {
        claude_apply_oauth_block(env, &env.profiles_dir(provider).join(&name))?;
    }
    let hide_email =
        read_meta(&env.profiles_dir(provider).join(&name)).is_some_and(|meta| meta.hide_email);
    Ok(LoginOutcome {
        profile: name,
        email: if hide_email { None } else { ident.email },
        updated_existing,
    })
}

/// 로그인 종료를 기다렸다가 결과를 프로필로 들여온다 (submit/wait 공용 꼬리)
fn finish_and_import(
    env: &Env,
    timeout: Duration,
    generation: u64,
) -> Result<LoginOutcome, String> {
    wait_for_exit(timeout, generation)?;
    let _completion = LOGIN_COMPLETION_LOCK
        .lock()
        .map_err(|_| "내부 잠금 오류")?;
    let (provider, dir, delete_epoch) =
        finish_session(generation).ok_or("로그인 세션이 사라졌습니다")?;
    import_started(env, provider, &dir, delete_epoch)
}

/// 브라우저에서 받은 코드를 CLI에 전달해 로그인을 끝낸다 (클로드).
/// 잘못된 코드는 CLI 화면의 거부 문구를 감지해 몇 초 안에 알린다 — 세션은 살아
/// 있으므로 프론트가 같은 패널에서 재입력을 받는다 (45초 타임아웃 대기 제거, #18).
pub fn submit_code(env: &Env, code: &str, generation: u64) -> Result<LoginOutcome, String> {
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
    // 세션 세대와 제출 직전의 절대 화면 위치를 함께 붙잡는다. 이후 취소→새 로그인
    // 경합이 나도 이 waiter는 새 세션을 관찰하거나 취소하지 않는다.
    let (mark, write_result) = {
        let guard = SESSION.lock().map_err(|_| "내부 잠금 오류")?;
        let session = guard.as_ref().ok_or("진행 중인 로그인이 없습니다")?;
        if session.generation != generation {
            return Err("이전 로그인 패널의 요청이라 무시했습니다".into());
        }
        if session.provider != Provider::Claude {
            return Err("코드 입력은 클로드 로그인에서만 사용합니다".into());
        }
        let mark = output_buffer()
            .lock()
            .map_err(|_| "내부 잠금 오류")?
            .position();
        let mut writer = session.writer.lock().map_err(|_| "내부 잠금 오류")?;
        let result = writer.write_all(format!("{code}\r").as_bytes()).map(|_| {
            writer.flush().ok();
        });
        (mark, result)
    };
    if let Err(e) = write_result {
        // 콘솔 통로가 죽은 세션을 남겨두면 앱 재시작 전까지 '계정 추가'가
        // "이미 진행 중" 오류로 막힌다 (#18) — 즉시 정리한다.
        // cancel()이 SESSION 잠금을 다시 잡으므로 위 블록이 끝난 뒤에 부른다.
        return Err(cancel_generation_with_reason(
            generation,
            format!("코드 전달 실패: {e}. '계정 추가'로 다시 시도하세요"),
        ));
    }
    match wait_for_exit_or_rejection(FINISH_TIMEOUT, mark, generation)? {
        SubmitWait::Exited => {
            let _completion = LOGIN_COMPLETION_LOCK
                .lock()
                .map_err(|_| "내부 잠금 오류")?;
            let (provider, dir, delete_epoch) =
                finish_session(generation).ok_or("로그인 세션이 사라졌습니다")?;
            import_started(env, provider, &dir, delete_epoch)
        }
        // 프론트는 이 문구("코드가 거부")로 재시도 가능 상태를 판별한다 (main.ts)
        SubmitWait::Rejected => Err(
            "코드가 거부됐습니다 — 붙여넣은 코드를 다시 확인하세요 (Invalid code)".into(),
        ),
    }
}

/// 브라우저에서 코드 입력까지 끝나면 CLI가 스스로 완료한다 (코덱스 device-auth)
pub fn wait_device(env: &Env, generation: u64) -> Result<LoginOutcome, String> {
    finish_and_import(env, DEVICE_TIMEOUT, generation)
}

/// 진행 중인 로그인을 중단하고 임시 폴더를 지운다.
/// Windows에서는 cmd 셔임을 거치므로 트리째 종료해야 CLI가 살아남지 않는다.
fn wait_for_exit_with<P, S>(
    label: &str,
    attempts: usize,
    require_success: bool,
    mut probe: P,
    mut pause: S,
) -> Result<(), String>
where
    P: FnMut() -> Result<Option<bool>, String>,
    S: FnMut(Duration),
{
    let attempts = attempts.max(1);
    for attempt in 0..attempts {
        match probe()? {
            Some(true) => return Ok(()),
            Some(false) if require_success => {
                return Err(format!("{label}가 실패 상태로 끝났습니다"));
            }
            Some(false) => return Ok(()),
            None if attempt + 1 < attempts => pause(TERMINATE_POLL),
            None => {}
        }
    }
    Err(format!("{label} 종료를 제한 시간 안에 확인하지 못했습니다"))
}

#[cfg(any(windows, test))]
fn confirm_tree_kill_or_natural_exit_with<P>(
    tree_kill: Result<(), String>,
    mut child_exited: P,
) -> Result<bool, String>
where
    P: FnMut() -> Result<bool, String>,
{
    match tree_kill {
        Ok(()) => Ok(false),
        Err(tree_kill_error) => match child_exited() {
            Ok(true) => Ok(true),
            Ok(false) => Err(tree_kill_error),
            Err(probe_error) => Err(format!(
                "{tree_kill_error}; 로그인 프로세스 재확인 실패: {probe_error}"
            )),
        },
    }
}

#[cfg(windows)]
fn taskkill_path_from_root(system_root: &Path) -> Result<PathBuf, String> {
    if !system_root.is_absolute() {
        return Err("SystemRoot가 절대 경로가 아닙니다".into());
    }
    Ok(system_root.join("System32").join("taskkill.exe"))
}

#[cfg(windows)]
fn run_windows_taskkill(pid: u32) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let system_root = std::env::var_os("SystemRoot")
        .ok_or("Windows SystemRoot를 찾지 못해 로그인 프로세스를 종료할 수 없습니다")?;
    let taskkill_path = taskkill_path_from_root(Path::new(&system_root))?;
    let mut taskkill = std::process::Command::new(&taskkill_path)
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("{} 실행 실패: {error}", taskkill_path.display()))?;
    let result = wait_for_exit_with(
        "taskkill.exe",
        TASKKILL_WAIT_ATTEMPTS,
        true,
        || {
            taskkill
                .try_wait()
                .map(|status| status.map(|status| status.success()))
                .map_err(|error| format!("taskkill.exe 상태 확인 실패: {error}"))
        },
        std::thread::sleep,
    );
    if result.is_err() {
        let _ = taskkill.kill();
        let _ = taskkill.try_wait();
    }
    result
}

fn terminate_child(child: &mut (dyn Child + Send + Sync)) -> Result<(), String> {
    match child.try_wait() {
        Ok(Some(_)) => return Ok(()),
        Ok(None) => {}
        Err(error) => return Err(format!("로그인 프로세스 상태 확인 실패: {error}")),
    }

    #[cfg(windows)]
    {
        let pid = child
            .process_id()
            .ok_or("로그인 프로세스 ID를 확인하지 못해 트리를 종료할 수 없습니다")?;
        let already_exited = confirm_tree_kill_or_natural_exit_with(
            run_windows_taskkill(pid),
            || {
                child
                    .try_wait()
                    .map(|status| status.is_some())
                    .map_err(|error| error.to_string())
            },
        )?;
        if already_exited {
            return Ok(());
        }
        return wait_for_exit_with(
            "로그인 프로세스",
            CHILD_EXIT_WAIT_ATTEMPTS,
            false,
            || {
                child
                    .try_wait()
                    .map(|status| status.map(|_| true))
                    .map_err(|error| format!("로그인 프로세스 상태 확인 실패: {error}"))
            },
            std::thread::sleep,
        );
    }

    #[cfg(not(windows))]
    {
        // PTY 자식은 세션 리더(setsid)다 — 그룹째 보내야 CLI 자손이 살아남지 않는다
        #[cfg(unix)]
        if let Some(pid) = child.process_id() {
            unsafe {
                libc::killpg(pid as libc::pid_t, libc::SIGKILL);
            }
        }
        let kill_error = child.kill().err();
        let wait_result = wait_for_exit_with(
            "로그인 프로세스",
            CHILD_EXIT_WAIT_ATTEMPTS,
            false,
            || {
                child
                    .try_wait()
                    .map(|status| status.map(|_| true))
                    .map_err(|error| format!("로그인 프로세스 상태 확인 실패: {error}"))
            },
            std::thread::sleep,
        );
        match (kill_error, wait_result) {
            (_, Ok(())) => Ok(()),
            (Some(kill_error), Err(wait_error)) => Err(format!(
                "로그인 프로세스 종료 요청 실패: {kill_error}; {wait_error}"
            )),
            (None, Err(wait_error)) => Err(wait_error),
        }
    }
}

fn terminate_and_take_with<T, F>(slot: &mut Option<T>, terminate: F) -> Result<Option<T>, String>
where
    F: FnOnce(&mut T) -> Result<(), String>,
{
    let Some(value) = slot.as_mut() else {
        return Ok(None);
    };
    terminate(value)?;
    Ok(slot.take())
}

fn terminate_and_take_session(guard: &mut Option<Session>) -> Result<Option<Session>, String> {
    let session = terminate_and_take_with(guard, |session| {
        terminate_child(session.child.as_mut())
    })?;
    if session.is_some() {
        // 종료 확인과 SESSION 제거가 같은 잠금 구간이어야 실패한 세션을 재시도할 수 있고,
        // 이전 reader가 다음 세션의 출력 버퍼를 오염시키지 않는다.
        SESSION_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    Ok(session)
}

fn cleanup_terminated_session(session: Session) -> Result<(), String> {
    cleanup_isolated(&session.config_dir).map_err(|error| {
        format!("로그인 프로세스는 종료했지만 격리 로그인 정리에 실패했습니다: {error}")
    })
}

fn finish_cancel_with_cleanup<T, F>(session: Option<T>, cleanup: F) -> CancelOutcome
where
    F: FnOnce(T) -> Result<(), String>,
{
    match session {
        Some(session) => CancelOutcome {
            cancelled: true,
            cleanup_error: cleanup(session).err(),
        },
        None => CancelOutcome {
            cancelled: false,
            cleanup_error: None,
        },
    }
}

fn finish_cancelled_session(session: Option<Session>) -> CancelOutcome {
    finish_cancel_with_cleanup(session, cleanup_terminated_session)
}

fn cancel_generation(generation: u64) -> Result<CancelOutcome, String> {
    let session = {
        let mut guard = SESSION.lock().map_err(|_| "내부 잠금 오류")?;
        if guard
            .as_ref()
            .is_none_or(|session| session.generation != generation)
        {
            return Ok(CancelOutcome {
                cancelled: false,
                cleanup_error: None,
            });
        }
        terminate_and_take_session(&mut guard)?
    };
    Ok(finish_cancelled_session(session))
}

fn cancel_generation_with_reason(generation: u64, reason: String) -> String {
    match cancel_generation(generation) {
        Ok(CancelOutcome {
            cancelled: true,
            cleanup_error: None,
        }) => format!("{reason} — 로그인을 정리했습니다"),
        Ok(CancelOutcome {
            cleanup_error: Some(cleanup_error),
            ..
        }) => format!("{reason}; {cleanup_error}"),
        Ok(_) => reason,
        Err(cancel_error) => format!("{reason}; {cancel_error}"),
    }
}

pub fn cancel() -> Result<CancelOutcome, String> {
    let _completion = LOGIN_COMPLETION_LOCK
        .lock()
        .map_err(|_| "내부 잠금 오류")?;
    let session = {
        let mut guard = SESSION.lock().map_err(|_| "내부 잠금 오류")?;
        terminate_and_take_session(&mut guard)?
    };
    Ok(finish_cancelled_session(session))
}

/// 프롬프트가 오기 전 취소. 예약된 요청만 취소 상태로 바꾸며, 모르는 ID는
/// 새 상태를 만들지 않는다. worker는 CLI를 만들기 전에 이 상태를 소비한다.
pub fn cancel_start(request_id: &str) -> Result<CancelOutcome, String> {
    validate_request_id(request_id)?;
    let _completion = LOGIN_COMPLETION_LOCK
        .lock()
        .map_err(|_| "내부 잠금 오류")?;
    let session = {
        let mut guard = SESSION.lock().map_err(|_| "내부 잠금 오류")?;
        match guard.as_ref() {
            Some(active) if active.request_id == request_id => {
                terminate_and_take_session(&mut guard)?
            }
            Some(_) => return Err("이전 로그인 시작 취소 요청이라 무시했습니다".into()),
            None => {
                return Ok(CancelOutcome {
                    cancelled: START_REQUESTS.cancel(request_id),
                    cleanup_error: None,
                });
            }
        }
    };
    Ok(finish_cancelled_session(session))
}

/// 현재 세션을 정확한 세대값으로 취소한다.
/// `Ok(false)`는 같은 세션의 완료 처리가 먼저 끝나 이미 정리됐다는 뜻이다.
pub fn cancel_session(generation: u64) -> Result<CancelOutcome, String> {
    let _completion = LOGIN_COMPLETION_LOCK
        .lock()
        .map_err(|_| "내부 잠금 오류")?;
    let current = SESSION
        .lock()
        .map_err(|_| "내부 잠금 오류")?
        .as_ref()
        .map(|session| session.generation);
    match current {
        Some(active) if active == generation => cancel_generation(generation),
        Some(_) => Err("이전 로그인 패널의 취소 요청이라 무시했습니다".into()),
        None => Ok(CancelOutcome {
            cancelled: false,
            cleanup_error: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::test_support::{fake_jwt, test_env};

    static START_REQUEST_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[cfg(windows)]
    const TASKKILL_TREE_ROLE: &str = "SWITCHER_TASKKILL_TREE_ROLE";
    #[cfg(windows)]
    const TASKKILL_TREE_PID_FILE: &str = "SWITCHER_TASKKILL_TREE_PID_FILE";

    #[cfg(windows)]
    struct TestProcessHandle(windows_sys::Win32::Foundation::HANDLE);

    #[cfg(windows)]
    impl TestProcessHandle {
        fn open(pid: u32) -> std::io::Result<Self> {
            use windows_sys::Win32::System::Threading::{
                OpenProcess, PROCESS_TERMINATE, SYNCHRONIZATION_SYNCHRONIZE,
            };

            let handle =
                unsafe { OpenProcess(PROCESS_TERMINATE | SYNCHRONIZATION_SYNCHRONIZE, 0, pid) };
            if handle.is_null() {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(Self(handle))
            }
        }

        fn wait(&self, timeout_ms: u32) -> u32 {
            unsafe {
                windows_sys::Win32::System::Threading::WaitForSingleObject(self.0, timeout_ms)
            }
        }

        fn terminate(&self) {
            unsafe {
                windows_sys::Win32::System::Threading::TerminateProcess(self.0, 1);
            }
        }
    }

    #[cfg(windows)]
    impl Drop for TestProcessHandle {
        fn drop(&mut self) {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }

    #[cfg(windows)]
    struct TestProcessTree {
        parent: std::process::Child,
        child: Option<TestProcessHandle>,
        pid_file: PathBuf,
    }

    #[cfg(windows)]
    impl Drop for TestProcessTree {
        fn drop(&mut self) {
            if matches!(self.parent.try_wait(), Ok(None)) {
                let _ = run_windows_taskkill(self.parent.id());
            }
            if matches!(self.parent.try_wait(), Ok(None)) {
                let _ = self.parent.kill();
            }
            let _ = self.parent.wait();

            if let Some(child) = &self.child {
                use windows_sys::Win32::Foundation::WAIT_TIMEOUT;

                if child.wait(0) == WAIT_TIMEOUT {
                    child.terminate();
                    let _ = child.wait(5_000);
                }
            }
            let _ = fs::remove_file(&self.pid_file);
        }
    }

    #[cfg(windows)]
    fn spawn_hidden_test_fixture(
        test_name: &str,
        role: &str,
        pid_file: &Path,
    ) -> std::io::Result<std::process::Child> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        std::process::Command::new(std::env::current_exe()?)
            .args(["--exact", test_name, "--nocapture"])
            .env(TASKKILL_TREE_ROLE, role)
            .env(TASKKILL_TREE_PID_FILE, pid_file)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    }

    #[cfg(windows)]
    fn wait_for_fixture_child_pid(
        parent: &mut std::process::Child,
        pid_file: &Path,
    ) -> Result<u32, String> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match fs::read_to_string(pid_file) {
                Ok(pid) => match pid.trim().parse::<u32>() {
                    Ok(pid) if pid > 0 => return Ok(pid),
                    _ => return Err(format!("fixture가 잘못된 자식 PID를 기록했습니다: {pid:?}")),
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("fixture 자식 PID 확인 실패: {error}")),
            }

            if let Some(status) = parent
                .try_wait()
                .map_err(|error| format!("fixture 부모 상태 확인 실패: {error}"))?
            {
                return Err(format!(
                    "fixture가 자식 PID를 기록하기 전에 종료했습니다: {status}"
                ));
            }
            if std::time::Instant::now() >= deadline {
                return Err("fixture 자식 PID 대기 시간이 초과됐습니다".into());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

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
    fn finds_codex_device_code_without_digits() {
        let text = "Enter this one-time code\nABCD-EFGH\n";
        assert_eq!(extract_device_code(text).as_deref(), Some("ABCD-EFGH"));
    }

    #[test]
    fn codex_device_timeout_matches_code_validity() {
        assert_eq!(DEVICE_TIMEOUT, Duration::from_secs(15 * 60));
        assert!(SWEEP_MIN_AGE > DEVICE_TIMEOUT);
    }

    #[test]
    fn cancel_after_completion_is_idempotent() {
        let _ = cancel();
        assert_eq!(
            cancel_session(u64::MAX).unwrap(),
            CancelOutcome {
                cancelled: false,
                cleanup_error: None,
            }
        );
    }

    #[test]
    fn device_code_rejects_separators_and_dates() {
        // 대시 구분선·날짜·소문자 토큰을 코드로 오인하면 안 된다
        assert!(extract_device_code("------------").is_none());
        assert!(extract_device_code("2026-07-28").is_none());
        assert!(extract_device_code("-V4GM-HT05").is_none());
        assert!(extract_device_code("A-------B").is_none());
        assert!(extract_device_code("ABCD--EFGH").is_none());
        assert!(extract_device_code("A-B-C-D-E").is_none());
        assert!(extract_device_code("ERROR-CODE").is_none());
        assert!(extract_device_code("HELLO-WORLD").is_none());
        assert_eq!(extract_device_code("A1B2-C3D4").as_deref(), Some("A1B2-C3D4"));
    }

    #[test]
    fn rejects_bad_codes_before_touching_session() {
        let env = test_env("badcode");
        assert!(submit_code(&env, "abc\ndef", 0).is_err());
        assert!(submit_code(&env, "   ", 0).is_err());
        assert!(submit_code(&env, &"x".repeat(300), 0).is_err());
    }

    #[test]
    fn detects_measured_rejection_but_not_login_screen() {
        // 실측 문구 (2026-08-11, claude CLI): 거부 시 이 한 줄이 화면에 남는다
        assert!(detect_code_rejection(
            "Invalid code. Please make sure the full code was copied."
        ));
        // 대소문자 무관 + 방어적 변형
        assert!(detect_code_rejection("ERROR: CODE EXPIRED, request a new one"));
        assert!(detect_code_rejection("Authentication failed. Try again."));
        // 로그인 화면 자체(안내문·주소·코드 프롬프트)에는 절대 반응하면 안 된다
        let screen = "Opening browser to sign in…\n\
            If the browser didn't open, visit: \
            https://claude.com/cai/oauth/authorize?code=true&state=abc123&code_challenge=Kx9\n\
            Paste code here if prompted > ";
        assert!(!detect_code_rejection(screen));
    }

    #[test]
    fn output_mark_survives_front_drain() {
        let mut buffer = OutputBuffer::default();
        buffer.append(&vec![b'x'; OUTPUT_CAP]);
        let mark = buffer.position();
        buffer.append(b"\r\nInvalid code. Please try again.");

        let fresh = strip_ansi(&buffer.since(mark));
        assert!(detect_code_rejection(&fresh));
        assert!(buffer.bytes.len() <= OUTPUT_CAP);
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
    fn relogin_keeps_hidden_email_out_of_outcome() {
        let env = test_env("hidden-email-relogin");
        let first = env.store.join("_login").join("first");
        fs::create_dir_all(&first).unwrap();
        fs::write(
            first.join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"fake-old-token"}}"#,
        )
        .unwrap();
        fs::write(
            first.join(".claude.json"),
            r#"{"oauthAccount":{"accountUuid":"uuid-private","emailAddress":"private@test.dev"}}"#,
        )
        .unwrap();
        let initial = import(&env, Provider::Claude, &first).unwrap();

        let profile_dir = env.profiles_dir(Provider::Claude).join(&initial.profile);
        let mut meta = read_meta(&profile_dir).unwrap();
        meta.hide_email = true;
        crate::accounts::atomic_write(
            &profile_dir.join("meta.json"),
            &serde_json::to_vec(&meta).unwrap(),
        )
        .unwrap();

        let second = env.store.join("_login").join("second");
        fs::create_dir_all(&second).unwrap();
        fs::write(
            second.join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"fake-new-token"}}"#,
        )
        .unwrap();
        fs::write(
            second.join(".claude.json"),
            r#"{"oauthAccount":{"accountUuid":"uuid-private","emailAddress":"private@test.dev"}}"#,
        )
        .unwrap();

        let outcome = import(&env, Provider::Claude, &second).unwrap();
        assert!(outcome.updated_existing);
        assert_eq!(outcome.profile, initial.profile);
        assert_eq!(outcome.email, None);
        assert!(read_meta(&profile_dir).unwrap().hide_email);
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
    fn login_started_before_delete_cannot_recreate_account_under_new_name() {
        let env = test_env("import-delete-tombstone");
        let existing = LiveIdentity {
            id: "uuid-deleted".into(),
            email: Some("old-name@test.dev".into()),
        };
        write_profile_parts(
            &env,
            Provider::Claude,
            "old-name",
            &existing,
            br#"{"claudeAiOauth":{"accessToken":"old-token"}}"#,
            Some(&serde_json::json!({
                "accountUuid": "uuid-deleted",
                "emailAddress": "old-name@test.dev"
            })),
        )
        .unwrap();
        let started_at = deletion_snapshot();
        crate::accounts::delete(&env, Provider::Claude, "old-name").unwrap();

        let cfg = env.store.join("_login").join("deleted-late-result");
        fs::create_dir_all(&cfg).unwrap();
        fs::write(
            cfg.join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"new-token"}}"#,
        )
        .unwrap();
        fs::write(
            cfg.join(".claude.json"),
            r#"{"oauthAccount":{"accountUuid":"uuid-deleted","emailAddress":"changed-name@test.dev"}}"#,
        )
        .unwrap();

        let error = import_started(&env, Provider::Claude, &cfg, started_at).unwrap_err();
        assert!(error.contains("삭제"));
        assert!(!cfg.exists(), "실패한 격리 로그인 폴더도 정리돼야 한다");
        assert!(crate::accounts::list(&env, Provider::Claude)
            .unwrap()
            .profiles
            .is_empty());
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
        let _ = cancel(); // 다른 테스트가 남긴 세션이 있으면 정리
        let env = test_env("missing-cli");
        let t0 = Instant::now();
        let err = start_impl(
            &env,
            Provider::Claude,
            ("switcher-no-such-cli-xyz", [].as_slice(), "CLAUDE_CONFIG_DIR"),
            String::new(),
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

    #[test]
    fn cancel_before_start_prevents_the_cli_from_launching() {
        let _requests = START_REQUEST_TEST_LOCK.lock().unwrap();
        let _ = cancel();
        let first = "00000000-0000-4000-8000-000000000001";
        let second = "00000000-0000-4000-8000-000000000002";
        reserve_start(Provider::Codex, first).unwrap();
        reserve_start(Provider::Codex, second).unwrap();
        assert!(cancel_start(first).unwrap().cancelled);
        assert!(cancel_start(second).unwrap().cancelled);
        let env = test_env("cancel-before-start");
        for request_id in [first, second] {
            let err = start_impl(
                &env,
                Provider::Codex,
                ("must-not-launch", [].as_slice(), "CODEX_HOME"),
                request_id.to_string(),
            )
            .unwrap_err();
            assert!(err.contains("취소"));
        }
        let leftover = fs::read_dir(env.store.join("_login"))
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(leftover, 0);
    }

    #[test]
    fn shutdown_block_cancels_reserved_starts_without_reviving_them() {
        let registry: &'static StartRequestRegistry =
            Box::leak(Box::new(StartRequestRegistry::new(4)));
        let cancelled = "00000000-0000-4000-8005-000000000001";
        let rejected = "00000000-0000-4000-8005-000000000002";
        let after_failure = "00000000-0000-4000-8005-000000000003";

        registry.reserve(cancelled, "codex", "로그인").unwrap();
        registry.block_for_shutdown();
        assert_eq!(registry.shutdown_block_count(), 1);
        assert!(registry.reserve(rejected, "codex", "로그인").is_err());

        registry.unblock_after_failed_shutdown();
        assert_eq!(registry.shutdown_block_count(), 0);
        assert!(registry
            .claim(cancelled, "codex", "로그인")
            .err()
            .unwrap()
            .contains("취소"));

        registry
            .reserve(after_failure, "codex", "로그인")
            .unwrap();
        drop(
            registry
                .claim(after_failure, "codex", "로그인")
                .unwrap(),
        );
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn overlapping_shutdown_blocks_must_all_fail_before_starts_resume() {
        let registry: &'static StartRequestRegistry =
            Box::leak(Box::new(StartRequestRegistry::new(4)));
        let first = "00000000-0000-4000-8006-000000000001";
        let second = "00000000-0000-4000-8006-000000000002";

        registry.block_for_shutdown();
        registry.block_for_shutdown();
        assert_eq!(registry.shutdown_block_count(), 2);

        registry.unblock_after_failed_shutdown();
        assert_eq!(registry.shutdown_block_count(), 1);
        assert!(registry.reserve(first, "claude", "로그인").is_err());

        registry.unblock_after_failed_shutdown();
        assert_eq!(registry.shutdown_block_count(), 0);
        registry.reserve(second, "claude", "로그인").unwrap();
        registry.release(second, "claude").unwrap();
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn shutdown_block_does_not_discard_an_already_claimed_worker() {
        let registry: &'static StartRequestRegistry =
            Box::leak(Box::new(StartRequestRegistry::new(4)));
        let request_id = "00000000-0000-4000-8007-000000000001";

        registry
            .reserve(request_id, "github", "GitHub 로그인")
            .unwrap();
        let lease = registry
            .claim(request_id, "github", "GitHub 로그인")
            .unwrap();
        registry.block_for_shutdown();

        assert_eq!(registry.len(), 1);
        drop(lease);
        assert_eq!(registry.len(), 0);
        registry.unblock_after_failed_shutdown();
    }

    #[test]
    fn failed_starts_and_late_cancels_do_not_grow_the_login_registry() {
        let _requests = START_REQUEST_TEST_LOCK.lock().unwrap();
        let baseline = START_REQUESTS.len();
        for index in 0..64 {
            let request_id = format!("00000000-0000-4000-8001-{index:012x}");
            reserve_start(Provider::Claude, &request_id).unwrap();
            let lease = START_REQUESTS
                .claim(&request_id, Provider::Claude.dir_name(), "로그인")
                .unwrap();
            drop(lease); // worker 시작 실패 또는 panic의 RAII 정리
            assert!(!cancel_start(&request_id).unwrap().cancelled);
        }
        assert_eq!(START_REQUESTS.len(), baseline);
    }

    #[test]
    fn login_start_requests_are_isolated_and_unknown_cancel_is_a_noop() {
        let _requests = START_REQUEST_TEST_LOCK.lock().unwrap();
        let baseline = START_REQUESTS.len();
        let first = "00000000-0000-4000-8002-000000000001";
        let second = "00000000-0000-4000-8002-000000000002";
        let unknown = "00000000-0000-4000-8002-000000000003";

        assert!(!cancel_start(unknown).unwrap().cancelled);
        assert_eq!(START_REQUESTS.len(), baseline);
        reserve_start(Provider::Claude, first).unwrap();
        reserve_start(Provider::Codex, second).unwrap();
        assert!(cancel_start(first).unwrap().cancelled);

        let second_lease = START_REQUESTS
            .claim(second, Provider::Codex.dir_name(), "로그인")
            .unwrap();
        drop(second_lease);
        assert!(START_REQUESTS
            .claim(first, Provider::Claude.dir_name(), "로그인")
            .err()
            .unwrap()
            .contains("취소"));
        assert_eq!(START_REQUESTS.len(), baseline);
    }

    #[test]
    fn login_start_request_cleanup_survives_panic_and_join_release() {
        let _requests = START_REQUEST_TEST_LOCK.lock().unwrap();
        let baseline = START_REQUESTS.len();
        let panics = "00000000-0000-4000-8003-000000000001";
        let never_joined = "00000000-0000-4000-8003-000000000002";

        reserve_start(Provider::Claude, panics).unwrap();
        let unwind = std::panic::catch_unwind(|| {
            let _lease = START_REQUESTS
                .claim(panics, Provider::Claude.dir_name(), "로그인")
                .unwrap();
            panic!("injected worker panic");
        });
        assert!(unwind.is_err());
        assert!(!cancel_start(panics).unwrap().cancelled);

        reserve_start(Provider::Codex, never_joined).unwrap();
        release_start(Provider::Codex, never_joined).unwrap();
        assert!(!cancel_start(never_joined).unwrap().cancelled);
        assert_eq!(START_REQUESTS.len(), baseline);
    }

    #[test]
    fn login_start_request_rejects_wrong_provider_and_unbounded_ids() {
        let _requests = START_REQUEST_TEST_LOCK.lock().unwrap();
        let request_id = "00000000-0000-4000-8004-000000000001";
        reserve_start(Provider::Claude, request_id).unwrap();
        assert!(START_REQUESTS
            .claim(request_id, Provider::Codex.dir_name(), "로그인")
            .err()
            .unwrap()
            .contains("종류"));
        release_start(Provider::Claude, request_id).unwrap();

        assert!(cancel_start("not-a-uuid").is_err());
        assert!(reserve_start(Provider::Claude, "not-a-uuid").is_err());
        assert!(session_for_request("not-a-uuid").is_err());
    }

    #[test]
    fn active_session_lookup_only_returns_the_exact_start_request() {
        let active = Some(("00000000-0000-4000-8000-000000000078", 78));

        assert_eq!(
            exact_session_id(active, "00000000-0000-4000-8000-000000000078").as_deref(),
            Some("78")
        );
        assert_eq!(
            exact_session_id(active, "00000000-0000-4000-8000-000000000079"),
            None
        );
        assert_eq!(
            exact_session_id(None, "00000000-0000-4000-8000-000000000078"),
            None
        );
    }

    #[test]
    fn cleanup_failure_is_a_warning_after_verified_cancellation() {
        let outcome = finish_cancel_with_cleanup(Some("isolated-login-path"), |_| {
            Err("folder locked".into())
        });

        assert!(outcome.cancelled);
        assert_eq!(outcome.cleanup_error.as_deref(), Some("folder locked"));
    }

    #[test]
    fn verified_normal_exit_releases_cancelled_session() {
        use std::cell::Cell;

        let probes = Cell::new(0);
        let pauses = Cell::new(0);
        let mut session = Some("isolated-login-path");
        let taken = terminate_and_take_with(&mut session, |_| {
            wait_for_exit_with(
                "로그인 프로세스",
                3,
                false,
                || {
                    let probe = probes.get() + 1;
                    probes.set(probe);
                    Ok((probe == 2).then_some(true))
                },
                |_| pauses.set(pauses.get() + 1),
            )
        })
        .unwrap();

        assert_eq!(taken, Some("isolated-login-path"));
        assert!(session.is_none());
        assert_eq!(probes.get(), 2);
        assert_eq!(pauses.get(), 1);
    }

    #[test]
    fn taskkill_failure_keeps_session_for_retry() {
        let mut session = Some("isolated-login-path");
        let error = terminate_and_take_with(&mut session, |_| {
            let taskkill = wait_for_exit_with(
                "taskkill.exe",
                1,
                true,
                || Ok(Some(false)),
                |_| {},
            );
            confirm_tree_kill_or_natural_exit_with(taskkill, || Ok(false)).map(|_| ())
        })
        .unwrap_err();

        assert!(error.contains("taskkill.exe"));
        assert!(error.contains("실패 상태"));
        assert_eq!(session, Some("isolated-login-path"));
    }

    #[test]
    fn taskkill_race_accepts_an_already_exited_child() {
        let already_exited = confirm_tree_kill_or_natural_exit_with(
            Err("taskkill.exe가 실패 상태로 끝났습니다".into()),
            || Ok(true),
        )
        .unwrap();

        assert!(already_exited);
    }

    #[test]
    fn child_exit_timeout_keeps_session_for_retry() {
        use std::cell::Cell;

        let probes = Cell::new(0);
        let pauses = Cell::new(0);
        let mut session = Some("isolated-login-path");
        let error = terminate_and_take_with(&mut session, |_| {
            wait_for_exit_with(
                "로그인 프로세스",
                3,
                false,
                || {
                    probes.set(probes.get() + 1);
                    Ok(None)
                },
                |_| pauses.set(pauses.get() + 1),
            )
        })
        .unwrap_err();

        assert!(error.contains("제한 시간"));
        assert_eq!(session, Some("isolated-login-path"));
        assert_eq!(probes.get(), 3);
        assert_eq!(pauses.get(), 2);
    }

    #[cfg(windows)]
    #[test]
    fn taskkill_path_uses_absolute_system32_binary() {
        let path = taskkill_path_from_root(Path::new(r"C:\Windows")).unwrap();
        assert!(path.is_absolute());
        assert_eq!(path, PathBuf::from(r"C:\Windows\System32\taskkill.exe"));
        assert!(taskkill_path_from_root(Path::new("Windows")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_taskkill_tree_child_fixture() {
        if std::env::var(TASKKILL_TREE_ROLE).as_deref() != Ok("child") {
            return;
        }
        std::thread::sleep(Duration::from_secs(120));
    }

    #[cfg(windows)]
    #[test]
    fn windows_taskkill_tree_parent_fixture() {
        if std::env::var(TASKKILL_TREE_ROLE).as_deref() != Ok("parent") {
            return;
        }

        let Some(pid_file) = std::env::var_os(TASKKILL_TREE_PID_FILE).map(PathBuf::from) else {
            return;
        };
        let mut child = spawn_hidden_test_fixture(
            "login::tests::windows_taskkill_tree_child_fixture",
            "child",
            &pid_file,
        )
        .expect("taskkill fixture 자식 실행 실패");
        if fs::write(&pid_file, child.id().to_string()).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
        let _ = child.wait();
    }

    #[cfg(windows)]
    #[test]
    fn absolute_taskkill_terminates_hidden_process_tree() {
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;

        let system_root = std::env::var_os("SystemRoot").expect("SystemRoot가 있어야 합니다");
        let taskkill = taskkill_path_from_root(Path::new(&system_root)).unwrap();
        assert!(taskkill.is_absolute());
        assert!(
            taskkill.is_file(),
            "taskkill.exe가 없습니다: {}",
            taskkill.display()
        );

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let pid_file = std::env::temp_dir().join(format!(
            "switcher-taskkill-tree-{}-{nonce}.pid",
            std::process::id()
        ));
        let _ = fs::remove_file(&pid_file);
        let parent = spawn_hidden_test_fixture(
            "login::tests::windows_taskkill_tree_parent_fixture",
            "parent",
            &pid_file,
        )
        .expect("taskkill fixture 부모 실행 실패");
        let mut tree = TestProcessTree {
            parent,
            child: None,
            pid_file,
        };

        let child_pid = wait_for_fixture_child_pid(&mut tree.parent, &tree.pid_file).unwrap();
        tree.child =
            Some(TestProcessHandle::open(child_pid).expect("fixture 자식 handle 열기 실패"));

        run_windows_taskkill(tree.parent.id()).expect("System32 taskkill /T /F 실행 실패");
        wait_for_exit_with(
            "taskkill fixture 부모",
            101,
            false,
            || {
                tree.parent
                    .try_wait()
                    .map(|status| status.map(|_| true))
                    .map_err(|error| error.to_string())
            },
            std::thread::sleep,
        )
        .unwrap();
        assert_eq!(
            tree.child.as_ref().unwrap().wait(5_000),
            WAIT_OBJECT_0,
            "taskkill /T가 fixture 자식을 종료하지 못했습니다"
        );
    }

    #[test]
    fn cleanup_keeps_retry_path_when_keychain_step_fails() {
        use std::cell::Cell;

        let env = test_env("cleanup-keychain-failure");
        let dir = env.store.join("_login").join("pending");
        fs::create_dir_all(&dir).unwrap();
        let remove_called = Cell::new(false);
        let error = cleanup_isolated_with(
            &dir,
            |_| Err("keychain locked".into()),
            |_| {
                remove_called.set(true);
                Ok(())
            },
        )
        .unwrap_err();

        assert!(error.contains("keychain locked"));
        assert!(!remove_called.get(), "키체인 실패 뒤 폴더를 지우면 안 된다");
        assert!(dir.exists(), "키체인 경로를 복원할 폴더가 남아야 한다");
        let marker = cleanup_marker_path(&dir).unwrap();
        assert!(
            marker.exists(),
            "다음 시작이 즉시 재시도할 표식이 남아야 한다"
        );

        fs::remove_dir_all(&dir).unwrap();
        fs::remove_file(marker).unwrap();
    }

    #[test]
    fn cleanup_still_attempts_keychain_when_session_folder_is_missing() {
        use std::cell::Cell;

        let env = test_env("cleanup-missing-folder");
        let dir = env.store.join("_login").join("already-removed");
        fs::create_dir_all(dir.parent().unwrap()).unwrap();
        let keychain_called = Cell::new(false);
        cleanup_isolated_with(
            &dir,
            |_| {
                keychain_called.set(true);
                Ok(())
            },
            |_| Ok(()),
        )
        .unwrap();

        assert!(keychain_called.get());
        assert!(!cleanup_marker_path(&dir).unwrap().exists());
    }

    #[test]
    fn cleanup_accepts_marker_removed_by_a_concurrent_sweep() {
        let env = test_env("cleanup-concurrent-sweep");
        let dir = env.store.join("_login").join("pending");
        fs::create_dir_all(&dir).unwrap();
        let marker = cleanup_marker_path(&dir).unwrap();

        cleanup_isolated_with(
            &dir,
            |_| Ok(()),
            |_| {
                // Reproduce the exact interleaving where another sweep finishes first.
                fs::remove_file(&marker).unwrap();
                Ok(())
            },
        )
        .unwrap();

        assert!(!marker.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cleanup_marker_removal_preserves_real_errors() {
        let env = test_env("cleanup-marker-error");
        let marker = env.store.join("_login").join("marker-directory");
        fs::create_dir_all(&marker).unwrap();

        let error = remove_cleanup_marker(&marker).unwrap_err();

        assert!(error.contains("정리 재시도 표식 삭제 실패"));
        assert!(error.contains(&marker.display().to_string()));
        fs::remove_dir_all(marker).unwrap();
    }

    #[test]
    fn pending_cleanup_marker_cannot_escape_its_root() {
        let root = Path::new("cleanup-root");
        assert_eq!(
            pending_cleanup_target(&root.join(".cleanup-pending-session")),
            Some(root.join("session"))
        );
        assert!(pending_cleanup_target(&root.join(".cleanup-pending-..")).is_none());
        assert!(pending_cleanup_target(&root.join(".cleanup-pending-.")).is_none());
        assert!(pending_cleanup_target(Path::new(".cleanup-pending-")).is_none());
    }

    #[test]
    fn remove_dir_retry_reports_the_last_failure() {
        use std::cell::Cell;

        let attempts = Cell::new(0);
        let target = Path::new("locked-login-folder");
        let error = remove_dir_retry_with(target, 3, Duration::ZERO, |_| {
            let attempt = attempts.get() + 1;
            attempts.set(attempt);
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("locked-{attempt}"),
            ))
        })
        .unwrap_err();

        assert_eq!(attempts.get(), 3);
        assert!(error.contains("locked-login-folder"));
        assert!(error.contains("locked-3"));
    }

    #[test]
    fn cleanup_failure_is_not_hidden_after_a_successful_import() {
        let result = combine_cleanup_result(Ok("saved"), Err("folder locked".into()));
        let error = result.unwrap_err();
        assert!(error.contains("계정 정보는 저장됐지만"));
        assert!(error.contains("folder locked"));

        let failed: Result<(), String> =
            combine_cleanup_result(Err("login failed".into()), Err("cleanup failed".into()));
        let error = failed.unwrap_err();
        assert!(error.contains("login failed"));
        assert!(error.contains("cleanup failed"));
    }

    #[cfg(not(windows))]
    #[test]
    fn program_resolution_never_sends_unknown_input_to_shell() {
        let value = "unknown; touch /tmp/should-not-run";
        assert_eq!(resolve_program(value), value);
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
        sweep_stale_checked(&env).unwrap();
        assert!(fresh.exists(), "방금 만든 폴더(다른 인스턴스의 진행 중 로그인일 수 있음)는 남겨야 한다");
    }

    #[test]
    fn sweep_retries_fresh_pending_cleanup_immediately() {
        let env = test_env("sweep-pending");
        let pending = env.store.join("_login").join("fresh-pending");
        fs::create_dir_all(&pending).unwrap();
        fs::write(pending.join("auth.json"), b"fixture-only").unwrap();
        let marker = cleanup_marker_path(&pending).unwrap();
        fs::write(&marker, b"").unwrap();

        sweep_stale_checked(&env).unwrap();

        assert!(!pending.exists());
        assert!(!marker.exists());
    }

    #[test]
    fn sweep_does_not_treat_a_marker_shaped_directory_as_a_marker() {
        let env = test_env("sweep-marker-directory");
        let root = env.store.join("_login");
        let victim = root.join("victim");
        let marker_shaped_dir = root.join(".cleanup-pending-victim");
        fs::create_dir_all(&victim).unwrap();
        fs::create_dir_all(&marker_shaped_dir).unwrap();

        sweep_stale_checked(&env).unwrap();

        assert!(victim.exists());
        assert!(marker_shaped_dir.exists());
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
        let _ = cancel(); // 앞선 테스트가 남긴 세션 정리
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

        let _ = cancel();
        assert_eq!(before, fs::read(&live_cred).unwrap(), "활성 토큰이 변경됐다");
    }

    /// 진단 프로브: 잘못된 코드를 붙여넣었을 때 claude CLI가 그리는 화면을 실측한다.
    /// 오류 문구 감지(빠른 실패 표시)의 근거 데이터 수집용 — 계정에는 영향이 없다
    /// (격리 폴더 + 무효 코드라 토큰 교환이 거부될 뿐이다).
    /// `cargo test -- --ignored real_probe_bad_code --nocapture --test-threads=1`
    #[test]
    #[ignore]
    fn real_probe_bad_code_screen() {
        let _ = cancel();
        let env = Env::real().unwrap();
        let prompt = start(&env, Provider::Claude).unwrap();
        assert!(prompt.needs_code);
        let mark = output_buffer().lock().unwrap().position();
        {
            let guard = SESSION.lock().unwrap();
            let session = guard.as_ref().expect("세션이 있어야 한다");
            let mut writer = session.writer.lock().unwrap();
            writer.write_all(b"THIS-IS-A-BOGUS-CODE-1234\r").unwrap();
            writer.flush().ok();
        }
        // 20초 동안 1초마다 새 출력을 찍는다 — 오류 문구가 언제 어떤 형태로 오는지 관찰
        for second in 1..=20 {
            std::thread::sleep(Duration::from_secs(1));
            let raw = output_buffer().lock().unwrap().since(mark);
            let fresh = strip_ansi(&raw);
            let trimmed: String = fresh
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" | ");
            if !trimmed.is_empty() {
                println!("[{second:2}s] {trimmed}");
            }
            let exited = {
                let mut guard = SESSION.lock().unwrap();
                guard
                    .as_mut()
                    .map(|s| matches!(s.child.try_wait(), Ok(Some(_))))
                    .unwrap_or(true)
            };
            if exited {
                println!("[{second:2}s] (CLI 종료)");
                break;
            }
        }
        let _ = cancel();
    }

    /// 실환경: 잘못된 코드가 45초 타임아웃이 아니라 몇 초 안에 "코드가 거부"로
    /// 실패하는지 — submit_code 전체 경로(마크·화면 감지·세션 유지)를 검증한다.
    /// `cargo test -- --ignored real_bad_code_fast --nocapture --test-threads=1`
    #[test]
    #[ignore]
    fn real_bad_code_fast_fail_keeps_session() {
        let _ = cancel();
        let env = Env::real().unwrap();
        let prompt = start(&env, Provider::Claude).unwrap();
        assert!(prompt.needs_code);
        let t0 = Instant::now();
        let generation = prompt.session_id.parse().unwrap();
        let err = submit_code(&env, "THIS-IS-A-BOGUS-CODE-1234", generation).unwrap_err();
        let took = t0.elapsed();
        println!("거부까지 {took:?}: {err}");
        assert!(err.contains("코드가 거부"), "예상과 다른 에러: {err}");
        assert!(
            took < Duration::from_secs(30),
            "빠른 실패여야 한다 (45초 타임아웃 경로 아님): {took:?}"
        );
        // 세션이 살아 있어야 같은 패널에서 재입력이 된다
        assert!(
            SESSION.lock().unwrap().is_some(),
            "거부 후에도 세션은 유지되어야 한다"
        );
        let _ = cancel();
    }

    /// 코덱스 device-auth가 주소와 일회용 코드를 주는지 확인한다.
    #[test]
    #[ignore]
    fn real_start_login_codex_device_code() {
        let _ = cancel(); // 앞선 테스트가 남긴 세션 정리
        let env = Env::real().unwrap();
        let prompt = start(&env, Provider::Codex).unwrap();
        println!(
            "코덱스 주소: {} / 코드: {:?}",
            prompt.url, prompt.device_code
        );
        assert!(prompt.url.contains("openai.com"), "주소: {}", prompt.url);
        assert!(prompt.device_code.is_some(), "일회용 코드가 없다");
        assert!(!prompt.needs_code, "코덱스는 위젯 코드 입력이 필요 없다");
        let _ = cancel();
    }
}

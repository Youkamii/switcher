//! 계정 프로필 저장·전환 코어.
//!
//! 불변 규칙 (CLAUDE.md 금기 항목과 동일):
//! - 전환 순서: 활성 파일을 현재 계정 프로필에 백업한 뒤에만 대상 프로필을 복사한다.
//!   토큰이 수시로 자동 갱신되므로 순서를 바꾸면 최신 토큰이 유실된다.
//! - 어떤 경로에서도 토큰 내용을 로그·에러 메시지에 싣지 않는다.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub fn parse(s: &str) -> Result<Provider, String> {
        match s {
            "claude" => Ok(Provider::Claude),
            "codex" => Ok(Provider::Codex),
            _ => Err(format!("알 수 없는 provider: {s}")),
        }
    }

    pub(crate) fn dir_name(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
        }
    }

    /// 프로필 폴더 안에 저장되는 토큰 파일 이름 (원본 파일명과 동일하게 유지)
    pub(crate) fn credential_file_name(self) -> &'static str {
        match self {
            Provider::Claude => "credentials.json",
            Provider::Codex => "auth.json",
        }
    }
}

/// 활성 클로드 자격증명이 사는 곳.
/// 윈도우·테스트는 파일이고, macOS 실환경은 키체인이다 — 맥의 claude CLI는 토큰을
/// 키체인 항목 "Claude Code-credentials"에 보관하며 파일은 구버전 잔재다 (실측 2026-07-29).
pub enum ClaudeLiveStore {
    /// 윈도우·리눅스 실환경과 모든 플랫폼의 테스트가 쓴다 (맥 실환경은 Keychain)
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    File(PathBuf),
    #[cfg(target_os = "macos")]
    Keychain {
        service: String,
        account: String,
        /// 키체인 도입 전 구버전이 남긴 파일 — 존재하면 함께 갱신해 두 저장소의 어긋남을 막는다
        legacy_file: PathBuf,
    },
}

/// 홈·보관소 경로 묶음. 테스트에서는 임시 디렉토리를 주입한다.
pub struct Env {
    pub home: PathBuf,
    pub store: PathBuf,
    pub claude_live: ClaudeLiveStore,
}

impl Env {
    pub fn real() -> Result<Env, String> {
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .ok_or("홈 디렉토리를 찾을 수 없습니다")?;
        let store = home.join(".switcher");
        let claude_file = home.join(".claude").join(".credentials.json");
        #[cfg(target_os = "macos")]
        let claude_live = ClaudeLiveStore::Keychain {
            service: keychain::CLAUDE_LIVE_SERVICE.to_string(),
            account: keychain::username(),
            legacy_file: claude_file,
        };
        #[cfg(not(target_os = "macos"))]
        let claude_live = ClaudeLiveStore::File(claude_file);
        Ok(Env {
            home,
            store,
            claude_live,
        })
    }

    pub(crate) fn profiles_dir(&self, provider: Provider) -> PathBuf {
        self.store.join(provider.dir_name()).join("profiles")
    }

    pub(crate) fn live_credential_path(&self, provider: Provider) -> PathBuf {
        match provider {
            Provider::Claude => self.home.join(".claude").join(".credentials.json"),
            Provider::Codex => self.home.join(".codex").join("auth.json"),
        }
    }

    fn claude_json_path(&self) -> PathBuf {
        self.home.join(".claude.json")
    }
}

/// macOS 키체인 읽기·쓰기 — claude CLI와 같은 통로(/usr/bin/security)를 쓴다.
/// 같은 통로라야 항목 접근 ACL이 일치해 허용 팝업 없이 동작한다 (실측 2026-07-29).
/// 토큰이 프로세스 인자에 노출되지 않도록 쓰기는 `security -i`(stdin) + hex(-X)로 전달한다.
#[cfg(target_os = "macos")]
pub(crate) mod keychain {
    use std::io::Write;
    use std::process::{Command, Stdio};

    /// 맥 claude CLI의 활성 토큰 항목 (실측). 격리 로그인은 여기에 접미사가 붙는다.
    pub(crate) const CLAUDE_LIVE_SERVICE: &str = "Claude Code-credentials";

    pub(crate) fn username() -> String {
        std::env::var("USER")
            .ok()
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| {
                Command::new("/usr/bin/id")
                    .arg("-un")
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default()
            })
    }

    fn run_security(args: &[&str]) -> Result<std::process::Output, String> {
        Command::new("/usr/bin/security")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("security 실행 실패: {e}"))
    }

    /// 항목이 없으면 Ok(None) — 미로그인은 정상 경로다
    pub(crate) fn read_item(service: &str) -> Result<Option<Vec<u8>>, String> {
        let out = run_security(&["find-generic-password", "-s", service, "-w"])?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            if err.contains("could not be found") {
                return Ok(None);
            }
            return Err(format!("키체인 읽기 실패 ({service}): {}", err.trim()));
        }
        let mut data = out.stdout;
        // -w는 출력 끝에 개행 하나를 붙인다
        if data.last() == Some(&b'\n') {
            data.pop();
        }
        Ok(Some(data))
    }

    pub(crate) fn item_exists(service: &str) -> bool {
        // -w 없이 조회하면 비밀에 접근하지 않고 존재만 확인한다
        run_security(&["find-generic-password", "-s", service])
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    pub(crate) fn write_item(service: &str, account: &str, data: &[u8]) -> Result<(), String> {
        use std::fmt::Write as _;
        let mut hex = String::with_capacity(data.len() * 2);
        for b in data {
            let _ = write!(hex, "{b:02x}");
        }
        let mut child = Command::new("/usr/bin/security")
            .arg("-i")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("security 실행 실패: {e}"))?;
        let cmd =
            format!("add-generic-password -U -a \"{account}\" -s \"{service}\" -X \"{hex}\"\n");
        child
            .stdin
            .take()
            .ok_or("security stdin 없음")?
            .write_all(cmd.as_bytes())
            .map_err(|e| format!("키체인 쓰기 실패 ({service}): {e}"))?;
        let out = child
            .wait_with_output()
            .map_err(|e| format!("키체인 쓰기 실패 ({service}): {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "키체인 쓰기 실패 ({service}): {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(())
    }

    /// 로그인 잔재 청소용 — 항목이 없는 것은 성공이지만 실제 삭제 실패는 호출자에게 알린다.
    pub(crate) fn delete_item(service: &str) -> Result<(), String> {
        let out = run_security(&["delete-generic-password", "-s", service])?;
        if out.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("could not be found") {
            return Ok(());
        }
        Err(format!("키체인 삭제 실패 ({service}): {}", err.trim()))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        struct TestItemGuard(String);

        impl Drop for TestItemGuard {
            fn drop(&mut self) {
                let _ = delete_item(&self.0);
            }
        }

        /// 실제 로그인 키체인에 시험 항목을 왕복시킨다 — security 경유는 팝업 없이
        /// 동작해야 한다 (실측 확인된 전제가 깨지면 이 테스트가 알려준다)
        #[test]
        fn roundtrip_via_security_cli() {
            let svc = format!("switcher-selftest-{}", std::process::id());
            let _cleanup = TestItemGuard(svc.clone());
            let payload = br#"{"probe":"not-a-secret"}"#;
            write_item(&svc, &username(), payload).unwrap();
            assert!(item_exists(&svc));
            let read = read_item(&svc)
                .unwrap()
                .expect("방금 쓴 항목이 있어야 한다");
            assert_eq!(read, payload);
            // 같은 항목 갱신(-U)도 되어야 한다 (전환마다 일어나는 일)
            let payload2 = br#"{"probe":"updated"}"#;
            write_item(&svc, &username(), payload2).unwrap();
            assert_eq!(read_item(&svc).unwrap().unwrap(), payload2);
            delete_item(&svc).unwrap();
            assert!(!item_exists(&svc));
            assert!(read_item(&svc).unwrap().is_none(), "삭제 후에는 None");
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct Meta {
    pub id: String,
    pub email: Option<String>,
    pub saved_at: u64,
    /// 이메일은 자격증명 안에도 들어 있을 수 있으므로 보안상 제거되는 값은 아니다.
    /// Switcher 화면에서만 이메일 대신 프로필 이름을 보이게 하는 표시 설정이다.
    #[serde(default)]
    pub hide_email: bool,
}

#[derive(Serialize, Clone)]
pub struct LiveIdentity {
    pub id: String,
    pub email: Option<String>,
}

#[derive(Serialize)]
pub struct ProfileInfo {
    pub name: String,
    pub id: String,
    pub email: Option<String>,
    /// 구독 레벨 (Max·Pro·Plus 등) — 토큰 파일에서 추출한 표시용 정보
    pub plan: Option<String>,
    /// 클로드 Max의 배수 (rateLimitTier "..._max_20x" → 20). 코덱스는 None.
    pub plan_tier: Option<u32>,
    pub saved_at: u64,
    pub active: bool,
}

#[derive(Serialize)]
pub struct Snapshot {
    pub profiles: Vec<ProfileInfo>,
    pub live: Option<LiveIdentity>,
    /// 현재 로그인 계정이 어느 프로필에도 저장되어 있지 않으면 false
    pub live_saved: bool,
}

#[derive(Serialize, Debug)]
pub struct SwitchResult {
    pub backed_up_to: Option<String>,
    pub switched_to: String,
}

/// 저장·전환·삭제·임포트는 파일을 옮기는 다단계 작업이라 동시에 두 개가 돌면
/// 백업과 교체가 교차해 계정이 어긋날 수 있다 — 이 잠금으로 직렬화한다.
pub(crate) static MUTATION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// vault 가져오기가 최종 commit되기 전인 프로필 폴더의 표식. 이 표식이 있으면
/// 일반 목록·전환·갱신 경로에서 절대 사용하지 않는다.
pub(crate) const PROFILE_IMPORT_MARKER: &str = ".vault-import-id";

/// 프로필 단위 수명 잠금. 토큰 재발급은 공유 상태(refreshes), 전환·삭제는 배타 상태로
/// 등록한다. "재발급이 없음을 확인 → 전환/삭제 시작" 사이의 틈까지 같은 잠금 안에서
/// 닫아, POST 응답이 삭제된 폴더를 되살리거나 전환 직후 활성 토큰을 무효화하지 못한다.
#[derive(Default)]
struct ProfileOperation {
    refreshes: usize,
    exclusive: bool,
}

#[allow(clippy::type_complexity)]
fn profile_operations() -> &'static (
    std::sync::Mutex<std::collections::HashMap<String, ProfileOperation>>,
    std::sync::Condvar,
) {
    static CELL: std::sync::OnceLock<(
        std::sync::Mutex<std::collections::HashMap<String, ProfileOperation>>,
        std::sync::Condvar,
    )> = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        (
            std::sync::Mutex::new(std::collections::HashMap::new()),
            std::sync::Condvar::new(),
        )
    })
}

pub(crate) fn refresh_key(env: &Env, provider: Provider, name: &str) -> String {
    format!("{}:{}:{name}", env.store.display(), provider.dir_name())
}

pub(crate) fn deletion_identity_key(env: &Env, provider: Provider, id: &str) -> String {
    format!("{}:{}:<id:{id}>", env.store.display(), provider.dir_name())
}

/// 로그인 시작 뒤 같은 이름의 프로필이 삭제됐는지 판별하는 tombstone 세대.
/// 늦게 끝난 로그인 임포트가 사용자의 삭제를 되돌려 폴더를 되살리지 않게 한다.
static DELETE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn profile_deletions() -> &'static std::sync::Mutex<std::collections::HashMap<String, u64>> {
    static CELL: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, u64>>> =
        std::sync::OnceLock::new();
    CELL.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

pub(crate) fn deletion_snapshot() -> u64 {
    DELETE_SEQ.load(std::sync::atomic::Ordering::SeqCst)
}

pub(crate) fn profile_deleted_after(key: &str, snapshot: u64) -> bool {
    profile_deletions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(key)
        .is_some_and(|deleted_at| *deleted_at > snapshot)
}

fn mark_profile_deleted(key: String) {
    let deleted_at = DELETE_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    profile_deletions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(key, deleted_at);
}

/// 재발급 시작 표시 — 전환·삭제가 이미 시작됐으면 이번 갱신은 건너뛴다.
pub(crate) fn refresh_begin(key: String) -> Result<RefreshInflightGuard, String> {
    let mut states = profile_operations()
        .0
        .lock()
        .map_err(|_| "내부 잠금 오류".to_string())?;
    let state = states.entry(key.clone()).or_default();
    if state.exclusive {
        return Err("프로필이 변경 중입니다".into());
    }
    state.refreshes += 1;
    Ok(RefreshInflightGuard { key })
}

pub(crate) struct RefreshInflightGuard {
    key: String,
}

impl Drop for RefreshInflightGuard {
    fn drop(&mut self) {
        let (lock, cv) = profile_operations();
        if let Ok(mut states) = lock.lock() {
            let remove = if let Some(state) = states.get_mut(&self.key) {
                state.refreshes = state.refreshes.saturating_sub(1);
                state.refreshes == 0 && !state.exclusive
            } else {
                false
            };
            if remove {
                states.remove(&self.key);
            }
        }
        cv.notify_all();
    }
}

pub(crate) struct ProfileExclusiveGuard {
    key: String,
}

impl Drop for ProfileExclusiveGuard {
    fn drop(&mut self) {
        let (lock, cv) = profile_operations();
        if let Ok(mut states) = lock.lock() {
            let remove = if let Some(state) = states.get_mut(&self.key) {
                state.exclusive = false;
                state.refreshes == 0
            } else {
                false
            };
            if remove {
                states.remove(&self.key);
            }
        }
        cv.notify_all();
    }
}

/// 기존 재발급이 끝날 때까지 기다린 뒤 전환·삭제 권한을 원자적으로 차지한다.
/// MUTATION_LOCK보다 먼저 얻어야 재발급의 파일 반영과 교착하지 않는다.
pub(crate) fn profile_exclusive_begin(
    key: String,
    timeout: std::time::Duration,
) -> Result<ProfileExclusiveGuard, String> {
    let (lock, cv) = profile_operations();
    let deadline = std::time::Instant::now() + timeout;
    let mut states = lock.lock().map_err(|_| "내부 잠금 오류".to_string())?;
    loop {
        let busy = states
            .get(&key)
            .is_some_and(|state| state.refreshes > 0 || state.exclusive);
        if !busy {
            states.entry(key.clone()).or_default().exclusive = true;
            return Ok(ProfileExclusiveGuard { key });
        }
        let Some(remain) = deadline.checked_duration_since(std::time::Instant::now()) else {
            return Err("프로필 토큰 갱신이 끝나지 않아 작업을 중단했습니다".into());
        };
        match cv.wait_timeout(states, remain) {
            Ok((next, timed_out)) => {
                states = next;
                if timed_out.timed_out() {
                    return Err("프로필 토큰 갱신이 끝나지 않아 작업을 중단했습니다".into());
                }
            }
            Err(_) => return Err("내부 잠금 오류".into()),
        }
    }
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 프로필 이름은 경로에 들어가므로 엄격히 제한한다 (경로 탈출 방지).
pub(crate) fn validate_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err("프로필 이름은 영문·숫자·하이픈·언더스코어 1~32자만 가능합니다".into())
    }
}

/// 임시 파일에 쓴 뒤 rename — 읽는 쪽이 절대 반쪽짜리 파일을 보지 않게 한다.
pub(crate) fn atomic_write(path: &Path, data: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("경로 오류: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|e| format!("폴더 생성 실패 {}: {e}", parent.display()))?;
    atomic_replace_in_parent(path, data, parent)
}

/// 이미 존재하는 프로필 폴더 안에만 쓴다. 삭제와 경합했을 때 `create_dir_all`로
/// 지워진 계정 폴더를 토큰 사이드카 하나만 든 채 되살리지 않기 위한 변형이다.
pub(crate) fn atomic_write_existing_parent(path: &Path, data: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("경로 오류: {}", path.display()))?;
    if !parent.is_dir() {
        return Err(format!("대상 폴더가 없습니다: {}", parent.display()));
    }
    atomic_replace_in_parent(path, data, parent)
}

fn atomic_replace_in_parent(path: &Path, data: &[u8], parent: &Path) -> Result<(), String> {
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("경로 오류: {}", path.display()))?
        .to_string_lossy()
        .to_string();
    // 동시 쓰기 경합 시 임시 파일이 겹치지 않게 일련번호를 붙인다
    static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let tmp = parent.join(format!(
        ".{file_name}.switcher-tmp-{}-{nonce}-{seq}",
        std::process::id()
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&tmp)
        .map_err(|e| format!("임시 파일 생성 실패 {}: {e}", tmp.display()))?;
    if let Err(e) = file.write_all(data).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&tmp);
        return Err(format!("쓰기 실패 {}: {e}", tmp.display()));
    }
    drop(file);
    replace_file(&tmp, path).map_err(|e| {
        // 실패 시 평문 토큰이 담긴 임시 파일을 남기지 않는다
        let _ = fs::remove_file(&tmp);
        format!("교체 실패 {}: {e}", path.display())
    })
}

#[cfg(not(windows))]
fn replace_file(tmp: &Path, path: &Path) -> std::io::Result<()> {
    fs::rename(tmp, path)
}

#[cfg(windows)]
fn replace_file(tmp: &Path, path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "Kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new_name: *const u16, flags: u32) -> i32;
    }

    let existing: Vec<u16> = tmp.as_os_str().encode_wide().chain(Some(0)).collect();
    let new_name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
    let ok = unsafe {
        MoveFileExW(
            existing.as_ptr(),
            new_name.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// 자격증명 정규화 — claude CLI 2.1.223부터(실측 2026-08-07) 맥 키체인 값이
/// "JSON의 16진수 문자열"로 저장되는 경우가 있다. CLI는 자기 형식을 스스로
/// 디코드해 읽으므로 CLI는 멀쩡하지만, JSON을 기대하는 위젯의 사용량 조회·토큰
/// 갱신이 전부 깨진다. 전환(바이트 복사)이 그 형식을 프로필 파일에도 전파하므로
/// 읽기 관문에서 투명하게 디코드한다. 순수 hex이고 디코드 결과가 JSON일 때만
/// 디코드하며(정상 JSON은 '{'로 시작해 hex일 수 없다), 아니면 원본 그대로 둔다.
pub(crate) fn normalize_cred(data: Vec<u8>) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(&data) else {
        return data;
    };
    let trimmed = text.trim();
    if trimmed.len() < 2
        || trimmed.len() % 2 != 0
        || !trimmed.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return data;
    }
    let decoded: Vec<u8> = trimmed
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            let hi = (pair[0] as char).to_digit(16).unwrap_or(0) as u8;
            let lo = (pair[1] as char).to_digit(16).unwrap_or(0) as u8;
            (hi << 4) | lo
        })
        .collect();
    if serde_json::from_slice::<Value>(&decoded).is_ok() {
        decoded
    } else {
        data
    }
}

pub(crate) fn read_json(path: &Path) -> Result<Value, String> {
    let bytes = fs::read(path).map_err(|e| format!("읽기 실패 {}: {e}", path.display()))?;
    let bytes = normalize_cred(bytes);
    serde_json::from_slice(&bytes).map_err(|e| format!("JSON 파싱 실패 {}: {e}", path.display()))
}

#[cfg(test)]
mod normalize_tests {
    use super::*;

    #[test]
    fn hex_wrapped_json_is_decoded_and_others_untouched() {
        let json = br#"{"claudeAiOauth":{"accessToken":"sk-test"}}"#.to_vec();
        // 정상 JSON은 그대로
        assert_eq!(normalize_cred(json.clone()), json);
        // hex로 감싼 JSON은 디코드된다 (CLI 2.1.223 키체인 실측 형식)
        let hex: String = json.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(normalize_cred(hex.clone().into_bytes()), json);
        // 키체인 -w 출력처럼 개행이 붙어도 된다
        assert_eq!(normalize_cred(format!("{hex}\n").into_bytes()), json);
        // hex처럼 보여도 디코드 결과가 JSON이 아니면 원본 유지
        let not_json = b"deadbeef".to_vec();
        assert_eq!(normalize_cred(not_json.clone()), not_json);
    }
}

/// 다른 프로그램(실행 중인 CLI)이 쓰는 도중의 반쪽짜리 파일을 읽을 수 있으므로
/// 파싱 실패 시 짧게 기다렸다가 재시도한다.
fn read_json_retry(path: &Path) -> Result<Value, String> {
    let mut last_err = String::new();
    for attempt in 0..3 {
        match read_json(path) {
            Ok(value) => return Ok(value),
            Err(e) => {
                last_err = e;
                if attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_millis(80));
                }
            }
        }
    }
    Err(last_err)
}

/// 활성 자격증명 읽기 — 코덱스는 항상 파일, 클로드는 저장소(파일/키체인)에 따른다.
/// 키체인 모드에서 항목이 없으면 구버전 파일로 폴백한다 (키체인 미사용 환경 대응).
pub(crate) fn read_live_cred(env: &Env, provider: Provider) -> Result<Vec<u8>, String> {
    read_live_cred_raw(env, provider).map(normalize_cred)
}

fn read_live_cred_raw(env: &Env, provider: Provider) -> Result<Vec<u8>, String> {
    let read_file = |path: &Path| -> Result<Vec<u8>, String> {
        fs::read(path).map_err(|e| format!("읽기 실패 {}: {e}", path.display()))
    };
    match provider {
        Provider::Codex => read_file(&env.live_credential_path(Provider::Codex)),
        Provider::Claude => match &env.claude_live {
            ClaudeLiveStore::File(path) => read_file(path),
            #[cfg(target_os = "macos")]
            ClaudeLiveStore::Keychain {
                service,
                legacy_file,
                ..
            } => match keychain::read_item(service)? {
                Some(data) => Ok(data),
                None if legacy_file.exists() => read_file(legacy_file),
                None => Err(
                    "클로드 로그인 정보가 없습니다 (키체인에 항목 없음) — 먼저 claude에서 로그인하세요"
                        .into(),
                ),
            },
        },
    }
}

/// 활성 자격증명이 존재하는가 (전환·저장 가능 여부 판단)
pub(crate) fn live_cred_exists(env: &Env, provider: Provider) -> bool {
    match provider {
        Provider::Codex => env.live_credential_path(Provider::Codex).exists(),
        Provider::Claude => match &env.claude_live {
            ClaudeLiveStore::File(path) => path.exists(),
            #[cfg(target_os = "macos")]
            ClaudeLiveStore::Keychain {
                service,
                legacy_file,
                ..
            } => keychain::item_exists(service) || legacy_file.exists(),
        },
    }
}

/// 활성 자격증명 쓰기 (전환 2단계). 키체인 모드에서는 구버전 파일이 남아 있으면
/// 함께 갱신한다 — 낡은 파일이 진실처럼 보이는 혼선을 막는다.
pub(crate) fn write_live_cred(env: &Env, provider: Provider, data: &[u8]) -> Result<(), String> {
    match provider {
        Provider::Codex => atomic_write(&env.live_credential_path(Provider::Codex), data),
        Provider::Claude => match &env.claude_live {
            ClaudeLiveStore::File(path) => atomic_write(path, data),
            #[cfg(target_os = "macos")]
            ClaudeLiveStore::Keychain {
                service,
                account,
                legacy_file,
            } => {
                // 키체인(구형 CLI ~2.1.222용) + 파일(신형 CLI 2.1.223+용) 둘 다 쓴다.
                // 실측(2026-08-07): 2.1.223은 네이티브 키체인 API로 바뀌어 위젯이
                // security 도구로 쓴 항목을 ACL 불일치로 읽지 못한다("Not logged in").
                // 대신 ~/.claude/.credentials.json이 있으면 그걸 읽어 자기 소유로
                // 키체인에 이관하고 파일을 지운다 — 그래서 파일이 신형 CLI로 가는
                // 신뢰 가능한 전달 통로다. 구형 CLI는 키체인(raw JSON)을 그대로 읽는다.
                keychain::write_item(service, account, data)?;
                atomic_write(legacy_file, data)?;
                Ok(())
            }
        },
    }
}

/// JWT payload를 디코딩한다 (서명 검증 없음 — 표시용 신원·만료 확인 목적).
pub(crate) fn jwt_payload(token: &str) -> Option<Value> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn jwt_claim(token: &str, claim: &str) -> Option<String> {
    jwt_payload(token)?.get(claim)?.as_str().map(String::from)
}

/// 파싱된 JSON에서 계정 신원을 뽑는다.
/// 클로드는 ~/.claude.json 루트를, 코덱스는 auth.json 루트를 받는다.
/// 활성 파일 판독과 격리 로그인 결과 임포트가 같은 파서를 쓴다 (한쪽만 고치는 사고 방지).
pub(crate) fn identity_from_value(provider: Provider, root: &Value) -> Option<LiveIdentity> {
    match provider {
        Provider::Claude => {
            let acc = root.get("oauthAccount")?;
            let id = acc.get("accountUuid").and_then(|v| v.as_str())?.to_string();
            let email = acc
                .get("emailAddress")
                .and_then(|v| v.as_str())
                .map(String::from);
            Some(LiveIdentity { id, email })
        }
        Provider::Codex => {
            let tokens = root.get("tokens")?;
            let id_token = tokens.get("id_token").and_then(|v| v.as_str());
            let id = tokens
                .get("account_id")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| id_token.and_then(|t| jwt_claim(t, "sub")))?;
            let email = id_token.and_then(|t| jwt_claim(t, "email"));
            Some(LiveIdentity { id, email })
        }
    }
}

/// 토큰 파일에서 구독 레벨을 뽑는다 (표시용).
/// 클로드: claudeAiOauth.subscriptionType ("max"), 코덱스: JWT auth claim의 chatgpt_plan_type ("pro").
fn plan_from_credential(provider: Provider, root: &Value) -> Option<String> {
    let raw = match provider {
        Provider::Claude => root
            .pointer("/claudeAiOauth/subscriptionType")?
            .as_str()?
            .to_string(),
        Provider::Codex => {
            let token = root.pointer("/tokens/id_token")?.as_str()?;
            jwt_payload(token)?
                .get("https://api.openai.com/auth")?
                .get("chatgpt_plan_type")?
                .as_str()?
                .to_string()
        }
    };
    // 첫 글자만 대문자로 (max → Max)
    let mut chars = raw.chars();
    Some(match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => return None,
    })
}

/// 클로드 Max의 배수를 뽑는다 ("default_claude_max_20x" → 20)
fn tier_from_credential(provider: Provider, root: &Value) -> Option<u32> {
    if provider != Provider::Claude {
        return None;
    }
    let tier = root.pointer("/claudeAiOauth/rateLimitTier")?.as_str()?;
    tier.strip_suffix('x')?.rsplit('_').next()?.parse().ok()
}

/// 현재 로그인된 계정의 신원. 파일이 없거나 식별 불가면 Ok(None).
pub(crate) fn live_identity(env: &Env, provider: Provider) -> Result<Option<LiveIdentity>, String> {
    let path = match provider {
        Provider::Claude => env.claude_json_path(),
        Provider::Codex => env.live_credential_path(Provider::Codex),
    };
    if !path.exists() {
        return Ok(None);
    }
    let root = read_json_retry(&path)?;
    Ok(identity_from_value(provider, &root))
}

/// ~/.claude.json의 oauthAccount 블록 (프로필에 함께 보관해 전환 시 복원)
pub(crate) fn claude_oauth_block(env: &Env) -> Result<Option<Value>, String> {
    let path = env.claude_json_path();
    if !path.exists() {
        return Ok(None);
    }
    Ok(read_json_retry(&path)?.get("oauthAccount").cloned())
}

/// 토큰 내용과 계정 정보를 직접 받아 프로필로 저장한다.
/// 활성 파일에서 저장할 때도, 격리 로그인 결과를 임포트할 때도 이 경로를 쓴다.
pub(crate) fn write_profile_parts(
    env: &Env,
    provider: Provider,
    name: &str,
    ident: &LiveIdentity,
    cred: &[u8],
    oauth_block: Option<&Value>,
) -> Result<(), String> {
    let dir = env.profiles_dir(provider).join(name);
    if dir.join(PROFILE_IMPORT_MARKER).exists() {
        return Err("중단된 인증정보 가져오기를 복구한 뒤 다시 시도하세요".into());
    }
    // 토큰 갱신·전환·재로그인은 표시 설정을 바꾸는 작업이 아니다. 기존 메타를
    // 먼저 읽어 둬 vault 가져오기로 지정한 이메일 숨김이 계속 유지되게 한다.
    let hide_email = read_meta(&dir).is_some_and(|meta| meta.hide_email);
    // 기존 토큰을 덮어쓰기 전에 한 세대 .bak으로 남긴다 — 잘못된 덮어쓰기의 최후 안전망
    let cred_path = dir.join(provider.credential_file_name());
    if cred_path.exists() {
        let _ = fs::copy(&cred_path, cred_path.with_extension("json.bak"));
    }
    write_profile_bundle_to_dir(&dir, provider, ident, cred, oauth_block, hide_email)
}

/// 새 프로필 디렉터리에 들어가는 필수 파일 묶음의 단일 쓰기 관문.
/// 각 파일은 `atomic_write`를 거쳐 Unix 0600과 sync/rename 보장을 그대로 받는다.
pub(crate) fn write_profile_bundle_to_dir(
    dir: &Path,
    provider: Provider,
    ident: &LiveIdentity,
    cred: &[u8],
    oauth_block: Option<&Value>,
    hide_email: bool,
) -> Result<(), String> {
    let oauth_bytes = oauth_block
        .map(serde_json::to_vec_pretty)
        .transpose()
        .map_err(|e| e.to_string())?;
    write_profile_bundle_bytes_to_dir(
        dir,
        provider,
        ident,
        cred,
        oauth_bytes.as_deref(),
        hide_email,
    )
}

/// 이미 검증된 oauthAccount 바이트를 그대로 쓰는 관문. vault 가져오기는 복호화한
/// 값을 평문 임시 파일에 두지 않고 최종 marked 프로필에 바로 반영할 때 사용한다.
pub(crate) fn write_profile_bundle_bytes_to_dir(
    dir: &Path,
    provider: Provider,
    ident: &LiveIdentity,
    cred: &[u8],
    oauth_bytes: Option<&[u8]>,
    hide_email: bool,
) -> Result<(), String> {
    atomic_write(&dir.join(provider.credential_file_name()), cred)?;
    if let Some(bytes) = oauth_bytes {
        atomic_write(&dir.join("oauth_account.json"), bytes)?;
    }
    let meta = Meta {
        id: ident.id.clone(),
        email: ident.email.clone(),
        saved_at: now(),
        hide_email,
    };
    let bytes = serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?;
    atomic_write(&dir.join("meta.json"), &bytes)
}

fn write_new_private_file(path: &Path, data: &[u8]) -> Result<(), String> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("새 파일 생성 실패 {}: {e}", path.display()))?;
    file.write_all(data)
        .and_then(|_| file.sync_all())
        .map_err(|e| format!("새 파일 쓰기 실패 {}: {e}", path.display()))
}

/// vault import 전용. marker로 숨겨진 새 최종 폴더에 곧바로 쓰므로 토큰이 든
/// `.tmp` 파일을 만들지 않는다. 어느 파일이든 실패하면 journal 복구가 폴더째 지운다.
pub(crate) fn write_new_marked_profile_bundle_to_dir(
    dir: &Path,
    provider: Provider,
    ident: &LiveIdentity,
    cred: &[u8],
    oauth_bytes: Option<&[u8]>,
    hide_email: bool,
) -> Result<(), String> {
    if !dir.join(PROFILE_IMPORT_MARKER).is_file() {
        return Err("가져오기 안전 표식이 없는 프로필에는 쓸 수 없습니다".into());
    }
    write_new_private_file(&dir.join(provider.credential_file_name()), cred)?;
    if let Some(bytes) = oauth_bytes {
        write_new_private_file(&dir.join("oauth_account.json"), bytes)?;
    }
    let meta = Meta {
        id: ident.id.clone(),
        email: ident.email.clone(),
        saved_at: now(),
        hide_email,
    };
    let bytes = serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?;
    write_new_private_file(&dir.join("meta.json"), &bytes)
}

/// 현재 활성 파일들을 지정 이름의 프로필로 저장한다 (덮어쓰기 허용).
fn write_profile(
    env: &Env,
    provider: Provider,
    name: &str,
    ident: &LiveIdentity,
) -> Result<(), String> {
    let data = read_live_cred(env, provider)?;
    let block = if provider == Provider::Claude {
        claude_oauth_block(env)?
    } else {
        None
    };
    write_profile_parts(env, provider, name, ident, &data, block.as_ref())
}

/// name 프로필이 이미 다른 계정의 것이면 에러 — 다른 계정 토큰을 덮어쓰지 않는다
pub(crate) fn ensure_name_not_owned_by_other(
    env: &Env,
    provider: Provider,
    name: &str,
    ident: &LiveIdentity,
) -> Result<(), String> {
    let dir = env.profiles_dir(provider).join(name);
    if dir.join(PROFILE_IMPORT_MARKER).exists() {
        return Err("중단된 인증정보 가져오기를 복구한 뒤 다시 시도하세요".into());
    }
    if let Some(meta) = read_meta(&dir) {
        if meta.id != ident.id {
            if meta.hide_email {
                return Err(format!(
                    "'{name}'은 이미 다른 계정의 프로필입니다 — 다른 이름을 쓰세요"
                ));
            }
            let owner = meta.email.unwrap_or(meta.id);
            return Err(format!(
                "'{name}'은 이미 다른 계정({owner})의 프로필입니다 — 다른 이름을 쓰세요"
            ));
        }
    }
    Ok(())
}

pub(crate) fn read_meta(dir: &Path) -> Option<Meta> {
    let path = dir.join("meta.json");
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub(crate) fn profile_dirs(
    env: &Env,
    provider: Provider,
) -> Result<Vec<(String, PathBuf)>, String> {
    let root = env.profiles_dir(provider);
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let entries = fs::read_dir(&root).map_err(|e| format!("읽기 실패 {}: {e}", root.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && !path.join(PROFILE_IMPORT_MARKER).exists() {
            out.push((entry.file_name().to_string_lossy().to_string(), path));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

pub(crate) fn find_profile_by_id(
    env: &Env,
    provider: Provider,
    id: &str,
) -> Result<Option<String>, String> {
    for (name, dir) in profile_dirs(env, provider)? {
        if let Some(meta) = read_meta(&dir) {
            if meta.id == id {
                return Ok(Some(name));
            }
        }
    }
    Ok(None)
}

/// 이메일 앞부분(또는 계정 id 앞 8자)으로 자동 프로필 이름을 만든다.
/// id·이메일은 외부 입력(JWT 클레임 등)이므로 허용 문자만 남긴다 — 결과는 항상 validate_name을 통과한다.
/// 다른 계정이 쓰는 이름과 절대 겹치지 않을 때까지 숫자를 올린다 (한 단계 접미사로는
/// 제3 계정이 기존 프로필을 덮어쓰는 사고가 났었다 — red-review 2라운드).
pub(crate) fn auto_name(env: &Env, provider: Provider, ident: &LiveIdentity) -> String {
    let clean = |s: &str, limit: usize| -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(limit)
            .collect::<String>()
            .to_lowercase()
    };
    let base = {
        let from_email = clean(
            ident
                .email
                .as_deref()
                .and_then(|e| e.split('@').next())
                .unwrap_or(""),
            20,
        );
        if from_email.is_empty() {
            let id_part = clean(&ident.id, 8);
            if id_part.is_empty() {
                "account".to_string()
            } else {
                format!("account-{id_part}")
            }
        } else {
            from_email
        }
    };
    let mut candidate = base.clone();
    let mut n = 1;
    loop {
        match read_meta(&env.profiles_dir(provider).join(&candidate)) {
            None => return candidate,                              // 빈 이름
            Some(meta) if meta.id == ident.id => return candidate, // 이미 이 계정의 프로필
            Some(_) => {
                n += 1;
                if n > 99 {
                    return format!("account-{}", now());
                }
                candidate = format!("{base}-{n}");
            }
        }
    }
}

/// 대상 프로필의 oauth_account.json을 ~/.claude.json에 반영한다.
/// 실행 중인 클로드 세션과의 쓰기 경합 창을 줄이기 위해 반영 직전에 새로 읽는다.
pub(crate) fn claude_apply_oauth_block(env: &Env, profile_dir: &Path) -> Result<(), String> {
    let block_path = profile_dir.join("oauth_account.json");
    let cj = env.claude_json_path();
    if !block_path.exists() || !cj.exists() {
        return Ok(());
    }
    let block = read_json(&block_path)?;
    let mut root = read_json_retry(&cj)?;
    let Some(obj) = root.as_object_mut() else {
        return Ok(());
    };
    obj.insert("oauthAccount".to_string(), block);
    let bytes = serde_json::to_vec_pretty(&root).map_err(|e| e.to_string())?;
    atomic_write(&cj, &bytes)
}

/// 현재 로그인 계정을 이름 붙여 프로필로 저장.
/// 이름이 비어 있으면 auto_name으로 자동 작명한다 (#18 UX — 첫 저장 마찰 제거).
/// 실제 저장된 이름을 돌려준다 (자동 작명 결과를 프론트가 안내에 쓴다).
pub fn save_current(env: &Env, provider: Provider, name: &str) -> Result<String, String> {
    // 변이 함수가 스스로 잠근다 — 호출자가 잠금을 잊을 수 없게 (관례 단일화)
    let _guard = MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    if !live_cred_exists(env, provider) {
        return Err("로그인 정보가 없습니다 — 먼저 해당 CLI에서 로그인하세요".into());
    }
    let ident = live_identity(env, provider)?
        .ok_or("현재 로그인 계정을 식별할 수 없습니다 (로그인 직후 다시 시도)")?;
    let name = if name.trim().is_empty() {
        auto_name(env, provider, &ident) // 항상 validate_name을 통과하는 이름을 만든다
    } else {
        validate_name(name)?;
        name.to_string()
    };
    // 같은 계정이 이미 다른 이름으로 저장돼 있으면 중복 프로필을 막는다
    if let Some(existing) = find_profile_by_id(env, provider, &ident.id)? {
        if existing != name {
            return Err(format!(
                "이 계정은 이미 '{existing}' 프로필로 저장되어 있습니다"
            ));
        }
    }
    // 다른 계정이 쓰는 이름을 덮어써 그 계정 토큰을 파괴하는 것을 막는다
    ensure_name_not_owned_by_other(env, provider, &name, &ident)?;
    write_profile(env, provider, &name, &ident)?;
    Ok(name)
}

/// 계정 전환. 순서 불변: 1) 현재 활성 파일 백업 → 2) 대상 프로필 복사
pub fn switch(env: &Env, provider: Provider, name: &str) -> Result<SwitchResult, String> {
    validate_name(name)?;
    // 기존 재발급을 기다리는 동시에 새 재발급의 시작도 막는다. 반드시
    // MUTATION_LOCK 밖에서 먼저 얻어야 재발급 파일 반영과 교착하지 않는다.
    let _profile_guard = profile_exclusive_begin(
        refresh_key(env, provider, name),
        std::time::Duration::from_secs(20),
    )?;
    let _guard = MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let profile_dir = env.profiles_dir(provider).join(name);
    if profile_dir.join(PROFILE_IMPORT_MARKER).exists() {
        return Err("가져오기가 완료되지 않은 프로필이라 전환할 수 없습니다".into());
    }
    let target_cred = profile_dir.join(provider.credential_file_name());
    // 직전 갱신의 본 파일 쓰기/활성 복구가 실패해 pending이 남았으면, 구토큰을
    // 활성 위치로 복사하기 전에 반드시 복구한다.
    crate::usage::rescue_pending_profile_locked(env, provider, name)?;
    if !target_cred.exists() {
        return Err(format!("프로필 '{name}'에 저장된 토큰이 없습니다"));
    }
    // 계정 정보 없이 토큰만 있는 프로필(구조용 백업)로 전환하면 이후 신원 판정이
    // 어긋나 다음 전환의 백업이 엉뚱한 프로필을 덮어쓸 수 있다 — 전환 대상에서 제외
    if provider == Provider::Claude && !profile_dir.join("oauth_account.json").exists() {
        return Err(format!(
            "프로필 '{name}'에는 계정 정보가 없어 전환할 수 없습니다 (구조용 백업) — 해당 계정으로 CLI 로그인 후 다시 저장하세요"
        ));
    }

    // 1) 백업 — 현재 활성 계정을 자기 프로필(없으면 자동 생성)에 저장
    let mut backed_up_to = None;
    if live_cred_exists(env, provider) {
        match live_identity(env, provider)? {
            Some(live) => {
                let back_name = match find_profile_by_id(env, provider, &live.id)? {
                    Some(existing) => existing,
                    None => auto_name(env, provider, &live),
                };
                write_profile(env, provider, &back_name, &live)?;
                backed_up_to = Some(back_name);
            }
            None => {
                // 신원 불명이어도 토큰을 버리지 않는다 — 구조용 프로필로 보관
                let rescue = format!("rescue-{}", now());
                let ident = LiveIdentity {
                    id: format!("unknown-{}", now()),
                    email: None,
                };
                write_profile(env, provider, &rescue, &ident)?;
                backed_up_to = Some(rescue);
            }
        }
    }

    // 2) 대상 프로필을 활성 위치로 복사
    let data =
        fs::read(&target_cred).map_err(|e| format!("읽기 실패 {}: {e}", target_cred.display()))?;
    write_live_cred(env, provider, &data)?;
    if provider == Provider::Claude {
        claude_apply_oauth_block(env, &profile_dir)?;
    }

    Ok(SwitchResult {
        backed_up_to,
        switched_to: name.to_string(),
    })
}

/// 프로필 목록 + 현재 로그인 계정 상태
pub fn list(env: &Env, provider: Provider) -> Result<Snapshot, String> {
    // 표시용 목록은 신원 읽기가 일시적으로 실패해도 화면 전체를 깨뜨리지 않는다
    // (전환·저장 경로는 여전히 엄격하게 실패한다)
    let mut live = live_identity(env, provider).unwrap_or(None);
    let live_id = live.as_ref().map(|l| l.id.clone());
    let mut profiles = Vec::new();
    for (name, dir) in profile_dirs(env, provider)? {
        if let Some(meta) = read_meta(&dir) {
            let cred = read_json(&dir.join(provider.credential_file_name())).ok();
            let plan = cred
                .as_ref()
                .and_then(|root| plan_from_credential(provider, root));
            let plan_tier = cred
                .as_ref()
                .and_then(|root| tier_from_credential(provider, root));
            let active = live_id.as_deref() == Some(meta.id.as_str());
            if active && meta.hide_email {
                if let Some(live) = live.as_mut() {
                    live.email = None;
                }
            }
            profiles.push(ProfileInfo {
                active,
                name,
                id: meta.id,
                email: if meta.hide_email { None } else { meta.email },
                plan,
                plan_tier,
                saved_at: meta.saved_at,
            });
        }
    }
    let live_saved = profiles.iter().any(|p| p.active);
    Ok(Snapshot {
        profiles,
        live,
        live_saved,
    })
}

/// 프로필 삭제 (보관함에서만 지운다 — 활성 로그인은 건드리지 않음)
pub fn delete(env: &Env, provider: Provider, name: &str) -> Result<(), String> {
    validate_name(name)?;
    let _profile_guard = profile_exclusive_begin(
        refresh_key(env, provider, name),
        std::time::Duration::from_secs(20),
    )?;
    let _guard = MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let dir = env.profiles_dir(provider).join(name);
    if !dir.exists() {
        return Err(format!("프로필 '{name}'이 없습니다"));
    }
    // 삭제 전에 계정 id를 붙잡아 둔다 — 사용량 캐시 정리(잔존 항목 무기한 축적 방지)용
    let meta_id = read_meta(&dir).map(|m| m.id);
    fs::remove_dir_all(&dir).map_err(|e| format!("삭제 실패 {}: {e}", dir.display()))?;
    mark_profile_deleted(refresh_key(env, provider, name));
    if let Some(id) = meta_id.as_deref() {
        // 로그인 도중 삭제된 계정이 이메일·자동 이름이 달라져도 다른 이름으로
        // 되살아나지 않게 계정 ID tombstone도 함께 남긴다.
        mark_profile_deleted(deletion_identity_key(env, provider, id));
    }
    crate::usage::purge_account_cache(env, provider, meta_id.as_deref(), name);
    Ok(())
}

/// 형제 모듈(usage·login) 테스트와 공유하는 픽스처 헬퍼.
/// 실토큰이 아닌 픽스처로만 검증한다 (CLAUDE.md 금기).
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub(crate) fn test_env(tag: &str) -> Env {
        let base = std::env::temp_dir().join(format!("switcher-test-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join(".claude")).unwrap();
        Env {
            home: base.clone(),
            store: base.join(".switcher"),
            // 테스트는 어느 플랫폼에서든 파일 저장소로 검증한다 (실키체인은 건드리지 않는다)
            claude_live: ClaudeLiveStore::File(base.join(".claude").join(".credentials.json")),
        }
    }

    /// 가짜 JWT (서명 없음) — claims JSON을 그대로 payload로 쓴다
    pub(crate) fn fake_jwt(claims_json: &str) -> String {
        use base64::Engine;
        let enc = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        format!(
            "{}.{}.{}",
            enc(r#"{"alg":"none"}"#),
            enc(claims_json),
            enc("sig")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{fake_jwt, test_env};
    use super::*;

    fn login_claude(env: &Env, uuid: &str, email: &str, token: &str) {
        fs::write(
            env.home.join(".claude").join(".credentials.json"),
            format!(r#"{{"claudeAiOauth":{{"accessToken":"{token}"}}}}"#),
        )
        .unwrap();
        fs::write(
            env.claude_json_path(),
            format!(
                r#"{{"numStartups":1,"oauthAccount":{{"accountUuid":"{uuid}","emailAddress":"{email}"}}}}"#
            ),
        )
        .unwrap();
    }

    fn live_token(env: &Env) -> String {
        fs::read_to_string(env.live_credential_path(Provider::Claude)).unwrap()
    }

    #[test]
    fn save_then_list_marks_active() {
        let env = test_env("save-list");
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        save_current(&env, Provider::Claude, "main").unwrap();

        let snap = list(&env, Provider::Claude).unwrap();
        assert_eq!(snap.profiles.len(), 1);
        assert_eq!(snap.profiles[0].name, "main");
        assert!(snap.profiles[0].active);
        assert!(snap.live_saved);
        assert_eq!(snap.profiles[0].email.as_deref(), Some("alice@test.dev"));
    }

    #[test]
    fn switch_backs_up_current_then_swaps() {
        let env = test_env("switch");
        // 계정 B를 먼저 저장해두고
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b1");
        save_current(&env, Provider::Claude, "second").unwrap();
        // 계정 A로 로그인된 상태(토큰이 그새 갱신됐다고 가정: tok-a2)
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a2");
        save_current(&env, Provider::Claude, "main").unwrap();
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a3"); // 저장 후 또 갱신됨

        let result = switch(&env, Provider::Claude, "second").unwrap();

        // 백업이 먼저: main 프로필에 최신 토큰(tok-a3)이 저장돼 있어야 한다
        assert_eq!(result.backed_up_to.as_deref(), Some("main"));
        let backed = fs::read_to_string(
            env.profiles_dir(Provider::Claude)
                .join("main")
                .join("credentials.json"),
        )
        .unwrap();
        assert!(backed.contains("tok-a3"));

        // 백업이 기존 사본을 덮어쓸 때 한 세대 .bak이 남아야 한다
        assert!(env
            .profiles_dir(Provider::Claude)
            .join("main")
            .join("credentials.json.bak")
            .exists());

        // 활성 파일은 B의 토큰으로 교체
        assert!(live_token(&env).contains("tok-b1"));
        // ~/.claude.json의 oauthAccount도 B로 반영
        let root = read_json(&env.claude_json_path()).unwrap();
        assert_eq!(root["oauthAccount"]["accountUuid"].as_str(), Some("uuid-b"));
        // 다른 키는 보존
        assert_eq!(root["numStartups"].as_i64(), Some(1));

        let snap = list(&env, Provider::Claude).unwrap();
        let active: Vec<_> = snap.profiles.iter().filter(|p| p.active).collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].name, "second");
    }

    #[test]
    fn switch_applies_pending_refresh_before_activating_profile() {
        let env = test_env("switch-pending");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b");
        save_current(&env, Provider::Claude, "second").unwrap();
        let dir = env.profiles_dir(Provider::Claude).join("second");
        let cred = dir.join("credentials.json");
        fs::write(
            &cred,
            r#"{"claudeAiOauth":{"accessToken":"old-a","refreshToken":"r-old","expiresAt":1000}}"#,
        )
        .unwrap();
        fs::write(
            cred.with_extension("json.pending"),
            r#"{"old_refresh":"r-old","response":{"access_token":"new-a","refresh_token":"r-new","expires_in":28800},"saved_at":1}"#,
        )
        .unwrap();
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a");

        switch(&env, Provider::Claude, "second").unwrap();
        let live: Value = serde_json::from_str(&live_token(&env)).unwrap();
        assert_eq!(live.pointer("/claudeAiOauth/accessToken").unwrap(), "new-a");
        assert_eq!(
            live.pointer("/claudeAiOauth/refreshToken").unwrap(),
            "r-new"
        );
        assert!(!cred.with_extension("json.pending").exists());
    }

    #[test]
    fn switch_auto_saves_unsaved_account() {
        let env = test_env("auto-save");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b1");
        save_current(&env, Provider::Claude, "second").unwrap();
        // 프로필로 저장한 적 없는 계정 A가 로그인된 상태에서 바로 전환
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");

        let result = switch(&env, Provider::Claude, "second").unwrap();

        // A 계정이 이메일 기반 자동 프로필로 구조됨
        assert_eq!(result.backed_up_to.as_deref(), Some("alice"));
        let backed = fs::read_to_string(
            env.profiles_dir(Provider::Claude)
                .join("alice")
                .join("credentials.json"),
        )
        .unwrap();
        assert!(backed.contains("tok-a1"));
        assert!(live_token(&env).contains("tok-b1"));
    }

    #[test]
    fn save_rejects_name_owned_by_other_account() {
        let env = test_env("foreign-name");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b1");
        save_current(&env, Provider::Claude, "main").unwrap();
        // 다른 계정 A로 로그인한 뒤 같은 이름 "main"으로 저장 시도 → 거부돼야 한다
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        let err = save_current(&env, Provider::Claude, "main").unwrap_err();
        assert!(err.contains("다른 계정"));
        // B의 토큰이 파괴되지 않고 그대로 보존
        let kept = fs::read_to_string(
            env.profiles_dir(Provider::Claude)
                .join("main")
                .join("credentials.json"),
        )
        .unwrap();
        assert!(kept.contains("tok-b1"));
    }

    #[test]
    fn hidden_profile_name_collision_does_not_expose_identity() {
        let env = test_env("hidden-foreign-name");
        login_claude(&env, "uuid-private", "private@test.dev", "tok-private");
        save_current(&env, Provider::Claude, "main").unwrap();
        let dir = env.profiles_dir(Provider::Claude).join("main");
        let mut meta = read_meta(&dir).unwrap();
        meta.hide_email = true;
        atomic_write(&dir.join("meta.json"), &serde_json::to_vec(&meta).unwrap()).unwrap();

        login_claude(&env, "uuid-other", "other@test.dev", "tok-other");
        let error = save_current(&env, Provider::Claude, "main").unwrap_err();
        assert!(error.contains("다른 계정"));
        assert!(!error.contains("private@test.dev"));
        assert!(!error.contains("uuid-private"));
    }

    #[test]
    fn incomplete_import_profile_is_hidden_and_cannot_switch() {
        let env = test_env("incomplete-import-profile");
        login_claude(&env, "uuid-active", "active@test.dev", "tok-active");
        save_current(&env, Provider::Claude, "active").unwrap();
        login_claude(&env, "uuid-import", "import@test.dev", "tok-import");
        save_current(&env, Provider::Claude, "incoming").unwrap();
        let incoming = env.profiles_dir(Provider::Claude).join("incoming");
        atomic_write(&incoming.join(PROFILE_IMPORT_MARKER), b"fixture-import-id").unwrap();

        let names: Vec<_> = list(&env, Provider::Claude)
            .unwrap()
            .profiles
            .into_iter()
            .map(|profile| profile.name)
            .collect();
        assert!(!names.iter().any(|name| name == "incoming"));
        let error = switch(&env, Provider::Claude, "incoming").unwrap_err();
        assert!(error.contains("완료되지 않은 프로필"));
        assert!(
            incoming.exists(),
            "복구 전에는 marked profile을 임의 삭제하지 않는다"
        );
    }

    #[test]
    fn switch_refuses_claude_profile_without_account_info() {
        let env = test_env("no-oauth-block");
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        save_current(&env, Provider::Claude, "main").unwrap();
        // 구조용 백업처럼 계정 정보(oauth_account.json) 없는 프로필
        let dir = env.profiles_dir(Provider::Claude).join("rescue1");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"tok-x"}}"#,
        )
        .unwrap();
        fs::write(
            dir.join("meta.json"),
            r#"{"id":"unknown-1","email":null,"saved_at":0}"#,
        )
        .unwrap();
        let err = switch(&env, Provider::Claude, "rescue1").unwrap_err();
        assert!(err.contains("계정 정보"));
        assert!(live_token(&env).contains("tok-a1"), "활성 토큰은 불변");
    }

    #[test]
    fn plan_is_extracted_from_credentials() {
        let claude: Value = serde_json::from_str(
            r#"{"claudeAiOauth":{"accessToken":"t","subscriptionType":"max","rateLimitTier":"default_claude_max_20x"}}"#,
        )
        .unwrap();
        assert_eq!(
            plan_from_credential(Provider::Claude, &claude).as_deref(),
            Some("Max")
        );
        assert_eq!(tier_from_credential(Provider::Claude, &claude), Some(20));
        let five: Value =
            serde_json::from_str(r#"{"claudeAiOauth":{"rateLimitTier":"default_claude_max_5x"}}"#)
                .unwrap();
        assert_eq!(tier_from_credential(Provider::Claude, &five), Some(5));
        let jwt = fake_jwt(
            r#"{"email":"x@test.dev","https://api.openai.com/auth":{"chatgpt_plan_type":"pro"}}"#,
        );
        let codex: Value = serde_json::from_str(&format!(
            r#"{{"tokens":{{"id_token":"{jwt}","account_id":"a"}}}}"#
        ))
        .unwrap();
        assert_eq!(
            plan_from_credential(Provider::Codex, &codex).as_deref(),
            Some("Pro")
        );
    }

    #[test]
    fn auto_name_sanitizes_untrusted_id() {
        let env = test_env("autoname");
        let ident = LiveIdentity {
            id: "auth0|../..evil".to_string(),
            email: None,
        };
        let name = auto_name(&env, Provider::Codex, &ident);
        assert!(validate_name(&name).is_ok(), "생성된 이름: {name}");
    }

    #[test]
    fn save_rejects_duplicate_account_under_new_name() {
        let env = test_env("dup");
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        save_current(&env, Provider::Claude, "main").unwrap();
        let err = save_current(&env, Provider::Claude, "other").unwrap_err();
        assert!(err.contains("main"));
    }

    #[test]
    fn name_validation_blocks_path_escape() {
        let env = test_env("names");
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        assert!(save_current(&env, Provider::Claude, "../evil").is_err());
        assert!(save_current(&env, Provider::Claude, "a b").is_err());
        assert!(switch(&env, Provider::Claude, "..").is_err());
        // 전환의 빈 이름은 여전히 거부 (자동 작명은 저장에만 있다)
        assert!(switch(&env, Provider::Claude, "").is_err());
    }

    /// 빈 이름 저장 = 자동 작명 (#18 UX) — 이메일 앞부분으로 짓고, 실제 이름을 돌려준다
    #[test]
    fn empty_name_saves_with_auto_name() {
        let env = test_env("auto-save-name");
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        let name = save_current(&env, Provider::Claude, "").unwrap();
        assert_eq!(name, "alice");
        // 공백만 있어도 자동 작명, 같은 계정은 같은 프로필로 (중복 생성 없음)
        let again = save_current(&env, Provider::Claude, "  ").unwrap();
        assert_eq!(again, "alice");
        assert_eq!(list(&env, Provider::Claude).unwrap().profiles.len(), 1);
    }

    fn login_codex(env: &Env, account_id: &str, email: &str, token: &str) {
        fs::create_dir_all(env.home.join(".codex")).unwrap();
        fs::write(
            env.home.join(".codex").join("auth.json"),
            format!(
                r#"{{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{{"id_token":"{}","access_token":"{token}","refresh_token":"r-{token}","account_id":"{account_id}"}},"last_refresh":"2026-01-01T00:00:00Z"}}"#,
                fake_jwt(&format!(r#"{{"email":"{email}","sub":"sub-x"}}"#))
            ),
        )
        .unwrap();
    }

    #[test]
    fn codex_switch_backs_up_then_swaps() {
        let env = test_env("codex");
        login_codex(&env, "acct-b", "bob@test.dev", "ctok-b1");
        save_current(&env, Provider::Codex, "personal").unwrap();
        login_codex(&env, "acct-a", "alice@test.dev", "ctok-a1");
        save_current(&env, Provider::Codex, "work").unwrap();
        login_codex(&env, "acct-a", "alice@test.dev", "ctok-a2"); // 갱신 가정

        let result = switch(&env, Provider::Codex, "personal").unwrap();

        assert_eq!(result.backed_up_to.as_deref(), Some("work"));
        let backed = fs::read_to_string(
            env.profiles_dir(Provider::Codex)
                .join("work")
                .join("auth.json"),
        )
        .unwrap();
        assert!(backed.contains("ctok-a2"));
        let live = fs::read_to_string(env.live_credential_path(Provider::Codex)).unwrap();
        assert!(live.contains("ctok-b1"));

        let snap = list(&env, Provider::Codex).unwrap();
        let active: Vec<_> = snap.profiles.iter().filter(|p| p.active).collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].name, "personal");
        assert_eq!(active[0].email.as_deref(), Some("bob@test.dev"));
    }

    #[test]
    fn codex_and_claude_profiles_are_isolated() {
        let env = test_env("isolation");
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        save_current(&env, Provider::Claude, "main").unwrap();
        login_codex(&env, "acct-a", "alice@test.dev", "ctok-a1");
        save_current(&env, Provider::Codex, "main").unwrap();

        // 같은 이름이라도 프로바이더별로 분리 보관
        assert!(env
            .profiles_dir(Provider::Claude)
            .join("main")
            .join("credentials.json")
            .exists());
        assert!(env
            .profiles_dir(Provider::Codex)
            .join("main")
            .join("auth.json")
            .exists());
    }

    /// 실계정 파일 검증: 현재 계정을 프로필로 저장 → 자기 자신으로 전환(무해한 왕복).
    /// 계정이 바뀌지 않아야 하고 토큰 파일 내용도 보존되어야 한다.
    /// CI에서는 돌지 않는다: `cargo test -- --ignored` 로만 실행.
    fn real_self_switch(provider: Provider) {
        let env = Env::real().unwrap();
        if !live_cred_exists(&env, provider) {
            panic!("로그인 정보가 없어 실환경 검증 불가");
        }
        let before_cred = read_live_cred(&env, provider).unwrap();
        let before_ident = live_identity(&env, provider).unwrap().unwrap();

        let name = match find_profile_by_id(&env, provider, &before_ident.id).unwrap() {
            Some(existing) => existing,
            None => {
                // 제품과 같은 규칙으로 이름을 정한다 (하드코딩 이름은 다른 계정 소유일 수 있어
                // ensure_name_not_owned_by_other에 막힌다 — 실제로 막힌 사례 있음)
                let name = auto_name(&env, provider, &before_ident);
                save_current(&env, provider, &name).unwrap();
                name
            }
        };

        let result = switch(&env, provider, &name).unwrap();
        assert_eq!(result.switched_to, name);
        assert_eq!(result.backed_up_to.as_deref(), Some(name.as_str()));

        let after_ident = live_identity(&env, provider).unwrap().unwrap();
        assert_eq!(before_ident.id, after_ident.id, "계정이 바뀌면 안 된다");
        let after_cred = read_live_cred(&env, provider).unwrap();
        assert_eq!(before_cred, after_cred, "토큰이 보존되어야 한다");

        let snap = list(&env, provider).unwrap();
        assert!(
            snap.live_saved,
            "전환 후 활성 계정이 프로필과 매칭되어야 한다"
        );
    }

    #[test]
    #[ignore]
    fn real_self_switch_claude() {
        real_self_switch(Provider::Claude);
    }

    #[test]
    #[ignore]
    fn real_self_switch_codex() {
        real_self_switch(Provider::Codex);
    }

    #[test]
    fn delete_removes_only_stored_profile() {
        let env = test_env("delete");
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        save_current(&env, Provider::Claude, "main").unwrap();
        // 비활성 계정 프로필 하나를 직접 만들어 두고 사용량 캐시도 심는다
        let dir = env.profiles_dir(Provider::Claude).join("other");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"tok-b"}}"#,
        )
        .unwrap();
        fs::write(
            dir.join("meta.json"),
            r#"{"id":"uuid-b","email":null,"saved_at":0}"#,
        )
        .unwrap();
        fs::create_dir_all(&env.store).unwrap();
        fs::write(
            env.store.join("usage-cache.json"),
            r#"{"claude:uuid-a":{"saved_at":1,"usage":{"windows":[]}},
                "claude:uuid-b":{"saved_at":1,"usage":{"windows":[]}}}"#,
        )
        .unwrap();

        delete(&env, Provider::Claude, "other").unwrap();
        // 삭제된 비활성 계정의 캐시 항목은 정리되고, 활성 계정 항목은 남는다 (#18)
        let cache = read_json(&env.store.join("usage-cache.json")).unwrap();
        assert!(
            cache.get("claude:uuid-b").is_none(),
            "삭제 계정 캐시가 남았다"
        );
        assert!(
            cache.get("claude:uuid-a").is_some(),
            "활성 계정 캐시는 남아야 한다"
        );

        delete(&env, Provider::Claude, "main").unwrap();
        assert!(list(&env, Provider::Claude).unwrap().profiles.is_empty());
        // 활성 로그인 파일은 그대로
        assert!(env.live_credential_path(Provider::Claude).exists());
    }

    /// 재발급 가드가 살아 있는 동안 전환·삭제 권한이 기다리고, 드랍되면 즉시 풀린다.
    #[test]
    fn profile_lifecycle_exclusive_waits_and_blocks_new_refresh() {
        let env = test_env("profile-lifecycle");
        let key = refresh_key(&env, Provider::Claude, "inflight-test");
        // 재발급이 없으면 배타 권한은 즉시 잡힌다.
        let t0 = std::time::Instant::now();
        let exclusive =
            profile_exclusive_begin(key.clone(), std::time::Duration::from_secs(5)).unwrap();
        assert!(t0.elapsed() < std::time::Duration::from_millis(100));
        assert!(
            refresh_begin(key.clone()).is_err(),
            "배타 작업 중 새 재발급 금지"
        );
        drop(exclusive);

        let refresh = refresh_begin(key.clone()).unwrap();
        let key2 = key.clone();
        let waiter = std::thread::spawn(move || {
            let t = std::time::Instant::now();
            let guard = profile_exclusive_begin(key2, std::time::Duration::from_secs(5)).unwrap();
            (t.elapsed(), guard)
        });
        std::thread::sleep(std::time::Duration::from_millis(150));
        drop(refresh);
        let (waited, exclusive) = waiter.join().unwrap();
        assert!(
            waited >= std::time::Duration::from_millis(100),
            "재발급 가드 생존 중에는 기다려야 한다: {waited:?}"
        );
        assert!(
            waited < std::time::Duration::from_secs(4),
            "드랍 즉시 풀려야 한다 (상한 대기 아님): {waited:?}"
        );
        assert!(refresh_begin(key.clone()).is_err());
        drop(exclusive);
        assert!(refresh_begin(key).is_ok());
    }

    #[test]
    fn delete_waits_for_refresh_and_deleted_parent_stays_gone() {
        let env = test_env("delete-refresh-race");
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a");
        save_current(&env, Provider::Claude, "main").unwrap();
        let dir = env.profiles_dir(Provider::Claude).join("main");
        let refresh = refresh_begin(refresh_key(&env, Provider::Claude, "main")).unwrap();
        let observed_dir = dir.clone();
        let deleter = std::thread::spawn(move || delete(&env, Provider::Claude, "main"));
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(observed_dir.exists(), "재발급 중에는 삭제가 기다려야 한다");
        drop(refresh);
        deleter.join().unwrap().unwrap();
        assert!(!observed_dir.exists());
        let pending = observed_dir.join("credentials.json.pending");
        assert!(atomic_write_existing_parent(&pending, b"secret").is_err());
        assert!(!observed_dir.exists());
    }
}

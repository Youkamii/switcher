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
use zeroize::{Zeroize, Zeroizing};

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
#[cfg(any(test, target_os = "macos"))]
fn keychain_find_args<'a>(service: &'a str, account: &'a str, reveal: bool) -> Vec<&'a str> {
    let mut args = vec!["find-generic-password", "-s", service, "-a", account];
    if reveal {
        args.push("-w");
    }
    args
}

#[cfg(any(test, target_os = "macos"))]
fn keychain_delete_args<'a>(service: &'a str, account: &'a str) -> [&'a str; 5] {
    ["delete-generic-password", "-s", service, "-a", account]
}

#[cfg(any(test, target_os = "macos"))]
fn keychain_item_exists_result(success: bool, stderr: &str, service: &str) -> Result<bool, String> {
    if success {
        Ok(true)
    } else if stderr.contains("could not be found") {
        Ok(false)
    } else {
        Err(format!(
            "키체인 항목 확인 실패 ({service}): {}",
            stderr.trim()
        ))
    }
}

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
    pub(crate) fn read_item(service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
        let out = run_security(&super::keychain_find_args(service, account, true))?;
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

    pub(crate) fn item_exists(service: &str, account: &str) -> Result<bool, String> {
        // -w 없이 조회하면 비밀에 접근하지 않고 존재만 확인한다
        let out = run_security(&super::keychain_find_args(service, account, false))?;
        super::keychain_item_exists_result(
            out.status.success(),
            &String::from_utf8_lossy(&out.stderr),
            service,
        )
    }

    pub(crate) fn write_item(service: &str, account: &str, data: &[u8]) -> Result<(), String> {
        use std::fmt::Write as _;
        // security의 대화형 입력 형식 때문에 hex 문자열이 필요하지만, 토큰 복사본이
        // 프로세스 메모리에 필요 이상 오래 남지 않도록 사용 직후 zeroize한다.
        let mut hex = zeroize::Zeroizing::new(String::with_capacity(data.len() * 2));
        for b in data {
            let _ = write!(&mut *hex, "{b:02x}");
        }
        let mut child = Command::new("/usr/bin/security")
            .arg("-i")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("security 실행 실패: {e}"))?;
        let cmd = zeroize::Zeroizing::new(format!(
            "add-generic-password -U -a \"{account}\" -s \"{service}\" -X \"{}\"\n",
            hex.as_str()
        ));
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
    pub(crate) fn delete_item(service: &str, account: &str) -> Result<(), String> {
        let out = run_security(&super::keychain_delete_args(service, account))?;
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

        struct TestItemGuard {
            service: String,
            accounts: Vec<String>,
        }

        impl Drop for TestItemGuard {
            fn drop(&mut self) {
                for account in &self.accounts {
                    let _ = delete_item(&self.service, account);
                }
            }
        }

        /// 실제 로그인 키체인에 시험 항목을 왕복시킨다 — security 경유는 팝업 없이
        /// 동작해야 한다 (실측 확인된 전제가 깨지면 이 테스트가 알려준다)
        #[test]
        fn roundtrip_via_security_cli() {
            let svc = format!("switcher-selftest-{}", std::process::id());
            let account_a = format!("{}-a", username());
            let account_b = format!("{}-b", username());
            let _cleanup = TestItemGuard {
                service: svc.clone(),
                accounts: vec![account_a.clone(), account_b.clone()],
            };
            let payload_a = br#"{"probe":"account-a"}"#;
            let payload_b = br#"{"probe":"account-b"}"#;
            write_item(&svc, &account_a, payload_a).unwrap();
            write_item(&svc, &account_b, payload_b).unwrap();
            assert!(item_exists(&svc, &account_a).unwrap());
            assert!(item_exists(&svc, &account_b).unwrap());
            let read = read_item(&svc, &account_a)
                .unwrap()
                .expect("방금 쓴 항목이 있어야 한다");
            assert_eq!(read, payload_a);
            assert_eq!(read_item(&svc, &account_b).unwrap().unwrap(), payload_b);
            // 같은 항목 갱신(-U)도 되어야 한다 (전환마다 일어나는 일)
            let payload2 = br#"{"probe":"updated"}"#;
            write_item(&svc, &account_a, payload2).unwrap();
            assert_eq!(read_item(&svc, &account_a).unwrap().unwrap(), payload2);
            assert_eq!(read_item(&svc, &account_b).unwrap().unwrap(), payload_b);
            delete_item(&svc, &account_a).unwrap();
            assert!(!item_exists(&svc, &account_a).unwrap());
            assert!(
                read_item(&svc, &account_a).unwrap().is_none(),
                "삭제 후에는 None"
            );
            assert_eq!(read_item(&svc, &account_b).unwrap().unwrap(), payload_b);
        }
    }
}

#[cfg(test)]
mod keychain_arg_tests {
    use super::*;

    #[test]
    fn keychain_read_and_delete_scope_to_service_and_account() {
        assert_eq!(
            keychain_find_args("service", "account", true),
            vec![
                "find-generic-password",
                "-s",
                "service",
                "-a",
                "account",
                "-w",
            ]
        );
        assert_eq!(
            keychain_find_args("service", "account", false),
            vec!["find-generic-password", "-s", "service", "-a", "account"]
        );
        assert_eq!(
            keychain_delete_args("service", "account"),
            ["delete-generic-password", "-s", "service", "-a", "account"]
        );
    }

    #[test]
    fn keychain_presence_distinguishes_missing_from_access_errors() {
        assert!(keychain_item_exists_result(true, "", "service").unwrap());
        assert!(
            !keychain_item_exists_result(
                false,
                "The specified item could not be found in the keychain.",
                "service",
            )
            .unwrap()
        );
        assert!(
            keychain_item_exists_result(false, "User interaction is not allowed.", "service")
                .is_err()
        );
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
/// macOS Claude의 키체인·전달 파일이 혼합된 채 원복되지 못했음을 나타낸다.
/// 비밀이나 계정 식별자는 넣지 않고, 명시적인 전환/재로그인 복구가 끝날 때까지
/// 일반 읽기·저장·내보내기 경로를 fail-closed로 막는다.
const CLAUDE_RECOVERY_MARKER: &str = ".claude-switch-recovery-required";
/// 활성 Claude 재로그인의 새 토큰을 프로필에 보관했지만 live Keychain/파일과
/// oauthAccount에 아직 완전히 적용하지 못했음을 나타낸다. 이 표식이 있는 프로필은
/// 다음 활성 백업·갱신·Vault 내보내기로 절대 덮지 않는다.
const CLAUDE_LIVE_APPLY_PENDING: &str = ".claude-live-apply-pending";

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
    let mut newly_created = Vec::new();
    let mut cursor = parent;
    while !cursor.exists() {
        newly_created.push(cursor.to_path_buf());
        let Some(next) = cursor.parent() else {
            break;
        };
        cursor = next;
    }
    fs::create_dir_all(parent).map_err(|e| format!("폴더 생성 실패 {}: {e}", parent.display()))?;
    sync_created_directory_entries(&newly_created)
        .map_err(|e| format!("폴더 생성 확정 실패 {}: {e}", parent.display()))?;
    atomic_replace_in_parent(path, data, parent)
}

#[cfg(unix)]
fn sync_created_directory_entries(created: &[PathBuf]) -> std::io::Result<()> {
    // deepest → highest 순서로 새 디렉터리와 그 부모를 함께 fsync한다. 그래야
    // 새 프로필 폴더 자체가 crash 뒤 사라져 "백업 후 전환" 순서가 역전되지 않는다.
    for directory in created {
        fs::File::open(directory)?.sync_all()?;
        if let Some(parent) = directory.parent() {
            fs::File::open(parent)?.sync_all()?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_created_directory_entries(_created: &[PathBuf]) -> std::io::Result<()> {
    Ok(())
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
    })?;
    // 파일 자체를 fsync한 뒤 rename만 하고 끝내면 전원/OS crash에서 디렉터리
    // 엔트리만 되돌아갈 수 있다. 저널 표식보다 자격증명·프로필 파일이 먼저
    // 사라지는 역전이 없도록 같은 상위 디렉터리까지 영속화한다.
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| format!("교체 확정 실패 {}: {e}", path.display()))?;
    Ok(())
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

fn claude_recovery_marker_path(env: &Env) -> PathBuf {
    env.store.join(CLAUDE_RECOVERY_MARKER)
}

fn claude_live_apply_pending_path(profile_dir: &Path) -> PathBuf {
    profile_dir.join(CLAUDE_LIVE_APPLY_PENDING)
}

/// 손상된 sidecar도 정상 프로필로 열지 않는다.
pub(crate) fn claude_live_apply_pending(profile_dir: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(claude_live_apply_pending_path(profile_dir)) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(
            "Claude 재로그인 적용 복구 표식이 손상되어 안전하게 진행할 수 없습니다"
                .into(),
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("Claude 재로그인 적용 복구 상태를 확인할 수 없습니다".into()),
    }
}

fn any_claude_live_apply_pending(env: &Env) -> Result<bool, String> {
    let root = env.profiles_dir(Provider::Claude);
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("Claude 재로그인 적용 복구 상태를 확인할 수 없습니다".into()),
    };
    for entry in entries {
        let entry =
            entry.map_err(|_| "Claude 재로그인 적용 복구 상태를 확인할 수 없습니다")?;
        let file_type = entry
            .file_type()
            .map_err(|_| "Claude 재로그인 적용 복구 상태를 확인할 수 없습니다")?;
        if file_type.is_symlink() {
            return Err(
                "Claude 프로필 폴더가 심볼릭 링크라 복구 상태를 안전하게 확인할 수 없습니다"
                    .into(),
            );
        }
        if file_type.is_dir() && claude_live_apply_pending(&entry.path())? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn mark_claude_live_apply_pending(profile_dir: &Path) -> Result<(), String> {
    atomic_write(
        &claude_live_apply_pending_path(profile_dir),
        b"live-apply-pending-v1\n",
    )
    .map_err(|_| "Claude 새 로그인 정보를 안전하게 보호하지 못했습니다".to_string())
}

pub(crate) fn clear_claude_live_apply_pending(profile_dir: &Path) -> Result<(), String> {
    let path = claude_live_apply_pending_path(profile_dir);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => return Err("Claude 재로그인 적용 복구 표식이 손상되었습니다".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            mark_claude_live_apply_pending(profile_dir)?;
            return Err("Claude 재로그인 적용 복구 표식이 예상보다 먼저 사라졌습니다".into());
        }
        Err(_) => return Err("Claude 재로그인 적용 복구 표식을 확인할 수 없습니다".into()),
    }
    fs::remove_file(&path)
        .map_err(|_| "Claude 재로그인 적용 복구 표식을 해제하지 못했습니다".to_string())?;
    #[cfg(unix)]
    if fs::File::open(profile_dir)
        .and_then(|directory| directory.sync_all())
        .is_err()
    {
        mark_claude_live_apply_pending(profile_dir)?;
        return Err("Claude 재로그인 적용 복구 표식 해제를 디스크에 확정하지 못했습니다".into());
    }
    match claude_live_apply_pending(profile_dir) {
        Ok(false) => Ok(()),
        Ok(true) => Err("다른 Claude 재로그인 복구 작업이 감지되었습니다".into()),
        Err(error) => Err(error),
    }
}

fn claude_uses_keychain_store(env: &Env) -> bool {
    #[cfg(target_os = "macos")]
    {
        matches!(&env.claude_live, ClaudeLiveStore::Keychain { .. })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = env;
        false
    }
}

/// 손상된 표식(디렉터리·심볼릭 링크)이나 메타데이터 오류도 정상 상태로 간주하지 않는다.
fn claude_recovery_required(env: &Env) -> Result<bool, String> {
    match fs::symlink_metadata(claude_recovery_marker_path(env)) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err("Claude 계정 전환 복구 표식이 손상되어 안전하게 진행할 수 없습니다".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("Claude 계정 전환 복구 상태를 확인할 수 없습니다".into()),
    }
}

fn sync_recovery_marker_parent(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .ok_or_else(|| "Claude 복구 표식 상위 위치를 확인할 수 없습니다".to_string())?;
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| "Claude 복구 표식 위치를 디스크에 동기화하지 못했습니다".into())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn mark_claude_recovery_required(path: &Path) -> Result<(), String> {
    atomic_write(path, b"switch-recovery-v1\n")
        .map_err(|_| "Claude 계정 전환 복구 상태를 안전하게 기록하지 못했습니다".to_string())?;
    sync_recovery_marker_parent(path)
}

pub(crate) fn ensure_claude_recovery_not_required(
    env: &Env,
    provider: Provider,
) -> Result<(), String> {
    if provider == Provider::Claude
        && (claude_recovery_required(env)? || any_claude_live_apply_pending(env)?)
    {
        return Err(
            "Claude 계정 전환 복구가 필요합니다 — 사용량 조회·저장·내보내기를 중단했습니다. 보류된 프로필을 다시 선택하거나 같은 계정으로 재로그인하세요"
                .into(),
        );
    }
    Ok(())
}

fn clear_claude_recovery_marker(env: &Env) -> Result<(), String> {
    let path = claude_recovery_marker_path(env);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => {
            return Err("Claude 계정 전환 복구 표식이 손상되어 자동으로 해제할 수 없습니다".into())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // 성공 확인 전에 표식이 사라진 경우 다시 세워 일반 경로가 열리지 않게 한다.
            mark_claude_recovery_required(&path)?;
            return Err("Claude 계정 전환 복구 표식이 예상보다 먼저 사라졌습니다".into());
        }
        Err(_) => return Err("Claude 계정 전환 복구 표식을 확인할 수 없습니다".into()),
    }
    fs::remove_file(&path)
        .map_err(|_| "Claude 계정 전환은 완료됐지만 복구 표식을 해제하지 못했습니다".to_string())?;
    if sync_recovery_marker_parent(&path).is_err() {
        // 삭제가 디렉터리에 영속화되지 않았으면 성공으로 열지 않는다. 표식을 다시
        // 기록해 다음 시작에서도 recovery mode가 유지되도록 한다.
        mark_claude_recovery_required(&path)?;
        return Err("Claude 계정 전환 복구 표식 해제를 디스크에 확정하지 못했습니다".into());
    }
    match claude_recovery_required(env) {
        Ok(false) => Ok(()),
        Ok(true) => Err("다른 Claude 복구 작업이 감지되어 일반 접근을 계속 차단합니다".into()),
        Err(error) => Err(error),
    }
}

/// 활성 자격증명 읽기 — 코덱스는 항상 파일, 클로드는 저장소(파일/키체인)에 따른다.
/// 키체인 모드에서 항목이 없으면 구버전 파일로 폴백한다 (키체인 미사용 환경 대응).
pub(crate) fn read_live_cred(env: &Env, provider: Provider) -> Result<Vec<u8>, String> {
    ensure_claude_recovery_not_required(env, provider)?;
    let mut credential = read_live_cred_unchecked(env, provider)?;
    if let Err(error) = ensure_claude_recovery_not_required(env, provider) {
        credential.zeroize();
        return Err(error);
    }
    Ok(credential)
}

fn read_live_cred_unchecked(env: &Env, provider: Provider) -> Result<Vec<u8>, String> {
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
                account,
                legacy_file,
            } => match keychain::read_item(service, account)? {
                Some(data) => Ok(data),
                None => {
                    if credential_path_exists(legacy_file)? {
                        read_file(legacy_file)
                    } else {
                        Err(
                            "클로드 로그인 정보가 없습니다 (키체인에 항목 없음) — 먼저 claude에서 로그인하세요"
                                .into(),
                        )
                    }
                }
            },
        },
    }
}

fn credential_path_exists(path: &Path) -> Result<bool, String> {
    path.try_exists()
        .map_err(|error| format!("활성 인증정보 확인 실패 {}: {error}", path.display()))
}

/// 활성 자격증명이 존재하는가 (전환·저장 가능 여부 판단)
pub(crate) fn live_cred_exists(env: &Env, provider: Provider) -> Result<bool, String> {
    ensure_claude_recovery_not_required(env, provider)?;
    let exists = live_cred_exists_unchecked(env, provider)?;
    ensure_claude_recovery_not_required(env, provider)?;
    Ok(exists)
}

fn live_cred_exists_unchecked(env: &Env, provider: Provider) -> Result<bool, String> {
    match provider {
        Provider::Codex => credential_path_exists(&env.live_credential_path(Provider::Codex)),
        Provider::Claude => match &env.claude_live {
            ClaudeLiveStore::File(path) => credential_path_exists(path),
            #[cfg(target_os = "macos")]
            ClaudeLiveStore::Keychain {
                service,
                account,
                legacy_file,
            } => {
                if keychain::item_exists(service, account)? {
                    Ok(true)
                } else {
                    credential_path_exists(legacy_file)
                }
            }
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
                let keychain_before = keychain::read_item(service, account)?.map(Zeroizing::new);
                keychain::write_item(service, account, data)?;
                if let Err(legacy_error) = atomic_write(legacy_file, data) {
                    let compensation = match keychain::read_item(service, account) {
                        Ok(Some(current)) if credential_equivalent(&current, data) => {
                            match keychain_before.as_deref() {
                                Some(before) => keychain::write_item(service, account, before),
                                None => keychain::delete_item(service, account),
                            }
                        }
                        Ok(_) => Ok(()),
                        Err(error) => Err(error),
                    };
                    return match compensation {
                        Ok(()) => Err(legacy_error),
                        Err(compensation_error) => Err(format!(
                            "{legacy_error}; 부분 적용된 키체인 원복 실패: {compensation_error}"
                        )),
                    };
                }
                Ok(())
            }
        },
    }
}

enum ClaudeLiveSnapshot {
    File {
        path: PathBuf,
        data: Option<Zeroizing<Vec<u8>>>,
    },
    #[cfg(target_os = "macos")]
    Keychain {
        service: String,
        account: String,
        keychain_data: Option<Zeroizing<Vec<u8>>>,
        legacy_file: PathBuf,
        legacy_data: Option<Zeroizing<Vec<u8>>>,
        recovery_marker: PathBuf,
    },
}

fn read_optional_secret(path: &Path) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    match fs::read(path) {
        Ok(data) => Ok(Some(Zeroizing::new(data))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("활성 인증정보 읽기 실패 {}: {error}", path.display())),
    }
}

fn snapshot_claude_live(env: &Env) -> Result<ClaudeLiveSnapshot, String> {
    match &env.claude_live {
        ClaudeLiveStore::File(path) => Ok(ClaudeLiveSnapshot::File {
            path: path.clone(),
            data: read_optional_secret(path)?,
        }),
        #[cfg(target_os = "macos")]
        ClaudeLiveStore::Keychain {
            service,
            account,
            legacy_file,
        } => Ok(ClaudeLiveSnapshot::Keychain {
            service: service.clone(),
            account: account.clone(),
            keychain_data: keychain::read_item(service, account)?.map(Zeroizing::new),
            legacy_file: legacy_file.clone(),
            legacy_data: read_optional_secret(legacy_file)?,
            recovery_marker: claude_recovery_marker_path(env),
        }),
    }
}

fn restore_optional_secret(
    path: &Path,
    data: Option<&Zeroizing<Vec<u8>>>,
) -> Result<(), String> {
    match data {
        Some(data) => atomic_write(path, data),
        None => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("활성 인증정보 제거 실패 {}: {error}", path.display())),
        },
    }
}

fn restore_optional_credential_if_unchanged(
    path: &Path,
    data: Option<&Zeroizing<Vec<u8>>>,
    applied_data: &[u8],
) -> Result<(), String> {
    let current = read_optional_secret(path)?;
    if !current
        .as_deref()
        .is_some_and(|current| credential_equivalent(current, applied_data))
    {
        return Ok(());
    }
    restore_optional_secret(path, data)
}

fn credential_equivalent(left: &[u8], right: &[u8]) -> bool {
    let left = Zeroizing::new(normalize_cred(left.to_vec()));
    let right = Zeroizing::new(normalize_cred(right.to_vec()));
    match (
        serde_json::from_slice::<Value>(&left),
        serde_json::from_slice::<Value>(&right),
    ) {
        (Ok(mut left), Ok(mut right)) => {
            let equivalent = left == right;
            zeroize_json_strings(&mut left);
            zeroize_json_strings(&mut right);
            equivalent
        }
        _ => left.as_slice() == right.as_slice(),
    }
}

fn zeroize_json_strings(value: &mut Value) {
    match value {
        Value::String(value) => value.zeroize(),
        Value::Array(values) => values.iter_mut().for_each(zeroize_json_strings),
        Value::Object(values) => values.values_mut().for_each(zeroize_json_strings),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn optional_credentials_equivalent(left: Option<&[u8]>, right: Option<&[u8]>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => credential_equivalent(left, right),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

fn claude_live_matches_snapshot(snapshot: &ClaudeLiveSnapshot) -> Result<bool, String> {
    match snapshot {
        ClaudeLiveSnapshot::File { path, data } => {
            let current = read_optional_secret(path)?;
            Ok(optional_credentials_equivalent(
                current.as_deref().map(Vec::as_slice),
                data.as_deref().map(Vec::as_slice),
            ))
        }
        #[cfg(target_os = "macos")]
        ClaudeLiveSnapshot::Keychain {
            service,
            account,
            keychain_data,
            legacy_file,
            legacy_data,
            ..
        } => {
            let current_keychain = keychain::read_item(service, account)?.map(Zeroizing::new);
            let current_legacy = read_optional_secret(legacy_file)?;
            Ok(optional_credentials_equivalent(
                current_keychain.as_deref().map(Vec::as_slice),
                keychain_data.as_deref().map(Vec::as_slice),
            ) && optional_credentials_equivalent(
                current_legacy.as_deref().map(Vec::as_slice),
                legacy_data.as_deref().map(Vec::as_slice),
            ))
        }
    }
}

fn claude_live_matches(env: &Env, applied_data: &[u8]) -> Result<bool, String> {
    match &env.claude_live {
        ClaudeLiveStore::File(path) => Ok(read_optional_secret(path)?
            .as_deref()
            .is_some_and(|current| credential_equivalent(current, applied_data))),
        #[cfg(target_os = "macos")]
        ClaudeLiveStore::Keychain {
            service,
            account,
            legacy_file,
        } => {
            let keychain = keychain::read_item(service, account)?.map(Zeroizing::new);
            let legacy = read_optional_secret(legacy_file)?;
            Ok(keychain
                .as_deref()
                .is_some_and(|current| credential_equivalent(current, applied_data))
                && legacy
                    .as_deref()
                    .is_none_or(|current| credential_equivalent(current, applied_data)))
        }
    }
}

fn restore_claude_live_if_unchanged(
    snapshot: &ClaudeLiveSnapshot,
    applied_data: &[u8],
) -> Result<(), String> {
    match snapshot {
        ClaudeLiveSnapshot::File { path, data } => {
            restore_optional_credential_if_unchanged(path, data.as_ref(), applied_data)
        }
        #[cfg(target_os = "macos")]
        ClaudeLiveSnapshot::Keychain {
            service,
            account,
            keychain_data,
            legacy_file,
            legacy_data,
            recovery_marker,
        } => {
            let current_keychain = keychain::read_item(service, account)?.map(Zeroizing::new);
            let current_legacy = read_optional_secret(legacy_file)?;
            let keychain_still_applied = current_keychain
                .as_deref()
                .is_some_and(|current| credential_equivalent(current, applied_data));
            let legacy_still_applied = current_legacy
                .as_deref()
                .is_some_and(|current| credential_equivalent(current, applied_data));

            // 두 저장소가 모두 우리가 쓴 값이면 한 거래처럼 이전 스냅샷으로 되돌린다.
            // 신형 Claude CLI가 전달용 파일을 이미 소비(삭제)한 경우도 동일한 거래다.
            if keychain_still_applied && (legacy_still_applied || current_legacy.is_none()) {
                match keychain_data {
                    Some(data) => keychain::write_item(service, account, data)?,
                    None => keychain::delete_item(service, account)?,
                }
                if let Err(legacy_error) =
                    restore_optional_secret(legacy_file, legacy_data.as_ref())
                {
                    let compensation = keychain::write_item(
                        service,
                        account,
                        current_keychain
                            .as_deref()
                            .expect("applied keychain was checked"),
                    );
                    return match compensation {
                        Ok(()) => Err(legacy_error),
                        Err(compensation_error) => Err(format!(
                            "{legacy_error}; 키체인 원복 보상 실패: {compensation_error}"
                        )),
                    };
                }
                return Ok(());
            }

            // 한쪽만 우리가 쓴 값이면 다른 프로세스가 두 저장소 사이에서 로그인
            // 상태를 바꾼 것이다. macOS 키체인과 일반 파일에는 값 기반 CAS가 없어
            // 지금 삭제/복원하면 이 확인 직후 들어온 외부 값을 지울 수 있다.
            // 어느 쪽도 건드리지 않고 사용자가 안정된 상태에서 재시도하게 한다.
            if keychain_still_applied || legacy_still_applied {
                mark_claude_recovery_required(recovery_marker)?;
                return Err(
                    "Claude 인증정보 원복 충돌: 외부 로그인 변경을 보존하기 위해 자동 원복을 중단했습니다 — Claude를 닫고 다시 시도하세요"
                        .into(),
                );
            }
            Ok(())
        }
    }
}

struct PreparedClaudeOauth {
    path: PathBuf,
    before: Option<Value>,
    target: Value,
    file_existed: bool,
}

fn read_claude_root(path: &Path) -> Result<(Value, bool), String> {
    let bytes = read_optional_secret(path)?;
    let file_existed = bytes.is_some();
    let root = match bytes.as_deref() {
        Some(bytes) => serde_json::from_slice(bytes)
            .map_err(|error| format!("Claude 계정 정보 읽기 실패 {}: {error}", path.display()))?,
        None => serde_json::json!({}),
    };
    if !root.is_object() {
        return Err(format!(
            "Claude 계정 정보 형식이 잘못되었습니다: {}",
            path.display()
        ));
    }
    Ok((root, file_existed))
}

fn write_claude_root(path: &Path, root: &Value) -> Result<(), String> {
    let bytes = Zeroizing::new(serde_json::to_vec_pretty(root).map_err(|error| error.to_string())?);
    atomic_write(path, &bytes)
}

fn prepare_claude_oauth_apply(
    env: &Env,
    profile_dir: &Path,
) -> Result<PreparedClaudeOauth, String> {
    let target = read_json(&profile_dir.join("oauth_account.json"))?;
    let path = env.claude_json_path();
    let (root, file_existed) = read_claude_root(&path)?;
    Ok(PreparedClaudeOauth {
        path,
        before: root.get("oauthAccount").cloned(),
        target,
        file_existed,
    })
}

fn apply_prepared_claude_oauth(prepared: &PreparedClaudeOauth) -> Result<(), String> {
    let (mut root, _) = read_claude_root(&prepared.path)?;
    if root.get("oauthAccount") != prepared.before.as_ref() {
        return Err("Claude 로그인이 전환 중 외부에서 변경되었습니다".into());
    }
    root.as_object_mut()
        .expect("read_claude_root verified an object")
        .insert("oauthAccount".to_string(), prepared.target.clone());
    write_claude_root(&prepared.path, &root)
}

fn restore_prepared_claude_oauth_if_unchanged(
    prepared: &PreparedClaudeOauth,
) -> Result<(), String> {
    let (mut root, _) = read_claude_root(&prepared.path)?;
    if root.get("oauthAccount") != Some(&prepared.target) {
        return Ok(());
    }
    let object = root
        .as_object_mut()
        .expect("read_claude_root verified an object");
    match prepared.before.as_ref() {
        Some(before) => {
            object.insert("oauthAccount".to_string(), before.clone());
        }
        None => {
            object.remove("oauthAccount");
        }
    }
    if !prepared.file_existed && object.is_empty() {
        restore_optional_secret(&prepared.path, None)
    } else {
        write_claude_root(&prepared.path, &root)
    }
}

fn prepared_claude_oauth_matches(prepared: &PreparedClaudeOauth) -> Result<bool, String> {
    let (root, _) = read_claude_root(&prepared.path)?;
    Ok(root.get("oauthAccount") == Some(&prepared.target))
}

fn claude_applied_state_is_stable(
    env: &Env,
    credential: &[u8],
    oauth: &PreparedClaudeOauth,
) -> Result<bool, String> {
    for pass in 0..3 {
        if !claude_live_matches(env, credential)? || !prepared_claude_oauth_matches(oauth)? {
            return Ok(false);
        }
        if pass < 2 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    Ok(true)
}

fn expected_live_profile_matches(
    env: &Env,
    provider: Provider,
    expected: Option<&LiveProfileSnapshot>,
) -> Result<bool, String> {
    let Some(expected) = expected else {
        return Ok(!live_cred_exists(env, provider)?);
    };
    let current = Zeroizing::new(read_live_cred(env, provider)?);
    if !credential_equivalent(&current, &expected.credential) {
        return Ok(false);
    }
    if provider == Provider::Claude {
        return Ok(claude_oauth_block(env)? == expected.oauth_block);
    }
    Ok(true)
}

fn expected_claude_apply_guard_matches(
    env: &Env,
    expected: &ClaudeApplyGuard,
    recovery_mode: bool,
) -> Result<bool, String> {
    let credential_matches = match expected.credential.as_deref() {
        Some(expected_credential) => {
            let current = Zeroizing::new(if recovery_mode {
                read_live_cred_unchecked(env, Provider::Claude)?
            } else {
                read_live_cred(env, Provider::Claude)?
            });
            credential_equivalent(&current, expected_credential)
        }
        None => {
            let exists = if recovery_mode {
                live_cred_exists_unchecked(env, Provider::Claude)?
            } else {
                live_cred_exists(env, Provider::Claude)?
            };
            !exists
        }
    };
    if !credential_matches {
        return Ok(false);
    }
    Ok(claude_oauth_block(env)? == expected.oauth_block)
}

fn apply_claude_profile_inner<B, A>(
    env: &Env,
    profile_dir: &Path,
    data: &[u8],
    expected_before: Option<&LiveProfileSnapshot>,
    before_credential_write: B,
    after_credential_write: A,
) -> Result<(), String>
where
    B: FnOnce(),
    A: FnOnce(),
{
    apply_claude_profile_inner_mode(
        env,
        profile_dir,
        data,
        expected_before,
        false,
        false,
        before_credential_write,
        after_credential_write,
    )
}

fn apply_claude_profile_inner_mode<B, A>(
    env: &Env,
    profile_dir: &Path,
    data: &[u8],
    expected_before: Option<&LiveProfileSnapshot>,
    recovery_mode: bool,
    journaled: bool,
    before_credential_write: B,
    after_credential_write: A,
) -> Result<(), String>
where
    B: FnOnce(),
    A: FnOnce(),
{
    let expected_guard = match expected_before {
        Some(snapshot) => ClaudeApplyGuard::from_snapshot(snapshot),
        None => capture_missing_claude_apply_guard(env, recovery_mode)?,
    };
    apply_claude_profile_inner_mode_guard(
        env,
        profile_dir,
        data,
        &expected_guard,
        recovery_mode,
        journaled,
        before_credential_write,
        after_credential_write,
    )
}

fn apply_claude_profile_inner_mode_guard<B, A>(
    env: &Env,
    profile_dir: &Path,
    data: &[u8],
    expected_before: &ClaudeApplyGuard,
    recovery_mode: bool,
    journaled: bool,
    before_credential_write: B,
    after_credential_write: A,
) -> Result<(), String>
where
    B: FnOnce(),
    A: FnOnce(),
{
    let expected_matches = |env: &Env| {
        expected_claude_apply_guard_matches(env, expected_before, recovery_mode)
    };
    if !expected_matches(env)? {
        return Err("Claude 로그인이 전환 중 외부에서 변경되었습니다".into());
    }
    let snapshot = snapshot_claude_live(env)?;
    let oauth = prepare_claude_oauth_apply(env, profile_dir)?;
    let marker = claude_recovery_marker_path(env);
    let mut journal_started = false;
    let applied = (|| {
        before_credential_write();
        if !expected_matches(env)? || !claude_live_matches_snapshot(&snapshot)? {
            return Err("Claude 로그인이 전환 중 외부에서 변경되었습니다".into());
        }
        // 준비·외부 변경 검사가 모두 끝난 뒤, 첫 활성 쓰기 직전에만 저널을
        // 영속화한다. 이보다 앞선 실패는 활성 상태를 전혀 건드리지 않았으므로
        // 다음 정상 전환에서 최신 토큰을 다시 백업할 수 있어야 한다.
        if journaled {
            mark_claude_recovery_required(&marker)?;
            journal_started = true;
            // marker 파일·부모 fsync 동안 외부 CLI 로그인이 끝날 수 있다. 실제
            // Keychain 쓰기 직전에 한 번 더 확인해 그 큰 I/O 창을 닫는다.
            if !expected_matches(env)? || !claude_live_matches_snapshot(&snapshot)? {
                return Err("Claude 로그인이 전환 중 외부에서 변경되었습니다".into());
            }
        }
        write_live_cred(env, Provider::Claude, data)?;
        after_credential_write();
        if !claude_live_matches(env, data)? {
            return Err("Claude 로그인이 전환 중 외부에서 변경되었습니다".into());
        }
        apply_prepared_claude_oauth(&oauth)?;
        if !claude_applied_state_is_stable(env, data, &oauth)? {
            return Err("Claude 로그인이 전환 중 외부에서 변경되었습니다".into());
        }
        Ok(())
    })();
    if let Err(apply_error) = applied {
        let oauth_restore = restore_prepared_claude_oauth_if_unchanged(&oauth);
        let credential_restore = restore_claude_live_if_unchanged(&snapshot, data);
        let mut restore_errors = [oauth_restore.err(), credential_restore.err()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        // 첫 활성 쓰기 직전 저널이 세워진 뒤에는 원복 성공 여부와 관계없이
        // 표식을 유지한다. 외부 writer와의 값 기반 CAS가 없는 macOS 키체인에서
        // 혼합 상태를 정상으로 오인하지 않게 하는 최종 fail-closed 관문이다.
        if journal_started {
            if let Err(marker_error) = mark_claude_recovery_required(&marker) {
                restore_errors.push(marker_error);
            }
        }
        return if restore_errors.is_empty() {
            Err(apply_error)
        } else {
            Err(format!(
                "{apply_error}; 전환 실패 뒤 활성 정보 원복 실패: {}",
                restore_errors.join("; ")
            ))
        };
    }
    if journaled {
        clear_claude_recovery_marker(env)
    } else {
        Ok(())
    }
}

fn apply_claude_profile(
    env: &Env,
    profile_dir: &Path,
    data: &[u8],
    expected_before: Option<&LiveProfileSnapshot>,
) -> Result<(), String> {
    apply_claude_profile_inner(env, profile_dir, data, expected_before, || {}, || {})
}

fn apply_claude_profile_recovery(
    env: &Env,
    profile_dir: &Path,
    data: &[u8],
    expected_before: Option<&LiveProfileSnapshot>,
) -> Result<(), String> {
    apply_claude_profile_inner_mode(
        env,
        profile_dir,
        data,
        expected_before,
        true,
        true,
        || {},
        || {},
    )
}

fn apply_claude_profile_journaled(
    env: &Env,
    profile_dir: &Path,
    data: &[u8],
    expected_before: Option<&LiveProfileSnapshot>,
) -> Result<(), String> {
    apply_claude_profile_recovery(env, profile_dir, data, expected_before)
}

fn apply_claude_profile_journaled_guard(
    env: &Env,
    profile_dir: &Path,
    data: &[u8],
    expected_before: &ClaudeApplyGuard,
) -> Result<(), String> {
    apply_claude_profile_inner_mode_guard(
        env,
        profile_dir,
        data,
        expected_before,
        true,
        true,
        || {},
        || {},
    )
}

/// 같은 활성 Claude 계정의 격리 재로그인 결과를 적용한다. 키체인 다중 저장소나
/// 기존 recovery mode에서는 저널을 유지하고, 파일 단일 저장소에서도 전환과 같은
/// expected guard·조건부 원복·적용 후 안정성 검사를 공유한다.
pub(crate) fn apply_claude_live_update(
    env: &Env,
    profile_dir: &Path,
    data: &[u8],
    expected_before: &ClaudeApplyGuard,
) -> Result<(), String> {
    if claude_uses_keychain_store(env)
        || claude_recovery_required(env)?
        || claude_live_apply_pending(profile_dir)?
    {
        apply_claude_profile_journaled_guard(env, profile_dir, data, expected_before)
    } else {
        apply_claude_profile_inner_mode_guard(
            env,
            profile_dir,
            data,
            expected_before,
            false,
            false,
            || {},
            || {},
        )
    }
}

fn apply_codex_profile_inner<B, A>(
    env: &Env,
    data: &[u8],
    expected_before: Option<&LiveProfileSnapshot>,
    before_credential_write: B,
    after_credential_write: A,
) -> Result<(), String>
where
    B: FnOnce(),
    A: FnOnce(),
{
    let path = env.live_credential_path(Provider::Codex);
    let before = read_optional_secret(&path)?;
    let applied = (|| {
        before_credential_write();
        if !expected_live_profile_matches(env, Provider::Codex, expected_before)? {
            return Err("Codex 로그인이 전환 중 외부에서 변경되었습니다".into());
        }
        atomic_write(&path, data)?;
        after_credential_write();
        for pass in 0..3 {
            let current = read_optional_secret(&path)?;
            if !current
                .as_deref()
                .is_some_and(|current| credential_equivalent(current, data))
            {
                return Err("Codex 로그인이 전환 중 외부에서 변경되었습니다".into());
            }
            if pass < 2 {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        Ok(())
    })();
    if let Err(apply_error) = applied {
        return match restore_optional_credential_if_unchanged(&path, before.as_ref(), data) {
            Ok(()) => Err(apply_error),
            Err(restore_error) => Err(format!(
                "{apply_error}; 전환 실패 뒤 활성 인증정보 원복 실패: {restore_error}"
            )),
        };
    }
    Ok(())
}

fn apply_codex_profile(
    env: &Env,
    data: &[u8],
    expected_before: Option<&LiveProfileSnapshot>,
) -> Result<(), String> {
    apply_codex_profile_inner(env, data, expected_before, || {}, || {})
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
    if !credential_path_exists(&path)? {
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
    write_profile_parts_mode(env, provider, name, ident, cred, oauth_block, false)
}

/// active relogin만 자기 pending sidecar가 있는 프로필을 새 로그인 결과로 갱신할
/// 수 있다. 그 밖의 백업·Vault 경로는 보호된 fresh credential을 덮지 못한다.
pub(crate) fn write_profile_parts_for_active_relogin(
    env: &Env,
    provider: Provider,
    name: &str,
    ident: &LiveIdentity,
    cred: &[u8],
    oauth_block: Option<&Value>,
) -> Result<(), String> {
    write_profile_parts_mode(env, provider, name, ident, cred, oauth_block, true)
}

/// active 재로그인용 sidecar보다 먼저 프로필 소유권을 영속화한다. 이 순서가
/// 지켜져야 bundle 쓰기 중 crash가 나도 다른 계정이 같은 자동 이름을 차지해
/// fresh credential을 덮지 못하고, 같은 계정의 다음 재로그인이 복구할 수 있다.
pub(crate) fn ensure_claude_relogin_profile_identity(
    env: &Env,
    name: &str,
    ident: &LiveIdentity,
) -> Result<(), String> {
    let dir = env.profiles_dir(Provider::Claude).join(name);
    if crate::vault::profile_import_blocked(env, Provider::Claude, name) {
        return Err("중단된 인증정보 가져오기를 복구한 뒤 다시 시도하세요".into());
    }
    if let Some(owner) = profile_owner_identity(&dir, Provider::Claude)? {
        if owner.id != ident.id {
            return Err(format!(
                "'{name}'은 이미 다른 Claude 계정의 프로필입니다 — 다른 이름을 쓰세요"
            ));
        }
    }
    if read_meta_checked(&dir)?.is_some() {
        return Ok(());
    }
    let meta = Meta {
        id: ident.id.clone(),
        email: ident.email.clone(),
        saved_at: now(),
        hide_email: false,
    };
    let bytes = serde_json::to_vec_pretty(&meta).map_err(|error| error.to_string())?;
    atomic_write(&dir.join("meta.json"), &bytes)
}

fn write_profile_parts_mode(
    env: &Env,
    provider: Provider,
    name: &str,
    ident: &LiveIdentity,
    cred: &[u8],
    oauth_block: Option<&Value>,
    allow_live_apply_pending: bool,
) -> Result<(), String> {
    let dir = env.profiles_dir(provider).join(name);
    if crate::vault::profile_import_blocked(env, provider, name) {
        return Err("중단된 인증정보 가져오기를 복구한 뒤 다시 시도하세요".into());
    }
    if provider == Provider::Claude
        && !allow_live_apply_pending
        && claude_live_apply_pending(&dir)?
    {
        return Err(
            "새 Claude 로그인 정보의 활성 적용이 보류되어 이 프로필을 덮을 수 없습니다 — 이 프로필을 다시 선택하거나 같은 계정으로 재로그인하세요"
                .into(),
        );
    }
    ensure_name_not_owned_by_other(env, provider, name, ident)?;
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

pub(crate) struct LiveProfileSnapshot {
    credential: Zeroizing<Vec<u8>>,
    oauth_block: Option<Value>,
    identity: Option<LiveIdentity>,
}

pub(crate) struct ClaudeApplyGuard {
    credential: Option<Zeroizing<Vec<u8>>>,
    oauth_block: Option<Value>,
    identity: Option<LiveIdentity>,
}

impl ClaudeApplyGuard {
    fn from_snapshot(snapshot: &LiveProfileSnapshot) -> Self {
        Self {
            credential: Some(Zeroizing::new(snapshot.credential.as_slice().to_vec())),
            oauth_block: snapshot.oauth_block.clone(),
            identity: snapshot.identity.clone(),
        }
    }

    pub(crate) fn belongs_to(&self, id: &str) -> bool {
        self.identity.as_ref().is_some_and(|identity| identity.id == id)
    }
}

fn read_claude_profile_snapshot_once(env: &Env) -> Result<LiveProfileSnapshot, String> {
    read_claude_profile_snapshot_once_mode(env, false)
}

fn read_claude_profile_snapshot_once_mode(
    env: &Env,
    recovery_mode: bool,
) -> Result<LiveProfileSnapshot, String> {
    let oauth_before = claude_oauth_block(env)?;
    let credential = Zeroizing::new(if recovery_mode {
        read_live_cred_unchecked(env, Provider::Claude)?
    } else {
        read_live_cred(env, Provider::Claude)?
    });
    let oauth_after = claude_oauth_block(env)?;
    if oauth_before != oauth_after {
        return Err("Claude 로그인이 변경되는 중이라 현재 계정 저장을 중단했습니다".into());
    }
    let identity = oauth_after.as_ref().and_then(|oauth| {
        identity_from_value(
            Provider::Claude,
            &serde_json::json!({ "oauthAccount": oauth }),
        )
    });
    Ok(LiveProfileSnapshot {
        credential,
        oauth_block: oauth_after,
        identity,
    })
}

fn read_stable_claude_profile_snapshot_with<R, P>(
    mut read: R,
    mut pause: P,
) -> Result<LiveProfileSnapshot, String>
where
    R: FnMut() -> Result<LiveProfileSnapshot, String>,
    P: FnMut(),
{
    let mut previous = read()?;
    let mut stable_intervals = 0usize;
    for _ in 0..5 {
        pause();
        let next = read()?;
        if credential_equivalent(&previous.credential, &next.credential)
            && previous.oauth_block == next.oauth_block
        {
            stable_intervals += 1;
            if stable_intervals >= 2 {
                return Ok(next);
            }
        } else {
            stable_intervals = 0;
        }
        previous = next;
    }
    Err("Claude 로그인이 변경되는 중이라 현재 계정 저장을 중단했습니다 — 로그인이 끝난 뒤 다시 시도하세요".into())
}

fn capture_live_profile(env: &Env, provider: Provider) -> Result<LiveProfileSnapshot, String> {
    match provider {
        Provider::Claude => read_stable_claude_profile_snapshot_with(
            || read_claude_profile_snapshot_once(env),
            || {
                if !cfg!(test) {
                    std::thread::sleep(std::time::Duration::from_millis(120));
                }
            },
        ),
        Provider::Codex => {
            let credential = Zeroizing::new(read_live_cred(env, Provider::Codex)?);
            let mut root: Value = serde_json::from_slice(&credential)
                .map_err(|error| format!("Codex 로그인 정보 형식이 잘못되었습니다: {error}"))?;
            let identity = identity_from_value(Provider::Codex, &root);
            zeroize_json_strings(&mut root);
            Ok(LiveProfileSnapshot {
                identity,
                credential,
                oauth_block: None,
            })
        }
    }
}

fn capture_claude_recovery_guard(env: &Env) -> Result<LiveProfileSnapshot, String> {
    read_stable_claude_profile_snapshot_with(
        || read_claude_profile_snapshot_once_mode(env, true),
        || {
            if !cfg!(test) {
                std::thread::sleep(std::time::Duration::from_millis(120));
            }
        },
    )
}

fn read_missing_claude_apply_guard_once(
    env: &Env,
    recovery_mode: bool,
) -> Result<ClaudeApplyGuard, String> {
    let oauth_before = claude_oauth_block(env)?;
    let credential_exists = if recovery_mode {
        live_cred_exists_unchecked(env, Provider::Claude)?
    } else {
        live_cred_exists(env, Provider::Claude)?
    };
    let oauth_after = claude_oauth_block(env)?;
    if credential_exists || oauth_before != oauth_after {
        return Err("Claude 로그인이 변경되는 중이라 적용을 중단했습니다".into());
    }
    let identity = oauth_after.as_ref().and_then(|oauth| {
        identity_from_value(
            Provider::Claude,
            &serde_json::json!({ "oauthAccount": oauth }),
        )
    });
    Ok(ClaudeApplyGuard {
        credential: None,
        oauth_block: oauth_after,
        identity,
    })
}

fn capture_missing_claude_apply_guard(
    env: &Env,
    recovery_mode: bool,
) -> Result<ClaudeApplyGuard, String> {
    let mut previous = read_missing_claude_apply_guard_once(env, recovery_mode)?;
    let mut stable_intervals = 0usize;
    for _ in 0..5 {
        if !cfg!(test) {
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        let next = read_missing_claude_apply_guard_once(env, recovery_mode)?;
        if previous.oauth_block == next.oauth_block {
            stable_intervals += 1;
            if stable_intervals >= 2 {
                return Ok(next);
            }
        } else {
            stable_intervals = 0;
        }
        previous = next;
    }
    Err("Claude 로그인이 변경되는 중이라 적용을 중단했습니다".into())
}

fn capture_claude_apply_guard(
    env: &Env,
    recovery_mode: bool,
) -> Result<ClaudeApplyGuard, String> {
    let credential_exists = if recovery_mode {
        live_cred_exists_unchecked(env, Provider::Claude)?
    } else {
        live_cred_exists(env, Provider::Claude)?
    };
    if credential_exists {
        let snapshot = if recovery_mode {
            capture_claude_recovery_guard(env)?
        } else {
            capture_live_profile(env, Provider::Claude)?
        };
        Ok(ClaudeApplyGuard::from_snapshot(&snapshot))
    } else {
        capture_missing_claude_apply_guard(env, recovery_mode)
    }
}

/// 격리 재로그인이 같은 활성 Claude 계정을 갱신하기 전에 잡는 optimistic guard.
/// 이미 복구 표식이 있으면 일반 읽기는 의도적으로 막혀 있으므로, 사용자가 같은
/// 계정으로 재로그인해 수렴시킬 수 있도록 unchecked 안정 스냅숏을 사용한다.
pub(crate) fn capture_claude_live_update_guard(
    env: &Env,
) -> Result<ClaudeApplyGuard, String> {
    let recovery_mode =
        claude_recovery_required(env)? || any_claude_live_apply_pending(env)?;
    capture_claude_apply_guard(env, recovery_mode)
}

/// 한 번에 읽은 활성 계정 스냅숏을 지정 이름의 프로필로 저장한다 (덮어쓰기 허용).
fn write_profile_snapshot(
    env: &Env,
    provider: Provider,
    name: &str,
    ident: &LiveIdentity,
    snapshot: &LiveProfileSnapshot,
) -> Result<(), String> {
    write_profile_parts(
        env,
        provider,
        name,
        ident,
        &snapshot.credential,
        snapshot.oauth_block.as_ref(),
    )
}

/// name 프로필이 이미 다른 계정의 것이면 에러 — 다른 계정 토큰을 덮어쓰지 않는다
pub(crate) fn ensure_name_not_owned_by_other(
    env: &Env,
    provider: Provider,
    name: &str,
    ident: &LiveIdentity,
) -> Result<(), String> {
    let dir = env.profiles_dir(provider).join(name);
    if crate::vault::profile_import_blocked(env, provider, name) {
        return Err("중단된 인증정보 가져오기를 복구한 뒤 다시 시도하세요".into());
    }
    let metadata = match fs::symlink_metadata(&dir) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "프로필 소유권 확인 실패 {}: {error}",
                dir.display()
            ))
        }
    };
    if let Some(metadata) = metadata.as_ref() {
        if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
            return Err(format!(
                "'{name}' 프로필 경로가 안전한 폴더가 아니라 덮어쓸 수 없습니다"
            ));
        }
    }
    if let Some(owner) = profile_owner_identity(&dir, provider)? {
        if owner.id != ident.id {
            let hidden = read_meta_checked(&dir)?.is_some_and(|meta| meta.hide_email);
            if hidden {
                return Err(format!(
                    "'{name}'은 이미 다른 계정의 프로필입니다 — 다른 이름을 쓰세요"
                ));
            }
            let owner = owner.email.unwrap_or(owner.id);
            return Err(format!(
                "'{name}'은 이미 다른 계정({owner})의 프로필입니다 — 다른 이름을 쓰세요"
            ));
        }
    } else if metadata.is_some() {
        return Err(format!(
            "'{name}' 프로필의 소유 계정을 확인할 수 없어 덮어쓸 수 없습니다"
        ));
    }
    Ok(())
}

pub(crate) fn read_meta(dir: &Path) -> Option<Meta> {
    let path = dir.join("meta.json");
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn read_meta_checked(dir: &Path) -> Result<Option<Meta>, String> {
    let path = dir.join("meta.json");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            let bytes = fs::read(&path)
                .map_err(|error| format!("프로필 정보 읽기 실패 {}: {error}", path.display()))?;
            serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| format!("프로필 정보 형식 오류 {}: {error}", path.display()))
        }
        Ok(_) => Err(format!(
            "프로필 정보 경로가 안전한 파일이 아닙니다: {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "프로필 정보 확인 실패 {}: {error}",
            path.display()
        )),
    }
}

/// meta가 아직 쓰이지 않은 중단 프로필도 oauthAccount를 소유권 증거로 사용한다.
/// pending인데 어느 쪽으로도 계정을 식별하지 못하면 덮어쓰기보다 실패를 택한다.
fn profile_owner_identity(
    dir: &Path,
    provider: Provider,
) -> Result<Option<LiveIdentity>, String> {
    if let Some(meta) = read_meta_checked(dir)? {
        return Ok(Some(LiveIdentity {
            id: meta.id,
            email: meta.email,
        }));
    }
    if provider != Provider::Claude {
        return Ok(None);
    }
    let pending = claude_live_apply_pending(dir)?;
    let oauth_path = dir.join("oauth_account.json");
    let oauth = match fs::symlink_metadata(&oauth_path) {
        Ok(metadata) if metadata.file_type().is_file() => Some(read_json(&oauth_path)?),
        Ok(_) => {
            return Err(format!(
                "Claude 프로필 계정 정보 경로가 안전한 파일이 아닙니다: {}",
                oauth_path.display()
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "Claude 프로필 계정 정보 확인 실패 {}: {error}",
                oauth_path.display()
            ))
        }
    };
    let owner = oauth.as_ref().and_then(|oauth| {
        identity_from_value(
            Provider::Claude,
            &serde_json::json!({ "oauthAccount": oauth }),
        )
    });
    if pending && owner.is_none() {
        return Err(
            "보류된 Claude 프로필의 소유 계정을 확인할 수 없어 안전하게 복구할 수 없습니다"
                .into(),
        );
    }
    Ok(owner)
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
    for entry in entries {
        let entry = entry.map_err(|error| format!("프로필 목록 읽기 실패: {error}"))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("프로필 경로 확인 실패 {}: {error}", path.display()))?;
        if file_type.is_symlink() {
            return Err(format!(
                "프로필 폴더가 심볼릭 링크라 안전하게 열 수 없습니다: {}",
                path.display()
            ));
        }
        if file_type.is_dir() && !crate::vault::profile_import_blocked(env, provider, &name) {
            out.push((name, path));
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
        if let Some(owner) = profile_owner_identity(&dir, provider)? {
            if owner.id == id {
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
        let candidate_dir = env.profiles_dir(provider).join(&candidate);
        match fs::symlink_metadata(&candidate_dir) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return candidate,
            Ok(_) if read_meta(&candidate_dir).is_some_and(|meta| meta.id == ident.id) => {
                return candidate;
            }
            Ok(_) | Err(_) => {
                n += 1;
                if n > 99 {
                    return format!("account-{}", now());
                }
                candidate = format!("{base}-{n}");
            }
        }
    }
}

/// 현재 로그인 계정을 이름 붙여 프로필로 저장.
/// 이름이 비어 있으면 auto_name으로 자동 작명한다 (#18 UX — 첫 저장 마찰 제거).
/// 실제 저장된 이름을 돌려준다 (자동 작명 결과를 프론트가 안내에 쓴다).
pub fn save_current(env: &Env, provider: Provider, name: &str) -> Result<String, String> {
    // 변이 함수가 스스로 잠근다 — 호출자가 잠금을 잊을 수 없게 (관례 단일화)
    let _guard = MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    if !live_cred_exists(env, provider)? {
        return Err("로그인 정보가 없습니다 — 먼저 해당 CLI에서 로그인하세요".into());
    }
    let snapshot = capture_live_profile(env, provider)?;
    let ident = snapshot
        .identity
        .as_ref()
        .ok_or("현재 로그인 계정을 식별할 수 없습니다 (로그인 직후 다시 시도)")?;
    let name = if name.trim().is_empty() {
        auto_name(env, provider, ident) // 항상 validate_name을 통과하는 이름을 만든다
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
    ensure_name_not_owned_by_other(env, provider, &name, ident)?;
    write_profile_snapshot(env, provider, &name, ident, &snapshot)?;
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
    let live_apply_pending =
        provider == Provider::Claude && claude_live_apply_pending(&profile_dir)?;
    let root_recovery = provider == Provider::Claude && claude_recovery_required(env)?;
    if provider == Provider::Claude
        && any_claude_live_apply_pending(env)?
        && !live_apply_pending
    {
        return Err(
            "새 Claude 로그인 정보의 활성 적용이 보류되어 있습니다 — 보류된 프로필을 선택하거나 같은 계정으로 재로그인하세요"
                .into(),
        );
    }
    let recovery_mode = root_recovery || live_apply_pending;
    if crate::vault::profile_import_blocked(env, provider, name) {
        return Err("가져오기가 완료되지 않은 프로필이라 전환할 수 없습니다".into());
    }
    let target_cred = profile_dir.join(provider.credential_file_name());
    // 직전 갱신의 본 파일 쓰기/활성 복구가 실패해 pending이 남았으면, 구토큰을
    // 활성 위치로 복사하기 전에 반드시 복구한다.
    crate::usage::rescue_pending_profile_locked(env, provider, name, !recovery_mode)?;
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

    if recovery_mode {
        // 혼합 상태는 어떤 계정의 최신 상태인지 증명할 수 없으므로 절대 프로필에
        // 백업하지 않는다. 현재 상태는 외부 변경 감지용 optimistic guard로만 읽고,
        // 사용자가 고른 완전한 프로필을 세 활성 저장소에 다시 적용해 수렴시킨다.
        let recovery_guard = capture_claude_apply_guard(env, true)?;
        let data = Zeroizing::new(normalize_cred(
            fs::read(&target_cred)
                .map_err(|e| format!("읽기 실패 {}: {e}", target_cred.display()))?,
        ));
        apply_claude_profile_journaled_guard(env, &profile_dir, &data, &recovery_guard)?;
        if live_apply_pending {
            clear_claude_live_apply_pending(&profile_dir)?;
        }
        crate::usage::clear_profile_backoff(env, provider, name);
        return Ok(SwitchResult {
            backed_up_to: None,
            switched_to: name.to_string(),
        });
    }

    // 1) 백업 — 현재 활성 계정을 자기 프로필(없으면 자동 생성)에 저장
    let mut backed_up_to = None;
    let live_snapshot = if live_cred_exists(env, provider)? {
        let snapshot = capture_live_profile(env, provider)?;
        match snapshot.identity.as_ref() {
            Some(live) => {
                let back_name = match find_profile_by_id(env, provider, &live.id)? {
                    Some(existing) => existing,
                    None => auto_name(env, provider, live),
                };
                write_profile_snapshot(env, provider, &back_name, live, &snapshot)?;
                backed_up_to = Some(back_name);
            }
            None => {
                // 신원 불명이어도 토큰을 버리지 않는다 — 구조용 프로필로 보관
                let rescue = format!("rescue-{}", now());
                let ident = LiveIdentity {
                    id: format!("unknown-{}", now()),
                    email: None,
                };
                write_profile_snapshot(env, provider, &rescue, &ident, &snapshot)?;
                backed_up_to = Some(rescue);
            }
        }
        Some(snapshot)
    } else {
        None
    };

    // 2) 대상 프로필을 활성 위치로 복사
    let data =
        fs::read(&target_cred).map_err(|e| format!("읽기 실패 {}: {e}", target_cred.display()))?;
    let data = Zeroizing::new(if provider == Provider::Claude {
        normalize_cred(data)
    } else {
        data
    });
    if provider == Provider::Claude {
        if claude_uses_keychain_store(env) {
            // 이 표식이 macOS 다중 저장소 전환의 작은 transaction journal이다.
            // 기록에 실패하면 첫 자격증명 쓰기 전에 중단하고, 성공한 전환만 해제한다.
            apply_claude_profile_journaled(env, &profile_dir, &data, live_snapshot.as_ref())?;
        } else {
            apply_claude_profile(env, &profile_dir, &data, live_snapshot.as_ref())?;
        }
    } else {
        apply_codex_profile(env, &data, live_snapshot.as_ref())?;
    }
    // 인증 세대가 바뀐 계정은 이전 조회 실패의 백오프를 상속하지 않는다.
    // 이 코어를 쓰는 버튼·고정 모드 더블클릭·TFSD 전환 모두에 동일하게 적용한다 (#122).
    crate::usage::clear_profile_backoff(env, provider, name);

    Ok(SwitchResult {
        backed_up_to,
        switched_to: name.to_string(),
    })
}

/// 프로필 목록 + 현재 로그인 계정 상태
pub fn list(env: &Env, provider: Provider) -> Result<Snapshot, String> {
    // 표시용 목록은 신원 읽기가 일시적으로 실패해도 화면 전체를 깨뜨리지 않는다
    // (전환·저장 경로는 여전히 엄격하게 실패한다). 단, 복구 중인 Claude의
    // oauthAccount는 혼합 상태의 일부라 활성 계정 판정에 쓰지 않는다. 프로필은
    // 계속 보여 사용자가 복구 대상을 직접 선택할 수 있게 한다.
    let mut live = match ensure_claude_recovery_not_required(env, provider) {
        Ok(()) => live_identity(env, provider).unwrap_or(None),
        Err(_) => None,
    };
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
    if crate::vault::profile_import_blocked(env, provider, name) {
        return Err("가져오기가 완료되지 않은 프로필이라 삭제할 수 없습니다".into());
    }
    if provider == Provider::Claude && claude_live_apply_pending(&dir)? {
        return Err(
            "새 Claude 로그인 정보의 활성 적용이 보류된 프로필은 삭제할 수 없습니다 — 먼저 이 프로필을 선택해 복구하세요"
                .into(),
        );
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

    #[cfg(target_os = "macos")]
    struct MacKeychainGuard {
        service: String,
        account: String,
    }

    #[cfg(target_os = "macos")]
    impl Drop for MacKeychainGuard {
        fn drop(&mut self) {
            let _ = keychain::delete_item(&self.service, &self.account);
        }
    }

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

    fn profile_bundle_bytes(env: &Env, name: &str) -> [Vec<u8>; 3] {
        let dir = env.profiles_dir(Provider::Claude).join(name);
        [
            fs::read(dir.join("credentials.json")).unwrap(),
            fs::read(dir.join("oauth_account.json")).unwrap(),
            fs::read(dir.join("meta.json")).unwrap(),
        ]
    }

    #[test]
    fn credential_presence_errors_do_not_become_logged_out() {
        let mut env = test_env("credential-presence-error");
        env.claude_live = ClaudeLiveStore::File(PathBuf::from("invalid\0credential"));

        assert!(live_cred_exists(&env, Provider::Claude).is_err());
        let error = save_current(&env, Provider::Claude, "must-not-exist").unwrap_err();
        assert!(error.contains("활성 인증정보 확인 실패"));
        assert!(!env
            .profiles_dir(Provider::Claude)
            .join("must-not-exist")
            .exists());
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
    fn claude_switch_normalizes_existing_hex_profile_before_live_write() {
        let env = test_env("switch-existing-hex-profile");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b1");
        save_current(&env, Provider::Claude, "second").unwrap();
        let target_path = env
            .profiles_dir(Provider::Claude)
            .join("second")
            .join("credentials.json");
        let target_raw = fs::read(&target_path).unwrap();
        let target_hex = target_raw
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        fs::write(&target_path, target_hex).unwrap();

        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        save_current(&env, Provider::Claude, "main").unwrap();
        switch(&env, Provider::Claude, "second").unwrap();

        assert_eq!(
            fs::read(env.live_credential_path(Provider::Claude)).unwrap(),
            target_raw,
            "기존 hex 프로필도 활성 위치에는 raw JSON으로 써야 한다"
        );
    }

    #[test]
    fn claude_switch_rolls_back_live_credential_when_oauth_apply_fails() {
        let env = test_env("switch-oauth-rollback");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b1");
        save_current(&env, Provider::Claude, "second").unwrap();
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        save_current(&env, Provider::Claude, "main").unwrap();
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a2");
        fs::write(
            env.profiles_dir(Provider::Claude)
                .join("second")
                .join("oauth_account.json"),
            b"invalid oauth fixture",
        )
        .unwrap();
        let live_before = fs::read(env.live_credential_path(Provider::Claude)).unwrap();
        let oauth_before = fs::read(env.claude_json_path()).unwrap();

        assert!(switch(&env, Provider::Claude, "second").is_err());

        assert_eq!(
            fs::read(env.live_credential_path(Provider::Claude)).unwrap(),
            live_before
        );
        assert_eq!(fs::read(env.claude_json_path()).unwrap(), oauth_before);
        let backed = fs::read_to_string(
            env.profiles_dir(Provider::Claude)
                .join("main")
                .join("credentials.json"),
        )
        .unwrap();
        assert!(backed.contains("tok-a2"), "백업 우선 순서는 유지해야 한다");
    }

    #[test]
    fn claude_switch_removes_new_live_credential_when_apply_fails() {
        let env = test_env("switch-empty-live-rollback");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b1");
        save_current(&env, Provider::Claude, "second").unwrap();
        fs::remove_file(env.live_credential_path(Provider::Claude)).unwrap();
        fs::write(
            env.profiles_dir(Provider::Claude)
                .join("second")
                .join("oauth_account.json"),
            b"invalid oauth fixture",
        )
        .unwrap();

        assert!(switch(&env, Provider::Claude, "second").is_err());

        assert!(!env.live_credential_path(Provider::Claude).exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_claude_switch_does_not_enter_recovery_on_prewrite_failure() {
        let mut env = test_env("switch-macos-keychain-rollback");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b1");
        save_current(&env, Provider::Claude, "second").unwrap();
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a2");
        fs::write(
            env.profiles_dir(Provider::Claude)
                .join("second")
                .join("oauth_account.json"),
            b"invalid oauth fixture",
        )
        .unwrap();

        let service = format!("switcher-switch-rollback-{}", std::process::id());
        let account = keychain::username();
        let _guard = MacKeychainGuard {
            service: service.clone(),
            account: account.clone(),
        };
        keychain::delete_item(&service, &account).unwrap();
        let legacy_file = env.live_credential_path(Provider::Claude);
        let legacy_before = fs::read(&legacy_file).unwrap();
        keychain::write_item(&service, &account, &legacy_before).unwrap();
        env.claude_live = ClaudeLiveStore::Keychain {
            service: service.clone(),
            account: account.clone(),
            legacy_file: legacy_file.clone(),
        };
        let keychain_before = keychain::read_item(&service, &account).unwrap().unwrap();
        let oauth_before = fs::read(env.claude_json_path()).unwrap();

        assert!(switch(&env, Provider::Claude, "second").is_err());

        assert!(
            !claude_recovery_marker_path(&env).exists(),
            "첫 활성 쓰기 전 준비 실패는 정상 상태를 복구 모드로 오인하면 안 된다"
        );
        assert_eq!(
            read_live_cred(&env, Provider::Claude).unwrap(),
            keychain_before
        );
        assert_eq!(
            keychain::read_item(&service, &account).unwrap().unwrap(),
            keychain_before
        );
        assert_eq!(fs::read(&legacy_file).unwrap(), legacy_before);
        assert_eq!(fs::read(env.claude_json_path()).unwrap(), oauth_before);
        let backed = fs::read_to_string(
            env.profiles_dir(Provider::Claude)
                .join("alice")
                .join("credentials.json"),
        )
        .unwrap();
        assert!(backed.contains("tok-a2"), "백업 우선 순서는 유지해야 한다");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_normal_switch_uses_and_clears_recovery_journal() {
        let mut env = test_env("switch-macos-journal-success");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b");
        save_current(&env, Provider::Claude, "second").unwrap();
        let target = fs::read(
            env.profiles_dir(Provider::Claude)
                .join("second")
                .join("credentials.json"),
        )
        .unwrap();
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a");
        save_current(&env, Provider::Claude, "main").unwrap();

        let service = format!("switcher-switch-journal-success-{}", std::process::id());
        let account = keychain::username();
        let _guard = MacKeychainGuard {
            service: service.clone(),
            account: account.clone(),
        };
        let legacy_file = env.live_credential_path(Provider::Claude);
        keychain::write_item(&service, &account, &fs::read(&legacy_file).unwrap()).unwrap();
        env.claude_live = ClaudeLiveStore::Keychain {
            service: service.clone(),
            account: account.clone(),
            legacy_file: legacy_file.clone(),
        };

        let result = switch(&env, Provider::Claude, "second").unwrap();

        assert_eq!(result.backed_up_to.as_deref(), Some("main"));
        assert!(!claude_recovery_marker_path(&env).exists());
        assert_eq!(
            keychain::read_item(&service, &account).unwrap().unwrap(),
            target
        );
        assert_eq!(fs::read(&legacy_file).unwrap(), target);
        assert_eq!(
            read_json(&env.claude_json_path()).unwrap()["oauthAccount"]["accountUuid"],
            "uuid-b"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_claude_rollback_normalizes_and_fails_closed_on_mixed_changes() {
        let mut env = test_env("switch-macos-keychain-atomic-rollback");
        let service = format!("switcher-switch-atomic-{}", std::process::id());
        let account = keychain::username();
        let _guard = MacKeychainGuard {
            service: service.clone(),
            account: account.clone(),
        };
        let legacy_file = env.live_credential_path(Provider::Claude);
        fs::create_dir_all(legacy_file.parent().unwrap()).unwrap();
        let before = br#"{"claudeAiOauth":{"accessToken":"before"}}"#;
        let applied = br#"{"claudeAiOauth":{"accessToken":"applied"}}"#;
        keychain::write_item(&service, &account, before).unwrap();
        fs::write(&legacy_file, before).unwrap();
        env.claude_live = ClaudeLiveStore::Keychain {
            service: service.clone(),
            account: account.clone(),
            legacy_file: legacy_file.clone(),
        };
        let snapshot = snapshot_claude_live(&env).unwrap();

        write_live_cred(&env, Provider::Claude, applied).unwrap();
        let hex_applied = applied
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        keychain::write_item(&service, &account, hex_applied.as_bytes()).unwrap();
        restore_claude_live_if_unchanged(&snapshot, applied).unwrap();
        assert_eq!(
            keychain::read_item(&service, &account).unwrap().unwrap(),
            before
        );
        assert_eq!(fs::read(&legacy_file).unwrap(), before);

        write_live_cred(&env, Provider::Claude, applied).unwrap();
        fs::remove_file(&legacy_file).unwrap();
        restore_claude_live_if_unchanged(&snapshot, applied).unwrap();
        assert_eq!(
            keychain::read_item(&service, &account).unwrap().unwrap(),
            before
        );
        assert_eq!(fs::read(&legacy_file).unwrap(), before);

        write_live_cred(&env, Provider::Claude, applied).unwrap();
        let external = br#"{"claudeAiOauth":{"accessToken":"external"}}"#;
        keychain::write_item(&service, &account, external).unwrap();
        let error = restore_claude_live_if_unchanged(&snapshot, applied).unwrap_err();
        assert!(error.contains("원복 충돌"));
        assert!(error.contains("다시 시도"));
        assert_eq!(
            keychain::read_item(&service, &account).unwrap().unwrap(),
            external
        );
        assert_eq!(fs::read(&legacy_file).unwrap(), applied);

        // 키체인이 외부에서 삭제됐어도 확인 뒤 파일을 지우는 동안 다른 writer가
        // 새 값을 넣을 수 있다. stale 가능성을 알리되 파일 자체는 보존한다.
        write_live_cred(&env, Provider::Claude, applied).unwrap();
        keychain::delete_item(&service, &account).unwrap();
        let error = restore_claude_live_if_unchanged(&snapshot, applied).unwrap_err();
        assert!(error.contains("원복 충돌"));
        assert!(keychain::read_item(&service, &account).unwrap().is_none());
        assert_eq!(fs::read(&legacy_file).unwrap(), applied);

        // 반대로 파일만 외부에서 바뀐 경우도 CAS 없는 키체인을 삭제하지 않는다.
        write_live_cred(&env, Provider::Claude, applied).unwrap();
        let external_file = br#"{"claudeAiOauth":{"accessToken":"external-file"}}"#;
        fs::write(&legacy_file, external_file).unwrap();
        let error = restore_claude_live_if_unchanged(&snapshot, applied).unwrap_err();
        assert!(error.contains("원복 충돌"));
        assert_eq!(
            keychain::read_item(&service, &account).unwrap().unwrap(),
            applied
        );
        assert_eq!(fs::read(&legacy_file).unwrap(), external_file);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_mixed_rollback_marker_blocks_reads_and_switch_retry_converges_without_backup() {
        use std::os::unix::fs::PermissionsExt;

        let mut env = test_env("switch-macos-persistent-recovery");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b");
        save_current(&env, Provider::Claude, "second").unwrap();
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a");
        save_current(&env, Provider::Claude, "main").unwrap();

        let main_before = profile_bundle_bytes(&env, "main");
        let second_before = profile_bundle_bytes(&env, "second");

        let service = format!("switcher-switch-recovery-{}", std::process::id());
        let account = keychain::username();
        let _guard = MacKeychainGuard {
            service: service.clone(),
            account: account.clone(),
        };
        let legacy_file = env.live_credential_path(Provider::Claude);
        let active_a = fs::read(&legacy_file).unwrap();
        keychain::write_item(&service, &account, &active_a).unwrap();
        env.claude_live = ClaudeLiveStore::Keychain {
            service: service.clone(),
            account: account.clone(),
            legacy_file: legacy_file.clone(),
        };

        let live_before_apply = snapshot_claude_live(&env).unwrap();
        let target_b = fs::read(
            env.profiles_dir(Provider::Claude)
                .join("second")
                .join("credentials.json"),
        )
        .unwrap();
        write_live_cred(&env, Provider::Claude, &target_b).unwrap();
        // keychain=A, legacy=B가 되어 원복 conflict를 재현한다.
        keychain::write_item(&service, &account, &active_a).unwrap();
        let error = restore_claude_live_if_unchanged(&live_before_apply, &target_b).unwrap_err();
        assert!(error.contains("원복 충돌"));

        let marker = claude_recovery_marker_path(&env);
        assert_eq!(fs::read(&marker).unwrap(), b"switch-recovery-v1\n");
        assert_eq!(
            fs::metadata(&marker).unwrap().permissions().mode() & 0o777,
            0o600
        );

        // 새 프로세스가 같은 홈·store를 연 것처럼 Env를 다시 만들어도 차단은 유지된다.
        let restarted = Env {
            home: env.home.clone(),
            store: env.store.clone(),
            claude_live: ClaudeLiveStore::Keychain {
                service: service.clone(),
                account: account.clone(),
                legacy_file: legacy_file.clone(),
            },
        };
        assert!(claude_recovery_required(&restarted).unwrap());
        let read_error = read_live_cred(&restarted, Provider::Claude).unwrap_err();
        assert!(read_error.contains("복구가 필요"));
        assert!(live_cred_exists(&restarted, Provider::Claude).is_err());
        assert!(save_current(&restarted, Provider::Claude, "must-not-save").is_err());
        assert!(!restarted
            .profiles_dir(Provider::Claude)
            .join("must-not-save")
            .exists());

        let result = switch(&restarted, Provider::Claude, "second").unwrap();
        assert_eq!(result.backed_up_to, None);
        assert!(!marker.exists());
        assert!(!claude_recovery_required(&restarted).unwrap());
        assert_eq!(
            keychain::read_item(&service, &account).unwrap().unwrap(),
            target_b
        );
        assert_eq!(fs::read(&legacy_file).unwrap(), target_b);
        assert_eq!(
            read_json(&restarted.claude_json_path()).unwrap()["oauthAccount"]["accountUuid"],
            "uuid-b"
        );
        assert_eq!(profile_bundle_bytes(&restarted, "main"), main_before);
        assert_eq!(profile_bundle_bytes(&restarted, "second"), second_before);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_recovery_marker_can_apply_target_when_all_live_credentials_are_missing() {
        let mut env = test_env("switch-macos-recovery-from-empty");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b");
        save_current(&env, Provider::Claude, "second").unwrap();
        let target = fs::read(
            env.profiles_dir(Provider::Claude)
                .join("second")
                .join("credentials.json"),
        )
        .unwrap();

        let service = format!("switcher-switch-recovery-empty-{}", std::process::id());
        let account = keychain::username();
        let _guard = MacKeychainGuard {
            service: service.clone(),
            account: account.clone(),
        };
        let legacy_file = env.live_credential_path(Provider::Claude);
        let _ = fs::remove_file(&legacy_file);
        keychain::delete_item(&service, &account).unwrap();
        env.claude_live = ClaudeLiveStore::Keychain {
            service: service.clone(),
            account: account.clone(),
            legacy_file: legacy_file.clone(),
        };
        mark_claude_recovery_required(&claude_recovery_marker_path(&env)).unwrap();

        let result = switch(&env, Provider::Claude, "second").unwrap();

        assert_eq!(result.backed_up_to, None);
        assert!(!claude_recovery_required(&env).unwrap());
        assert_eq!(
            keychain::read_item(&service, &account).unwrap().unwrap(),
            target
        );
        assert_eq!(fs::read(&legacy_file).unwrap(), target);
    }

    #[test]
    fn missing_recovery_marker_during_clear_is_recreated_fail_closed() {
        let env = test_env("recovery-marker-clear-missing");
        let error = clear_claude_recovery_marker(&env).unwrap_err();

        assert!(error.contains("예상보다 먼저 사라졌습니다"));
        assert!(claude_recovery_required(&env).unwrap());
        assert!(read_live_cred(&env, Provider::Claude).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_claude_profile_cannot_hide_a_pending_recovery_marker() {
        use std::os::unix::fs::symlink;

        let env = test_env("pending-profile-symlink");
        let outside = env.home.join("outside-pending-profile");
        fs::create_dir_all(&outside).unwrap();
        mark_claude_live_apply_pending(&outside).unwrap();
        let profiles = env.profiles_dir(Provider::Claude);
        fs::create_dir_all(&profiles).unwrap();
        symlink(&outside, profiles.join("linked")).unwrap();

        let gate = ensure_claude_recovery_not_required(&env, Provider::Claude).unwrap_err();
        assert!(gate.contains("심볼릭 링크"));
        let listing = profile_dirs(&env, Provider::Claude).unwrap_err();
        assert!(listing.contains("심볼릭 링크"));
    }

    #[test]
    fn recovery_switch_merges_pending_target_without_reading_blocked_live_state() {
        let env = test_env("recovery-marker-pending-target");
        login_claude(&env, "uuid-b", "bob@test.dev", "old-a");
        let old = br#"{"claudeAiOauth":{"accessToken":"old-a","refreshToken":"r-old","expiresAt":1000}}"#;
        fs::write(env.live_credential_path(Provider::Claude), old).unwrap();
        save_current(&env, Provider::Claude, "second").unwrap();

        let target = env
            .profiles_dir(Provider::Claude)
            .join("second")
            .join("credentials.json");
        let pending = serde_json::json!({
            "old_refresh": "r-old",
            "response": {
                "access_token": "new-a",
                "refresh_token": "r-new",
                "expires_in": 28_800
            },
            "saved_at": now()
        });
        fs::write(
            crate::usage::pending_path(&target),
            serde_json::to_vec(&pending).unwrap(),
        )
        .unwrap();
        mark_claude_recovery_required(&claude_recovery_marker_path(&env)).unwrap();

        // 혼합 상태의 oauthAccount를 자동 전환의 활성 판정에 쓰면 안 되지만,
        // 사용자가 복구 대상을 고를 수 있도록 저장 프로필은 계속 보여야 한다.
        let recovering = list(&env, Provider::Claude).unwrap();
        assert!(recovering.live.is_none());
        assert!(recovering.profiles.iter().all(|profile| !profile.active));
        assert_eq!(recovering.profiles.len(), 1);

        let result = switch(&env, Provider::Claude, "second").unwrap();

        assert_eq!(result.backed_up_to, None);
        assert!(!claude_recovery_marker_path(&env).exists());
        assert!(!crate::usage::pending_path(&target).exists());
        let profile = read_json(&target).unwrap();
        let live: Value = serde_json::from_slice(
            &read_live_cred(&env, Provider::Claude).unwrap(),
        )
        .unwrap();
        assert_eq!(profile.pointer("/claudeAiOauth/accessToken").unwrap(), "new-a");
        assert_eq!(profile.pointer("/claudeAiOauth/refreshToken").unwrap(), "r-new");
        assert_eq!(live.pointer("/claudeAiOauth/accessToken").unwrap(), "new-a");
        assert_eq!(live.pointer("/claudeAiOauth/refreshToken").unwrap(), "r-new");
    }

    #[test]
    fn recovery_marker_preflight_failure_never_starts_live_apply() {
        let env = test_env("recovery-marker-preflight-failure");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b");
        save_current(&env, Provider::Claude, "second").unwrap();
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a");
        let expected = capture_live_profile(&env, Provider::Claude).unwrap();
        let profile_dir = env.profiles_dir(Provider::Claude).join("second");
        let target = fs::read(profile_dir.join("credentials.json")).unwrap();
        let credential_before = fs::read(env.live_credential_path(Provider::Claude)).unwrap();
        let oauth_before = fs::read(env.claude_json_path()).unwrap();
        fs::create_dir_all(claude_recovery_marker_path(&env)).unwrap();

        let error = apply_claude_profile_journaled(&env, &profile_dir, &target, Some(&expected))
            .unwrap_err();

        assert!(error.contains("안전하게 기록하지 못했습니다"));
        assert!(!error.contains(&env.store.display().to_string()));
        assert_eq!(
            fs::read(env.live_credential_path(Provider::Claude)).unwrap(),
            credential_before
        );
        assert_eq!(fs::read(env.claude_json_path()).unwrap(), oauth_before);
        assert!(claude_recovery_marker_path(&env).is_dir());
    }

    #[test]
    fn journal_is_not_created_when_final_external_change_guard_fails() {
        let env = test_env("recovery-journal-after-final-guard");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b");
        save_current(&env, Provider::Claude, "second").unwrap();
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a");
        let expected = capture_live_profile(&env, Provider::Claude).unwrap();
        let profile_dir = env.profiles_dir(Provider::Claude).join("second");
        let target = fs::read(profile_dir.join("credentials.json")).unwrap();

        let error = apply_claude_profile_inner_mode(
            &env,
            &profile_dir,
            &target,
            Some(&expected),
            true,
            true,
            || login_claude(&env, "uuid-c", "carol@test.dev", "tok-c"),
            || {},
        )
        .unwrap_err();

        assert!(error.contains("외부에서 변경되었습니다"));
        assert!(live_token(&env).contains("tok-c"));
        assert_eq!(
            read_json(&env.claude_json_path()).unwrap()["oauthAccount"]["accountUuid"],
            "uuid-c"
        );
        assert!(
            !claude_recovery_marker_path(&env).exists(),
            "첫 활성 쓰기 전 guard 실패는 복구 표식을 남기면 안 된다"
        );
    }

    #[test]
    fn active_claude_relogin_preserves_external_login_after_guard_snapshot() {
        let env = test_env("active-relogin-external-login");
        login_claude(&env, "uuid-a", "alice@test.dev", "old-a");
        let expected = capture_claude_live_update_guard(&env).unwrap();
        let ident = LiveIdentity {
            id: "uuid-a".into(),
            email: Some("alice@test.dev".into()),
        };
        let fresh = br#"{"claudeAiOauth":{"accessToken":"fresh-a"}}"#;
        let oauth = serde_json::json!({
            "accountUuid": "uuid-a",
            "emailAddress": "alice@test.dev"
        });
        let profile_dir = env.profiles_dir(Provider::Claude).join("alice");
        ensure_claude_relogin_profile_identity(&env, "alice", &ident).unwrap();
        mark_claude_live_apply_pending(&profile_dir).unwrap();
        write_profile_parts_for_active_relogin(
            &env,
            Provider::Claude,
            "alice",
            &ident,
            fresh,
            Some(&oauth),
        )
        .unwrap();

        // 격리 로그인 결과를 프로필에 안전하게 보관한 직후 외부 CLI가 다른
        // 계정으로 로그인해도 그 새 활성 상태를 덮지 않아야 한다.
        login_claude(&env, "uuid-b", "bob@test.dev", "external-b");
        let error = apply_claude_live_update(&env, &profile_dir, fresh, &expected).unwrap_err();

        assert!(error.contains("외부에서 변경되었습니다"));
        assert!(live_token(&env).contains("external-b"));
        assert_eq!(
            read_json(&env.claude_json_path()).unwrap()["oauthAccount"]["accountUuid"],
            "uuid-b"
        );
        assert_eq!(fs::read(profile_dir.join("credentials.json")).unwrap(), fresh);
        assert!(!claude_recovery_marker_path(&env).exists());
        assert!(claude_live_apply_pending(&profile_dir).unwrap());
    }

    #[test]
    fn missing_live_guard_rejects_oauth_change_after_capture() {
        let env = test_env("missing-live-guard-oauth-change");
        fs::write(
            env.claude_json_path(),
            r#"{"oauthAccount":{"accountUuid":"uuid-a","emailAddress":"alice@test.dev"}}"#,
        )
        .unwrap();
        assert!(!env.live_credential_path(Provider::Claude).exists());
        let expected = capture_claude_live_update_guard(&env).unwrap();
        assert!(expected.belongs_to("uuid-a"));

        let ident = LiveIdentity {
            id: "uuid-a".into(),
            email: Some("alice@test.dev".into()),
        };
        let fresh = br#"{"claudeAiOauth":{"accessToken":"fresh-a"}}"#;
        let oauth = serde_json::json!({
            "accountUuid": "uuid-a",
            "emailAddress": "alice@test.dev"
        });
        let profile_dir = env.profiles_dir(Provider::Claude).join("alice");
        ensure_claude_relogin_profile_identity(&env, "alice", &ident).unwrap();
        mark_claude_live_apply_pending(&profile_dir).unwrap();
        write_profile_parts_for_active_relogin(
            &env,
            Provider::Claude,
            "alice",
            &ident,
            fresh,
            Some(&oauth),
        )
        .unwrap();

        // credential은 계속 없는 상태지만 외부 CLI가 oauthAccount만 B로 바꿨다.
        // "없음"만 기억한 guard라면 이 변경을 놓치고 B를 덮어쓰게 된다.
        fs::write(
            env.claude_json_path(),
            r#"{"oauthAccount":{"accountUuid":"uuid-b","emailAddress":"bob@test.dev"}}"#,
        )
        .unwrap();
        let error = apply_claude_live_update(&env, &profile_dir, fresh, &expected).unwrap_err();

        assert!(error.contains("외부에서 변경되었습니다"));
        assert!(!env.live_credential_path(Provider::Claude).exists());
        assert_eq!(
            read_json(&env.claude_json_path()).unwrap()["oauthAccount"]["accountUuid"],
            "uuid-b"
        );
        assert_eq!(fs::read(profile_dir.join("credentials.json")).unwrap(), fresh);
        assert!(claude_live_apply_pending(&profile_dir).unwrap());
    }

    #[test]
    fn adversarial_marker_create_failure_must_not_let_next_backup_erase_fresh_relogin() {
        let env = test_env("active-relogin-marker-create-failure");
        login_claude(&env, "uuid-a", "alice@test.dev", "old-live-token");
        save_current(&env, Provider::Claude, "alice").unwrap();
        let expected = capture_claude_live_update_guard(&env).unwrap();
        let profile_dir = env.profiles_dir(Provider::Claude).join("alice");
        let ident = LiveIdentity {
            id: "uuid-a".into(),
            email: Some("alice@test.dev".into()),
        };
        let fresh = br#"{"claudeAiOauth":{"accessToken":"only-fresh-relogin-copy"}}"#;
        let oauth = serde_json::json!({
            "accountUuid": "uuid-a",
            "emailAddress": "alice@test.dev"
        });
        mark_claude_live_apply_pending(&profile_dir).unwrap();
        write_profile_parts_for_active_relogin(
            &env,
            Provider::Claude,
            "alice",
            &ident,
            fresh,
            Some(&oauth),
        )
        .unwrap();

        // guard 스냅숏과 profile 저장 뒤 root journal 위치가 일시적으로 망가져
        // 첫 live write 전에 실패하는 상황을 합성한다.
        fs::create_dir_all(claude_recovery_marker_path(&env)).unwrap();
        assert!(
            apply_claude_live_update(&env, &profile_dir, fresh, &expected).is_err()
        );
        assert_eq!(fs::read(profile_dir.join("credentials.json")).unwrap(), fresh);
        assert!(live_token(&env).contains("old-live-token"));
        assert!(claude_live_apply_pending(&profile_dir).unwrap());

        // 파일시스템 장애가 풀린 다음 일반 전환이 들어와도 old live를 먼저
        // 백업해 fresh profile을 지우지 않고, pending target을 복구 모드로 적용한다.
        fs::remove_dir(claude_recovery_marker_path(&env)).unwrap();
        let recovering = list(&env, Provider::Claude).unwrap();
        assert!(recovering.live.is_none());
        assert!(recovering.profiles.iter().all(|profile| !profile.active));
        let result = switch(&env, Provider::Claude, "alice").unwrap();

        assert_eq!(result.backed_up_to, None);
        assert_eq!(fs::read(profile_dir.join("credentials.json")).unwrap(), fresh);
        assert_eq!(read_live_cred(&env, Provider::Claude).unwrap(), fresh);
        assert!(!claude_live_apply_pending(&profile_dir).unwrap());
        assert!(!claude_recovery_marker_path(&env).exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_claude_partial_write_restores_keychain() {
        let mut env = test_env("switch-macos-keychain-partial-write");
        let service = format!("switcher-switch-partial-{}", std::process::id());
        let account = keychain::username();
        let _guard = MacKeychainGuard {
            service: service.clone(),
            account: account.clone(),
        };
        let before = br#"{"claudeAiOauth":{"accessToken":"before"}}"#;
        let applied = br#"{"claudeAiOauth":{"accessToken":"applied"}}"#;
        keychain::write_item(&service, &account, before).unwrap();
        env.claude_live = ClaudeLiveStore::Keychain {
            service: service.clone(),
            account: account.clone(),
            legacy_file: PathBuf::from("invalid\0legacy-credential"),
        };

        assert!(write_live_cred(&env, Provider::Claude, applied).is_err());
        assert_eq!(
            keychain::read_item(&service, &account).unwrap().unwrap(),
            before
        );
    }

    #[test]
    fn claude_switch_creates_missing_live_identity_file() {
        let env = test_env("switch-missing-identity");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b1");
        save_current(&env, Provider::Claude, "second").unwrap();
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        fs::remove_file(env.claude_json_path()).unwrap();

        switch(&env, Provider::Claude, "second").unwrap();

        let root = read_json(&env.claude_json_path()).unwrap();
        assert_eq!(root["oauthAccount"]["accountUuid"], "uuid-b");
        assert_eq!(root["oauthAccount"]["emailAddress"], "bob@test.dev");
    }

    #[test]
    fn claude_switch_rejects_non_object_identity_and_restores_credential() {
        let env = test_env("switch-non-object-identity");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b1");
        save_current(&env, Provider::Claude, "second").unwrap();
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        let live_before = fs::read(env.live_credential_path(Provider::Claude)).unwrap();
        fs::write(env.claude_json_path(), b"[]").unwrap();

        assert!(switch(&env, Provider::Claude, "second").is_err());

        assert_eq!(
            fs::read(env.live_credential_path(Provider::Claude)).unwrap(),
            live_before
        );
        assert_eq!(fs::read(env.claude_json_path()).unwrap(), b"[]");
    }

    #[test]
    fn claude_restore_preserves_credential_changed_after_target_write() {
        let env = test_env("switch-conditional-rollback");
        let path = env.live_credential_path(Provider::Claude);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"old credential fixture").unwrap();
        let snapshot = snapshot_claude_live(&env).unwrap();
        let target = b"target credential fixture";
        fs::write(&path, target).unwrap();
        fs::write(&path, b"external credential fixture").unwrap();

        restore_claude_live_if_unchanged(&snapshot, target).unwrap();

        assert_eq!(fs::read(path).unwrap(), b"external credential fixture");
    }

    #[test]
    fn credential_equivalence_accepts_hex_wrapped_json() {
        let raw = br#"{"claudeAiOauth":{"accessToken":"fixture"}}"#;
        let hex = raw
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        assert!(credential_equivalent(raw, hex.as_bytes()));
    }

    #[test]
    fn claude_switch_rejects_and_preserves_a_concurrent_external_login() {
        let env = test_env("switch-concurrent-external-login");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b");
        save_current(&env, Provider::Claude, "second").unwrap();
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a");
        let expected = capture_live_profile(&env, Provider::Claude).unwrap();
        let profile_dir = env.profiles_dir(Provider::Claude).join("second");
        let target = fs::read(profile_dir.join("credentials.json")).unwrap();

        let error = apply_claude_profile_inner(
            &env,
            &profile_dir,
            &target,
            Some(&expected),
            || {},
            || login_claude(&env, "uuid-c", "carol@test.dev", "tok-c"),
        )
        .unwrap_err();

        assert!(error.contains("외부에서 변경되었습니다"));
        assert!(live_token(&env).contains("tok-c"));
        assert_eq!(
            read_json(&env.claude_json_path()).unwrap()["oauthAccount"]["accountUuid"],
            "uuid-c"
        );
    }

    #[test]
    fn claude_switch_rejects_an_external_login_before_target_write() {
        let env = test_env("switch-external-login-before-write");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b");
        save_current(&env, Provider::Claude, "second").unwrap();
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a");
        let expected = capture_live_profile(&env, Provider::Claude).unwrap();
        let profile_dir = env.profiles_dir(Provider::Claude).join("second");
        let target = fs::read(profile_dir.join("credentials.json")).unwrap();

        let error = apply_claude_profile_inner(
            &env,
            &profile_dir,
            &target,
            Some(&expected),
            || login_claude(&env, "uuid-c", "carol@test.dev", "tok-c"),
            || {},
        )
        .unwrap_err();

        assert!(error.contains("외부에서 변경되었습니다"));
        assert!(live_token(&env).contains("tok-c"));
        assert_eq!(
            read_json(&env.claude_json_path()).unwrap()["oauthAccount"]["accountUuid"],
            "uuid-c"
        );
    }

    #[test]
    fn claude_oauth_rollback_preserves_unrelated_settings() {
        let env = test_env("switch-oauth-unrelated-setting");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b");
        save_current(&env, Provider::Claude, "second").unwrap();
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a");
        let profile_dir = env.profiles_dir(Provider::Claude).join("second");
        let prepared = prepare_claude_oauth_apply(&env, &profile_dir).unwrap();

        apply_prepared_claude_oauth(&prepared).unwrap();
        let mut current = read_json(&env.claude_json_path()).unwrap();
        current["theme"] = Value::String("changed-externally".into());
        atomic_write(
            &env.claude_json_path(),
            &serde_json::to_vec_pretty(&current).unwrap(),
        )
        .unwrap();

        assert!(prepared_claude_oauth_matches(&prepared).unwrap());
        restore_prepared_claude_oauth_if_unchanged(&prepared).unwrap();
        let restored = read_json(&env.claude_json_path()).unwrap();
        assert_eq!(restored["oauthAccount"]["accountUuid"], "uuid-a");
        assert_eq!(restored["theme"], "changed-externally");
    }

    #[test]
    fn claude_credential_rollback_accepts_equivalent_json_bytes() {
        let env = test_env("switch-equivalent-credential-rollback");
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a");
        let snapshot = snapshot_claude_live(&env).unwrap();
        let applied = br#"{"claudeAiOauth":{"accessToken":"tok-b","expiresAt":2}}"#;
        write_live_cred(&env, Provider::Claude, applied).unwrap();
        atomic_write(
            &env.live_credential_path(Provider::Claude),
            br#"{ "claudeAiOauth": { "expiresAt": 2, "accessToken": "tok-b" } }"#,
        )
        .unwrap();

        restore_claude_live_if_unchanged(&snapshot, applied).unwrap();

        assert!(live_token(&env).contains("tok-a"));
    }

    #[test]
    fn captured_backup_snapshot_ignores_a_later_login() {
        let env = test_env("captured-backup-snapshot");
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a");
        let snapshot = capture_live_profile(&env, Provider::Claude).unwrap();
        let identity = snapshot.identity.as_ref().unwrap();
        login_claude(&env, "uuid-c", "carol@test.dev", "tok-c");

        write_profile_snapshot(&env, Provider::Claude, "captured", identity, &snapshot).unwrap();

        let dir = env.profiles_dir(Provider::Claude).join("captured");
        assert!(fs::read_to_string(dir.join("credentials.json"))
            .unwrap()
            .contains("tok-a"));
        assert_eq!(
            read_json(&dir.join("oauth_account.json")).unwrap()["accountUuid"],
            "uuid-a"
        );
        assert_eq!(read_meta(&dir).unwrap().id, "uuid-a");
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
    fn codex_switch_rejects_an_external_login_before_target_write() {
        let env = test_env("codex-external-login-before-write");
        login_codex(&env, "acct-b", "bob@test.dev", "ctok-b");
        save_current(&env, Provider::Codex, "second").unwrap();
        login_codex(&env, "acct-a", "alice@test.dev", "ctok-a");
        let expected = capture_live_profile(&env, Provider::Codex).unwrap();
        let target = fs::read(
            env.profiles_dir(Provider::Codex)
                .join("second")
                .join("auth.json"),
        )
        .unwrap();

        let error = apply_codex_profile_inner(
            &env,
            &target,
            Some(&expected),
            || login_codex(&env, "acct-c", "carol@test.dev", "ctok-c"),
            || {},
        )
        .unwrap_err();

        assert!(error.contains("외부에서 변경되었습니다"));
        assert_eq!(
            live_identity(&env, Provider::Codex)
                .unwrap()
                .unwrap()
                .id,
            "acct-c"
        );
        assert!(fs::read_to_string(env.live_credential_path(Provider::Codex))
            .unwrap()
            .contains("ctok-c"));
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
        if !live_cred_exists(&env, provider).expect("활성 인증정보 존재 확인 실패") {
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

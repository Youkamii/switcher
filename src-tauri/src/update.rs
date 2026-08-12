//! 자동 업데이트 (Windows·macOS).
//!
//! GitHub 릴리스 latest를 확인해 새 버전이면 zip을 받아 검증·준비한다. macOS는
//! 바로 원자 교체하고, Windows는 종료 뒤 helper 교체에 필요한 경로를 돌려준다 —
//! 이후는 호출부 몫이다:
//! - 시작 시 자동 확인: 교체만 해 두고 **다음 실행부터** 반영 (켜자마자 재시작
//!   루프를 돌지 않게 — update-ready 토스트로 알린다)
//! - 트레이 "업데이트 확인": 지금 업데이트하겠다는 명시적 의사이므로 교체 후
//!   **즉시 재시작**한다 (lib.rs restart_into — 사용자 지시 2026-08-10)
//!
//! 교체 원리 (양쪽 다 느린 복사를 먼저 끝내고 OS의 원자 교체를 사용한다):
//! - Windows: 실행 중인 exe를 직접 덮지 않는다. 검증한 `.new`와 작은 helper 복사본을
//!   준비하고, 현재 프로세스가 끝난 뒤 helper가 `MoveFileExW(REPLACE_EXISTING)`로
//!   교체한 다음 새 exe를 실행한다.
//! - macOS: 새 번들을 `switcher.app.new`로 준비한 뒤 `renamex_np(RENAME_SWAP)`로
//!   현재 번들과 한 번에 맞바꾼다.
//!   ditto로 풀고 복사해 애드혹 서명·심링크가 보존된다 (npm 채널과 같은 경로).
//!   `.old`는 그 시점엔 실행 중이라 지울 수 없으므로 다음 시작 때 치운다.
#![cfg(any(windows, target_os = "macos"))]
// dev 빌드는 확인 자체를 건너뛰므로(lib.rs, debug_assertions) 본체가 미사용으로 잡힌다
#![cfg_attr(debug_assertions, allow(dead_code))]

use std::fs;
use std::path::{Path, PathBuf};

const RELEASE_LATEST_API: &str = "https://api.github.com/repos/Youkamii/switcher/releases/latest";
#[cfg(windows)]
const ASSET_NAME: &str = "switcher-win-x64.zip";
#[cfg(target_os = "macos")]
const ASSET_NAME: &str = "switcher-mac-arm64.zip";
/// 자산 URL은 반드시 이 저장소의 릴리스 다운로드 경로여야 한다 — API 응답이 어떤 이유로든
/// 다른 호스트를 가리켜도 따라가지 않는다 (업데이트 채널의 신뢰 뿌리를 저장소 하나로 고정)
const ASSET_URL_PREFIX: &str = "https://github.com/Youkamii/switcher/releases/download/";
#[cfg(windows)]
const VERSION_FILE: &str = "switcher-version.txt";

/// "v1.2.3" / "1.2.3-rc1" → (1, 2, 3). 해석 불가면 None (업데이트를 건너뛴다).
fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    fn number(part: &str) -> Option<u64> {
        if part.is_empty()
            || !part.bytes().all(|byte| byte.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return None;
        }
        part.parse().ok()
    }

    fn identifiers(value: &str) -> bool {
        !value.is_empty()
            && value.split('.').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
    }

    let value = s.trim();
    let value = value.strip_prefix('v').unwrap_or(value);
    let suffix_at = value
        .char_indices()
        .find_map(|(index, ch)| (ch == '-' || ch == '+').then_some(index));
    let (core, suffix) = match suffix_at {
        Some(index) => (&value[..index], Some(&value[index..])),
        None => (value, None),
    };
    if let Some(suffix) = suffix {
        let valid = if let Some(rest) = suffix.strip_prefix('-') {
            match rest.split_once('+') {
                Some((pre, build)) => identifiers(pre) && identifiers(build),
                None => identifiers(rest),
            }
        } else {
            suffix
                .strip_prefix('+')
                .is_some_and(identifiers)
        };
        if !valid {
            return None;
        }
    }
    let mut parts = core.split('.');
    let major = number(parts.next()?)?;
    let minor = number(parts.next()?)?;
    let patch = number(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// 지난 업데이트가 남긴 잔재 청소: 옛 실행 파일(교체 당시엔 그 프로세스가 살아 있어
/// 못 지운다), 중단된 준비 파일, 실패한 업데이트의 임시 폴더(pid 키라 그 실행만 안다)
pub fn sweep_old_exe() {
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        let staged = exe.with_extension("exe.new");
        // 다음 실행에서 적용할 검증된 pending은 보존한다. 표식이 없거나 현재보다
        // 새 버전이 아니면 중단된 준비 파일이므로 함께 치운다.
        if pending_windows_update(&exe).is_none() {
            let _ = fs::remove_file(&staged);
            let _ = fs::remove_file(pending_version_path(&staged));
        }
        // .old 계열 전부 — 교체가 잠긴 .old를 피해 옆 이름(.old<pid>)으로
        // 비켜둔 잔재까지 청소한다 (apply의 대체 이름 방어와 짝)
        if let (Some(dir), Some(name)) = (exe.parent(), exe.file_name()) {
            let prefix = format!("{}.old", name.to_string_lossy());
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if entry.file_name().to_string_lossy().starts_with(&prefix) {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(bundle) = current_bundle() {
        let _ = fs::remove_dir_all(bundle.with_extension("app.old"));
        let _ = fs::remove_dir_all(bundle.with_extension("app.new"));
    }
    if let Ok(entries) = fs::read_dir(std::env::temp_dir()) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with("switcher-update-")
            {
                let path = entry.path();
                if path.is_dir() {
                    let _ = fs::remove_dir_all(path);
                } else {
                    let _ = fs::remove_file(path);
                }
            }
        }
    }
}

/// 실행 파일에서 자기 .app 번들 루트를 찾는다 — 번들 밖 실행(개발 등)이면 None
#[cfg(target_os = "macos")]
fn current_bundle() -> Option<PathBuf> {
    // .../switcher.app/Contents/MacOS/switcher → 세 단계 위가 번들 루트
    let exe = std::env::current_exe().ok()?;
    let bundle = exe.parent()?.parent()?.parent()?;
    if bundle.extension()? == "app" {
        Some(bundle.to_path_buf())
    } else {
        None
    }
}

/// 새 버전 확인 → 다운로드 → 제자리 교체. 교체했으면 새 버전 문자열을 돌려준다.
/// 자동(시작 시)·수동(트레이) 확인의 전 구간 직렬화 — 겹치면 같은 임시 폴더
/// (switcher-update-<pid>)와 같은 파일을 두 경로가 동시에 만져 교체가 깨진다
static UPDATE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 이번 프로세스가 이미 받아 교체해 둔 업데이트 (버전, 재실행 경로).
/// 시작 시 자동 확인이 교체를 끝낸 뒤 수동 확인이 오면, 같은 파일을 또
/// 교체하려다 .old(실행 중인 우리 이미지 — 삭제 잠김)에 막혀 항상 실패했다
/// (실측 2026-08-10: "업데이트 확인이 안 먹힘" — .new 잔존이 물증). 이미 적용된
/// 버전이면 재교체 없이 이 경로로 재시작만 하면 된다.
static APPLIED: std::sync::Mutex<
    Option<(String, std::path::PathBuf, Option<std::path::PathBuf>)>,
> =
    std::sync::Mutex::new(None);

pub enum UpdateOutcome {
    Current { version: String },
    Applied {
        version: String,
        relaunch: std::path::PathBuf,
        /// Windows는 실행 중 파일 잠금 때문에 종료 뒤 helper가 이 경로를 교체한다.
        /// macOS는 이미 swap을 마쳤으므로 None이다.
        replace_target: Option<std::path::PathBuf>,
    },
}

/// API 응답에서 이 플랫폼의 새 자산만 고른다. 판정과 URL 검증을 네트워크·파일
/// 교체에서 분리해 "동일 버전", "새 버전", "깨진 릴리스"를 회귀 테스트한다.
fn release_plan(
    release: &serde_json::Value,
    current: (u64, u64, u64),
) -> Result<Option<(String, String)>, String> {
    let tag = release["tag_name"].as_str().unwrap_or_default();
    let latest = parse_version(tag)
        .ok_or_else(|| format!("업데이트 태그를 해석할 수 없습니다: {tag:?}"))?;
    if latest <= current {
        return Ok(None);
    }
    let version = tag.trim_start_matches('v').to_string();
    let url = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|asset| asset["name"].as_str() == Some(ASSET_NAME))
        .and_then(|asset| asset["browser_download_url"].as_str())
        .ok_or("최신 릴리스에 이 OS용 빌드 자산이 없습니다")?
        .to_string();
    if !url.starts_with(ASSET_URL_PREFIX) {
        return Err(format!("업데이트 자산 주소가 예상 밖입니다: {url}"));
    }
    Ok(Some((version, url)))
}

/// 성공 시 (버전, 재실행에 쓸 실행 파일 경로). 경로는 교체를 마친 시점에 apply가
/// 직접 계산한 것 — 교체 후 current_exe()가 rename을 따라가 .old를 가리키는
/// 플랫폼 미묘함에 걸지 않기 위함이다.
pub async fn check_and_apply() -> Result<UpdateOutcome, String> {
    let _gate = UPDATE_LOCK.lock().await;
    // 자동 확인이 이미 교체를 끝냈다면 수동 버튼은 네트워크 상태와 무관하게 바로
    // 그 파일로 재시작해야 한다. API 요청을 먼저 하면 오프라인에서 재시작까지 막힌다.
    if let Ok(guard) = APPLIED.lock() {
        if let Some((version, relaunch, replace_target)) = guard.as_ref() {
            return Ok(UpdateOutcome::Applied {
                version: version.clone(),
                relaunch: relaunch.clone(),
                replace_target: replace_target.clone(),
            });
        }
    }
    let current_text = env!("CARGO_PKG_VERSION").to_string();
    let current = parse_version(&current_text).ok_or("현재 버전을 해석할 수 없습니다")?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("HTTP 클라이언트 생성 실패: {e}"))?;
    // GitHub API는 User-Agent 없는 요청을 거부한다
    let release: serde_json::Value = client
        .get(RELEASE_LATEST_API)
        .header("User-Agent", "switcher-widget")
        .send()
        .await
        .map_err(|e| format!("업데이트 확인 실패: {e}"))?
        .error_for_status()
        .map_err(|e| format!("업데이트 확인 실패: {e}"))?
        .json()
        .await
        .map_err(|e| format!("업데이트 응답 해석 실패: {e}"))?;
    let Some((expected, url)) = release_plan(&release, current)? else {
        return Ok(UpdateOutcome::Current {
            version: current_text,
        });
    };
    let bytes = client
        .get(&url)
        .header("User-Agent", "switcher-widget")
        .send()
        .await
        .map_err(|e| format!("업데이트 다운로드 실패: {e}"))?
        .error_for_status()
        .map_err(|e| format!("업데이트 다운로드 실패: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("업데이트 다운로드 실패: {e}"))?;
    // 파일·프로세스 작업은 블로킹 풀에서
    let version = expected.clone();
    let (relaunch, replace_target) =
        tauri::async_runtime::spawn_blocking(move || apply(&bytes, &version))
        .await
        .map_err(|e| format!("업데이트 적용 작업 실패: {e}"))??;
    if let Ok(mut guard) = APPLIED.lock() {
        *guard = Some((
            expected.clone(),
            relaunch.clone(),
            replace_target.clone(),
        ));
    }
    Ok(UpdateOutcome::Applied {
        version: expected,
        relaunch,
        replace_target,
    })
}

/// zip을 임시 폴더에 풀고 현재 실행 파일 옆에 검증된 새 exe를 준비한다.
/// 반환은 (준비 파일, helper가 교체할 현재 exe).
#[cfg(windows)]
fn apply(
    zip_bytes: &[u8],
    expected_version: &str,
) -> Result<(std::path::PathBuf, Option<std::path::PathBuf>), String> {
    let work = std::env::temp_dir().join(format!("switcher-update-{}", std::process::id()));
    let current = std::env::current_exe().map_err(|e| format!("현재 경로 확인 실패: {e}"))?;
    let staged = apply_windows_to(zip_bytes, &current, &work, expected_version)?;
    Ok((staged, Some(current)))
}

#[cfg(windows)]
fn windows_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn windows_binary_version(path: &Path) -> Result<(u64, u64, u64), String> {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FixedFileInfo {
        signature: u32,
        struct_version: u32,
        file_version_ms: u32,
        file_version_ls: u32,
        product_version_ms: u32,
        product_version_ls: u32,
        file_flags_mask: u32,
        file_flags: u32,
        file_os: u32,
        file_type: u32,
        file_subtype: u32,
        file_date_ms: u32,
        file_date_ls: u32,
    }

    #[link(name = "Version")]
    extern "system" {
        fn GetFileVersionInfoSizeW(file: *const u16, handle: *mut u32) -> u32;
        fn GetFileVersionInfoW(
            file: *const u16,
            handle: u32,
            len: u32,
            data: *mut std::ffi::c_void,
        ) -> i32;
        fn VerQueryValueW(
            block: *const std::ffi::c_void,
            sub_block: *const u16,
            value: *mut *mut std::ffi::c_void,
            len: *mut u32,
        ) -> i32;
    }

    let path = windows_path(path);
    let mut ignored = 0u32;
    let size = unsafe { GetFileVersionInfoSizeW(path.as_ptr(), &mut ignored) };
    if size == 0 {
        return Err(format!(
            "받은 실행 파일 버전 크기 확인 실패: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut data = vec![0u8; size as usize];
    if unsafe {
        GetFileVersionInfoW(
            path.as_ptr(),
            0,
            size,
            data.as_mut_ptr().cast::<std::ffi::c_void>(),
        )
    } == 0
    {
        return Err(format!(
            "받은 실행 파일 버전 읽기 실패: {}",
            std::io::Error::last_os_error()
        ));
    }
    let root = ['\\' as u16, 0];
    let mut value = std::ptr::null_mut();
    let mut value_len = 0u32;
    if unsafe {
        VerQueryValueW(
            data.as_ptr().cast::<std::ffi::c_void>(),
            root.as_ptr(),
            &mut value,
            &mut value_len,
        )
    } == 0
        || value.is_null()
        || value_len < std::mem::size_of::<FixedFileInfo>() as u32
    {
        return Err("받은 실행 파일에 고정 버전 정보가 없습니다".into());
    }
    let info = unsafe { std::ptr::read_unaligned(value.cast::<FixedFileInfo>()) };
    if info.signature != 0xFEEF_04BD {
        return Err("받은 실행 파일의 버전 정보가 올바르지 않습니다".into());
    }
    let major = (info.file_version_ms >> 16) as u64;
    let minor = (info.file_version_ms & 0xffff) as u64;
    let patch = (info.file_version_ls >> 16) as u64;
    let revision = info.file_version_ls & 0xffff;
    if revision != 0 {
        return Err(format!(
            "받은 실행 파일에 예상하지 않은 네 번째 버전 값이 있습니다: {major}.{minor}.{patch}.{revision}"
        ));
    }
    Ok((major, minor, patch))
}

#[cfg(windows)]
fn move_file_replace_atomic(staged: &Path, current: &Path) -> Result<(), String> {
    #[link(name = "Kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new_name: *const u16, flags: u32) -> i32;
    }

    let staged = windows_path(staged);
    let current = windows_path(current);
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
    let ok = unsafe {
        MoveFileExW(
            staged.as_ptr(),
            current.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok != 0 {
        Ok(())
    } else {
        Err(format!(
            "실행 파일 원자 교체 실패: {}",
            std::io::Error::last_os_error()
        ))
    }
}

#[cfg(windows)]
fn available_backup_path(current: &Path) -> Result<PathBuf, String> {
    let primary = current.with_extension("exe.old");
    match fs::remove_file(&primary) {
        Ok(()) => return Ok(primary),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(primary),
        Err(_) => {}
    }
    for attempt in 0..100 {
        let candidate = current.with_extension(format!(
            "exe.old{}-{attempt}",
            std::process::id()
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("업데이트 롤백 파일 경로를 준비하지 못했습니다".into())
}

#[cfg(windows)]
fn pending_version_path(staged: &Path) -> PathBuf {
    staged.with_extension("new.version")
}

#[cfg(windows)]
fn read_pending_version(staged: &Path) -> Result<String, String> {
    let path = pending_version_path(staged);
    let version = fs::read_to_string(&path)
        .map_err(|e| format!("pending 업데이트 버전 읽기 실패 {}: {e}", path.display()))?;
    let version = version.trim().trim_start_matches('\u{feff}');
    parse_version(version)
        .ok_or_else(|| "pending 업데이트 버전 형식이 올바르지 않습니다".to_string())?;
    Ok(version.to_string())
}

#[cfg(windows)]
fn pending_windows_update(current: &Path) -> Option<(PathBuf, String)> {
    let staged = current.with_extension("exe.new");
    if fs::metadata(&staged).ok()?.len() < 1_000_000 {
        return None;
    }
    let version = read_pending_version(&staged).ok()?;
    let pending = parse_version(&version)?;
    let running = parse_version(env!("CARGO_PKG_VERSION"))?;
    (pending > running).then_some((staged, version))
}

#[cfg(windows)]
fn finalize_windows_to(
    staged: &Path,
    current: &Path,
    expected_version: &str,
) -> Result<PathBuf, String> {
    finalize_windows_to_with_version(
        staged,
        current,
        expected_version,
        windows_binary_version,
    )
}

#[cfg(windows)]
fn finalize_windows_to_with_version(
    staged: &Path,
    current: &Path,
    expected_version: &str,
    inspect_version: impl FnOnce(&Path) -> Result<(u64, u64, u64), String>,
) -> Result<PathBuf, String> {
    let got = read_pending_version(staged)?;
    if got != expected_version {
        return Err(format!(
            "pending 업데이트 버전({got})이 helper 버전({expected_version})과 다릅니다"
        ));
    }
    let expected = parse_version(expected_version)
        .ok_or_else(|| format!("helper 버전을 해석할 수 없습니다: {expected_version}"))?;
    let binary_version = inspect_version(staged)?;
    if binary_version != expected {
        return Err(format!(
            "pending 실행 파일 버전({}.{}.{})이 helper 버전({expected_version})과 다릅니다",
            binary_version.0, binary_version.1, binary_version.2
        ));
    }
    if fs::metadata(staged)
        .map_err(|e| format!("pending 실행 파일 확인 실패: {e}"))?
        .len()
        < 1_000_000
    {
        return Err("pending 실행 파일이 비정상적으로 작습니다".into());
    }
    if !current.is_file() {
        return Err(format!("교체할 기존 실행 파일이 없습니다: {}", current.display()));
    }

    let old = available_backup_path(current)?;
    fs::copy(current, &old).map_err(|e| format!("롤백 실행 파일 준비 실패: {e}"))?;
    fs::OpenOptions::new()
        .write(true)
        .open(&old)
        .and_then(|file| file.sync_all())
        .map_err(|e| format!("롤백 실행 파일 동기화 실패: {e}"))?;
    // helper는 별도 복사본에서 실행되고 있어 staged와 current 모두 닫힌 상태다.
    // 같은 폴더·볼륨의 replace rename 한 번으로 경로가 비는 구간 없이 바꾼다.
    move_file_replace_atomic(staged, current)?;
    let _ = fs::remove_file(pending_version_path(staged));
    Ok(old)
}

#[cfg(windows)]
fn helper_path() -> PathBuf {
    static HELPER_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = HELPER_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "switcher-update-helper-{}-{seq}.exe",
        std::process::id()
    ))
}

/// 현재 앱이 끝난 뒤 교체를 맡을 별도 exe 복사본을 띄운다. helper는 Tauri를
/// 초기화하지 않고 run() 첫머리에서 교체·재실행만 하고 끝난다.
#[cfg(windows)]
pub fn spawn_windows_update_helper(
    staged: &Path,
    current: &Path,
    wait_pid: u32,
) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    let expected_staged = current.with_extension("exe.new");
    if staged != expected_staged {
        return Err("pending 업데이트 경로가 예상과 다릅니다".into());
    }
    let pending = read_pending_version(staged)?;
    let running = parse_version(env!("CARGO_PKG_VERSION"))
        .ok_or("현재 버전을 해석할 수 없습니다")?;
    if parse_version(&pending).is_none_or(|version| version <= running) {
        return Err("pending 업데이트가 현재 버전보다 새 버전이 아닙니다".into());
    }

    let helper = helper_path();
    let _ = fs::remove_file(&helper);
    fs::copy(staged, &helper).map_err(|e| format!("업데이트 helper 준비 실패: {e}"))?;
    fs::OpenOptions::new()
        .write(true)
        .open(&helper)
        .and_then(|file| file.sync_all())
        .map_err(|e| format!("업데이트 helper 동기화 실패: {e}"))?;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let spawn = std::process::Command::new(&helper)
        .env("SWITCHER_WAIT_PID", wait_pid.to_string())
        .env("SWITCHER_UPDATE_HELPER", "1")
        .env("SWITCHER_UPDATE_STAGED", staged)
        .env("SWITCHER_UPDATE_TARGET", current)
        .current_dir(current.parent().unwrap_or_else(|| Path::new(".")))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    match spawn {
        Ok(_) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&helper);
            Err(format!("업데이트 helper 실행 실패: {error}"))
        }
    }
}

/// 다음 실행에 적용하도록 남겨 둔 자동 업데이트가 있으면 helper를 띄우고 true.
/// 호출자는 Tauri 초기화 전에 바로 return해 현재 exe의 파일 잠금을 풀어야 한다.
#[cfg(windows)]
pub fn launch_pending_windows_update() -> bool {
    let Ok(current) = std::env::current_exe() else {
        return false;
    };
    let Some((staged, _)) = pending_windows_update(&current) else {
        return false;
    };
    match spawn_windows_update_helper(&staged, &current, std::process::id()) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("pending 업데이트 helper 시작 실패: {error}");
            false
        }
    }
}

/// SWITCHER_UPDATE_HELPER로 시작된 별도 복사본의 전용 경로. 성공·실패 어느 쪽이든
/// 설치 경로의 앱을 다시 띄우고 true를 반환해 helper 자체는 UI를 만들지 않는다.
#[cfg(windows)]
pub fn run_windows_update_helper(predecessor_gone: bool) -> bool {
    if std::env::var_os("SWITCHER_UPDATE_HELPER").is_none() {
        return false;
    }
    let staged = std::env::var_os("SWITCHER_UPDATE_STAGED").map(PathBuf::from);
    let current = std::env::var_os("SWITCHER_UPDATE_TARGET").map(PathBuf::from);
    let (Some(staged), Some(current)) = (staged, current) else {
        eprintln!("업데이트 helper 환경 변수가 없습니다");
        return true;
    };

    let result = if predecessor_gone {
        finalize_windows_to(&staged, &current, env!("CARGO_PKG_VERSION"))
    } else {
        Err("이전 앱이 제한 시간 안에 종료되지 않아 교체를 보류했습니다".into())
    };
    if let Err(error) = &result {
        // 같은 실패를 시작 때마다 무한 반복하지 않는다. staged는 다음 sweep이 치운다.
        let _ = fs::remove_file(pending_version_path(&staged));
        eprintln!("업데이트 helper 교체 실패: {error}");
    }
    if current.is_file() {
        let mut command = std::process::Command::new(&current);
        command
            .env_remove("SWITCHER_WAIT_PID")
            .env_remove("SWITCHER_UPDATE_HELPER")
            .env_remove("SWITCHER_UPDATE_STAGED")
            .env_remove("SWITCHER_UPDATE_TARGET");
        if let Some(parent) = current.parent() {
            command.current_dir(parent);
        }
        if let Err(error) = command.spawn() {
            eprintln!("업데이트 뒤 앱 재실행 실패: {error}");
        }
    }
    true
}

#[cfg(windows)]
fn apply_windows_to(
    zip_bytes: &[u8],
    current: &Path,
    work: &Path,
    expected_version: &str,
) -> Result<std::path::PathBuf, String> {
    apply_windows_to_with_version(
        zip_bytes,
        current,
        work,
        expected_version,
        windows_binary_version,
    )
}

#[cfg(windows)]
fn apply_windows_to_with_version(
    zip_bytes: &[u8],
    current: &Path,
    work: &Path,
    expected_version: &str,
    inspect_version: impl FnOnce(&Path) -> Result<(u64, u64, u64), String>,
) -> Result<std::path::PathBuf, String> {
    let _ = fs::remove_dir_all(work);
    fs::create_dir_all(work).map_err(|e| format!("임시 폴더 생성 실패: {e}"))?;
    let zip = work.join(ASSET_NAME);
    fs::write(&zip, zip_bytes).map_err(|e| format!("zip 저장 실패: {e}"))?;
    extract_zip(&zip, work)?;
    let version_path = work.join(VERSION_FILE);
    let got = fs::read_to_string(&version_path)
        .map_err(|_| format!("받은 zip 안에 {VERSION_FILE}이 없습니다"))?;
    let got = got.trim().trim_start_matches('\u{feff}');
    if got != expected_version || parse_version(got).is_none() {
        return Err(format!(
            "릴리스 자산의 앱 버전({got})이 태그({expected_version})와 다릅니다 — 교체를 중단합니다"
        ));
    }
    let new_exe = work.join("switcher.exe");
    let size = fs::metadata(&new_exe)
        .map_err(|_| "받은 zip 안에 switcher.exe가 없습니다".to_string())?
        .len();
    // 반쯤 받아진 파일·빈 파일로 교체하는 사고 방어
    if size < 1_000_000 {
        return Err("받은 실행 파일이 비정상적으로 작습니다 — 교체를 중단합니다".to_string());
    }
    let expected = parse_version(expected_version)
        .ok_or_else(|| format!("기대 버전을 해석할 수 없습니다: {expected_version}"))?;
    let binary_version = inspect_version(&new_exe)?;
    if binary_version != expected {
        return Err(format!(
            "릴리스 자산의 실행 파일 버전({}.{}.{})이 태그({expected_version})와 다릅니다 — 교체를 중단합니다",
            binary_version.0, binary_version.1, binary_version.2
        ));
    }
    // 느린 복사를 같은 폴더의 .new로 먼저 끝내고 디스크에 밀어 둔다. 실행 중인
    // current는 여기서 건드리지 않고, 종료 뒤 별도 helper가 교체한다.
    let staged = current.with_extension("exe.new");
    let _ = fs::remove_file(&staged);
    fs::copy(&new_exe, &staged).map_err(|e| format!("새 실행 파일 준비 실패: {e}"))?;
    fs::OpenOptions::new()
        .write(true)
        .open(&staged)
        .and_then(|file| file.sync_all())
        .map_err(|e| format!("새 실행 파일 동기화 실패: {e}"))?;
    if let Err(error) = crate::accounts::atomic_write_existing_parent(
        &pending_version_path(&staged),
        expected_version.as_bytes(),
    ) {
        let _ = fs::remove_file(&staged);
        return Err(format!("pending 업데이트 표식 저장 실패: {error}"));
    }
    let _ = fs::remove_dir_all(work);
    Ok(staged)
}

/// zip을 임시 폴더에 풀고 현재 앱 번들(.app)을 새것으로 교체한다. 반환은
/// 교체된 번들 안의 실행 바이너리 경로 — 재시작이 이걸 실행한다 (경로 문자열은
/// 그대로, 내용물이 새 번들. 맥 실기기 미검증 — 다음 맥 세션에서 확인할 것)
#[cfg(target_os = "macos")]
fn apply(
    zip_bytes: &[u8],
    expected_version: &str,
) -> Result<(std::path::PathBuf, Option<std::path::PathBuf>), String> {
    let Some(bundle) = current_bundle() else {
        return Err("앱 번들 실행이 아니라 업데이트를 건너뜁니다".to_string());
    };
    // 격리 실행(translocation)은 읽기 전용 임의 경로라 교체할 수 없다.
    // npm 채널·직접 다운로드(우클릭 열기 후)에서는 발생하지 않는다.
    if bundle.to_string_lossy().contains("/AppTranslocation/") {
        return Err(
            "격리 실행(translocation) 상태라 업데이트할 수 없습니다 — 앱을 다른 폴더로 옮기면 풀립니다"
                .to_string(),
        );
    }
    let work = std::env::temp_dir().join(format!("switcher-update-{}", std::process::id()));
    apply_macos_to(zip_bytes, &bundle, &work, expected_version).map(|path| (path, None))
}

#[cfg(target_os = "macos")]
fn apply_macos_to(
    zip_bytes: &[u8],
    bundle: &Path,
    work: &Path,
    expected_version: &str,
) -> Result<std::path::PathBuf, String> {
    let _ = fs::remove_dir_all(work);
    fs::create_dir_all(&work).map_err(|e| format!("임시 폴더 생성 실패: {e}"))?;
    let zip = work.join(ASSET_NAME);
    fs::write(&zip, zip_bytes).map_err(|e| format!("zip 저장 실패: {e}"))?;
    extract_zip(&zip, work)?;
    let new_app = work.join("switcher.app");
    let new_bin = new_app.join("Contents").join("MacOS").join("switcher");
    let size = fs::metadata(&new_bin)
        .map_err(|_| "받은 zip 안에 switcher.app이 없습니다".to_string())?
        .len();
    // 반쯤 받아진 파일·빈 파일로 교체하는 사고 방어
    if size < 1_000_000 {
        return Err("받은 실행 파일이 비정상적으로 작습니다 — 교체를 중단합니다".to_string());
    }
    // 받은 앱의 실제 버전이 릴리스 태그와 같은지 검증 — 릴리스에 옛 zip이
    // 재사용돼 있으면(실제 사고: v1.5.1 태그에 v0.3.0 맥 zip) 조용한
    // 다운그레이드가 되므로 교체를 거부한다
    let got = bundle_version(&new_app)?;
    if got != expected_version {
        return Err(format!(
            "릴리스 자산의 앱 버전({got})이 태그({expected_version})와 다릅니다 — 자산이 갱신되지 않은 것 같아 교체를 건너뜁니다"
        ));
    }
    // 느린 복사를 번들 옆 .new로 먼저 끝내고, 같은 볼륨의 두 경로를 원자적으로 맞바꾼다.
    let staged = bundle.with_extension("app.new");
    let _ = fs::remove_dir_all(&staged);
    ditto_copy(&new_app, &staged)?;
    if let Err(error) = swap_paths_atomic(&bundle, &staged) {
        let _ = fs::remove_dir_all(&staged);
        return Err(error);
    }
    // swap 뒤 staged에는 실행 중인 이전 번들이 있다. 롤백용 이름으로 옮기지 못해도
    // 현재 bundle은 이미 정상적인 새 앱이므로 성공으로 본다. 다음 시작 청소가 .new도 지운다.
    let old = bundle.with_extension("app.old");
    let _ = fs::remove_dir_all(&old);
    if let Err(error) = fs::rename(&staged, &old) {
        eprintln!("이전 앱 번들 이름 정리 보류 (다음 시작에 정리): {error}");
    }
    let _ = fs::remove_dir_all(work);
    Ok(bundle.join("Contents").join("MacOS").join("switcher"))
}

#[cfg(target_os = "macos")]
fn swap_paths_atomic(left: &Path, right: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;

    extern "C" {
        fn renamex_np(from: *const std::ffi::c_char, to: *const std::ffi::c_char, flags: u32)
            -> std::ffi::c_int;
    }

    const RENAME_SWAP: u32 = 0x0000_0002;
    let left = std::ffi::CString::new(left.as_os_str().as_bytes())
        .map_err(|_| "현재 앱 경로에 NUL 문자가 있습니다".to_string())?;
    let right = std::ffi::CString::new(right.as_os_str().as_bytes())
        .map_err(|_| "새 앱 경로에 NUL 문자가 있습니다".to_string())?;
    let result = unsafe { renamex_np(left.as_ptr(), right.as_ptr(), RENAME_SWAP) };
    if result == 0 {
        Ok(())
    } else {
        Err(format!(
            "앱 원자 교체 실패: {}",
            std::io::Error::last_os_error()
        ))
    }
}

/// 번들의 CFBundleShortVersionString — 내장 PlistBuddy로 읽는다
#[cfg(target_os = "macos")]
fn bundle_version(app: &Path) -> Result<String, String> {
    let output = std::process::Command::new("/usr/libexec/PlistBuddy")
        .arg("-c")
        .arg("Print CFBundleShortVersionString")
        .arg(app.join("Contents").join("Info.plist"))
        .output()
        .map_err(|e| format!("받은 앱 버전 확인 실패: {e}"))?;
    if !output.status.success() {
        return Err("받은 앱의 버전 정보를 읽을 수 없습니다".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// ditto 복사 — 애드혹 서명·심볼릭 링크·리소스 포크를 보존한다 (cp -R은 서명이 깨질 수 있다)
#[cfg(target_os = "macos")]
fn ditto_copy(src: &Path, dst: &Path) -> Result<(), String> {
    let status = std::process::Command::new("/usr/bin/ditto")
        .arg(src)
        .arg(dst)
        .status()
        .map_err(|e| format!("ditto 실행 실패: {e}"))?;
    if !status.success() {
        return Err("새 앱 준비 복사 실패".to_string());
    }
    Ok(())
}

/// macOS 내장 ditto로 zip을 푼다 — npm 채널(scripts/dist.mjs)과 같은 통로라
/// 애드혹 서명이 온전히 보존되는 것이 실측 확인돼 있다.
#[cfg(target_os = "macos")]
fn extract_zip(zip: &Path, dir: &Path) -> Result<(), String> {
    let status = std::process::Command::new("/usr/bin/ditto")
        .arg("-xk")
        .arg(zip)
        .arg(dir)
        .status()
        .map_err(|e| format!("압축 해제 실행 실패: {e}"))?;
    if !status.success() {
        return Err("업데이트 zip 압축 해제 실패".to_string());
    }
    Ok(())
}

/// Windows 내장 bsdtar 절대 경로로 zip을 푼다.
/// PATH의 tar는 Git Bash에서 GNU tar(zip 해제 불가)로 잡힐 수 있다 — dist.mjs와 동일한 함정.
/// bsdtar는 절대 경로·`..` 항목을 기본 거부하므로 zip 경로 탈출도 함께 막힌다.
#[cfg(windows)]
fn extract_zip(zip: &Path, dir: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let tar = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()))
        .join("System32")
        .join("tar.exe");
    if !tar.exists() {
        // PATH 폴백은 두지 않는다 — 업데이트 경로에서 임의 tar를 부르는 위험보다
        // 이번 업데이트를 건너뛰는 편이 낫다 (bsdtar는 Windows 10 1803+ 기본 탑재)
        return Err("System32 tar.exe를 찾을 수 없어 업데이트를 건너뜁니다".to_string());
    }
    const CREATE_NO_WINDOW: u32 = 0x0800_0000; // 콘솔 창을 띄우지 않는다
    let status = std::process::Command::new(tar)
        .arg("-xf")
        .arg(zip)
        .arg("-C")
        .arg(dir)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("압축 해제 실행 실패: {e}"))?;
    if !status.success() {
        return Err("업데이트 zip 압축 해제 실패".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parsing_and_ordering() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.0.0"), Some((1, 0, 0)));
        assert_eq!(parse_version("v2.0.1-rc1"), Some((2, 0, 1)));
        assert_eq!(parse_version("v2.0.1-rc.1+build-7"), Some((2, 0, 1)));
        assert_eq!(parse_version("garbage"), None);
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("1.2.3garbage"), None);
        assert_eq!(parse_version("1.2.3-"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version("01.2.3"), None);
        assert!(parse_version("v1.1.0") > parse_version("v1.0.9"));
        assert!(parse_version("v1.10.0") > parse_version("v1.9.9"));
    }

    fn release(tag: &str, asset_name: &str, url: &str) -> serde_json::Value {
        serde_json::json!({
            "tag_name": tag,
            "assets": [{"name": asset_name, "browser_download_url": url}]
        })
    }

    #[test]
    fn release_plan_distinguishes_current_and_newer_version() {
        let same = release(
            "v1.7.33",
            ASSET_NAME,
            &format!("{ASSET_URL_PREFIX}v1.7.33/{ASSET_NAME}"),
        );
        assert!(release_plan(&same, (1, 7, 33)).unwrap().is_none());
        // 로컬 소스가 배포 채널보다 앞선 현재 상황도 다운그레이드하지 않는다.
        assert!(release_plan(&same, (1, 7, 35)).unwrap().is_none());

        let newer = release(
            "v1.7.36",
            ASSET_NAME,
            &format!("{ASSET_URL_PREFIX}v1.7.36/{ASSET_NAME}"),
        );
        let (version, url) = release_plan(&newer, (1, 7, 35)).unwrap().unwrap();
        assert_eq!(version, "1.7.36");
        assert!(url.ends_with(ASSET_NAME));
    }

    #[test]
    fn malformed_or_foreign_release_is_an_error_not_current() {
        let malformed = release(
            "latest",
            ASSET_NAME,
            &format!("{ASSET_URL_PREFIX}latest/{ASSET_NAME}"),
        );
        assert!(release_plan(&malformed, (1, 7, 35)).is_err());

        let foreign = release(
            "v1.7.36",
            ASSET_NAME,
            "https://example.com/switcher.zip",
        );
        assert!(release_plan(&foreign, (1, 7, 35)).is_err());

        let missing = release(
            "v1.7.36",
            "wrong-platform.zip",
            &format!("{ASSET_URL_PREFIX}v1.7.36/wrong-platform.zip"),
        );
        assert!(release_plan(&missing, (1, 7, 35)).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_zip_prepares_then_finalizes_atomic_replacement() {
        let base = std::env::temp_dir().join(format!(
            "switcher-update-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        let install = base.join("install");
        let payload = base.join("payload");
        fs::create_dir_all(&install).unwrap();
        fs::create_dir_all(&payload).unwrap();
        let current = install.join("switcher.exe");
        fs::write(&current, vec![b'o'; 1_100_000]).unwrap();
        fs::write(payload.join("switcher.exe"), vec![b'n'; 1_200_000]).unwrap();
        fs::write(payload.join(VERSION_FILE), "9.8.7").unwrap();

        let archive = base.join("fixture.zip");
        let tar = PathBuf::from(
            std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()),
        )
        .join("System32")
        .join("tar.exe");
        let status = std::process::Command::new(tar)
            .current_dir(&payload)
            .args(["-a", "-cf"])
            .arg(&archive)
            .arg("switcher.exe")
            .arg(VERSION_FILE)
            .status()
            .unwrap();
        assert!(status.success());

        let staged = apply_windows_to_with_version(
            &fs::read(&archive).unwrap(),
            &current,
            &base.join("work"),
            "9.8.7",
            |_| Ok((9, 8, 7)),
        )
        .unwrap();
        assert_eq!(staged, current.with_extension("exe.new"));
        assert_eq!(fs::metadata(&current).unwrap().len(), 1_100_000);
        assert_eq!(fs::metadata(&staged).unwrap().len(), 1_200_000);
        assert_eq!(read_pending_version(&staged).unwrap(), "9.8.7");

        let old = finalize_windows_to_with_version(
            &staged,
            &current,
            "9.8.7",
            |_| Ok((9, 8, 7)),
        )
        .unwrap();
        assert_eq!(fs::metadata(&current).unwrap().len(), 1_200_000);
        assert_eq!(fs::metadata(old).unwrap().len(), 1_100_000);
        assert!(!staged.exists());
        let _ = fs::remove_dir_all(&base);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_zip_atomically_swaps_bundle_and_keeps_rollback_copy() {
        fn make_bundle(path: &Path, byte: u8, version: &str) {
            let macos = path.join("Contents").join("MacOS");
            fs::create_dir_all(&macos).unwrap();
            fs::write(macos.join("switcher"), vec![byte; 1_200_000]).unwrap();
            fs::write(
                path.join("Contents").join("Info.plist"),
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleShortVersionString</key><string>{version}</string>
</dict></plist>"#
                ),
            )
            .unwrap();
        }

        let base = std::env::temp_dir().join(format!(
            "switcher-mac-update-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        let bundle = base.join("install").join("switcher.app");
        let payload = base.join("payload").join("switcher.app");
        make_bundle(&bundle, b'o', "1.0.0");
        make_bundle(&payload, b'n', "9.8.7");

        let archive = base.join("fixture.zip");
        let status = std::process::Command::new("/usr/bin/ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(&payload)
            .arg(&archive)
            .status()
            .unwrap();
        assert!(status.success());

        let relaunch = apply_macos_to(
            &fs::read(&archive).unwrap(),
            &bundle,
            &base.join("work"),
            "9.8.7",
        )
        .unwrap();
        assert_eq!(relaunch, bundle.join("Contents").join("MacOS").join("switcher"));
        assert_eq!(fs::read(&relaunch).unwrap()[0], b'n');
        assert_eq!(
            fs::read(
                bundle
                    .with_extension("app.old")
                    .join("Contents")
                    .join("MacOS")
                    .join("switcher")
            )
            .unwrap()[0],
            b'o'
        );
        let _ = fs::remove_dir_all(&base);
    }
}

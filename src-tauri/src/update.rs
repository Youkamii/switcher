//! 자동 업데이트 (Windows·macOS).
//!
//! GitHub 릴리스 latest를 확인해 새 버전이면 zip을 받아 실행 파일을 제자리에서
//! 교체한다. 떠 있는 프로세스는 건드리지 않고 재실행에 쓸 경로만 돌려준다 —
//! 이후는 호출부 몫이다:
//! - 시작 시 자동 확인: 교체만 해 두고 **다음 실행부터** 반영 (켜자마자 재시작
//!   루프를 돌지 않게 — update-ready 토스트로 알린다)
//! - 트레이 "업데이트 확인": 지금 업데이트하겠다는 명시적 의사이므로 교체 후
//!   **즉시 재시작**한다 (lib.rs restart_into — 사용자 지시 2026-08-10)
//!
//! 교체 원리 (양쪽 다 "느린 복사 먼저, 빠른 rename 두 번" — 어느 시점에 죽어도
//! 원본 또는 .old가 남는다):
//! - Windows: 실행 중인 exe의 "삭제"는 막지만 "이름 바꾸기"는 허용한다.
//!   현재 exe → `switcher.exe.old`로 비켜두고 새 exe를 원래 이름으로 복사한다.
//! - macOS: 실행 중인 앱의 번들 폴더도 이름 바꾸기가 허용된다(파일은 inode로
//!   열려 있음). 새 번들을 `switcher.app.new`로 준비해 두고 rename으로 바꾼다.
//!   ditto로 풀고 복사해 애드혹 서명·심링크가 보존된다 (npm 채널과 같은 경로).
//! `.old`는 그 시점엔 실행 중이라 지울 수 없으므로 다음 시작 때 치운다.
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

/// "v1.2.3" / "1.2.3-rc1" → (1, 2, 3). 해석 불가면 None (업데이트를 건너뛴다).
fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let mut parts = s.trim().trim_start_matches('v').splitn(3, '.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch_raw = parts.next()?;
    let digits: String = patch_raw.chars().take_while(|c| c.is_ascii_digit()).collect();
    let patch = digits.parse().ok()?;
    Some((major, minor, patch))
}

/// 지난 업데이트가 남긴 잔재 청소: 옛 실행 파일(교체 당시엔 그 프로세스가 살아 있어
/// 못 지운다), 중단된 준비 파일, 실패한 업데이트의 임시 폴더(pid 키라 그 실행만 안다)
pub fn sweep_old_exe() {
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        let _ = fs::remove_file(exe.with_extension("exe.new"));
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
                let _ = fs::remove_dir_all(entry.path());
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
static APPLIED: std::sync::Mutex<Option<(String, std::path::PathBuf)>> =
    std::sync::Mutex::new(None);

/// 성공 시 (버전, 재실행에 쓸 실행 파일 경로). 경로는 교체를 마친 시점에 apply가
/// 직접 계산한 것 — 교체 후 current_exe()가 rename을 따라가 .old를 가리키는
/// 플랫폼 미묘함에 걸지 않기 위함이다.
pub async fn check_and_apply() -> Result<Option<(String, std::path::PathBuf)>, String> {
    let _gate = UPDATE_LOCK.lock().await;
    let current =
        parse_version(env!("CARGO_PKG_VERSION")).ok_or("현재 버전을 해석할 수 없습니다")?;
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
    let tag = release["tag_name"].as_str().unwrap_or_default().to_string();
    let Some(latest) = parse_version(&tag) else {
        // 태깅 실수(v1.2, 이상한 접두사 등)를 조용히 삼키면 "업데이트가 안 됨"만 남는다
        eprintln!("업데이트 태그를 해석할 수 없습니다: {tag:?}");
        return Ok(None);
    };
    if latest <= current {
        return Ok(None);
    }
    let expected = tag.trim_start_matches('v').to_string();
    // 이 버전을 이미 받아 교체해 뒀으면(시작 시 자동 확인 등) 재다운로드·재교체
    // 없이 재시작 경로만 돌려준다 (APPLIED 주석 — 잠긴 .old에 막히던 실패의 본체)
    if let Ok(guard) = APPLIED.lock() {
        if let Some((version, relaunch)) = guard.as_ref() {
            if *version == expected {
                return Ok(Some((version.clone(), relaunch.clone())));
            }
        }
    }
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
    let relaunch = tauri::async_runtime::spawn_blocking(move || apply(&bytes, &version))
        .await
        .map_err(|e| format!("업데이트 적용 작업 실패: {e}"))??;
    if let Ok(mut guard) = APPLIED.lock() {
        *guard = Some((expected.clone(), relaunch.clone()));
    }
    Ok(Some((expected, relaunch)))
}

/// zip을 임시 폴더에 풀고 현재 실행 파일을 새것으로 교체한다. 반환은 교체된
/// (= 새 버전이 들어앉은) exe 경로 — 재시작이 이걸 실행한다.
/// (exe에는 쉽게 읽을 버전 메타데이터가 없어 자산 버전 검증은 macOS만 한다)
#[cfg(windows)]
fn apply(zip_bytes: &[u8], _expected_version: &str) -> Result<std::path::PathBuf, String> {
    let work = std::env::temp_dir().join(format!("switcher-update-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| format!("임시 폴더 생성 실패: {e}"))?;
    let zip = work.join(ASSET_NAME);
    fs::write(&zip, zip_bytes).map_err(|e| format!("zip 저장 실패: {e}"))?;
    extract_zip(&zip, &work)?;
    let new_exe = work.join("switcher.exe");
    let size = fs::metadata(&new_exe)
        .map_err(|_| "받은 zip 안에 switcher.exe가 없습니다".to_string())?
        .len();
    // 반쯤 받아진 파일·빈 파일로 교체하는 사고 방어
    if size < 1_000_000 {
        return Err("받은 실행 파일이 비정상적으로 작습니다 — 교체를 중단합니다".to_string());
    }
    let current = std::env::current_exe().map_err(|e| format!("현재 경로 확인 실패: {e}"))?;
    let mut old = current.with_extension("exe.old");
    // 크래시 안전 교체: 느린 단계(복사)를 같은 폴더의 .new로 먼저 끝내 두고,
    // rename 두 번(빠른 메타데이터 연산)만으로 바꾼다 — 어느 시점에 죽어도
    // 원본 또는 .old가 남아 실행 파일이 통째로 사라지는 창이 없다.
    let staged = current.with_extension("exe.new");
    let _ = fs::remove_file(&staged);
    fs::copy(&new_exe, &staged).map_err(|e| format!("새 실행 파일 준비 실패: {e}"))?;
    let _ = fs::remove_file(&old);
    if old.exists() {
        // 남은 .old = 이전 교체가 비켜둔 "실행 중인 이미지"라 삭제가 잠긴다 —
        // rename 대상이 존재하면 교체 전체가 실패하므로 옆 이름으로 비킨다
        // (sweep_old_exe가 다음 시작에 .old* 패턴으로 청소)
        old = current.with_extension(format!("exe.old{}", std::process::id()));
        let _ = fs::remove_file(&old);
    }
    fs::rename(&current, &old).map_err(|e| format!("실행 파일 교체 준비 실패: {e}"))?;
    if let Err(e) = fs::rename(&staged, &current) {
        // 되돌린다 — 실행 파일이 사라진 채 남지 않게
        let _ = fs::rename(&old, &current);
        let _ = fs::remove_file(&staged);
        return Err(format!("실행 파일 교체 실패: {e}"));
    }
    let _ = fs::remove_dir_all(&work);
    Ok(current)
}

/// zip을 임시 폴더에 풀고 현재 앱 번들(.app)을 새것으로 교체한다. 반환은
/// 교체된 번들 안의 실행 바이너리 경로 — 재시작이 이걸 실행한다 (경로 문자열은
/// 그대로, 내용물이 새 번들. 맥 실기기 미검증 — 다음 맥 세션에서 확인할 것)
#[cfg(target_os = "macos")]
fn apply(zip_bytes: &[u8], expected_version: &str) -> Result<std::path::PathBuf, String> {
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
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| format!("임시 폴더 생성 실패: {e}"))?;
    let zip = work.join(ASSET_NAME);
    fs::write(&zip, zip_bytes).map_err(|e| format!("zip 저장 실패: {e}"))?;
    extract_zip(&zip, &work)?;
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
    // 느린 단계(복사)를 번들 옆 .new로 먼저 끝내 둔다 — 같은 폴더(같은 볼륨)라
    // 이후 rename 두 번이 확실히 되고, 어느 시점에 죽어도 실행 파일이 사라지는 창이 없다.
    let staged = bundle.with_extension("app.new");
    let old = bundle.with_extension("app.old");
    let _ = fs::remove_dir_all(&staged);
    ditto_copy(&new_app, &staged)?;
    let _ = fs::remove_dir_all(&old);
    fs::rename(&bundle, &old).map_err(|e| format!("앱 교체 준비 실패: {e}"))?;
    if let Err(e) = fs::rename(&staged, &bundle) {
        // 되돌린다 — 앱이 사라진 채 남지 않게
        let _ = fs::rename(&old, &bundle);
        let _ = fs::remove_dir_all(&staged);
        return Err(format!("앱 교체 실패: {e}"));
    }
    let _ = fs::remove_dir_all(&work);
    Ok(bundle.join("Contents").join("MacOS").join("switcher"))
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
        assert_eq!(parse_version("garbage"), None);
        assert_eq!(parse_version(""), None);
        assert!(parse_version("v1.1.0") > parse_version("v1.0.9"));
        assert!(parse_version("v1.10.0") > parse_version("v1.9.9"));
    }
}

//! 사용량 조회 — 저장된 OAuth 토큰으로 각 계정의 한도 소진율을 읽는다.
//!
//! 클로드: GET https://api.anthropic.com/api/oauth/usage
//!   (Authorization: Bearer <accessToken> + anthropic-beta: oauth-2025-04-20)
//! 토큰 값은 절대 로그·에러 메시지에 싣지 않는다.

use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::accounts::{
    atomic_write, atomic_write_existing_parent, identity_from_value, jwt_payload, live_cred_exists,
    live_identity, now, read_json, read_live_cred, read_meta, Env, Provider, MUTATION_LOCK,
};
use serde::Deserialize;

const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CLAUDE_OAUTH_BETA: &str = "oauth-2025-04-20";
const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

// ─── 토큰 재발급 (클로드·코덱스 공통 뼈대) ───────────────────────
// 클로드: 액세스 토큰 수명이 실측 3~5시간이라, 보관함 프로필은 refreshToken으로
// 위젯이 직접 재발급한다 (CLI와 같은 방식). 주소·client_id는 설치된
// claude.exe 바이너리에서 실측 추출한 값이다.
const CLAUDE_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
// 코덱스: 수명이 길어(며칠 단위 JWT) 갱신 빈도는 낮지만 같은 뼈대로 재발급한다.
// 주소·client_id·scope는 설치된 codex 바이너리에서 실측 추출한 값이다
// (codex-cli 0.144.1, strings 추출 2026-08-11 — /oauth/token, app_EMoamEEZ…, openid profile email).
const CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// 만료 이만큼 전부터 미리 재발급한다 (조회 도중 만료 방지)
const REFRESH_MARGIN_SECS: i64 = 300;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UsageWindow {
    pub key: String,
    pub label: String,
    pub percent: f64,
    pub resets_at: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Usage {
    pub windows: Vec<UsageWindow>,
    /// true면 지금 조회가 막혀(429·토큰 만료 등) 마지막 성공 수치를 대신 보여주는 것
    #[serde(default)]
    pub stale: bool,
    /// stale일 때 그 수치가 몇 초 전 것인지 — 프론트가 "n시간 전 값" 라벨로 보여준다
    #[serde(default)]
    pub stale_age_secs: Option<u64>,
}

/// 조회 대상 토큰 파일: 프로필 이름이 있으면 보관함, 없으면 활성 파일.
/// 프로필 이름은 경로에 들어가므로 여기서도 반드시 검증한다 (경로 탈출 방지).
fn credential_path(
    env: &Env,
    provider: Provider,
    profile: Option<&str>,
) -> Result<PathBuf, String> {
    match profile {
        Some(name) => {
            crate::accounts::validate_name(name)?;
            Ok(env
                .profiles_dir(provider)
                .join(name)
                .join(provider.credential_file_name()))
        }
        None => Ok(env.live_credential_path(provider)),
    }
}

// 실계정 e2e 테스트 전용 — 실경로는 auth_snapshot이 키·토큰을 함께 잡는다
#[cfg(test)]
fn claude_access_token(env: &Env, profile: Option<&str>) -> Result<String, String> {
    let root = match profile {
        Some(_) => {
            let path = credential_path(env, Provider::Claude, profile)?;
            if !path.exists() {
                return Err("토큰 파일이 없습니다".into());
            }
            read_json(&path)?
        }
        // 활성 계정은 저장소(파일 또는 macOS 키체인)를 통해 읽는다
        None => serde_json::from_slice(&read_live_cred(env, Provider::Claude)?)
            .map_err(|e| format!("활성 토큰 파싱 실패: {e}"))?,
    };
    claude_access_token_from_root(&root)
}

fn claude_access_token_from_root(root: &Value) -> Result<String, String> {
    let oauth = root
        .get("claudeAiOauth")
        .ok_or("토큰 파일 형식이 다릅니다 (claudeAiOauth 없음)")?;
    if let Some(exp_ms) = oauth.get("expiresAt").and_then(|v| v.as_i64()) {
        if exp_ms < (now() as i64) * 1000 {
            return Err(
                "토큰이 만료됐습니다 — 이 계정으로 전환해 클로드를 한 번 실행하면 갱신됩니다"
                    .into(),
            );
        }
    }
    oauth
        .get("accessToken")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| "토큰 파일에 accessToken이 없습니다".into())
}

/// 만료(임박) 판정. expiresAt이 없으면 갱신 대상이 아니다 (기존 동작대로 조회 시도).
fn claude_token_expiring(root: &Value) -> bool {
    root.pointer("/claudeAiOauth/expiresAt")
        .and_then(|v| v.as_i64())
        .map(|exp_ms| exp_ms < (now() as i64 + REFRESH_MARGIN_SECS) * 1000)
        .unwrap_or(false)
}

/// 코덱스 만료(임박) 판정 — 만료 시각은 access_token JWT의 exp 클레임.
/// exp를 못 읽으면 갱신 대상이 아니다 (읽기 관문 codex_token과 같은 관용).
fn codex_token_expiring(root: &Value) -> bool {
    root.pointer("/tokens/access_token")
        .and_then(|v| v.as_str())
        .and_then(jwt_payload)
        .and_then(|p| p.get("exp").and_then(|v| v.as_i64()))
        .map(|exp| exp < now() as i64 + REFRESH_MARGIN_SECS)
        .unwrap_or(false)
}

fn token_expiring(provider: Provider, root: &Value) -> bool {
    match provider {
        Provider::Claude => claude_token_expiring(root),
        Provider::Codex => codex_token_expiring(root),
    }
}

/// 토큰 파일에서 리프레시 토큰을 꺼낸다 (재발급 요청·파일 교체 감지 공용)
fn extract_refresh_token(provider: Provider, root: &Value) -> Option<String> {
    let path = match provider {
        Provider::Claude => "/claudeAiOauth/refreshToken",
        Provider::Codex => "/tokens/refresh_token",
    };
    root.pointer(path).and_then(|v| v.as_str()).map(String::from)
}

/// 유닉스 초 → RFC3339 UTC ("2026-01-01T00:00:00Z") — 코덱스 auth.json의
/// last_refresh 필드용. 날짜 크레이트 없이 civil-from-days 표준 알고리즘으로 계산한다.
fn rfc3339_utc(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let (hh, mi, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{hh:02}:{mi:02}:{ss:02}Z")
}

/// 코덱스 재발급 응답(id_token·access_token·refresh_token)을 tokens 블록에 병합한다.
/// auth_mode·OPENAI_API_KEY·account_id 등 다른 필드는 보존하고 last_refresh를 갱신한다.
/// 만료 시각은 새 access_token JWT의 exp가 스스로 말하므로 expires_in이 필요 없다.
fn merge_refreshed_codex(root: &mut Value, resp: &Value) -> Result<(), String> {
    let access = resp
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "갱신 응답에 access_token이 없습니다".to_string())?
        .to_string();
    let new_refresh = resp
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(String::from);
    let new_id = resp.get("id_token").and_then(|v| v.as_str()).map(String::from);
    let tokens = root
        .get_mut("tokens")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| "토큰 파일 형식이 다릅니다 (tokens 없음)".to_string())?;
    tokens.insert("access_token".to_string(), Value::String(access));
    if let Some(rt) = new_refresh {
        tokens.insert("refresh_token".to_string(), Value::String(rt));
    }
    if let Some(idt) = new_id {
        tokens.insert("id_token".to_string(), Value::String(idt));
    }
    if let Some(obj) = root.as_object_mut() {
        obj.insert("last_refresh".to_string(), Value::String(rfc3339_utc(now())));
    }
    Ok(())
}

fn merge_refreshed(provider: Provider, root: &mut Value, resp: &Value) -> Result<(), String> {
    match provider {
        Provider::Claude => merge_refreshed_claude(root, resp),
        Provider::Codex => merge_refreshed_codex(root, resp),
    }
}

// ─── 재발급 응답의 착지 순서 (#18 견고성) ─────────────────────────
// 재발급 성공 순간 서버는 토큰 패밀리를 회전시켰고, 새 토큰의 유일본은 아직
// 메모리(응답)뿐이다. 본 파일 병합·쓰기가 실패하면 유일본이 증발해 재로그인으로
// 밀린다 — 그래서 응답을 먼저 pending 사이드카에 착지시킨 뒤 본 파일을 만지고,
// 성공하면 사이드카를 지운다. 잔존 사이드카는 다음 기회에 복구를 시도한다.

pub(crate) fn pending_path(cred_path: &Path) -> std::path::PathBuf {
    cred_path.with_extension("json.pending")
}

/// 내보내기용 읽기 전용 스냅숏. pending 응답이 현재 자격증명의 refresh token에서
/// 만들어진 것이면 메모리에서만 병합한다. 원본·활성 파일·pending은 절대 쓰거나
/// 지우지 않는다. 전환의 `apply_pending_rescue`와 같은 병합 규칙을 공유한다.
pub(crate) fn merge_pending_snapshot(
    provider: Provider,
    credential: &[u8],
    pending: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let Some(pending) = pending else {
        return Ok(credential.to_vec());
    };
    let entry: Value = serde_json::from_slice(pending)
        .map_err(|_| "갱신 복구 파일 형식이 올바르지 않습니다".to_string())?;
    let (Some(old_refresh), Some(response)) = (
        entry.get("old_refresh").and_then(Value::as_str),
        entry.get("response"),
    ) else {
        return Err("갱신 복구 파일 형식이 올바르지 않습니다".into());
    };
    let mut current: Value = serde_json::from_slice(credential)
        .map_err(|_| "저장된 인증정보 형식이 올바르지 않습니다".to_string())?;
    if extract_refresh_token(provider, &current).as_deref() != Some(old_refresh) {
        return Ok(credential.to_vec());
    }
    merge_refreshed(provider, &mut current, response)?;
    serde_json::to_vec_pretty(&current)
        .map_err(|_| "갱신된 인증정보 스냅숏을 만들 수 없습니다".to_string())
}

fn write_pending(cred_path: &Path, old_refresh: &str, resp: &Value) -> Result<(), String> {
    let entry = serde_json::json!({
        "old_refresh": old_refresh,
        "response": resp,
        "saved_at": now(),
    });
    let bytes = serde_json::to_vec(&entry).map_err(|e| format!("갱신 복구 파일 생성 실패: {e}"))?;
    atomic_write_existing_parent(&pending_path(cred_path), &bytes)
}

fn live_holds_refresh(
    env: &Env,
    provider: Provider,
    expected_refresh: &str,
) -> Result<bool, String> {
    let data = read_live_cred(env, provider)?;
    let root: Value = serde_json::from_slice(&data)
        .map_err(|e| format!("활성 토큰 파싱 실패: {e}"))?;
    Ok(extract_refresh_token(provider, &root).as_deref() == Some(expected_refresh))
}

/// 잔존 pending 사이드카 복구 — 본 파일의 리프레시 토큰이 응답을 만들 때 쓴 것과
/// 같을 때만 병합한다 (그 사이 전환 백업 등으로 파일이 더 새것이면 낡은 응답을
/// 버린다). 활성 위치가 회전 전 토큰을 들고 있으면 함께 고친 뒤에만 사이드카를
/// 지운다. MUTATION_LOCK 안에서 부를 것.
fn apply_pending_rescue(
    env: &Env,
    provider: Provider,
    name: &str,
    cred_path: &Path,
) -> Result<(), String> {
    let side = pending_path(cred_path);
    if !side.exists() {
        return Ok(());
    }
    let entry = read_json(&side)?;
    let (Some(old_refresh), Some(resp)) = (
        entry.get("old_refresh").and_then(|v| v.as_str()),
        entry.get("response"),
    ) else {
        std::fs::remove_file(&side)
            .map_err(|e| format!("깨진 갱신 복구 파일 정리 실패 {}: {e}", side.display()))?;
        return Err("갱신 복구 파일 형식이 올바르지 않아 정리했습니다".into());
    };
    let mut current = read_json(cred_path)?;
    if extract_refresh_token(provider, &current).as_deref() == Some(old_refresh) {
        merge_refreshed(provider, &mut current, resp)?;
        let _ = std::fs::copy(cred_path, cred_path.with_extension("json.bak"));
        let bytes = serde_json::to_vec_pretty(&current)
            .map_err(|e| format!("갱신 토큰 직렬화 실패: {e}"))?;
        atomic_write_existing_parent(cred_path, &bytes)?;
    }

    // POST 도중 이 프로필이 활성화된 뒤 활성 위치 쓰기만 실패한 경우도 복구한다.
    match live_identity(env, provider) {
        Ok(Some(live)) => {
            let profile_dir = env.profiles_dir(provider).join(name);
            let meta = read_meta(&profile_dir)
                .ok_or("프로필 정보가 없어 갱신 복구를 보류합니다")?;
            if live.id == meta.id {
                let live_holds_rotated_out = live_holds_refresh(env, provider, old_refresh)
                    .map_err(|e| format!("활성 계정 확인 실패 — 갱신 복구를 보류합니다: {e}"))?;
                if live_holds_rotated_out {
                    let bytes = serde_json::to_vec_pretty(&current)
                        .map_err(|e| format!("갱신 토큰 직렬화 실패: {e}"))?;
                    crate::accounts::write_live_cred(env, provider, &bytes)?;
                }
            }
        }
        Ok(None) if live_cred_exists(env, provider) => {
            return Err("활성 계정 신원을 확인할 수 없어 갱신 복구를 보류합니다".into());
        }
        Ok(None) => {}
        Err(e) => return Err(format!("활성 계정 확인 실패 — 갱신 복구를 보류합니다: {e}")),
    }

    std::fs::remove_file(&side)
        .map_err(|e| format!("갱신 복구 파일 정리 실패 {}: {e}", side.display()))
}

/// 계정 전환 경로가 MUTATION_LOCK을 쥔 채 호출하는 pending 복구 관문.
pub(crate) fn rescue_pending_profile_locked(
    env: &Env,
    provider: Provider,
    name: &str,
) -> Result<(), String> {
    let cred = env
        .profiles_dir(provider)
        .join(name)
        .join(provider.credential_file_name());
    apply_pending_rescue(env, provider, name, &cred)
}

/// 재발급 응답(access_token·refresh_token·expires_in)을 기존 claudeAiOauth 블록에
/// 병합한다. subscriptionType·rateLimitTier 등 다른 필드는 보존한다.
fn merge_refreshed_claude(root: &mut Value, resp: &Value) -> Result<(), String> {
    let access = resp
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "갱신 응답에 access_token이 없습니다".to_string())?
        .to_string();
    let new_refresh = resp
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(String::from);
    // expires_in이 없으면 옛 expiresAt(과거)이 남아 조회마다 재발급을 반복하는
    // 침묵 루프가 된다 — 필수로 요구해 한 번의 요란한 실패로 끝낸다
    let expires_in = resp
        .get("expires_in")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| "갱신 응답에 expires_in이 없습니다".to_string())?;
    let expires_at_ms = (now() as i64 + expires_in) * 1000;
    let oauth = root
        .get_mut("claudeAiOauth")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| "토큰 파일 형식이 다릅니다 (claudeAiOauth 없음)".to_string())?;
    oauth.insert("accessToken".to_string(), Value::String(access));
    if let Some(rt) = new_refresh {
        oauth.insert("refreshToken".to_string(), Value::String(rt));
    }
    oauth.insert("expiresAt".to_string(), Value::from(expires_at_ms));
    Ok(())
}

/// 모든 재발급을 직렬화한다 — 시작 스윕과 사용량 조회가 같은 프로필을
/// 동시에 두 번 회전시키면 두 번째가 거부돼 재로그인이 필요해질 수 있다.
static REFRESH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 보관함 프로필의 토큰이 만료(임박)면 리프레시 토큰으로 재발급해 파일에 되쓴다.
/// 활성 저장소(~/.claude·~/.codex)는 건드리지 않는다 — 실행 중 CLI와의 회전
/// 경합을 피하고, 활성 계정은 CLI가 스스로 갱신하기 때문이다.
/// (예외 하나: POST 도중 그 프로필로 전환된 경우의 사후 복구 — 함수 끝 참조)
async fn ensure_fresh_profile(env: &Env, provider: Provider, name: &str) -> Result<(), FetchErr> {
    let path = credential_path(env, provider, Some(name))?;
    if !path.exists() {
        return Ok(()); // 이후 조회 단계가 기존 방식대로 안내한다
    }
    let has_pending = pending_path(&path).exists();
    // 흔한 경로(만료 전 + 복구 없음)는 잠금 없이 싸게 통과
    if !has_pending && !token_expiring(provider, &read_json(&path).map_err(FetchErr::Msg)?) {
        return Ok(());
    }
    let _gate = REFRESH_LOCK.lock().await;
    // 전환·삭제와 같은 프로필 수명 잠금에 등록한다. 이 지점부터 응답 반영이 끝날
    // 때까지 삭제는 기다리고, 전환은 회전 전 토큰을 복사할 수 없다.
    let _inflight =
        crate::accounts::refresh_begin(crate::accounts::refresh_key(env, provider, name))
            .map_err(|_| FetchErr::Transient)?;
    // 가져오기는 MUTATION_LOCK을 쥔 채 marked 최종 폴더를 만들고 commit 또는
    // rollback한다. 같은 잠금 아래에서 marker와 토큰·meta를 다시 잡아야, 늦게
    // 시작한 사용량 조회가 아직 원복될 수 있는 리프레시 토큰을 회전시키지 않는다.
    let (root, meta) = {
        let _guard = MUTATION_LOCK
            .lock()
            .map_err(|_| FetchErr::Msg("내부 잠금 오류".into()))?;
        let profile_dir = env.profiles_dir(provider).join(name);
        if profile_dir
            .join(crate::accounts::PROFILE_IMPORT_MARKER)
            .exists()
            || !path.exists()
        {
            return Ok(());
        }
        if pending_path(&path).exists() {
            apply_pending_rescue(env, provider, name, &path).map_err(FetchErr::Msg)?;
        }
        // 잠금을 기다리는 사이 다른 경로가 이미 갱신했으면 끝
        let root = read_json(&path).map_err(FetchErr::Msg)?;
        if !token_expiring(provider, &root) {
            return Ok(());
        }
        // 활성 계정 보호는 여기(백엔드)가 강제한다 — 프론트 인자나 list()의 active
        // 스냅숏에 의존하지 않는다. 활성 계정의 보관함 사본을 회전시키면 실행 중
        // CLI의 토큰 패밀리와 충돌해 재로그인으로 밀릴 수 있다. 신원을 판정할 수
        // 없으면(파일 경합 등) 갱신을 보류한다 — 다음 기회에 다시 시도하면 된다.
        let Some(meta) = read_meta(&profile_dir) else {
            return Ok(());
        };
        match live_identity(env, provider) {
            Ok(Some(live)) if live.id == meta.id => return Ok(()), // 활성 계정 — CLI 소관
            Ok(Some(_)) => {}
            Ok(None) if live_cred_exists(env, provider) => return Ok(()),
            Ok(None) => {}
            Err(_) => return Ok(()), // 신원 불명 — 보류 (fail-closed)
        }
        (root, meta)
    };
    let refresh_token = extract_refresh_token(provider, &root)
        .ok_or_else(|| FetchErr::Msg("토큰 파일에 리프레시 토큰이 없습니다".into()))?;

    let client = reqwest::Client::builder()
        // 토큰 엔드포인트는 리다이렉트를 따라가지 않는다 — 307/308이 리프레시 토큰이
        // 담긴 본문을 다른 호스트로 재전송하는 것을 차단
        .redirect(reqwest::redirect::Policy::none())
        // REFRESH_LOCK을 쥔 채 도는 요청 — 무응답 서버가 전체 갱신을 멈추지 않게
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|_| FetchErr::Transient)?;
    let request = match provider {
        Provider::Claude => client.post(CLAUDE_TOKEN_URL).json(&serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": CLAUDE_CLIENT_ID,
        })),
        Provider::Codex => client.post(CODEX_TOKEN_URL).json(&serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": CODEX_CLIENT_ID,
            "scope": "openid profile email",
        })),
    };
    let resp = request.send().await.map_err(|_| FetchErr::Transient)?;
    let status = resp.status();
    if status.as_u16() == 429 || status.is_server_error() {
        return Err(FetchErr::Transient);
    }
    if !status.is_success() {
        // 리프레시 토큰이 거부됨 — 재시도해도 소용없다. 백오프를 걸어 5분 렌더
        // 주기마다 무의미한 POST가 영구 반복되는 것을 막고, 재로그인을 안내한다.
        backoff_bump(&cache_key(env, provider, Some(name)));
        return Err(FetchErr::Msg(
            "로그인이 만료됐습니다 — '계정 추가'에서 이 계정으로 다시 로그인하세요".into(),
        ));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| FetchErr::Msg(format!("갱신 응답 파싱 실패: {e}")))?;

    // 회전된 새 토큰의 유일본(응답)을 본 파일을 만지기 전에 착지시킨다 —
    // 아래 병합·쓰기가 실패해도 사이드카가 살아 다음 기회에 복구된다 (#18 견고성)
    let pending_error = write_pending(&path, &refresh_token, &body).err();

    // 파일 반영 구간만 변이 잠금 (저장·전환과 직렬화, 잠금 중 await 없음)
    let _guard = MUTATION_LOCK
        .lock()
        .map_err(|_| FetchErr::Msg("내부 잠금 오류".into()))?;
    let mut current = read_json(&path).map_err(FetchErr::Msg)?;
    // 그 사이 전환 백업 등으로 파일이 교체됐으면 우리 결과를 버린다 (더 새 토큰이 이미 있다)
    if extract_refresh_token(provider, &current).as_deref() != Some(refresh_token.as_str()) {
        // 프로필은 더 새것이어도 활성 위치가 회전 전 토큰을 들고 있을 수 있다.
        // 공용 복구 관문이 그 경우까지 확인한 뒤에만 pending을 지운다.
        apply_pending_rescue(env, provider, name, &path).map_err(FetchErr::Msg)?;
        return Ok(());
    }
    if let Err(merge_error) = merge_refreshed(provider, &mut current, &body) {
        return Err(FetchErr::Msg(match pending_error {
            Some(side_error) => format!(
                "갱신 응답 반영 실패: {merge_error}; 복구 파일 저장도 실패했습니다: {side_error}"
            ),
            None => merge_error,
        }));
    }
    // 덮어쓰기 전에 한 세대 .bak — 잘못된 갱신의 최후 안전망
    let _ = std::fs::copy(&path, path.with_extension("json.bak"));
    let bytes = serde_json::to_vec_pretty(&current).map_err(|e| FetchErr::Msg(e.to_string()))?;
    if let Err(main_error) = atomic_write_existing_parent(&path, &bytes) {
        return Err(FetchErr::Msg(match pending_error {
            Some(side_error) => format!(
                "새 토큰 저장 실패: {main_error}; 복구 파일 저장도 실패했습니다: {side_error}"
            ),
            None => main_error,
        }));
    }

    // 사후 복구 (#14 잔여 창의 반대편): POST가 나는 사이 이 프로필로 전환됐다면
    // 활성 저장소에는 방금 회전시켜 무효가 된 구토큰이 남아 있다 — 정확히 그 경우
    // (활성 계정 일치 + 활성 리프레시 토큰이 우리가 회전시킨 그 값)에만 활성도
    // 새 토큰으로 맞춘다. 같은 계정의 더 새 토큰이므로 "활성은 CLI 소관" 원칙의
    // 예외가 아니라 우리가 고아로 만든 토큰의 원상 복구다. 확인·쓰기 실패 시에는
    // pending을 남기고 오류를 돌려 다음 전환/조회에서 반드시 재시도한다.
    match live_identity(env, provider) {
        Ok(Some(live)) if live.id == meta.id => {
            let live_holds_rotated_out = match live_holds_refresh(env, provider, &refresh_token) {
                Ok(holds) => holds,
                Err(e) => {
                    let rescue_error = (!pending_path(&path).exists())
                        .then(|| write_pending(&path, &refresh_token, &body).err())
                        .flatten();
                    return Err(FetchErr::Msg(format!(
                        "활성 계정 확인 실패 — 다음 시도에서 갱신 복구를 재시도합니다: {e}{}",
                        rescue_error
                            .map(|error| format!("; 복구 파일 저장 실패: {error}"))
                            .unwrap_or_default()
                    )));
                }
            };
            if live_holds_rotated_out {
                if let Err(e) = crate::accounts::write_live_cred(env, provider, &bytes) {
                    // pending 착지가 실패했더라도 본 파일 쓰기는 성공했으므로 한 번 더
                    // 남겨, 다음 전환/조회에서 활성 위치 복구를 재시도할 수 있게 한다.
                    let rescue_error = (!pending_path(&path).exists())
                        .then(|| write_pending(&path, &refresh_token, &body).err())
                        .flatten();
                    return Err(FetchErr::Msg(format!(
                        "활성 계정의 갱신 토큰 반영 실패 — 다음 시도에서 복구합니다: {e}{}",
                        rescue_error
                            .map(|error| format!("; 복구 파일 저장 실패: {error}"))
                            .unwrap_or_default()
                    )));
                }
            }
        }
        Ok(Some(_)) => {}
        Ok(None) if live_cred_exists(env, provider) => {
            let rescue_error = (!pending_path(&path).exists())
                .then(|| write_pending(&path, &refresh_token, &body).err())
                .flatten();
            return Err(FetchErr::Msg(format!(
                "활성 계정 신원을 확인할 수 없어 갱신 복구를 보류합니다{}",
                rescue_error
                    .map(|error| format!("; 복구 파일 저장 실패: {error}"))
                    .unwrap_or_default()
            )));
        }
        Ok(None) => {}
        Err(e) => {
            let rescue_error = (!pending_path(&path).exists())
                .then(|| write_pending(&path, &refresh_token, &body).err())
                .flatten();
            return Err(FetchErr::Msg(format!(
                "활성 계정 확인 실패 — 다음 시도에서 갱신 복구를 재시도합니다: {e}{}",
                rescue_error
                    .map(|error| format!("; 복구 파일 저장 실패: {error}"))
                    .unwrap_or_default()
            )));
        }
    }
    if pending_path(&path).exists() {
        std::fs::remove_file(pending_path(&path))
            .map_err(|e| FetchErr::Msg(format!("갱신 복구 파일 정리 실패: {e}")))?;
    }
    Ok(())
}

/// 앱 시작 시 1회 무조건 도는 일괄 갱신 — 밤새 꺼져 있던 컴퓨터에서도
/// 위젯이 뜨자마자 비활성 프로필 사용량이 되살아난다. 재발급은 만료(임박)된
/// 프로필만 수행하고, 실패는 조용히 넘긴다 (조회 경로가 재시도·안내를 담당).
pub async fn refresh_all_profiles(env: &Env) {
    for provider in [Provider::Claude, Provider::Codex] {
        let Ok(snap) = crate::accounts::list(env, provider) else {
            continue;
        };
        for profile in snap.profiles.into_iter().filter(|p| !p.active) {
            let _ = ensure_fresh_profile(env, provider, &profile.name).await;
        }
    }
}

fn parse_claude_usage(body: &Value) -> Usage {
    let mut windows = Vec::new();
    if let Some(limits) = body.get("limits").and_then(|v| v.as_array()) {
        for limit in limits {
            let Some(percent) = limit.get("percent").and_then(|v| v.as_f64()) else {
                continue;
            };
            let kind = limit.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let scope_model = limit
                .pointer("/scope/model/display_name")
                .and_then(|v| v.as_str());
            let (key, label) = match (kind, scope_model) {
                ("session", _) => ("session".to_string(), "5 Hours".to_string()),
                ("weekly_all", _) => ("weekly".to_string(), "Weekly".to_string()),
                // 모델별 주간 한도는 모델 이름만 (예: Fable)
                ("weekly_scoped", Some(model)) => (format!("weekly:{model}"), model.to_string()),
                ("weekly_scoped", None) => ("weekly_scoped".to_string(), "Weekly (model)".to_string()),
                (other, _) => (other.to_string(), other.to_string()),
            };
            windows.push(UsageWindow {
                key,
                label,
                percent,
                resets_at: limit
                    .get("resets_at")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            });
        }
    }
    // 구형 응답 폴백: limits 배열이 없으면 five_hour/seven_day 필드 사용
    if windows.is_empty() {
        for (field, key, label) in [
            ("five_hour", "session", "5 Hours"),
            ("seven_day", "weekly", "Weekly"),
        ] {
            if let Some(percent) = body
                .pointer(&format!("/{field}/utilization"))
                .and_then(|v| v.as_f64())
            {
                windows.push(UsageWindow {
                    key: key.to_string(),
                    label: label.to_string(),
                    percent,
                    resets_at: body
                        .pointer(&format!("/{field}/resets_at"))
                        .and_then(|v| v.as_str())
                        .map(String::from),
                });
            }
        }
    }
    Usage {
        windows,
        stale: false,
        stale_age_secs: None,
    }
}

/// 조회 실패의 두 갈래 — 일시적(요청 제한·서버 오류·네트워크)은 백오프 대상이고
/// 화면에 문구를 띄우지 않는다. 그 외(토큰 만료 등)는 사용자에게 보여준다.
#[derive(Debug)]
enum FetchErr {
    Transient,
    Msg(String),
}

impl From<String> for FetchErr {
    fn from(message: String) -> Self {
        FetchErr::Msg(message)
    }
}

async fn get_json(request: reqwest::RequestBuilder) -> Result<Value, FetchErr> {
    let resp = request
        .send()
        .await
        .map_err(|_| FetchErr::Transient)?; // 네트워크 단절도 일시 장애로 취급
    let status = resp.status();
    if status.as_u16() == 429 || status.is_server_error() {
        return Err(FetchErr::Transient);
    }
    if !status.is_success() {
        return Err(FetchErr::Msg(format!(
            "사용량 조회 실패: HTTP {}",
            status.as_u16()
        )));
    }
    resp.json()
        .await
        .map_err(|e| FetchErr::Msg(format!("응답 파싱 실패: {e}")))
}

struct AuthSnapshot {
    key: String,
    root: Value,
}

/// 캐시 계정 ID와 실제 요청 토큰을 같은 MUTATION_LOCK 스냅숏에서 읽는다.
/// 전환이 둘 사이에 끼면 A 키에 B 사용량을 저장하는 사고가 난다.
fn auth_snapshot(
    env: &Env,
    provider: Provider,
    profile: Option<&str>,
) -> Result<AuthSnapshot, String> {
    let _guard = MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let (account, root) = match profile {
        Some(name) => {
            let path = credential_path(env, provider, Some(name))?;
            let root = read_json(&path)?;
            let account = read_meta(&env.profiles_dir(provider).join(name))
                .map(|meta| meta.id)
                .unwrap_or_else(|| format!("<name:{name}>"));
            (account, root)
        }
        None => {
            let data = read_live_cred(env, provider)?;
            let root: Value = serde_json::from_slice(&data)
                .map_err(|e| format!("활성 토큰 파싱 실패: {e}"))?;
            let ident = match provider {
                Provider::Claude => live_identity(env, provider)?,
                Provider::Codex => identity_from_value(provider, &root),
            };
            let account = ident
                .map(|identity| identity.id)
                .unwrap_or_else(|| "<live-unknown>".to_string());
            (account, root)
        }
    };
    Ok(AuthSnapshot {
        key: format!("{}:{account}", provider.dir_name()),
        root,
    })
}

/// 토큰을 읽지 않고 캐시 계정 키만 MUTATION_LOCK 아래서 잡는다. 특히 macOS의
/// 활성 Claude 캐시 적중 때마다 `/usr/bin/security`를 실행하는 비용을 피한다.
fn auth_key_snapshot(
    env: &Env,
    provider: Provider,
    profile: Option<&str>,
) -> Result<String, String> {
    let _guard = MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let account = match profile {
        Some(name) => read_meta(&env.profiles_dir(provider).join(name))
            .map(|meta| meta.id)
            .unwrap_or_else(|| format!("<name:{name}>")),
        None => live_identity(env, provider)?
            .map(|identity| identity.id)
            .unwrap_or_else(|| "<live-unknown>".to_string()),
    };
    Ok(format!("{}:{account}", provider.dir_name()))
}

/// 스냅숏이 실패했을 때 캐시 조회에만 쓰는 계정 키 — 토큰을 읽지 않고 아는 만큼만.
/// 활성 클로드의 신원은 전환이 같은 MUTATION_LOCK 아래서 갱신하는 ~/.claude.json을
/// 따르므로, 전환 직후에도 다른 계정의 수치를 집어 오지 않는다. 신원조차 모르면
/// None — 어느 계정의 캐시인지 보증할 수 없으면 보여주지 않는다.
fn snapshot_fallback_key(env: &Env, provider: Provider, profile: Option<&str>) -> Option<String> {
    let account = match profile {
        Some(name) => read_meta(&env.profiles_dir(provider).join(name))
            .map(|meta| meta.id)
            .unwrap_or_else(|| format!("<name:{name}>")),
        None => match provider {
            Provider::Claude => match live_identity(env, provider) {
                Ok(Some(identity)) => identity.id,
                Ok(None) => "<live-unknown>".to_string(),
                Err(_) => return None,
            },
            // 코덱스 신원은 토큰 파일 자체에서 나온다 — 그 읽기가 실패한 상황이면 알 수 없다
            Provider::Codex => return None,
        },
    };
    Some(format!("{}:{account}", provider.dir_name()))
}

async fn request_auth(
    env: &Env,
    provider: Provider,
    profile: Option<&str>,
) -> Result<(AuthSnapshot, Option<FetchErr>), FetchErr> {
    let refresh_err = match profile {
        Some(name) => ensure_fresh_profile(env, provider, name).await.err(),
        None => None,
    };
    let auth = auth_snapshot(env, provider, profile).map_err(FetchErr::Msg)?;
    Ok((auth, refresh_err))
}

async fn fetch_claude_attempt(
    auth: AuthSnapshot,
    refresh_err: Option<FetchErr>,
) -> Result<Usage, FetchErr> {
    // 재발급이 실패해도 바로 포기하지 않는다: 마진(5분) 창에서는 기존 토큰이 아직
    // 유효해 조회가 성공한다. 토큰마저 못 읽으면 그때는 재발급 실패 사유
    // (재로그인 안내·백오프)가 더 정확하므로 그쪽을 우선해 알린다.
    let token = match claude_access_token_from_root(&auth.root) {
        Ok(token) => token,
        Err(message) => return Err(refresh_err.unwrap_or(FetchErr::Msg(message))),
    };
    let body = get_json(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|_| FetchErr::Transient)?
            .get(CLAUDE_USAGE_URL)
            .bearer_auth(&token)
            .header("anthropic-beta", CLAUDE_OAUTH_BETA),
    )
    .await?;
    Ok(parse_claude_usage(&body))
}

#[cfg(test)]
async fn fetch_claude(env: &Env, profile: Option<&str>) -> Result<Usage, FetchErr> {
    let (auth, refresh_err) = request_auth(env, Provider::Claude, profile).await?;
    fetch_claude_attempt(auth, refresh_err).await
}

#[cfg(test)]
fn codex_token(env: &Env, profile: Option<&str>) -> Result<(String, Option<String>), String> {
    let path = credential_path(env, Provider::Codex, profile)?;
    if !path.exists() {
        return Err("토큰 파일이 없습니다".into());
    }
    let root = read_json(&path)?;
    codex_token_from_root(&root)
}

fn codex_token_from_root(root: &Value) -> Result<(String, Option<String>), String> {
    let tokens = root
        .get("tokens")
        .ok_or("토큰 파일 형식이 다릅니다 (tokens 없음)")?;
    let access = tokens
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or("토큰 파일에 access_token이 없습니다")?;
    if let Some(exp) = jwt_payload(access).and_then(|p| p.get("exp").and_then(|v| v.as_i64())) {
        if exp < now() as i64 {
            return Err(
                "토큰이 만료됐습니다 — 이 계정으로 전환해 코덱스를 한 번 실행하면 갱신됩니다"
                    .into(),
            );
        }
    }
    let account_id = tokens
        .get("account_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    Ok((access.to_string(), account_id))
}

/// 한도 창 길이를 사람이 읽는 라벨로. 코덱스 응답이 언젠가 일간(86400초) 창을
/// 추가해도 이 함수가 "Daily"로 자동 대응한다 — 파서는 창 길이만 보므로 코드 수정 불필요.
fn window_label(seconds: Option<i64>) -> String {
    match seconds {
        Some(s) if s >= 6 * 86400 => "Weekly".to_string(),
        Some(s) if s >= 2 * 86400 => format!("{} Days", s / 86400),
        Some(s) if s >= 86400 => "Daily".to_string(),
        Some(s) if s >= 2 * 3600 => format!("{} Hours", s / 3600),
        Some(s) if s >= 3600 => "1 Hour".to_string(),
        _ => "Limit".to_string(),
    }
}

fn push_codex_window(windows: &mut Vec<UsageWindow>, key: &str, label: Option<&str>, w: &Value) {
    let Some(percent) = w.get("used_percent").and_then(|v| v.as_f64()) else {
        return;
    };
    let label = label.map(String::from).unwrap_or_else(|| {
        window_label(w.get("limit_window_seconds").and_then(|v| v.as_i64()))
    });
    windows.push(UsageWindow {
        key: key.to_string(),
        label,
        percent,
        resets_at: w
            .get("reset_at")
            .and_then(|v| v.as_i64())
            .map(|t| t.to_string()),
    });
}

fn parse_codex_usage(body: &Value) -> Usage {
    let mut windows = Vec::new();
    if let Some(w) = body.pointer("/rate_limit/primary_window") {
        push_codex_window(&mut windows, "primary", None, w);
    }
    if let Some(w) = body.pointer("/rate_limit/secondary_window") {
        push_codex_window(&mut windows, "secondary", None, w);
    }
    if let Some(extra) = body.get("additional_rate_limits").and_then(|v| v.as_array()) {
        for item in extra {
            let name = item
                .get("limit_name")
                .and_then(|v| v.as_str())
                .unwrap_or("Model");
            if let Some(w) = item.pointer("/rate_limit/primary_window") {
                // 모델별 한도는 모델 이름만 표시
                push_codex_window(&mut windows, &format!("model:{name}"), Some(name), w);
            }
        }
    }
    Usage {
        windows,
        stale: false,
        stale_age_secs: None,
    }
}

async fn fetch_codex_attempt(
    auth: AuthSnapshot,
    refresh_err: Option<FetchErr>,
) -> Result<Usage, FetchErr> {
    let (token, account_id) = match codex_token_from_root(&auth.root) {
        Ok(pair) => pair,
        Err(message) => return Err(refresh_err.unwrap_or(FetchErr::Msg(message))),
    };
    let mut req = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|_| FetchErr::Transient)?
        .get(CODEX_USAGE_URL)
        .bearer_auth(&token);
    if let Some(id) = account_id {
        req = req.header("ChatGPT-Account-Id", id);
    }
    let body = get_json(req).await?;
    Ok(parse_codex_usage(&body))
}

#[cfg(test)]
async fn fetch_codex(env: &Env, profile: Option<&str>) -> Result<Usage, FetchErr> {
    let (auth, refresh_err) = request_auth(env, Provider::Codex, profile).await?;
    fetch_codex_attempt(auth, refresh_err).await
}

/// 새로고침 연타·재렌더마다 API를 때리지 않도록 60초 캐시를 둔다.
/// 조회가 실패해도(예: 요청 제한 429) 너무 오래되지 않은 직전 값이 있으면 그걸 보여준다.
fn cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, Usage)>>
{
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, Usage)>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);
/// 실패 시 대신 보여줄 수 있는 직전 값의 최대 나이. 클로드 액세스 토큰 수명이
/// 몇 시간뿐이라(실측 3~5시간) 비활성 프로필은 금방 만료 상태가 된다 — 하루 안의
/// 마지막 성공 수치를 나이 라벨("n시간 전 값")과 함께 계속 보여줘 "어느 계정에
/// 여유가 있나"를 판단할 근거를 남긴다. 이보다 오래되면 에러(만료 안내)를 그대로 보여준다.
const STALE_MAX: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

fn fresh_memory_cache(key: &str) -> Option<Usage> {
    cache().lock().ok().and_then(|map| {
        map.get(key)
            .filter(|(at, _)| at.elapsed() < CACHE_TTL)
            .map(|(_, cached)| cached.clone())
    })
}

fn fresh_cache(env: &Env, key: &str) -> Option<Usage> {
    if let Some(cached) = fresh_memory_cache(key) {
        return Some(cached);
    }
    let (fresh, _) = disk_cache_load(env, key, CACHE_TTL)?;
    if let Ok(mut map) = cache().lock() {
        map.insert(key.to_string(), (std::time::Instant::now(), fresh.clone()));
    }
    Some(fresh)
}

/// 같은 계정의 실제 조회는 한 번만 진행한다. 웹뷰 새로고침과 TFSD 평가가
/// 겹쳐도 뒤 요청은 앞 요청이 채운 캐시·백오프를 다시 확인한다.
fn fetch_gate(key: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    static GATES: std::sync::OnceLock<
        std::sync::Mutex<
            std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>,
        >,
    > = std::sync::OnceLock::new();
    let gates = GATES.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut gates = gates.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    gates
        .entry(key.to_string())
        .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

/// 캐시 키는 "누구의 사용량인가"(계정 id) 기준이다.
/// 전환 직후 활성 파일의 계정이 바뀌면 키도 바뀌어 이전 계정 수치가 새 계정 카드에
/// 붙는 일이 없다 (red-review 2라운드 지적).
fn cache_key(env: &Env, provider: Provider, profile: Option<&str>) -> String {
    let account = match profile {
        None => live_identity(env, provider)
            .ok()
            .flatten()
            .map(|l| l.id)
            .unwrap_or_else(|| "<live-unknown>".to_string()),
        Some(name) => read_meta(&env.profiles_dir(provider).join(name))
            .map(|m| m.id)
            .unwrap_or_else(|| format!("<name:{name}>")),
    };
    format!("{}:{account}", provider.dir_name())
}

/// 디스크 캐시 — 위젯을 재시작하면 메모리 캐시가 사라져, 재시작 직후 조회가
/// 막히면(요청 제한 429 등) 보여줄 직전 값조차 없다. 마지막 성공 수치를
/// 파일로 남겨 재시작 후에도 STALE_MAX 안이면 그걸 먼저 보여준다.
/// (사용량 퍼센트만 저장한다 — 토큰은 절대 넣지 않는다)
fn disk_cache_path(env: &Env) -> std::path::PathBuf {
    env.store.join("usage-cache.json")
}

/// usage-cache.json은 계정별 조회가 병렬로 끝나며 동시에 read-modify-write 할 수 있다.
/// 원자 rename만으로는 두 writer의 마지막 저장이 다른 계정 값을 지우는 것을 못 막으므로
/// 파일 한 개의 전체 RMW를 이 잠금으로 묶는다.
static DISK_CACHE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Serialize, Deserialize)]
struct DiskEntry {
    saved_at: u64,
    usage: Usage,
}

/// 디스크의 마지막 성공 수치를 (값, 나이(초))로 읽는다. max_age보다 오래되면 None.
fn disk_cache_load(env: &Env, key: &str, max_age: std::time::Duration) -> Option<(Usage, u64)> {
    let _guard = DISK_CACHE_LOCK.lock().ok()?;
    let root = read_json(&disk_cache_path(env)).ok()?;
    let entry: DiskEntry = serde_json::from_value(root.get(key)?.clone()).ok()?;
    let age = now().saturating_sub(entry.saved_at);
    if age < max_age.as_secs() {
        Some((entry.usage, age))
    } else {
        None
    }
}

fn disk_cache_root(path: &Path) -> Result<Value, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Value::Object(Default::default()));
        }
        Err(error) => return Err(format!("읽기 실패 {}: {error}", path.display())),
    };
    let root: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("JSON 파싱 실패 {}: {error}", path.display()))?;
    if root.is_object() {
        Ok(root)
    } else {
        Err(format!("캐시 형식이 올바르지 않습니다: {}", path.display()))
    }
}

fn disk_cache_store(env: &Env, key: &str, usage: &Usage) -> Result<(), String> {
    let _guard = DISK_CACHE_LOCK
        .lock()
        .map_err(|_| "사용량 캐시 잠금 오류".to_string())?;
    let path = disk_cache_path(env);
    // NotFound만 새 캐시로 취급한다. 손상·권한 오류를 빈 객체로 덮어 다른 계정의
    // 마지막 성공값까지 지우지 않는다.
    let mut root = disk_cache_root(&path)?;
    if let Some(obj) = root.as_object_mut() {
        let entry = DiskEntry {
            saved_at: now(),
            usage: usage.clone(),
        };
        let value = serde_json::to_value(&entry)
            .map_err(|error| format!("사용량 캐시 직렬화 실패: {error}"))?;
        obj.insert(key.to_string(), value);
    }
    let bytes = serde_json::to_vec_pretty(&root)
        .map_err(|error| format!("사용량 캐시 직렬화 실패: {error}"))?;
    atomic_write(&path, &bytes)
}

/// 프로필 삭제 시 그 계정의 사용량 캐시(메모리·디스크·백오프)를 정리한다 —
/// 안 지우면 usage-cache.json에 삭제된 계정 항목이 무기한 쌓인다 (#18 견고성).
/// 단 그 계정이 지금 활성 로그인이면 계정 키는 남긴다: 활성 카드가 계속 그 수치를
/// 폴백(마지막 성공 값)으로 쓰기 때문. meta.json이 없던 프로필의 이름 폴백 키는 항상 지운다.
pub(crate) fn purge_account_cache(
    env: &Env,
    provider: Provider,
    account_id: Option<&str>,
    name: &str,
) {
    let mut keys = vec![format!("{}:<name:{name}>", provider.dir_name())];
    if let Some(id) = account_id {
        let live_is_same = live_identity(env, provider)
            .ok()
            .flatten()
            .map(|l| l.id == id)
            .unwrap_or(false);
        if !live_is_same {
            keys.push(format!("{}:{id}", provider.dir_name()));
        }
    }
    if let Ok(mut map) = cache().lock() {
        for key in &keys {
            map.remove(key);
        }
    }
    if let Ok(mut map) = backoff().lock() {
        for key in &keys {
            map.remove(key);
        }
    }
    let path = disk_cache_path(env);
    let Ok(_guard) = DISK_CACHE_LOCK.lock() else {
        return;
    };
    let mut root = match disk_cache_root(&path) {
        Ok(root) => root,
        Err(error) => {
            eprintln!("사용량 캐시 정리 보류: {error}");
            return;
        }
    };
    if let Some(obj) = root.as_object_mut() {
        let mut changed = false;
        for key in &keys {
            changed |= obj.remove(key).is_some();
        }
        if changed {
            if let Ok(bytes) = serde_json::to_vec_pretty(&root) {
                if let Err(error) = atomic_write(&path, &bytes) {
                    eprintln!("사용량 캐시 정리 저장 실패: {error}");
                }
            }
        }
    }
}

/// 일시 장애(429 등) 후의 재시도 자제 시간표 — 거절당한 키는 이 시간 동안
/// API를 아예 부르지 않는다. 거절이 반복되면 2분→4분→8분→최대 15분으로 늘린다.
fn backoff() -> &'static std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, u32)>>
{
    static BACKOFF: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, u32)>>,
    > = std::sync::OnceLock::new();
    BACKOFF.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn backoff_active(key: &str) -> bool {
    backoff()
        .lock()
        .ok()
        .and_then(|map| map.get(key).map(|(until, _)| *until > std::time::Instant::now()))
        .unwrap_or(false)
}

fn backoff_bump(key: &str) {
    if let Ok(mut map) = backoff().lock() {
        let count = map.get(key).map(|(_, c)| *c).unwrap_or(0) + 1;
        let secs = (120u64 << (count - 1).min(3)).min(900); // 120·240·480·900
        map.insert(
            key.to_string(),
            (
                std::time::Instant::now() + std::time::Duration::from_secs(secs),
                count,
            ),
        );
    }
}

fn backoff_clear(key: &str) {
    if let Ok(mut map) = backoff().lock() {
        map.remove(key);
    }
}

pub async fn fetch(
    env: &Env,
    provider: Provider,
    profile: Option<&str>,
) -> Result<Usage, String> {
    // 1) 토큰을 열기 전에 계정 키만 잡아 신선한 메모리/디스크 캐시를 확인한다.
    // macOS 활성 Claude는 이 경로가 `/usr/bin/security`를 전혀 띄우지 않는다.
    let preliminary_key = auth_key_snapshot(env, provider, profile)
        .ok()
        .or_else(|| snapshot_fallback_key(env, provider, profile));
    if let Some(key) = preliminary_key.as_deref() {
        if let Some(fresh) = fresh_cache(env, key) {
            return Ok(fresh);
        }
    }

    // 캐시 miss에서만 요청 토큰과 계정 키를 같은 잠금 스냅숏으로 읽는다. 전환이
    // 끼어도 A 키에 B 사용량을 저장하지 않는다. 실패하면 아는 계정 키의 stale로 버틴다.
    let initial_auth = auth_snapshot(env, provider, profile);
    let key = match &initial_auth {
        Ok(auth) => auth.key.clone(),
        Err(error) => preliminary_key
            .clone()
            .ok_or_else(|| error.clone())?,
    };
    // 키만 읽은 뒤 실제 토큰 스냅숏 사이에 계정이 바뀌었으면 새 키 캐시를 다시 본다.
    if preliminary_key.as_deref() != Some(key.as_str()) {
        if let Some(fresh) = fresh_cache(env, &key) {
            return Ok(fresh);
        }
    }

    // 계정별 단일 실행 문. 기다린 뒤 캐시를 다시 봐 앞 요청이 성공했으면 네트워크와
    // 토큰 재발급을 반복하지 않는다. 실패했어도 아래 백오프를 다시 읽게 된다.
    let gate = fetch_gate(&key);
    let _request = gate.lock().await;
    if let Some(fresh) = fresh_cache(env, &key) {
        return Ok(fresh);
    }

    // 2) 실패 시 대신 내보낼 마지막 수치와 그 나이 (메모리 → 디스크 순, STALE_MAX 상한)
    let stale_value = || -> Option<(Usage, u64)> {
        if let Ok(map) = cache().lock() {
            if let Some((at, cached)) = map.get(&key) {
                if at.elapsed() < STALE_MAX {
                    return Some((cached.clone(), at.elapsed().as_secs()));
                }
            }
        }
        disk_cache_load(env, &key, STALE_MAX)
    };
    let mark_stale = |(mut usage, age): (Usage, u64)| {
        usage.stale = true;
        usage.stale_age_secs = Some(age);
        usage
    };

    // 스냅숏이 실패했다면 여기까지의 캐시 폴백이 전부다 — 새 요청은 불가능하다.
    // stale마저 없을 때만 원래 에러(키체인·파일 문제)를 노출한다.
    let initial_auth = match initial_auth {
        Ok(auth) => auth,
        Err(error) => {
            return stale_value().map(mark_stale).ok_or(error);
        }
    };

    // 3) 백오프 중이면 API를 부르지 않고 마지막 수치로 버틴다
    if backoff_active(&key) {
        return stale_value()
            .map(mark_stale)
            .ok_or_else(|| "사용량 조회 대기중".into());
    }

    // 4) 실제 조회. 비활성 프로필은 캐시가 없을 때만 재발급을 시도하고 새 토큰을
    // 다시 스냅숏으로 잡는다. 활성 계정은 위에서 잡은 토큰·키를 끝까지 유지한다.
    let prepared = match profile {
        Some(_) => request_auth(env, provider, profile).await,
        None => Ok((initial_auth, None)),
    };
    let (actual_key, result) = match prepared {
        Ok((auth, refresh_err)) => {
            let actual_key = auth.key.clone();
            let result = match provider {
                Provider::Claude => fetch_claude_attempt(auth, refresh_err).await,
                Provider::Codex => fetch_codex_attempt(auth, refresh_err).await,
            };
            (actual_key, result)
        }
        Err(error) => (key.clone(), Err(error)),
    };
    match result {
        Ok(usage) => {
            backoff_clear(&actual_key);
            if let Ok(mut map) = cache().lock() {
                map.insert(
                    actual_key.clone(),
                    (std::time::Instant::now(), usage.clone()),
                );
            }
            if let Err(e) = disk_cache_store(env, &actual_key, &usage) {
                eprintln!("사용량 캐시 저장 실패: {e}");
            }
            Ok(usage)
        }
        Err(FetchErr::Transient) => {
            // 요청 제한·서버 오류 — 재시도를 자제하고 마지막 수치로 조용히 버틴다
            backoff_bump(&actual_key);
            let actual_stale = || -> Option<(Usage, u64)> {
                if let Ok(map) = cache().lock() {
                    if let Some((at, cached)) = map.get(&actual_key) {
                        if at.elapsed() < STALE_MAX {
                            return Some((cached.clone(), at.elapsed().as_secs()));
                        }
                    }
                }
                disk_cache_load(env, &actual_key, STALE_MAX)
            };
            actual_stale()
                .map(mark_stale)
                .ok_or_else(|| "사용량 조회 대기중".into())
        }
        // 만료 토큰 등 — 하루 안의 마지막 수치가 있으면 나이 라벨과 함께 보여주고,
        // 그마저 없을 때만 원래 에러(전환해 갱신하라는 안내)를 노출한다
        Err(FetchErr::Msg(message)) => {
            let actual_stale = if actual_key == key {
                stale_value()
            } else if let Ok(map) = cache().lock() {
                map.get(&actual_key).and_then(|(at, cached)| {
                    (at.elapsed() < STALE_MAX)
                        .then(|| (cached.clone(), at.elapsed().as_secs()))
                })
            } else {
                None
            }
            .or_else(|| disk_cache_load(env, &actual_key, STALE_MAX));
            actual_stale.map(mark_stale).ok_or(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::test_support::{fake_jwt, test_env};
    use std::fs;

    #[test]
    fn parse_real_shape_uses_limits_array() {
        let body: Value = serde_json::from_str(
            r#"{
              "five_hour": {"utilization": 62.0, "resets_at": "2026-07-27T10:50:00Z"},
              "seven_day": {"utilization": 12.0, "resets_at": "2026-08-02T23:00:00Z"},
              "limits": [
                {"kind": "session", "percent": 62, "resets_at": "2026-07-27T10:50:00Z", "scope": null},
                {"kind": "weekly_all", "percent": 12, "resets_at": "2026-08-02T23:00:00Z", "scope": null},
                {"kind": "weekly_scoped", "percent": 15, "resets_at": "2026-08-02T23:00:00Z",
                 "scope": {"model": {"id": null, "display_name": "Fable"}}}
              ]
            }"#,
        )
        .unwrap();
        let usage = parse_claude_usage(&body);
        assert_eq!(usage.windows.len(), 3);
        assert_eq!(usage.windows[0].key, "session");
        assert_eq!(usage.windows[0].label, "5 Hours");
        assert_eq!(usage.windows[0].percent, 62.0);
        assert_eq!(usage.windows[1].label, "Weekly");
        assert_eq!(usage.windows[2].key, "weekly:Fable");
        assert_eq!(usage.windows[2].label, "Fable");
    }

    #[test]
    fn parse_falls_back_without_limits() {
        let body: Value = serde_json::from_str(
            r#"{"five_hour": {"utilization": 30.5, "resets_at": null},
                "seven_day": {"utilization": 7.0, "resets_at": null}}"#,
        )
        .unwrap();
        let usage = parse_claude_usage(&body);
        assert_eq!(usage.windows.len(), 2);
        assert_eq!(usage.windows[0].percent, 30.5);
    }

    #[test]
    fn profile_arg_path_escape_is_rejected() {
        let env = test_env("usage-escape");
        let err = claude_access_token(&env, Some("../../evil")).unwrap_err();
        assert!(err.contains("프로필 이름"));
        let err = codex_token(&env, Some("..\\evil")).unwrap_err();
        assert!(err.contains("프로필 이름"));
    }

    #[test]
    fn expired_token_is_rejected_with_guidance() {
        let env = test_env("usage-expired");
        fs::write(
            env.live_credential_path(Provider::Claude),
            r#"{"claudeAiOauth":{"accessToken":"fake","expiresAt":1000}}"#,
        )
        .unwrap();
        let err = claude_access_token(&env, None).unwrap_err();
        assert!(err.contains("만료"));
    }

    #[test]
    fn parse_codex_real_shape() {
        let body: Value = serde_json::from_str(
            r#"{
              "plan_type": "plus",
              "rate_limit": {
                "allowed": true,
                "primary_window": {"used_percent": 30, "limit_window_seconds": 604800,
                                   "reset_after_seconds": 512095, "reset_at": 1785660320},
                "secondary_window": {"used_percent": 7.5, "limit_window_seconds": 18000,
                                     "reset_after_seconds": 900, "reset_at": 1785661000}
              },
              "additional_rate_limits": [
                {"limit_name": "GPT-Test-Model", "rate_limit": {
                  "primary_window": {"used_percent": 0, "limit_window_seconds": 604800,
                                     "reset_at": 1785753026}}}
              ]
            }"#,
        )
        .unwrap();
        let usage = parse_codex_usage(&body);
        assert_eq!(usage.windows.len(), 3);
        assert_eq!(usage.windows[0].key, "primary");
        assert_eq!(usage.windows[0].label, "Weekly");
        assert_eq!(usage.windows[0].percent, 30.0);
        assert_eq!(usage.windows[1].label, "5 Hours");
        assert_eq!(usage.windows[2].key, "model:GPT-Test-Model");
        assert_eq!(usage.windows[2].label, "GPT-Test-Model");
    }

    #[test]
    fn codex_daily_window_is_labeled_automatically() {
        // 코덱스가 일간 한도를 도입해도 창 길이(86400초)만 보고 자동으로 Daily로 표시된다
        let body: Value = serde_json::from_str(
            r#"{"rate_limit": {
                "primary_window": {"used_percent": 12, "limit_window_seconds": 86400, "reset_at": 1785661000},
                "secondary_window": {"used_percent": 30, "limit_window_seconds": 604800, "reset_at": 1785663000}
            }}"#,
        )
        .unwrap();
        let usage = parse_codex_usage(&body);
        assert_eq!(usage.windows[0].label, "Daily");
        assert_eq!(usage.windows[1].label, "Weekly");
    }

    #[test]
    fn codex_expired_token_is_rejected() {
        let expired_jwt = fake_jwt(r#"{"exp":1000}"#);
        let env = test_env("usage-codex-expired");
        fs::create_dir_all(env.home.join(".codex")).unwrap();
        fs::write(
            env.live_credential_path(Provider::Codex),
            format!(r#"{{"tokens":{{"access_token":"{expired_jwt}","account_id":"acct-x"}}}}"#),
        )
        .unwrap();
        let err = codex_token(&env, None).unwrap_err();
        assert!(err.contains("만료"));
    }

    /// 스냅숏 실패(키체인 잠김·토큰 파일 손상)에도 마지막 수치가 있으면 그것으로
    /// 버틴다 — 맥에서 키체인 일시 실패가 활성 카드를 에러로 바꾸던 회귀 방지.
    /// 신원(~/.claude.json)마저 못 읽으면 어느 계정의 캐시인지 보증할 수 없으므로
    /// 그때만 에러를 노출한다.
    #[test]
    fn snapshot_failure_serves_cached_value_instead_of_error() {
        let env = test_env("snap-fallback");
        fs::create_dir_all(&env.store).unwrap();
        // 활성 토큰은 깨져 있고(스냅숏 실패), 신원 파일은 멀쩡하다
        fs::write(env.live_credential_path(Provider::Claude), b"not-json").unwrap();
        fs::write(
            env.home.join(".claude.json"),
            r#"{"oauthAccount":{"accountUuid":"uuid-snapfb","emailAddress":"s@test.dev"}}"#,
        )
        .unwrap();
        let usage = Usage {
            windows: vec![UsageWindow {
                key: "session".into(),
                label: "5 Hours".into(),
                percent: 7.0,
                resets_at: None,
            }],
            stale: false,
            stale_age_secs: None,
        };
        disk_cache_store(&env, "claude:uuid-snapfb", &usage).unwrap();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let got = rt
            .block_on(fetch(&env, Provider::Claude, None))
            .expect("캐시가 있으면 스냅숏 실패가 에러로 새어 나오면 안 된다");
        assert_eq!(got.windows[0].percent, 7.0);

        // 신원조차 없으면 원래 에러가 그대로 나온다
        fs::remove_file(env.home.join(".claude.json")).unwrap();
        fs::write(env.live_credential_path(Provider::Claude), b"still-broken").unwrap();
        assert!(rt.block_on(fetch(&env, Provider::Claude, None)).is_err());
    }

    #[test]
    fn disk_cache_roundtrip_and_expiry() {
        let env = test_env("disk-cache");
        fs::create_dir_all(&env.store).unwrap();
        let usage = Usage {
            windows: vec![UsageWindow {
                key: "session".into(),
                label: "5 Hours".into(),
                percent: 42.0,
                resets_at: None,
            }],
            stale: false,
            stale_age_secs: None,
        };
        disk_cache_store(&env, "claude:acct-1", &usage).unwrap();
        let (loaded, age) = disk_cache_load(&env, "claude:acct-1", STALE_MAX)
            .expect("방금 저장한 값이 읽혀야 한다");
        assert_eq!(loaded.windows[0].percent, 42.0);
        assert!(age <= 1, "방금 저장한 값의 나이는 0이어야 한다: {age}");
        // 다른 키는 없음
        assert!(disk_cache_load(&env, "claude:acct-2", STALE_MAX).is_none());
        // 오래된 항목은 버려진다
        let path = disk_cache_path(&env);
        let mut root = read_json(&path).unwrap();
        root["claude:acct-1"]["saved_at"] = serde_json::json!(1000);
        fs::write(&path, serde_json::to_vec(&root).unwrap()).unwrap();
        assert!(disk_cache_load(&env, "claude:acct-1", STALE_MAX).is_none());
    }

    #[test]
    fn parallel_disk_cache_writes_keep_every_account() {
        let env = std::sync::Arc::new(test_env("disk-cache-parallel"));
        fs::create_dir_all(&env.store).unwrap();
        let writers = 32;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(writers));
        let mut threads = Vec::new();
        for i in 0..writers {
            let env = env.clone();
            let barrier = barrier.clone();
            threads.push(std::thread::spawn(move || {
                let usage = Usage {
                    windows: vec![UsageWindow {
                        key: "session".into(),
                        label: "5 Hours".into(),
                        percent: i as f64,
                        resets_at: None,
                    }],
                    stale: false,
                    stale_age_secs: None,
                };
                barrier.wait();
                disk_cache_store(&env, &format!("claude:acct-{i}"), &usage).unwrap();
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        let root = read_json(&disk_cache_path(&env)).unwrap();
        assert_eq!(root.as_object().unwrap().len(), writers);
    }

    /// 핵심 시나리오: 비활성 프로필의 토큰이 만료돼도(수명 실측 3~5시간)
    /// 하루 안의 마지막 성공 수치를 나이와 함께 보여줘야 한다.
    #[test]
    fn expired_token_falls_back_to_last_value_with_age() {
        let env = test_env("stale-age");
        // 만료된 활성 토큰 + 계정 신원 (cache_key가 ~/.claude.json의 계정 id를 쓴다)
        fs::write(
            env.live_credential_path(Provider::Claude),
            r#"{"claudeAiOauth":{"accessToken":"fake","expiresAt":1000}}"#,
        )
        .unwrap();
        fs::write(
            env.home.join(".claude.json"),
            r#"{"oauthAccount":{"accountUuid":"uuid-stale","emailAddress":"s@test.dev"}}"#,
        )
        .unwrap();
        // 3시간 전의 마지막 성공 수치를 디스크에 심는다
        fs::create_dir_all(&env.store).unwrap();
        let usage = Usage {
            windows: vec![UsageWindow {
                key: "session".into(),
                label: "5 Hours".into(),
                percent: 61.0,
                resets_at: None,
            }],
            stale: false,
            stale_age_secs: None,
        };
        disk_cache_store(&env, "claude:uuid-stale", &usage).unwrap();
        let path = disk_cache_path(&env);
        let mut root = read_json(&path).unwrap();
        root["claude:uuid-stale"]["saved_at"] = serde_json::json!(now() - 3 * 3600);
        fs::write(&path, serde_json::to_vec(&root).unwrap()).unwrap();

        // 만료 토큰은 API 호출 전에 걸러지므로 네트워크 없이도 폴백 경로가 검증된다
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let got = rt.block_on(fetch(&env, Provider::Claude, None)).unwrap();
        assert!(got.stale, "폴백 값은 stale로 표시돼야 한다");
        let age = got.stale_age_secs.expect("나이가 실려야 한다");
        assert!((age as i64 - 3 * 3600).abs() < 10, "age={age}");
        assert_eq!(got.windows[0].percent, 61.0);

        // 하루를 넘긴 값은 버려지고 원래 에러(만료 안내)가 그대로 나간다
        let mut root = read_json(&path).unwrap();
        root["claude:uuid-stale"]["saved_at"] = serde_json::json!(now() - 25 * 3600);
        fs::write(&path, serde_json::to_vec(&root).unwrap()).unwrap();
        let err = rt.block_on(fetch(&env, Provider::Claude, None)).unwrap_err();
        assert!(err.contains("만료"), "만료 안내가 아니다: {err}");
    }

    #[test]
    fn refresh_response_merges_into_credential() {
        let mut root: Value = serde_json::from_str(
            r#"{"claudeAiOauth":{"accessToken":"old-a","refreshToken":"old-r","expiresAt":1000,
                "subscriptionType":"max","rateLimitTier":"default_claude_max_20x"},"other":1}"#,
        )
        .unwrap();
        let resp: Value = serde_json::from_str(
            r#"{"access_token":"new-a","refresh_token":"new-r","expires_in":28800}"#,
        )
        .unwrap();
        merge_refreshed_claude(&mut root, &resp).unwrap();
        assert_eq!(root.pointer("/claudeAiOauth/accessToken").unwrap(), "new-a");
        assert_eq!(root.pointer("/claudeAiOauth/refreshToken").unwrap(), "new-r");
        let exp = root
            .pointer("/claudeAiOauth/expiresAt")
            .unwrap()
            .as_i64()
            .unwrap();
        let want = (now() as i64 + 28800) * 1000;
        assert!((exp - want).abs() < 5000, "expiresAt: {exp} vs {want}");
        // 다른 필드는 보존
        assert_eq!(root.pointer("/claudeAiOauth/subscriptionType").unwrap(), "max");
        assert_eq!(root["other"], 1);

        // 응답에 refresh_token이 없으면 기존 값을 유지한다
        let resp2: Value =
            serde_json::from_str(r#"{"access_token":"new2","expires_in":100}"#).unwrap();
        merge_refreshed_claude(&mut root, &resp2).unwrap();
        assert_eq!(root.pointer("/claudeAiOauth/refreshToken").unwrap(), "new-r");

        // expires_in이 없으면 실패해야 한다 (침묵 갱신 루프 방지)
        let bad: Value = serde_json::from_str(r#"{"access_token":"x"}"#).unwrap();
        let err = merge_refreshed_claude(&mut root, &bad).unwrap_err();
        assert!(err.contains("expires_in"), "에러: {err}");
    }

    #[test]
    fn codex_refresh_response_merges_and_updates_last_refresh() {
        let mut root: Value = serde_json::from_str(
            r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":null,
                "tokens":{"id_token":"old-i","access_token":"old-a","refresh_token":"old-r","account_id":"acct-1"},
                "last_refresh":"2026-01-01T00:00:00Z"}"#,
        )
        .unwrap();
        let resp: Value = serde_json::from_str(
            r#"{"id_token":"new-i","access_token":"new-a","refresh_token":"new-r"}"#,
        )
        .unwrap();
        merge_refreshed_codex(&mut root, &resp).unwrap();
        assert_eq!(root.pointer("/tokens/access_token").unwrap(), "new-a");
        assert_eq!(root.pointer("/tokens/refresh_token").unwrap(), "new-r");
        assert_eq!(root.pointer("/tokens/id_token").unwrap(), "new-i");
        // 다른 필드는 보존
        assert_eq!(root.pointer("/tokens/account_id").unwrap(), "acct-1");
        assert_eq!(root["auth_mode"], "chatgpt");
        assert!(root["OPENAI_API_KEY"].is_null());
        // last_refresh는 지금 시각(RFC3339)으로 갱신
        let lr = root["last_refresh"].as_str().unwrap();
        assert_ne!(lr, "2026-01-01T00:00:00Z");
        assert_eq!(lr, rfc3339_utc(now()));

        // 응답에 refresh_token·id_token이 없으면 기존 값을 유지한다
        let resp2: Value = serde_json::from_str(r#"{"access_token":"new2"}"#).unwrap();
        merge_refreshed_codex(&mut root, &resp2).unwrap();
        assert_eq!(root.pointer("/tokens/refresh_token").unwrap(), "new-r");
        assert_eq!(root.pointer("/tokens/id_token").unwrap(), "new-i");

        // access_token이 없으면 실패해야 한다
        let bad: Value = serde_json::from_str(r#"{"refresh_token":"x"}"#).unwrap();
        assert!(merge_refreshed_codex(&mut root, &bad).is_err());
    }

    #[test]
    fn codex_token_expiring_detection() {
        let make = |claims: &str| -> Value {
            serde_json::from_str(&format!(
                r#"{{"tokens":{{"access_token":"{}","refresh_token":"r"}}}}"#,
                fake_jwt(claims)
            ))
            .unwrap()
        };
        assert!(codex_token_expiring(&make(r#"{"exp":1000}"#)));
        let soon = now() as i64 + 60;
        assert!(codex_token_expiring(&make(&format!(r#"{{"exp":{soon}}}"#))));
        let future = now() as i64 + 86400;
        assert!(!codex_token_expiring(&make(&format!(r#"{{"exp":{future}}}"#))));
        // exp를 못 읽으면 갱신 대상 아님 (읽기 관문과 같은 관용)
        assert!(!codex_token_expiring(&make(r#"{"sub":"x"}"#)));
        let no_token: Value = serde_json::from_str(r#"{"tokens":{}}"#).unwrap();
        assert!(!codex_token_expiring(&no_token));
    }

    #[test]
    fn rfc3339_utc_formats_known_instants() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(1_767_225_600), "2026-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(1_786_417_445), "2026-08-11T03:04:05Z");
        // 윤년 2월 말일 (2024-02-29T23:59:59Z = 1709251199)
        assert_eq!(rfc3339_utc(1_709_251_199), "2024-02-29T23:59:59Z");
    }

    /// pending 사이드카 복구: 본 파일의 리프레시 토큰이 응답 생성 시점과 같으면
    /// 병합하고, 파일이 이미 더 새것이면 낡은 응답을 버린다 (#18 착지 순서)
    #[test]
    fn pending_rescue_applies_then_drops_stale() {
        let env = test_env("pending-rescue");
        let dir = env.profiles_dir(Provider::Claude).join("p");
        fs::create_dir_all(&dir).unwrap();
        let cred = dir.join("credentials.json");
        fs::write(
            &cred,
            r#"{"claudeAiOauth":{"accessToken":"old-a","refreshToken":"r-old","expiresAt":1000}}"#,
        )
        .unwrap();
        let resp: Value = serde_json::from_str(
            r#"{"access_token":"new-a","refresh_token":"r-new","expires_in":28800}"#,
        )
        .unwrap();
        write_pending(&cred, "r-old", &resp).unwrap();
        assert!(pending_path(&cred).exists());

        apply_pending_rescue(&env, Provider::Claude, "p", &cred).unwrap();
        let root = read_json(&cred).unwrap();
        assert_eq!(root.pointer("/claudeAiOauth/accessToken").unwrap(), "new-a");
        assert_eq!(root.pointer("/claudeAiOauth/refreshToken").unwrap(), "r-new");
        assert!(!pending_path(&cred).exists(), "복구 후 사이드카는 지워져야 한다");
        assert!(cred.with_extension("json.bak").exists());

        // 파일이 이미 더 새것(다른 리프레시 토큰)이면 응답을 버리고 사이드카만 정리
        let stale_resp: Value =
            serde_json::from_str(r#"{"access_token":"zzz","expires_in":1}"#).unwrap();
        write_pending(&cred, "r-departed", &stale_resp).unwrap();
        apply_pending_rescue(&env, Provider::Claude, "p", &cred).unwrap();
        let root = read_json(&cred).unwrap();
        assert_eq!(
            root.pointer("/claudeAiOauth/accessToken").unwrap(),
            "new-a",
            "낡은 응답이 파일을 덮으면 안 된다"
        );
        assert!(!pending_path(&cred).exists());
    }

    #[test]
    fn pending_write_never_recreates_deleted_profile_directory() {
        let env = test_env("pending-deleted");
        let dir = env.profiles_dir(Provider::Claude).join("gone");
        let cred = dir.join("credentials.json");
        let resp: Value = serde_json::from_str(
            r#"{"access_token":"new-a","refresh_token":"new-r","expires_in":28800}"#,
        )
        .unwrap();
        assert!(write_pending(&cred, "old-r", &resp).is_err());
        assert!(!dir.exists(), "pending 쓰기가 삭제된 프로필 폴더를 되살리면 안 된다");
    }

    #[test]
    fn pending_rescue_repairs_active_copy_before_deleting_sidecar() {
        let env = test_env("pending-active-repair");
        let dir = env.profiles_dir(Provider::Claude).join("p");
        fs::create_dir_all(&dir).unwrap();
        let old = r#"{"claudeAiOauth":{"accessToken":"old-a","refreshToken":"r-old","expiresAt":1000}}"#;
        let cred = dir.join("credentials.json");
        fs::write(&cred, old).unwrap();
        fs::write(env.live_credential_path(Provider::Claude), old).unwrap();
        fs::write(
            env.home.join(".claude.json"),
            r#"{"oauthAccount":{"accountUuid":"uuid-p","emailAddress":"p@test.dev"}}"#,
        )
        .unwrap();
        fs::write(
            dir.join("meta.json"),
            r#"{"id":"uuid-p","email":"p@test.dev","saved_at":1}"#,
        )
        .unwrap();
        let resp: Value = serde_json::from_str(
            r#"{"access_token":"new-a","refresh_token":"r-new","expires_in":28800}"#,
        )
        .unwrap();
        write_pending(&cred, "r-old", &resp).unwrap();

        apply_pending_rescue(&env, Provider::Claude, "p", &cred).unwrap();
        let profile = read_json(&cred).unwrap();
        let live: Value = serde_json::from_slice(&read_live_cred(&env, Provider::Claude).unwrap())
            .unwrap();
        assert_eq!(profile.pointer("/claudeAiOauth/refreshToken").unwrap(), "r-new");
        assert_eq!(live.pointer("/claudeAiOauth/refreshToken").unwrap(), "r-new");
        assert!(!pending_path(&cred).exists());
    }

    /// 활성 코덱스 계정의 보관함 사본은 만료 상태여도 재발급하지 않는다
    /// (보호가 빠지면 이 테스트는 네트워크로 나가 실패한다 — 클로드 쪽과 동일한 방식)
    #[test]
    fn active_codex_profile_is_never_refreshed() {
        let env = test_env("codex-active-skip");
        fs::create_dir_all(env.home.join(".codex")).unwrap();
        let expired = fake_jwt(r#"{"exp":1000}"#);
        fs::write(
            env.live_credential_path(Provider::Codex),
            format!(r#"{{"tokens":{{"access_token":"live","refresh_token":"lr","account_id":"acct-act"}}}}"#),
        )
        .unwrap();
        let dir = env.profiles_dir(Provider::Codex).join("me");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("auth.json"),
            format!(
                r#"{{"tokens":{{"access_token":"{expired}","refresh_token":"pr","account_id":"acct-act"}}}}"#
            ),
        )
        .unwrap();
        fs::write(
            dir.join("meta.json"),
            r#"{"id":"acct-act","email":null,"saved_at":0}"#,
        )
        .unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(ensure_fresh_profile(&env, Provider::Codex, "me"))
            .unwrap();
        let root = read_json(&dir.join("auth.json")).unwrap();
        assert_eq!(
            root.pointer("/tokens/refresh_token").unwrap(),
            "pr",
            "활성 계정 프로필 토큰은 불변이어야 한다"
        );
    }

    /// 프로필 삭제 시 사용량 캐시 정리 — 삭제된 계정 키는 지우고,
    /// 그 계정이 활성 로그인이면 폴백 수치를 위해 남긴다 (#18 견고성)
    #[test]
    fn purge_removes_deleted_but_keeps_live_account() {
        let env = test_env("purge");
        fs::create_dir_all(&env.store).unwrap();
        // 활성 계정: uuid-live
        fs::write(
            env.home.join(".claude.json"),
            r#"{"oauthAccount":{"accountUuid":"uuid-live"}}"#,
        )
        .unwrap();
        let usage = Usage {
            windows: vec![],
            stale: false,
            stale_age_secs: None,
        };
        disk_cache_store(&env, "claude:uuid-live", &usage).unwrap();
        disk_cache_store(&env, "claude:uuid-gone", &usage).unwrap();
        disk_cache_store(&env, "claude:<name:ghost>", &usage).unwrap();

        // 비활성 계정 삭제 → 키 제거
        purge_account_cache(&env, Provider::Claude, Some("uuid-gone"), "gone");
        let root = read_json(&disk_cache_path(&env)).unwrap();
        assert!(root.get("claude:uuid-gone").is_none(), "삭제 계정 키는 지워져야 한다");
        assert!(root.get("claude:uuid-live").is_some());

        // meta 없던 프로필의 이름 폴백 키도 지워진다
        purge_account_cache(&env, Provider::Claude, None, "ghost");
        let root = read_json(&disk_cache_path(&env)).unwrap();
        assert!(root.get("claude:<name:ghost>").is_none());

        // 활성 계정의 프로필을 삭제해도 계정 키는 남는다 (활성 카드 폴백 보존)
        purge_account_cache(&env, Provider::Claude, Some("uuid-live"), "livep");
        let root = read_json(&disk_cache_path(&env)).unwrap();
        assert!(root.get("claude:uuid-live").is_some(), "활성 계정 키는 남아야 한다");
    }

    /// 실계정: 비활성 코덱스 프로필의 재발급이 실제로 도는지 확인 (클로드 쪽과 동일 절차).
    /// 강제 만료는 하지 않는다 — 코덱스 JWT는 수명이 길어, 실행 시점에 임박 상태가
    /// 아니면 fresh-skip을 확인하고 끝난다. 재발급 자체를 강제로 태우려면
    /// SWITCHER_TEST_FORCE_CODEX_REFRESH=1 로 실행 (성공 시 프로필에 새 토큰 저장).
    /// CI에서는 돌지 않는다: `cargo test -- --ignored real_refresh_codex` 로만 실행.
    #[test]
    #[ignore]
    fn real_refresh_codex_inactive_profile() {
        let env = Env::real().unwrap();
        let snap = crate::accounts::list(&env, Provider::Codex).unwrap();
        // 비활성 프로필이 없으면 조용히 스킵 — 표준 e2e 일괄 실행(real_)을 깨지 않는다
        // (검증하려면 코덱스 계정을 하나 더 추가한 뒤 실행)
        let Some(target) = snap.profiles.iter().find(|p| !p.active) else {
            println!("skip: 비활성 코덱스 프로필이 없어 실환경 재발급 검증 불가");
            return;
        };
        let path = env
            .profiles_dir(Provider::Codex)
            .join(&target.name)
            .join("auth.json");
        let original = fs::read(&path).unwrap();
        let force = std::env::var("SWITCHER_TEST_FORCE_CODEX_REFRESH").is_ok();
        if force {
            // 만료 임박처럼 보이게 exp만 과거로 — 리프레시 토큰 자체는 그대로라
            // 재발급은 진짜 자격으로 진행된다. 실패 시 원본 복구.
            let mut root = read_json(&path).unwrap();
            let expired = crate::accounts::test_support::fake_jwt(r#"{"exp":1000}"#);
            root["tokens"]["access_token"] = serde_json::json!(expired);
            fs::write(&path, serde_json::to_vec_pretty(&root).unwrap()).unwrap();
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let refreshed = rt.block_on(ensure_fresh_profile(&env, Provider::Codex, &target.name));
        if let Err(e) = refreshed {
            fs::write(&path, &original).unwrap(); // 원상 복구
            panic!("재발급 실패: {e:?}");
        }
        if force {
            let after = read_json(&path).unwrap();
            let access = after.pointer("/tokens/access_token").and_then(|v| v.as_str()).unwrap();
            let exp = jwt_payload(access)
                .and_then(|p| p.get("exp").and_then(|v| v.as_i64()))
                .expect("새 access_token에 exp가 있어야 한다");
            assert!(exp > now() as i64, "새 exp가 미래여야 한다");
            // 재발급된 토큰으로 실제 사용량 조회까지 되는지
            let usage = rt.block_on(fetch_codex(&env, Some(&target.name))).unwrap();
            assert!(!usage.windows.is_empty());
        }
    }

    /// 활성 계정의 보관함 사본은 만료 상태여도 절대 재발급하지 않는다 —
    /// 실행 중 CLI의 토큰 패밀리와 회전이 충돌하면 재로그인으로 밀려난다.
    /// (보호가 빠지면 이 테스트는 네트워크로 나가 실패한다)
    #[test]
    fn active_profile_is_never_refreshed() {
        let env = test_env("active-skip");
        fs::write(
            env.home.join(".claude.json"),
            r#"{"oauthAccount":{"accountUuid":"uuid-act"}}"#,
        )
        .unwrap();
        let dir = env.profiles_dir(Provider::Claude).join("me");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"a","refreshToken":"r","expiresAt":1000}}"#,
        )
        .unwrap();
        fs::write(
            dir.join("meta.json"),
            r#"{"id":"uuid-act","email":null,"saved_at":0}"#,
        )
        .unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(ensure_fresh_profile(&env, Provider::Claude, "me"))
            .unwrap();
        let root = read_json(&dir.join("credentials.json")).unwrap();
        assert_eq!(
            root.pointer("/claudeAiOauth/accessToken").unwrap(),
            "a",
            "활성 프로필 토큰은 불변이어야 한다"
        );
    }

    #[test]
    fn token_expiring_detection() {
        let past: Value = serde_json::from_str(r#"{"claudeAiOauth":{"expiresAt":1000}}"#).unwrap();
        assert!(claude_token_expiring(&past));
        let future_ms = (now() as i64 + 86400) * 1000;
        let future: Value =
            serde_json::from_str(&format!(r#"{{"claudeAiOauth":{{"expiresAt":{future_ms}}}}}"#))
                .unwrap();
        assert!(!claude_token_expiring(&future));
        // 임박(5분 안)도 갱신 대상
        let soon_ms = (now() as i64 + 60) * 1000;
        let soon: Value =
            serde_json::from_str(&format!(r#"{{"claudeAiOauth":{{"expiresAt":{soon_ms}}}}}"#))
                .unwrap();
        assert!(claude_token_expiring(&soon));
        // 필드가 없으면 갱신 대상 아님 (기존 동작대로 조회를 시도한다)
        let none: Value = serde_json::from_str(r#"{"claudeAiOauth":{}}"#).unwrap();
        assert!(!claude_token_expiring(&none));
    }

    #[test]
    fn fresh_or_missing_profile_skips_refresh_without_network() {
        let env = test_env("fresh-skip");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        // 프로필 파일이 없으면 그냥 통과 (이후 조회 단계가 안내)
        rt.block_on(ensure_fresh_profile(&env, Provider::Claude, "nope"))
            .unwrap();
        // 만료 전 토큰이면 네트워크 없이 즉시 통과하고 파일도 불변
        let dir = env.profiles_dir(Provider::Claude).join("p1");
        fs::create_dir_all(&dir).unwrap();
        let future_ms = (now() as i64 + 86400) * 1000;
        fs::write(
            dir.join("credentials.json"),
            format!(
                r#"{{"claudeAiOauth":{{"accessToken":"a","refreshToken":"r","expiresAt":{future_ms}}}}}"#
            ),
        )
        .unwrap();
        rt.block_on(ensure_fresh_profile(&env, Provider::Claude, "p1"))
            .unwrap();
        let root = read_json(&dir.join("credentials.json")).unwrap();
        assert_eq!(root.pointer("/claudeAiOauth/accessToken").unwrap(), "a");

        // 가져오기 commit 전 marked 프로필은 만료 토큰과 meta가 모두 있어도
        // 네트워크 갱신에 쓰지 않는다. rollback될 토큰을 회전시키면 유일본을 잃는다.
        let marked = env.profiles_dir(Provider::Claude).join("marked");
        fs::create_dir_all(&marked).unwrap();
        let marked_cred = marked.join("credentials.json");
        fs::write(
            &marked_cred,
            r#"{"claudeAiOauth":{"accessToken":"old","refreshToken":"never-send","expiresAt":1000}}"#,
        )
        .unwrap();
        fs::write(
            marked.join("meta.json"),
            r#"{"id":"marked-id","email":null,"saved_at":1,"hide_email":true}"#,
        )
        .unwrap();
        fs::write(
            marked.join(crate::accounts::PROFILE_IMPORT_MARKER),
            b"test-import",
        )
        .unwrap();
        let marked_before = fs::read(&marked_cred).unwrap();
        rt.block_on(ensure_fresh_profile(&env, Provider::Claude, "marked"))
            .unwrap();
        assert_eq!(fs::read(marked_cred).unwrap(), marked_before);
    }

    /// 실계정: 비활성 프로필의 expiresAt을 과거로 강제한 뒤 재발급이 실제로 도는지 확인.
    /// 성공하면 프로필에는 진짜 새 토큰이 저장된다. 실패 시 원본 파일을 되돌린다.
    /// CI에서는 돌지 않는다: `cargo test -- --ignored real_refresh` 로만 실행.
    #[test]
    #[ignore]
    fn real_refresh_inactive_claude_profile() {
        let env = Env::real().unwrap();
        let snap = crate::accounts::list(&env, Provider::Claude).unwrap();
        let target = snap
            .profiles
            .iter()
            .find(|p| !p.active)
            .expect("비활성 클로드 프로필이 없어 실환경 검증 불가");
        let path = env
            .profiles_dir(Provider::Claude)
            .join(&target.name)
            .join("credentials.json");
        let original = fs::read(&path).unwrap();

        // 만료를 강제해 갱신 경로를 태운다 (토큰 값 자체는 그대로)
        let mut root = read_json(&path).unwrap();
        root["claudeAiOauth"]["expiresAt"] = serde_json::json!(1000);
        fs::write(&path, serde_json::to_vec_pretty(&root).unwrap()).unwrap();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let refreshed = rt.block_on(ensure_fresh_profile(&env, Provider::Claude, &target.name));
        if let Err(e) = refreshed {
            fs::write(&path, &original).unwrap(); // 원상 복구
            panic!("재발급 실패: {e:?}");
        }
        let after = read_json(&path).unwrap();
        let exp = after
            .pointer("/claudeAiOauth/expiresAt")
            .and_then(|v| v.as_i64())
            .unwrap();
        assert!(exp > (now() as i64) * 1000, "새 expiresAt이 미래여야 한다");
        // 재발급된 토큰으로 실제 사용량 조회까지 되는지
        let usage = rt
            .block_on(fetch_claude(&env, Some(&target.name)))
            .unwrap();
        assert!(!usage.windows.is_empty());
    }

    #[test]
    fn backoff_escalates_and_clears() {
        let key = "test:backoff-key";
        assert!(!backoff_active(key));
        backoff_bump(key);
        assert!(backoff_active(key), "첫 거절 후에는 재시도를 자제해야 한다");
        backoff_clear(key);
        assert!(!backoff_active(key), "성공하면 즉시 정상 주기로 돌아온다");
    }

    /// 실계정 토큰으로 실제 엔드포인트를 호출하는 스모크 테스트.
    /// CI에서는 돌지 않는다: `cargo test -- --ignored` 로만 실행.
    #[test]
    #[ignore]
    fn real_codex_usage_smoke() {
        let env = Env::real().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let usage = rt.block_on(fetch_codex(&env, None)).unwrap();
        assert!(!usage.windows.is_empty());
        for w in &usage.windows {
            assert!((0.0..=100.0).contains(&w.percent));
        }
    }

    /// 실계정 토큰으로 실제 엔드포인트를 호출하는 스모크 테스트.
    /// CI에서는 돌지 않는다: `cargo test -- --ignored` 로만 실행.
    #[test]
    #[ignore]
    fn real_claude_usage_smoke() {
        let env = Env::real().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let usage = rt.block_on(fetch_claude(&env, None)).unwrap();
        assert!(!usage.windows.is_empty());
        for w in &usage.windows {
            assert!((0.0..=100.0).contains(&w.percent));
        }
    }
}

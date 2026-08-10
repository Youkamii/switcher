//! 사용량 조회 — 저장된 OAuth 토큰으로 각 계정의 한도 소진율을 읽는다.
//!
//! 클로드: GET https://api.anthropic.com/api/oauth/usage
//!   (Authorization: Bearer <accessToken> + anthropic-beta: oauth-2025-04-20)
//! 토큰 값은 절대 로그·에러 메시지에 싣지 않는다.

use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;

use crate::accounts::{
    atomic_write, jwt_payload, live_identity, now, read_json, read_live_cred, read_meta, Env,
    Provider, MUTATION_LOCK,
};
use serde::Deserialize;

const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CLAUDE_OAUTH_BETA: &str = "oauth-2025-04-20";
const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

// ─── 클로드 토큰 재발급 ───────────────────────────────────────────
// 액세스 토큰 수명이 실측 3~5시간이라, 보관함 프로필은 refreshToken으로
// 위젯이 직접 재발급한다 (CLI와 같은 방식). 주소·client_id는 설치된
// claude.exe 바이너리에서 실측 추출한 값이다.
const CLAUDE_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
/// 만료 이만큼 전부터 미리 재발급한다 (조회 도중 만료 방지)
const REFRESH_MARGIN_SECS: i64 = 300;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UsageWindow {
    pub key: String,
    pub label: String,
    pub percent: f64,
    pub resets_at: Option<String>,
    /// 예측 소진(#45) — 현재 소모 속도가 유지되면 몇 초 뒤 100%에 닿는지.
    /// 실조회 성공 응답에만 실리고, stale·디스크 재기동 값에는 싣지 않는다
    /// (낡은 예측 금지). 표시·TFSD 선제 전환이 이 값을 쓴다.
    #[serde(default)]
    pub exhausts_in_secs: Option<i64>,
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

/// 보관함 프로필의 토큰이 만료(임박)면 refreshToken으로 재발급해 파일에 되쓴다.
/// 활성 파일(~/.claude)은 건드리지 않는다 — 실행 중 CLI와의 회전 경합을 피하고,
/// 활성 계정은 CLI가 스스로 갱신하기 때문이다.
async fn ensure_fresh_claude_profile(env: &Env, name: &str) -> Result<(), FetchErr> {
    let path = credential_path(env, Provider::Claude, Some(name))?;
    if !path.exists() {
        return Ok(()); // 이후 조회 단계가 기존 방식대로 안내한다
    }
    // 흔한 경로(만료 전)는 잠금 없이 싸게 통과
    if !claude_token_expiring(&read_json(&path).map_err(FetchErr::Msg)?) {
        return Ok(());
    }
    let _gate = REFRESH_LOCK.lock().await;
    // 잠금을 기다리는 사이 다른 경로가 이미 갱신했으면 끝
    let root = read_json(&path).map_err(FetchErr::Msg)?;
    if !claude_token_expiring(&root) {
        return Ok(());
    }
    // 활성 계정 보호는 여기(백엔드)가 강제한다 — 프론트 인자나 list()의 active
    // 스냅숏에 의존하지 않는다. 활성 계정의 보관함 사본을 회전시키면 실행 중
    // CLI의 토큰 패밀리와 충돌해 재로그인으로 밀릴 수 있다. 신원을 판정할 수
    // 없으면(파일 경합 등) 갱신을 보류한다 — 다음 기회에 다시 시도하면 된다.
    let Some(meta) = read_meta(&env.profiles_dir(Provider::Claude).join(name)) else {
        return Ok(());
    };
    match live_identity(env, Provider::Claude) {
        Ok(Some(live)) if live.id == meta.id => return Ok(()), // 활성 계정 — CLI 소관
        Ok(_) => {}
        Err(_) => return Ok(()), // 신원 불명 — 보류 (fail-closed)
    }
    let refresh_token = root
        .pointer("/claudeAiOauth/refreshToken")
        .and_then(|v| v.as_str())
        .ok_or_else(|| FetchErr::Msg("토큰 파일에 refreshToken이 없습니다".into()))?
        .to_string();

    let client = reqwest::Client::builder()
        // 토큰 엔드포인트는 리다이렉트를 따라가지 않는다 — 307/308이 리프레시 토큰이
        // 담긴 본문을 다른 호스트로 재전송하는 것을 차단
        .redirect(reqwest::redirect::Policy::none())
        // REFRESH_LOCK을 쥔 채 도는 요청 — 무응답 서버가 전체 갱신을 멈추지 않게
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|_| FetchErr::Transient)?;
    let resp = client
        .post(CLAUDE_TOKEN_URL)
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": CLAUDE_CLIENT_ID,
        }))
        .send()
        .await
        .map_err(|_| FetchErr::Transient)?;
    let status = resp.status();
    if status.as_u16() == 429 || status.is_server_error() {
        return Err(FetchErr::Transient);
    }
    if !status.is_success() {
        // 리프레시 토큰이 거부됨 — 재시도해도 소용없다. 백오프를 걸어 5분 렌더
        // 주기마다 무의미한 POST가 영구 반복되는 것을 막고, 재로그인을 안내한다.
        backoff_bump(&cache_key(env, Provider::Claude, Some(name)));
        return Err(FetchErr::Msg(
            "로그인이 만료됐습니다 — '계정 추가'에서 이 계정으로 다시 로그인하세요".into(),
        ));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| FetchErr::Msg(format!("갱신 응답 파싱 실패: {e}")))?;

    // 파일 반영 구간만 변이 잠금 (저장·전환과 직렬화, 잠금 중 await 없음)
    let _guard = MUTATION_LOCK
        .lock()
        .map_err(|_| FetchErr::Msg("내부 잠금 오류".into()))?;
    let mut current = read_json(&path).map_err(FetchErr::Msg)?;
    // 그 사이 전환 백업 등으로 파일이 교체됐으면 우리 결과를 버린다 (더 새 토큰이 이미 있다)
    if current
        .pointer("/claudeAiOauth/refreshToken")
        .and_then(|v| v.as_str())
        != Some(refresh_token.as_str())
    {
        return Ok(());
    }
    merge_refreshed_claude(&mut current, &body).map_err(FetchErr::Msg)?;
    // 덮어쓰기 전에 한 세대 .bak — 잘못된 갱신의 최후 안전망
    let _ = std::fs::copy(&path, path.with_extension("json.bak"));
    let bytes = serde_json::to_vec_pretty(&current).map_err(|e| FetchErr::Msg(e.to_string()))?;
    atomic_write(&path, &bytes).map_err(FetchErr::Msg)
}

/// 앱 시작 시 1회 무조건 도는 일괄 갱신 — 밤새 꺼져 있던 컴퓨터에서도
/// 위젯이 뜨자마자 비활성 프로필 사용량이 되살아난다. 재발급은 만료(임박)된
/// 프로필만 수행하고, 실패는 조용히 넘긴다 (조회 경로가 재시도·안내를 담당).
pub async fn refresh_all_claude_profiles(env: &Env) {
    let Ok(snap) = crate::accounts::list(env, Provider::Claude) else {
        return;
    };
    for profile in snap.profiles.into_iter().filter(|p| !p.active) {
        let _ = ensure_fresh_claude_profile(env, &profile.name).await;
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
                exhausts_in_secs: None,
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
                    exhausts_in_secs: None,
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

async fn fetch_claude(env: &Env, profile: Option<&str>) -> Result<Usage, FetchErr> {
    // 보관함 토큰이 만료(임박)면 조회 전에 재발급한다 — 활성 파일은 CLI 소관.
    // 재발급이 실패해도 바로 포기하지 않는다: 마진(5분) 창에서는 기존 토큰이 아직
    // 유효해 조회가 성공한다. 토큰마저 못 읽으면 그때는 재발급 실패 사유
    // (재로그인 안내·백오프)가 더 정확하므로 그쪽을 우선해 알린다.
    let refresh_err = match profile {
        Some(name) => ensure_fresh_claude_profile(env, name).await.err(),
        None => None,
    };
    let token = match claude_access_token(env, profile) {
        Ok(token) => token,
        Err(message) => return Err(refresh_err.unwrap_or(FetchErr::Msg(message))),
    };
    let body = get_json(
        reqwest::Client::new()
            .get(CLAUDE_USAGE_URL)
            .bearer_auth(&token)
            .header("anthropic-beta", CLAUDE_OAUTH_BETA),
    )
    .await?;
    Ok(parse_claude_usage(&body))
}

fn codex_token(env: &Env, profile: Option<&str>) -> Result<(String, Option<String>), String> {
    let path = credential_path(env, Provider::Codex, profile)?;
    if !path.exists() {
        return Err("토큰 파일이 없습니다".into());
    }
    let root = read_json(&path)?;
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
        exhausts_in_secs: None,
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

async fn fetch_codex(env: &Env, profile: Option<&str>) -> Result<Usage, FetchErr> {
    let (token, account_id) = codex_token(env, profile)?;
    let mut req = reqwest::Client::new().get(CODEX_USAGE_URL).bearer_auth(&token);
    if let Some(id) = account_id {
        req = req.header("ChatGPT-Account-Id", id);
    }
    let body = get_json(req).await?;
    Ok(parse_codex_usage(&body))
}

// ── 예측 소진 (#45) ──────────────────────────────────────────────────
// 실조회 성공마다 (유닉스초, percent) 샘플을 계정 id·창 키별로 축적하고,
// 최소제곱 기울기로 "현재 속도면 몇 초 뒤 100%에 닿는지"를 계산한다.
// 실가치는 주간 창(Fable·Weekly)에 있다 — 5 Hours는 coasting이 감당하는
// 단기 창이고, 주간 창 소진은 며칠짜리 벽이라 조기 경보가 필요하다.

/// 샘플 저장소 — 키는 cache_key와 같은 계정 id 기준이라 전환·활성 여부와
/// 무관하게 계정을 따라간다 (메모리 전용 — 재시작하면 예측도 새로 쌓인다)
fn samples() -> &'static std::sync::Mutex<std::collections::HashMap<String, Vec<(i64, f64)>>> {
    static SAMPLES: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, Vec<(i64, f64)>>>,
    > = std::sync::OnceLock::new();
    SAMPLES.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 기울기 관측 지평 — "현재 세션의 페이스"를 반영할 만큼 짧게
const SAMPLE_HORIZON_SECS: i64 = 45 * 60;
/// 예측 게이트: 최소 샘플 수·최소 관측 폭 — 점 두 개짜리 요행 기울기 방지
const PREDICT_MIN_SAMPLES: usize = 3;
const PREDICT_MIN_SPAN_SECS: i64 = 10 * 60;
/// 이보다 먼 예측은 의미 없는 외삽 — 싣지 않는다
const PREDICT_MAX_HORIZON_SECS: i64 = 24 * 3600;
/// 연속 샘플 간 이만큼 넘는 하락 = 창 리셋 — 이력을 비워 기울기 오염을 막는다
const RESET_DROP: f64 = 5.0;

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 실조회 성공 수치를 이력에 기록 — 리셋 감지·지평 밖 정리 포함
fn record_samples(account_key: &str, usage: &Usage, now: i64) {
    let Ok(mut map) = samples().lock() else { return };
    for window in &usage.windows {
        let key = format!("{account_key}|{}", window.key);
        let entry = map.entry(key).or_default();
        if entry
            .last()
            .is_some_and(|(_, last)| last - window.percent > RESET_DROP)
        {
            entry.clear();
        }
        entry.push((now, window.percent));
        entry.retain(|(t, _)| now - t <= SAMPLE_HORIZON_SECS);
    }
}

/// 최소제곱 기울기(%/초) — 시각이 한 점에 몰려 있으면 None
fn slope_per_sec(points: &[(i64, f64)]) -> Option<f64> {
    let n = points.len() as f64;
    let mean_t = points.iter().map(|(t, _)| *t as f64).sum::<f64>() / n;
    let mean_p = points.iter().map(|(_, p)| *p).sum::<f64>() / n;
    let mut cov = 0.0;
    let mut var = 0.0;
    for (t, p) in points {
        let dt = *t as f64 - mean_t;
        cov += dt * (p - mean_p);
        var += dt * dt;
    }
    if var <= 0.0 {
        return None;
    }
    Some(cov / var)
}

/// 이력으로 소진(100%)까지 남은 초를 예측 — 게이트를 못 넘으면 None
fn predict_exhaust(points: &[(i64, f64)]) -> Option<i64> {
    if points.len() < PREDICT_MIN_SAMPLES {
        return None;
    }
    let span = points.last()?.0 - points.first()?.0;
    if span < PREDICT_MIN_SPAN_SECS {
        return None;
    }
    let (last_t, last_percent) = *points.last()?;
    if last_percent >= 100.0 {
        return Some(0);
    }
    let slope = slope_per_sec(points)?;
    if slope <= 0.0 {
        return None; // 안 오르는 중 — 예측할 벽이 없다
    }
    // 외삽 기준점은 원시 마지막 샘플이 아니라 회귀선 값 — 마지막 점 하나의
    // 노이즈(대형 요청 직후 계단)가 예측을 흔들지 않게 (red-review #45)
    let n = points.len() as f64;
    let mean_t = points.iter().map(|(t, _)| *t as f64).sum::<f64>() / n;
    let mean_p = points.iter().map(|(_, p)| *p).sum::<f64>() / n;
    let fitted = mean_p + slope * (last_t as f64 - mean_t);
    if fitted >= 100.0 {
        return Some(0);
    }
    let secs = ((100.0 - fitted) / slope).round() as i64;
    if secs > PREDICT_MAX_HORIZON_SECS {
        None
    } else {
        Some(secs.max(0))
    }
}

/// 응답에 예측을 싣는다 — **활성 계정 조회의 반환 직전**에만 부른다.
/// 비활성 계정은 "현재 속도 유지" 전제가 성립하지 않는다 — 떠나기 전의
/// 상승분이 이력에 남아 유령 소진 경고를 만든다 (red-review #45).
/// 캐시에는 싣지 않고 반환 직전마다 계산한다.
fn annotate_exhausts(account_key: &str, usage: &mut Usage) {
    annotate_exhausts_at(account_key, usage, unix_now());
}

/// 시간 주입판 — predict_exhaust는 마지막 샘플 시각 기준이라, 캐시 히트로
/// 샘플이 안 쌓인 동안(최대 CACHE_TTL) 지난 시간을 빼서 "지금 기준"으로 맞춘다.
/// 안 빼면 TFSD가 남은 시간을 최대 60초 과대평가해 전환 대신 대기한다 (리뷰 #53)
fn annotate_exhausts_at(account_key: &str, usage: &mut Usage, now: i64) {
    let Ok(map) = samples().lock() else { return };
    for window in &mut usage.windows {
        let key = format!("{account_key}|{}", window.key);
        window.exhausts_in_secs = map.get(&key).and_then(|points| {
            let secs = predict_exhaust(points)?;
            let last_t = points.last()?.0;
            Some((secs - (now - last_t)).max(0))
        });
    }
}

/// 예측 제거 — stale 응답·디스크 재기동 값은 낡은 예측을 실어선 안 된다
fn scrub_exhausts(usage: &mut Usage) {
    for window in &mut usage.windows {
        window.exhausts_in_secs = None;
    }
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

#[derive(Serialize, Deserialize)]
struct DiskEntry {
    saved_at: u64,
    usage: Usage,
}

/// 디스크의 마지막 성공 수치를 (값, 나이(초))로 읽는다. max_age보다 오래되면 None.
fn disk_cache_load(env: &Env, key: &str, max_age: std::time::Duration) -> Option<(Usage, u64)> {
    let root = read_json(&disk_cache_path(env)).ok()?;
    let entry: DiskEntry = serde_json::from_value(root.get(key)?.clone()).ok()?;
    let age = now().saturating_sub(entry.saved_at);
    if age < max_age.as_secs() {
        Some((entry.usage, age))
    } else {
        None
    }
}

fn disk_cache_store(env: &Env, key: &str, usage: &Usage) {
    let path = disk_cache_path(env);
    let mut root = read_json(&path).unwrap_or_else(|_| Value::Object(Default::default()));
    if let Some(obj) = root.as_object_mut() {
        let entry = DiskEntry {
            saved_at: now(),
            usage: usage.clone(),
        };
        if let Ok(value) = serde_json::to_value(&entry) {
            obj.insert(key.to_string(), value);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&root) {
            let _ = atomic_write(&path, &bytes);
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
    let key = cache_key(env, provider, profile);

    // 1) 메모리 캐시가 신선하면 그대로 — 예측(#45)은 캐시에 실려 있지 않고
    //    반환 직전에 현재 시점 기준으로 계산한다 (활성 조회만 — 유령 예측 방지)
    let annotated = |mut usage: Usage| {
        if profile.is_none() {
            annotate_exhausts(&key, &mut usage);
        }
        usage
    };
    if let Ok(map) = cache().lock() {
        if let Some((at, cached)) = map.get(&key) {
            if at.elapsed() < CACHE_TTL {
                return Ok(annotated(cached.clone()));
            }
        }
    }

    // 2) 재시작 직후: 디스크의 마지막 수치가 아직 신선하면 API를 부르지 않는다
    //    (재시작 때마다 일제 호출로 요청 제한에 걸리던 원인 제거)
    if let Some((mut fresh, _)) = disk_cache_load(env, &key, CACHE_TTL) {
        // 재시작 직후라 샘플 이력이 없다 — 디스크에 남은 낡은 예측은 싣지 않는다
        scrub_exhausts(&mut fresh);
        if let Ok(mut map) = cache().lock() {
            map.insert(key.clone(), (std::time::Instant::now(), fresh.clone()));
        }
        return Ok(fresh);
    }

    // 실패 시 대신 내보낼 마지막 수치와 그 나이 (메모리 → 디스크 순, STALE_MAX 상한)
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
        // 낡은 수치로는 예측하지 않는다 — TFSD의 stale 원칙과 같은 결
        scrub_exhausts(&mut usage);
        usage
    };

    // 3) 백오프 중이면 API를 부르지 않고 마지막 수치로 버틴다
    if backoff_active(&key) {
        return stale_value()
            .map(mark_stale)
            .ok_or_else(|| "사용량 조회 대기중".into());
    }

    // 4) 실제 조회
    let result = match provider {
        Provider::Claude => fetch_claude(env, profile).await,
        Provider::Codex => fetch_codex(env, profile).await,
    };
    match result {
        Ok(usage) => {
            backoff_clear(&key);
            // 예측 소진(#45): 이력만 기록한다 — 파싱 결과에는 예측이 없고,
            // 캐시·디스크에도 그대로(예측 없이) 저장한다. 예측은 아래 annotated가
            // 반환 직전 현재 시점 기준으로 계산해 싣는다.
            record_samples(&key, &usage, unix_now());
            if let Ok(mut map) = cache().lock() {
                map.insert(key.clone(), (std::time::Instant::now(), usage.clone()));
            }
            disk_cache_store(env, &key, &usage);
            Ok(annotated(usage))
        }
        Err(FetchErr::Transient) => {
            // 요청 제한·서버 오류 — 재시도를 자제하고 마지막 수치로 조용히 버틴다
            backoff_bump(&key);
            stale_value()
                .map(mark_stale)
                .ok_or_else(|| "사용량 조회 대기중".into())
        }
        // 만료 토큰 등 — 하루 안의 마지막 수치가 있으면 나이 라벨과 함께 보여주고,
        // 그마저 없을 때만 원래 에러(전환해 갱신하라는 안내)를 노출한다
        Err(FetchErr::Msg(message)) => stale_value().map(mark_stale).ok_or(message),
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

    #[test]
    fn predicts_exhaustion_from_slope() {
        // 10분 간격 3개, 10분당 10%p 증가(60→70→80) → 남은 20%는 1200초 뒤 소진
        let t0 = 1_700_000_000i64;
        let points = vec![(t0, 60.0), (t0 + 600, 70.0), (t0 + 1200, 80.0)];
        let secs = predict_exhaust(&points).expect("예측이 나와야 한다");
        assert!((secs - 1200).abs() <= 2, "secs={secs}");
        // 게이트: 샘플 부족·관측 폭 부족·하락 기울기·너무 먼 외삽은 전부 None
        assert_eq!(predict_exhaust(&points[..2]), None);
        let narrow = vec![(t0, 60.0), (t0 + 60, 61.0), (t0 + 120, 62.0)];
        assert_eq!(predict_exhaust(&narrow), None);
        let falling = vec![(t0, 80.0), (t0 + 600, 70.0), (t0 + 1200, 60.0)];
        assert_eq!(predict_exhaust(&falling), None);
        let crawl = vec![(t0, 10.0), (t0 + 600, 10.01), (t0 + 1200, 10.02)];
        assert_eq!(predict_exhaust(&crawl), None);
        // 외삽 기준은 회귀선 — 마지막 점의 노이즈에 흔들리지 않는다:
        // (0,0)·(600,50)·(1200,40): slope=1/30%/s, 회귀선상 마지막=50 → 1500초
        // (원시 마지막 40 기준이면 1800초로 어긋난다)
        let noisy = vec![(t0, 0.0), (t0 + 600, 50.0), (t0 + 1200, 40.0)];
        let secs = predict_exhaust(&noisy).expect("양의 기울기 — 예측이 나와야 한다");
        assert!((secs - 1500).abs() <= 2, "secs={secs}");
    }

    #[test]
    fn annotation_subtracts_elapsed_since_last_sample() {
        let mk = |p: f64| Usage {
            windows: vec![UsageWindow {
                key: "session".into(),
                label: "5 Hours".into(),
                percent: p,
                resets_at: None,
                exhausts_in_secs: None,
            }],
            stale: false,
            stale_age_secs: None,
        };
        let t0 = 1_700_000_000i64;
        record_samples("test:acct-elapsed", &mk(60.0), t0);
        record_samples("test:acct-elapsed", &mk(70.0), t0 + 600);
        record_samples("test:acct-elapsed", &mk(80.0), t0 + 1200);
        // 예측은 마지막 샘플 기준 1200초 뒤 소진 — 60초 지난 시점의 조회는
        // 그만큼 빼서 1140초로 실려야 한다 (캐시 히트 동안 낡는 문제, 리뷰 #53)
        let mut usage = mk(80.0);
        annotate_exhausts_at("test:acct-elapsed", &mut usage, t0 + 1260);
        let secs = usage.windows[0].exhausts_in_secs.expect("예측이 실려야 한다");
        assert!((secs - 1140).abs() <= 2, "secs={secs}");
    }

    #[test]
    fn sample_history_resets_on_drop() {
        let mk = |p: f64| Usage {
            windows: vec![UsageWindow {
                key: "session".into(),
                label: "5 Hours".into(),
                percent: p,
                resets_at: None,
                exhausts_in_secs: None,
            }],
            stale: false,
            stale_age_secs: None,
        };
        let t0 = 1_700_000_000i64;
        record_samples("test:acct-drop", &mk(50.0), t0);
        record_samples("test:acct-drop", &mk(70.0), t0 + 600);
        // 창 리셋(70 → 3): 이력이 비워지고 새 점 하나만 남는다 — 기울기 오염 방지
        record_samples("test:acct-drop", &mk(3.0), t0 + 1200);
        let map = samples().lock().unwrap();
        let points = map.get("test:acct-drop|session").unwrap();
        assert_eq!(points.as_slice(), &[(t0 + 1200, 3.0)]);
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
                exhausts_in_secs: None,
            }],
            stale: false,
            stale_age_secs: None,
        };
        disk_cache_store(&env, "claude:acct-1", &usage);
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
                exhausts_in_secs: None,
            }],
            stale: false,
            stale_age_secs: None,
        };
        disk_cache_store(&env, "claude:uuid-stale", &usage);
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
        rt.block_on(ensure_fresh_claude_profile(&env, "me")).unwrap();
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
        rt.block_on(ensure_fresh_claude_profile(&env, "nope")).unwrap();
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
        rt.block_on(ensure_fresh_claude_profile(&env, "p1")).unwrap();
        let root = read_json(&dir.join("credentials.json")).unwrap();
        assert_eq!(root.pointer("/claudeAiOauth/accessToken").unwrap(), "a");
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
        let refreshed = rt.block_on(ensure_fresh_claude_profile(&env, &target.name));
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

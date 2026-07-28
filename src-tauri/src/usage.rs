//! 사용량 조회 — 저장된 OAuth 토큰으로 각 계정의 한도 소진율을 읽는다.
//!
//! 클로드: GET https://api.anthropic.com/api/oauth/usage
//!   (Authorization: Bearer <accessToken> + anthropic-beta: oauth-2025-04-20)
//! 토큰 값은 절대 로그·에러 메시지에 싣지 않는다.

use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;

use crate::accounts::{jwt_payload, live_identity, now, read_json, read_meta, Env, Provider};

const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CLAUDE_OAUTH_BETA: &str = "oauth-2025-04-20";
const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

#[derive(Serialize, Clone)]
pub struct UsageWindow {
    pub key: String,
    pub label: String,
    pub percent: f64,
    pub resets_at: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct Usage {
    pub windows: Vec<UsageWindow>,
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
    let path = credential_path(env, Provider::Claude, profile)?;
    if !path.exists() {
        return Err("토큰 파일이 없습니다".into());
    }
    let root = read_json(&path)?;
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
                ("session", _) => ("session".to_string(), "5시간".to_string()),
                ("weekly_all", _) => ("weekly".to_string(), "주간".to_string()),
                ("weekly_scoped", Some(model)) => {
                    (format!("weekly:{model}"), format!("주간 · {model}"))
                }
                ("weekly_scoped", None) => ("weekly_scoped".to_string(), "주간(모델)".to_string()),
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
            ("five_hour", "session", "5시간"),
            ("seven_day", "weekly", "주간"),
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
    Usage { windows }
}

async fn get_json(request: reqwest::RequestBuilder) -> Result<Value, String> {
    let resp = request
        .send()
        .await
        .map_err(|e| format!("사용량 요청 실패: {e}"))?;
    let status = resp.status();
    if status.as_u16() == 429 {
        return Err("요청이 잦아 잠시 제한되었습니다 — 잠시 후 자동으로 다시 조회됩니다".into());
    }
    if !status.is_success() {
        return Err(format!("사용량 조회 실패: HTTP {}", status.as_u16()));
    }
    resp.json()
        .await
        .map_err(|e| format!("응답 파싱 실패: {e}"))
}

async fn fetch_claude(env: &Env, profile: Option<&str>) -> Result<Usage, String> {
    let token = claude_access_token(env, profile)?;
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

/// 한도 창 길이를 사람이 읽는 라벨로 (604800초=7일 → "주간")
fn window_label(seconds: Option<i64>) -> String {
    match seconds {
        Some(s) if s >= 6 * 86400 => "주간".to_string(),
        Some(s) if s >= 86400 => format!("{}일", s / 86400),
        Some(s) if s >= 3600 => format!("{}시간", s / 3600),
        _ => "한도".to_string(),
    }
}

fn push_codex_window(windows: &mut Vec<UsageWindow>, key: &str, label_prefix: &str, w: &Value) {
    let Some(percent) = w.get("used_percent").and_then(|v| v.as_f64()) else {
        return;
    };
    let label = format!(
        "{}{}",
        label_prefix,
        window_label(w.get("limit_window_seconds").and_then(|v| v.as_i64()))
    );
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
        push_codex_window(&mut windows, "primary", "", w);
    }
    if let Some(w) = body.pointer("/rate_limit/secondary_window") {
        push_codex_window(&mut windows, "secondary", "", w);
    }
    if let Some(extra) = body.get("additional_rate_limits").and_then(|v| v.as_array()) {
        for item in extra {
            let name = item
                .get("limit_name")
                .and_then(|v| v.as_str())
                .unwrap_or("모델");
            if let Some(w) = item.pointer("/rate_limit/primary_window") {
                push_codex_window(
                    &mut windows,
                    &format!("model:{name}"),
                    &format!("{name} · "),
                    w,
                );
            }
        }
    }
    Usage { windows }
}

async fn fetch_codex(env: &Env, profile: Option<&str>) -> Result<Usage, String> {
    let (token, account_id) = codex_token(env, profile)?;
    let mut req = reqwest::Client::new().get(CODEX_USAGE_URL).bearer_auth(&token);
    if let Some(id) = account_id {
        req = req.header("ChatGPT-Account-Id", id);
    }
    let body = get_json(req).await?;
    Ok(parse_codex_usage(&body))
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
/// 실패 시 대신 보여줄 수 있는 직전 값의 최대 나이 — 이보다 오래되면 에러를 그대로 보여준다
/// (만료 안내 같은 진짜 문제를 몇 시간 전 숫자로 가리지 않기 위함)
const STALE_MAX: std::time::Duration = std::time::Duration::from_secs(600);

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

pub async fn fetch(
    env: &Env,
    provider: Provider,
    profile: Option<&str>,
) -> Result<Usage, String> {
    let key = cache_key(env, provider, profile);
    if let Ok(map) = cache().lock() {
        if let Some((at, cached)) = map.get(&key) {
            if at.elapsed() < CACHE_TTL {
                return Ok(cached.clone());
            }
        }
    }
    let result = match provider {
        Provider::Claude => fetch_claude(env, profile).await,
        Provider::Codex => fetch_codex(env, profile).await,
    };
    match result {
        Ok(usage) => {
            if let Ok(mut map) = cache().lock() {
                map.insert(key, (std::time::Instant::now(), usage.clone()));
            }
            Ok(usage)
        }
        Err(e) => {
            if let Ok(map) = cache().lock() {
                if let Some((at, cached)) = map.get(&key) {
                    if at.elapsed() < STALE_MAX {
                        return Ok(cached.clone());
                    }
                }
            }
            Err(e)
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
        assert_eq!(usage.windows[0].percent, 62.0);
        assert_eq!(usage.windows[2].key, "weekly:Fable");
        assert_eq!(usage.windows[2].label, "주간 · Fable");
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
        assert_eq!(usage.windows[0].label, "주간");
        assert_eq!(usage.windows[0].percent, 30.0);
        assert_eq!(usage.windows[1].label, "5시간");
        assert_eq!(usage.windows[2].key, "model:GPT-Test-Model");
        assert_eq!(usage.windows[2].label, "GPT-Test-Model · 주간");
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

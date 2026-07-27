//! 사용량 조회 — 저장된 OAuth 토큰으로 각 계정의 한도 소진율을 읽는다.
//!
//! 클로드: GET https://api.anthropic.com/api/oauth/usage
//!   (Authorization: Bearer <accessToken> + anthropic-beta: oauth-2025-04-20)
//! 토큰 값은 절대 로그·에러 메시지에 싣지 않는다.

use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;

use crate::accounts::{now, read_json, Env, Provider};

const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CLAUDE_OAUTH_BETA: &str = "oauth-2025-04-20";

#[derive(Serialize)]
pub struct UsageWindow {
    pub key: String,
    pub label: String,
    pub percent: f64,
    pub resets_at: Option<String>,
}

#[derive(Serialize)]
pub struct Usage {
    pub windows: Vec<UsageWindow>,
}

/// 조회 대상 토큰 파일: 프로필 이름이 있으면 보관함, 없으면 활성 파일
fn credential_path(env: &Env, provider: Provider, profile: Option<&str>) -> PathBuf {
    match profile {
        Some(name) => env
            .profiles_dir(provider)
            .join(name)
            .join(provider.credential_file_name()),
        None => env.live_credential_path(provider),
    }
}

fn claude_access_token(env: &Env, profile: Option<&str>) -> Result<String, String> {
    let path = credential_path(env, Provider::Claude, profile);
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

async fn fetch_claude(env: &Env, profile: Option<&str>) -> Result<Usage, String> {
    let token = claude_access_token(env, profile)?;
    let client = reqwest::Client::new();
    let resp = client
        .get(CLAUDE_USAGE_URL)
        .bearer_auth(&token)
        .header("anthropic-beta", CLAUDE_OAUTH_BETA)
        .send()
        .await
        .map_err(|e| format!("사용량 요청 실패: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("사용량 조회 실패: HTTP {}", status.as_u16()));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("응답 파싱 실패: {e}"))?;
    Ok(parse_claude_usage(&body))
}

pub async fn fetch(
    env: &Env,
    provider: Provider,
    profile: Option<&str>,
) -> Result<Usage, String> {
    match provider {
        Provider::Claude => fetch_claude(env, profile).await,
        Provider::Codex => Err("코덱스 사용량은 아직 준비 중입니다 (이슈 #5)".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn expired_token_is_rejected_with_guidance() {
        let base = std::env::temp_dir().join(format!(
            "switcher-usage-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join(".claude")).unwrap();
        let env = Env {
            home: base.clone(),
            store: base.join(".switcher"),
        };
        fs::write(
            env.live_credential_path(Provider::Claude),
            r#"{"claudeAiOauth":{"accessToken":"fake","expiresAt":1000}}"#,
        )
        .unwrap();
        let err = claude_access_token(&env, None).unwrap_err();
        assert!(err.contains("만료"));
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

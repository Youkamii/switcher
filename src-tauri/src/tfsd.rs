//! TFSD (Token Full Self-Driving) — 사용량 기반 자동 계정 전환.
//!
//! 활성 계정의 기준 창 사용률이 임계치(90%)에 닿으면, 같은 기준으로 여유가
//! 가장 많은 계정으로 스스로 갈아탄다. 기준 창(사용자 결정, #35):
//! - 클로드: "Fable" 창 — 모델 전용 한도가 실질 병목이라 5 Hours가 널널해도
//!   Fable이 차면 그 계정은 끝이다. Fable 창이 없으면 "5 Hours" 폴백.
//! - 코덱스: "5 Hours" (없으면 첫 창).
//!
//! 규칙:
//! - stale(조회 막힘) 수치로는 판단하지 않는다 — 낡은 정보로 갈아타지 않는다
//! - 전환 후 프로바이더당 10분 쿨다운 — 핑퐁 방지
//! - CLI 세션 사용 중 전환도 허용 — 사용자 실측으로 문제 없음 확인 (#35),
//!   별도 프로세스 감지 안전장치는 두지 않는다

use crate::accounts::{self, Env, Provider};
use crate::settings;
use crate::usage::{self, UsageWindow};
use std::time::{Duration, Instant};
use tauri::Emitter;

const THRESHOLD: f64 = 90.0;
const COOLDOWN: Duration = Duration::from_secs(600);
/// 감시 주기 — 사용량 캐시(60초)보다 길게 잡아 API 부하를 늘리지 않는다
const TICK: Duration = Duration::from_secs(120);

/// 판단 기준 창의 사용률 — 기준 창이 없으면 None (판단 보류)
fn criterion_percent(windows: &[UsageWindow], provider: Provider) -> Option<f64> {
    if provider == Provider::Claude {
        if let Some(window) = windows.iter().find(|w| w.label == "Fable") {
            return Some(window.percent);
        }
    }
    windows
        .iter()
        .find(|w| w.label == "5 Hours")
        .or_else(|| windows.first())
        .map(|w| w.percent)
}

/// 후보 중 목표 고르기 — 임계치 미만이면서 기준 사용률이 가장 낮은 계정
fn choose_target(candidates: &[(String, f64)]) -> Option<&str> {
    candidates
        .iter()
        .filter(|(_, percent)| *percent < THRESHOLD)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(name, _)| name.as_str())
}

/// 한 프로바이더 평가 — 전환해야 하면 (현재 표시명, 대상 프로필명, 대상 표시명)
async fn evaluate(env: &Env, provider: Provider) -> Option<(String, String, String)> {
    let snap = accounts::list(env, provider).ok()?;
    if snap.profiles.len() < 2 {
        return None;
    }
    let active = snap.profiles.iter().find(|p| p.active)?;
    let active_usage = usage::fetch(env, provider, None).await.ok()?;
    if active_usage.stale {
        return None;
    }
    let active_percent = criterion_percent(&active_usage.windows, provider)?;
    if active_percent < THRESHOLD {
        return None;
    }
    let mut candidates: Vec<(String, f64)> = Vec::new();
    for profile in snap.profiles.iter().filter(|p| !p.active) {
        let Ok(profile_usage) = usage::fetch(env, provider, Some(&profile.name)).await else {
            continue;
        };
        if profile_usage.stale {
            continue;
        }
        if let Some(percent) = criterion_percent(&profile_usage.windows, provider) {
            candidates.push((profile.name.clone(), percent));
        }
    }
    let target = choose_target(&candidates)?.to_string();
    let to_display = snap
        .profiles
        .iter()
        .find(|p| p.name == target)
        .and_then(|p| p.email.clone())
        .unwrap_or_else(|| target.clone());
    let from_display = active.email.clone().unwrap_or_else(|| active.name.clone());
    Some((from_display, target, to_display))
}

/// 감시 루프 — 앱 수명 동안 돌며, 설정이 꺼져 있으면 틱마다 조용히 지나간다
pub fn spawn(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last_switch: [Option<Instant>; 2] = [None, None];
        loop {
            tokio::time::sleep(TICK).await;
            let Ok(env) = Env::real() else { continue };
            if !settings::load_flag(&env.store, settings::KEY_TFSD, false) {
                continue;
            }
            for (index, provider) in [Provider::Claude, Provider::Codex].into_iter().enumerate()
            {
                if last_switch[index].is_some_and(|at| at.elapsed() < COOLDOWN) {
                    continue;
                }
                let Some((from, target, to)) = evaluate(&env, provider).await else {
                    continue;
                };
                match accounts::switch(&env, provider, &target) {
                    Ok(_) => {
                        last_switch[index] = Some(Instant::now());
                        let _ = app.emit(
                            "tfsd-switched",
                            serde_json::json!({
                                "provider": provider.dir_name().to_uppercase(),
                                "from": from,
                                "to": to,
                            }),
                        );
                    }
                    Err(e) => eprintln!("TFSD 전환 실패: {e}"),
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(label: &str, percent: f64) -> UsageWindow {
        UsageWindow {
            key: label.to_lowercase(),
            label: label.to_string(),
            percent,
            resets_at: None,
        }
    }

    #[test]
    fn claude_judges_by_fable_when_present() {
        let windows = vec![window("5 Hours", 10.0), window("Weekly", 30.0), window("Fable", 95.0)];
        assert_eq!(criterion_percent(&windows, Provider::Claude), Some(95.0));
        // 코덱스는 Fable 창을 무시하고 5 Hours를 본다
        assert_eq!(criterion_percent(&windows, Provider::Codex), Some(10.0));
    }

    #[test]
    fn falls_back_to_five_hours_then_first_window() {
        let five = vec![window("5 Hours", 42.0), window("Weekly", 80.0)];
        assert_eq!(criterion_percent(&five, Provider::Claude), Some(42.0));
        let weekly_only = vec![window("Weekly", 63.0)];
        assert_eq!(criterion_percent(&weekly_only, Provider::Claude), Some(63.0));
        assert_eq!(criterion_percent(&[], Provider::Claude), None);
    }

    #[test]
    fn chooses_lowest_candidate_under_threshold() {
        let candidates = vec![
            ("a".to_string(), 88.0),
            ("b".to_string(), 15.0),
            ("c".to_string(), 40.0),
        ];
        assert_eq!(choose_target(&candidates), Some("b"));
        // 전부 임계치 이상이면 전환하지 않는다 — 더 나쁜 곳으로 갈아타지 않기
        let full = vec![("a".to_string(), 95.0), ("b".to_string(), 91.0)];
        assert_eq!(choose_target(&full), None);
        assert_eq!(choose_target(&[]), None);
    }
}

//! TFSD (Token Full Self-Driving) — 사용량 기반 자동 계정 전환.
//!
//! 활성 계정의 기준 창 사용률이 임계치(90%)에 닿으면, 같은 기준으로 여유가
//! 가장 많은 계정으로 스스로 갈아탄다. 기준 창(사용자 결정, #35):
//! - 클로드: "Fable" 창 — 모델 전용 한도가 실질 병목이라 5 Hours가 널널해도
//!   Fable이 차면 그 계정은 끝이다. Fable 창이 없으면 "5 Hours" 폴백.
//! - 코덱스: "5 Hours" (없으면 첫 창).
//!
//! 규칙 (red-review 반영):
//! - stale(조회 막힘) 수치로는 판단하지 않는다 — 낡은 정보로 갈아타지 않는다
//! - 전환 직전에 활성 계정을 재확인한다 — 평가하는 사이 사용자가 손수 전환했으면
//!   그 선택을 존중하고 물러난다 (TOCTOU)
//! - 전환 성공·실패 후 10분, 전 계정 만석이면 5분 물러난다 — 핑퐁·무한 재시도·
//!   지속 폴링 방지
//! - 무인 전환은 ~/.switcher/tfsd-history.log에 영속 기록한다 — 위젯이 숨어
//!   토스트를 못 봐도 사후 확인 가능 (토큰은 절대 싣지 않는다)
//! - CLI 세션 사용 중 전환도 허용 — 사용자 실측으로 문제 없음 확인 (#35)

use crate::accounts::{self, Env, Provider};
use crate::settings;
use crate::usage::{self, UsageWindow};
use std::time::{Duration, Instant};
use tauri::Emitter;

const THRESHOLD: f64 = 90.0;
const COOLDOWN: Duration = Duration::from_secs(600);
/// 전 계정 만석(갈 곳 없음)일 때의 백오프 — 틱마다 전 계정 실조회를 반복하지 않는다
const SATURATED_BACKOFF: Duration = Duration::from_secs(300);
/// 감시 주기 — 사용량 캐시 TTL(60초) **안쪽**이어야 연속 틱이 캐시를 재사용한다.
/// (TTL보다 길면 매 틱이 캐시 미스 = 실호출이 된다 — red-review) 실호출은 ~2틱당 1회.
const TICK: Duration = Duration::from_secs(55);

/// 판단 기준 창의 사용률 — 기준 창이 없으면 None (판단 보류)
fn criterion_percent(windows: &[UsageWindow], provider: Provider) -> Option<f64> {
    if provider == Provider::Claude {
        // 완전일치가 아니라 contains — API의 모델 표시명이 "Fable 5" 등으로 바뀌어도
        // 판정 기준이 조용히 폴백으로 뒤바뀌지 않게 (red-review)
        if let Some(window) = windows.iter().find(|w| w.label.contains("Fable")) {
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

/// 한 프로바이더의 평가 결과
enum Verdict {
    /// 전환 불필요 — 임계 미달·프로필 부족·판단 보류
    Idle,
    /// 활성은 꽉 찼는데 갈 곳이 없다 — 백오프 대상
    Saturated,
    Switch {
        /// 평가 시점의 활성 프로필 이름 — 전환 직전 재확인에 쓴다
        active_name: String,
        from: String,
        target: String,
        to: String,
    },
}

async fn evaluate(env: &Env, provider: Provider) -> Verdict {
    let Ok(snap) = accounts::list(env, provider) else {
        return Verdict::Idle;
    };
    if snap.profiles.len() < 2 {
        return Verdict::Idle;
    }
    let Some(active) = snap.profiles.iter().find(|p| p.active) else {
        return Verdict::Idle;
    };
    let Ok(active_usage) = usage::fetch(env, provider, None).await else {
        return Verdict::Idle;
    };
    if active_usage.stale {
        return Verdict::Idle;
    }
    let Some(active_percent) = criterion_percent(&active_usage.windows, provider) else {
        return Verdict::Idle;
    };
    if active_percent < THRESHOLD {
        return Verdict::Idle;
    }
    let mut candidates: Vec<(String, f64)> = Vec::new();
    for profile in snap.profiles.iter().filter(|p| !p.active) {
        let Ok(profile_usage) = usage::fetch(env, provider, Some(&profile.name)).await else {
            continue;
        };
        if profile_usage.stale {
            continue;
        }
        // 세션 게이트: 기준 창(Fable)이 널널해도 5 Hours가 만석이면 전환 직후
        // 막힌다 — 그런 후보는 제외한다 (red-review)
        let session_open = profile_usage
            .windows
            .iter()
            .find(|w| w.label == "5 Hours")
            .map_or(true, |w| w.percent < THRESHOLD);
        if !session_open {
            continue;
        }
        if let Some(percent) = criterion_percent(&profile_usage.windows, provider) {
            candidates.push((profile.name.clone(), percent));
        }
    }
    let Some(target) = choose_target(&candidates).map(str::to_string) else {
        return Verdict::Saturated;
    };
    let to = snap
        .profiles
        .iter()
        .find(|p| p.name == target)
        .and_then(|p| p.email.clone())
        .unwrap_or_else(|| target.clone());
    Verdict::Switch {
        active_name: active.name.clone(),
        from: active.email.clone().unwrap_or_else(|| active.name.clone()),
        target,
        to,
    }
}

/// 무인 전환의 영속 흔적 — 유닉스 초·프로바이더·표시명만. 64KB를 넘으면 새로 시작.
fn append_history(store: &std::path::Path, provider: Provider, from: &str, to: &str) {
    use std::io::Write;
    let path = store.join("tfsd-history.log");
    if std::fs::metadata(&path)
        .map(|m| m.len() > 64 * 1024)
        .unwrap_or(false)
    {
        let _ = std::fs::remove_file(&path);
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{secs}\t{}\t{from} -> {to}", provider.dir_name());
    }
}

/// 감시 루프 — 앱 수명 동안 돌며, 설정이 꺼져 있으면 틱마다 조용히 지나간다
pub fn spawn(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        // 프로바이더별 보류 만료 시각 (전환·실패 10분, 만석 5분)
        let mut hold_until: [Option<Instant>; 2] = [None, None];
        loop {
            tokio::time::sleep(TICK).await;
            let Ok(env) = Env::real() else { continue };
            for (index, provider) in [Provider::Claude, Provider::Codex].into_iter().enumerate()
            {
                // 평가·전환 사이에도 꺼짐이 즉시 먹도록 프로바이더마다 다시 읽는다
                if !settings::load_flag(&env.store, settings::KEY_TFSD, false) {
                    break;
                }
                if hold_until[index].is_some_and(|until| Instant::now() < until) {
                    continue;
                }
                // 타임아웃 — usage GET에 타임아웃이 없어(기존 코드) 행 걸린 커넥션
                // 하나가 이 루프를 영구 정지시킬 수 있다 (red-review)
                let verdict = match tokio::time::timeout(
                    Duration::from_secs(90),
                    evaluate(&env, provider),
                )
                .await
                {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                match verdict {
                    Verdict::Idle => {}
                    Verdict::Saturated => {
                        hold_until[index] = Some(Instant::now() + SATURATED_BACKOFF);
                    }
                    Verdict::Switch {
                        active_name,
                        from,
                        target,
                        to,
                    } => {
                        // 평가하는 사이(네트워크 대기 포함) 사용자가 손수 전환했으면
                        // 그 선택을 존중한다 — 무인 프로세스가 사람을 이기면 안 된다
                        let still_active = accounts::list(&env, provider)
                            .ok()
                            .and_then(|s| s.profiles.into_iter().find(|p| p.active))
                            .map(|p| p.name == active_name)
                            .unwrap_or(false);
                        if !still_active {
                            continue;
                        }
                        // 파일·키체인 작업이라 블로킹 풀에서
                        let target_owned = target.clone();
                        let switched = tauri::async_runtime::spawn_blocking(move || {
                            let env = Env::real()?;
                            accounts::switch(&env, provider, &target_owned).map(|_| ())
                        })
                        .await
                        .map_err(|e| format!("전환 작업 실패: {e}"))
                        .and_then(|r| r);
                        match switched {
                            Ok(()) => {
                                hold_until[index] = Some(Instant::now() + COOLDOWN);
                                append_history(&env.store, provider, &from, &to);
                                let _ = app.emit(
                                    "tfsd-switched",
                                    serde_json::json!({
                                        "provider": match provider {
                                            Provider::Claude => "Claude",
                                            Provider::Codex => "Codex",
                                        },
                                        "from": from,
                                        "to": to,
                                    }),
                                );
                            }
                            Err(e) => {
                                // 실패도 쿨다운 — 120초마다 무한 재시도하지 않는다
                                hold_until[index] = Some(Instant::now() + COOLDOWN);
                                eprintln!("TFSD 전환 실패: {e}");
                            }
                        }
                    }
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

    #[test]
    fn history_appends_and_resets_when_large() {
        let store = std::env::temp_dir().join(format!("switcher-tfsd-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&store);
        std::fs::create_dir_all(&store).unwrap();
        append_history(&store, Provider::Claude, "a@x.com", "b@x.com");
        let text = std::fs::read_to_string(store.join("tfsd-history.log")).unwrap();
        assert!(text.contains("claude\ta@x.com -> b@x.com"));
        assert!(!text.contains("token"), "토큰류 문자열이 실릴 자리가 없어야 한다");
        // 64KB 초과 시 새로 시작
        std::fs::write(store.join("tfsd-history.log"), vec![b'x'; 70 * 1024]).unwrap();
        append_history(&store, Provider::Codex, "c", "d");
        let text = std::fs::read_to_string(store.join("tfsd-history.log")).unwrap();
        assert!(text.len() < 1024);
        assert!(text.contains("codex\tc -> d"));
        let _ = std::fs::remove_dir_all(&store);
    }
}

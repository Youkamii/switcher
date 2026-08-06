//! TFSD (Token Full Self-Driving) — 사용량 기반 자동 계정 전환.
//!
//! 판정은 병목 규칙(#37) 하나다: 계정 상태 = 모든 사용량 창 중 최댓값(병목).
//! 어느 창이든 하나가 차면(5 Hours든 Weekly든 Fable이든) 실사용이 막히는 건
//! 같으므로, 활성 계정의 병목이 임계치(90%)에 닿으면 — 모든 창에 여유가 있는
//! 후보 중 — 병목이 가장 낮은 계정으로 스스로 갈아탄다.
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

/// 계정의 병목 사용률 — 모든 창 중 최댓값. 창이 없으면 None (판단 보류).
/// 새 창 종류가 API에 추가돼도 자동으로 판정에 들어간다 — 라벨 특례 없음.
fn bottleneck_percent(windows: &[UsageWindow]) -> Option<f64> {
    windows.iter().map(|w| w.percent).reduce(f64::max)
}

/// 후보 중 목표 고르기 — 병목이 임계치 미만이면서 가장 낮은 계정
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
    /// 활성은 꽉 찼는데 갈 곳이 없다 — 후보가 전부 만석이거나 전부
    /// 조회 불가(stale·실패)인 경우를 구분하지 않는다. 백오프 대상.
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
    let Some(active_percent) = bottleneck_percent(&active_usage.windows) else {
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
        // 병목이 임계치 이상인 후보는 choose_target이 걸러낸다 — 어느 창이
        // 막혔든(Weekly 포함) 전환 직후 막히는 계정으로는 가지 않는다 (#37)
        if let Some(percent) = bottleneck_percent(&profile_usage.windows) {
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
                        // 평가하는 사이(네트워크 대기 최대 90초) 사용자가 TFSD를
                        // 껐으면 전환을 강행하지 않는다 (red-review #37)
                        if !settings::load_flag(&env.store, settings::KEY_TFSD, false) {
                            break;
                        }
                        // 평가하는 사이 사용자가 손수 전환했으면 그 선택을
                        // 존중한다 — 무인 프로세스가 사람을 이기면 안 된다
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
                                // 실패도 쿨다운 — 틱(55초)마다 무한 재시도하지 않는다
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
    fn bottleneck_is_worst_window_regardless_of_label() {
        // Fable이 병목이면 Fable을 본다 — #35 결정의 일반화
        let fable_bound = vec![window("5 Hours", 10.0), window("Weekly", 30.0), window("Fable", 95.0)];
        assert_eq!(bottleneck_percent(&fable_bound), Some(95.0));
        // Weekly가 병목이면 Weekly를 본다 — 기존 규칙이 놓치던 경우 (#37)
        let weekly_bound = vec![window("5 Hours", 10.0), window("Weekly", 100.0), window("Fable", 50.0)];
        assert_eq!(bottleneck_percent(&weekly_bound), Some(100.0));
        let single = vec![window("5 Hours", 42.0)];
        assert_eq!(bottleneck_percent(&single), Some(42.0));
        assert_eq!(bottleneck_percent(&[]), None);
    }

    #[test]
    fn weekly_full_candidate_is_never_chosen() {
        // Fable 널널 + 5 Hours 널널이어도 Weekly 만석이면 병목 100 → choose_target이 거른다
        let trap = bottleneck_percent(&[
            window("5 Hours", 5.0),
            window("Weekly", 100.0),
            window("Fable", 10.0),
        ])
        .unwrap();
        let healthy = bottleneck_percent(&[
            window("5 Hours", 60.0),
            window("Weekly", 40.0),
            window("Fable", 30.0),
        ])
        .unwrap();
        let candidates = vec![("trap".to_string(), trap), ("healthy".to_string(), healthy)];
        assert_eq!(choose_target(&candidates), Some("healthy"));
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

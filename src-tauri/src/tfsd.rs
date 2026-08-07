//! TFSD (Token Full Self-Driving) — 사용량 기반 자동 계정 전환.
//!
//! 판정은 병목 규칙(#37) 하나다: 계정 상태 = 모든 사용량 창 중 최댓값(병목).
//! 어느 창이든 하나가 차면(5 Hours든 Weekly든 Fable이든) 실사용이 막히는 건
//! 같으므로, 활성 계정의 병목이 임계치(90%)에 닿으면 — 모든 창에 여유가 있는
//! 후보 중 — 병목이 가장 낮은 계정으로 스스로 갈아탄다.
//!
//! 규칙 (red-review 반영):
//! - 꽉 찬 창들이 전부 30분 안에 리셋되면 전환을 보류한다 — 리셋 대기 (#39)
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
/// 리셋 대기(coasting) 유예 — 꽉 찬 창들이 전부 이 시간 안에 리셋되면 전환을
/// 보류한다. 90% 시점에 남은 10%는 5 Hours 창 기준 비례 사용량 ~30분이라,
/// 그 여유로 리셋까지 버티는 쪽이 계정 교체보다 낫다 (#39).
const RESET_GRACE: Duration = Duration::from_secs(30 * 60);
/// 선제 전환 지평(#45) — 소진 예측이 이 안이면 임계(90%) 전이라도 발동한다.
/// 고속 소모 중엔 90% 도달 직후 몇 분 만에 벽이라, 닿기 전에 미리 갈아탄다.
const EXHAUST_SOON: i64 = 600;

/// 계정의 병목 사용률 — 모든 창 중 최댓값. 창이 없으면 None (판단 보류).
/// 새 창 종류가 API에 추가돼도 자동으로 판정에 들어간다 — 라벨 특례 없음.
fn bottleneck_percent(windows: &[UsageWindow]) -> Option<f64> {
    windows.iter().map(|w| w.percent).reduce(f64::max)
}

/// 그레고리력 날짜 → 1970-01-01 기준 일수 (Howard Hinnant days_from_civil).
/// chrono 없이 ISO-8601을 유닉스 초로 바꾸기 위한 최소 조각.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// resets_at → 유닉스 초. 실측 형식 둘만 감당한다: 코덱스는 유닉스 초 문자열,
/// 클로드는 ISO-8601("2026-07-27T10:50:00Z"; 소수초·±HH:MM 오프셋 관용).
/// 그 외 형식은 None — 보류 판단을 포기할 뿐 전환 판정 자체는 막지 않는다.
fn parse_resets_at(text: &str) -> Option<i64> {
    let text = text.trim();
    if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
        return text.parse().ok();
    }
    let bytes = text.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    // 필드는 숫자로만 — str::parse는 "+9" 같은 부호도 받아들이므로 직접 거른다 (red-review)
    let num = |range: std::ops::Range<usize>| -> Option<i64> {
        let field = text.get(range)?;
        if !field.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        field.parse().ok()
    };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    // 존재하지 않는 날짜(2월 31일, 평년 2월 29일)는 거른다 (red-review)
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if !(1..=month_days[(mo - 1) as usize]).contains(&d) {
        return None;
    }
    let mut idx = 19;
    if bytes.get(idx) == Some(&b'.') {
        idx += 1;
        while bytes.get(idx).is_some_and(|b| b.is_ascii_digit()) {
            idx += 1;
        }
    }
    let offset_secs = match bytes.get(idx) {
        None => 0, // 오프셋 표기가 없으면 UTC로 간주
        Some(b'Z') if idx + 1 == bytes.len() => 0,
        Some(sign @ (b'+' | b'-')) => {
            let rest = text.get(idx + 1..)?;
            let (oh, om) = if rest.len() == 5 && rest.as_bytes()[2] == b':' {
                (num(idx + 1..idx + 3)?, num(idx + 4..idx + 6)?)
            } else if rest.len() == 4 {
                (num(idx + 1..idx + 3)?, num(idx + 3..idx + 5)?)
            } else {
                return None;
            };
            if oh > 23 || om > 59 {
                return None;
            }
            let total = oh * 3600 + om * 60;
            if *sign == b'+' {
                total
            } else {
                -total
            }
        }
        _ => return None,
    };
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s - offset_secs)
}

/// 리셋 대기(coasting) — 막힘이 임박한 창들(임계 이상 또는 소진 예측 임박)이
/// **전부** 유예 안에 리셋되고 그 리셋이 예측 소진보다 이를 때만 참.
/// 하나라도 리셋이 멀거나·리셋 시각을 모르거나·리셋 전에 벽에 닿을 전망이면
/// 거짓 — 모르는 정보로 전환을 막지 않는다 (#39, #45).
fn saturated_windows_reset_soon(windows: &[UsageWindow], now_secs: i64) -> bool {
    let grace = RESET_GRACE.as_secs() as i64;
    let mut any = false;
    for window in windows.iter().filter(|w| {
        w.percent >= THRESHOLD || w.exhausts_in_secs.is_some_and(|s| s <= EXHAUST_SOON)
    }) {
        any = true;
        // 이미 다 찬 창은 기다릴 여유 자체가 없다 — 리셋이 코앞이어도 전환이
        // 즉시 구제한다. 이력 유무로 행동이 갈리던 비결정성 제거 (red-review #45)
        if window.percent >= 100.0 {
            return false;
        }
        let Some(reset) = window.resets_at.as_deref().and_then(parse_resets_at) else {
            return false;
        };
        // 리셋이 유예보다 한참 과거인데 창이 여전히 꽉 차 있으면 데이터 이상 —
        // 영구 보류에 빠지지 않게 보류를 포기한다 (리셋 직후 캐시 지연은 유예가 흡수)
        let diff = reset - now_secs;
        if diff > grace || diff < -grace {
            return false;
        }
        // 리셋 전에 벽에 닿을 전망이면 기다릴 이유가 없다 (#45)
        if window.exhausts_in_secs.is_some_and(|s| s < diff) {
            return false;
        }
    }
    any
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
    // 예측 소진(#45): 어느 창이든 현재 속도로 10분 안에 벽에 닿을 전망이면
    // 임계 전이라도 발동한다 — 특히 Fable 같은 주간 창은 소진되면 며칠짜리다
    let exhaust_imminent = active_usage
        .windows
        .iter()
        .any(|w| w.exhausts_in_secs.is_some_and(|s| s <= EXHAUST_SOON));
    if active_percent < THRESHOLD && !exhaust_imminent {
        return Verdict::Idle;
    }
    // 리셋 대기: 꽉 찬 창들이 전부 30분 안에 리셋되면 전환하지 않는다 —
    // 남은 여유로 버티는 게 낫다. 다음 틱이 재평가하고, 리셋이 지나면 자연 해소.
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    if saturated_windows_reset_soon(&active_usage.windows, now_secs) {
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
            exhausts_in_secs: None,
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
    fn parses_both_reset_formats() {
        // 코덱스: 유닉스 초 문자열 / 클로드: ISO-8601 (2026-01-01T00:00:00Z = 1767225600)
        assert_eq!(parse_resets_at("1767225600"), Some(1767225600));
        assert_eq!(parse_resets_at("2026-01-01T00:00:00Z"), Some(1767225600));
        // KST 오프셋과 소수초도 같은 시각으로 수렴한다
        assert_eq!(parse_resets_at("2026-01-01T09:00:00+09:00"), Some(1767225600));
        assert_eq!(parse_resets_at("2026-01-01T00:00:00.123Z"), Some(1767225600));
        assert_eq!(parse_resets_at(""), None);
        assert_eq!(parse_resets_at("곧"), None);
        assert_eq!(parse_resets_at("2026-13-01T00:00:00Z"), None);
        // 존재하지 않는 날짜·쓰레기 오프셋·부호 섞인 필드는 전부 거른다 (red-review)
        assert_eq!(parse_resets_at("2026-02-31T00:00:00Z"), None);
        assert_eq!(parse_resets_at("2100-02-29T00:00:00Z"), None); // 2100은 평년
        assert_eq!(parse_resets_at("2024-02-29T00:00:00Z"), Some(1709164800)); // 윤년은 유효
        assert_eq!(parse_resets_at("2026-01-01T00:00:00+99:99"), None);
        assert_eq!(parse_resets_at("2026-+9-01T00:00:00Z"), None);
    }

    #[test]
    fn coasts_only_when_every_full_window_resets_soon() {
        let now = 1_700_000_000i64;
        let soon = (now + 600).to_string(); // 10분 뒤
        let far = (now + 7200).to_string(); // 2시간 뒤
        let win = |percent: f64, resets: Option<&str>| UsageWindow {
            key: "k".to_string(),
            label: "w".to_string(),
            percent,
            resets_at: resets.map(String::from),
            exhausts_in_secs: None,
        };
        // 꽉 찬 창이 10분 뒤 리셋 → 보류 (여유 창의 먼 리셋은 무관)
        assert!(saturated_windows_reset_soon(
            &[win(95.0, Some(&soon)), win(50.0, Some(&far))],
            now
        ));
        // 리셋이 먼 창이 90 이상이면 그게 진짜 병목 → 보류 안 함
        assert!(!saturated_windows_reset_soon(
            &[win(95.0, Some(&soon)), win(91.0, Some(&far))],
            now
        ));
        // 리셋 시각을 모르는 꽉 찬 창 → 보류 안 함 (기존 동작 유지)
        assert!(!saturated_windows_reset_soon(&[win(95.0, None)], now));
        // 꽉 찬 창이 없으면 거짓
        assert!(!saturated_windows_reset_soon(&[win(10.0, Some(&soon))], now));
        // 리셋이 이미 지났으면(낡은 캐시) 곧 풀릴 것 — 보류로 친다
        let past = (now - 60).to_string();
        assert!(saturated_windows_reset_soon(&[win(99.0, Some(&past))], now));
        // 리셋이 유예보다 한참 과거인데 여전히 만석 = 데이터 이상 — 영구 보류 방지 (red-review)
        let stale_past = (now - 86400 * 30).to_string();
        assert!(!saturated_windows_reset_soon(&[win(99.0, Some(&stale_past))], now));
    }

    #[test]
    fn exhaustion_prediction_beats_coasting_and_triggers_early() {
        let now = 1_700_000_000i64;
        let reset_10m = (now + 600).to_string();
        let mk = |percent: f64, resets: Option<&str>, exhausts: Option<i64>| UsageWindow {
            key: "k".to_string(),
            label: "w".to_string(),
            percent,
            resets_at: resets.map(String::from),
            exhausts_in_secs: exhausts,
        };
        // 리셋 10분 뒤인데 소진 예측 5분 — 기다리면 벽에 박는다 → 보류 안 함 (#45)
        assert!(!saturated_windows_reset_soon(
            &[mk(92.0, Some(&reset_10m), Some(300))],
            now
        ));
        // 소진 예측 15분 > 리셋 10분 — 리셋이 먼저 온다 → 보류
        assert!(saturated_windows_reset_soon(
            &[mk(92.0, Some(&reset_10m), Some(900))],
            now
        ));
        // 임계 미만이어도 소진 임박이면 막힘 후보 — 리셋이 멀면 보류 안 함 (선제 전환 경로)
        let reset_2h = (now + 7200).to_string();
        assert!(!saturated_windows_reset_soon(
            &[mk(85.0, Some(&reset_2h), Some(300))],
            now
        ));
        // 소진 임박 창의 리셋이 코앞이면(2분) 보류 — 갈아탈 필요가 없다
        let reset_2m = (now + 120).to_string();
        assert!(saturated_windows_reset_soon(
            &[mk(85.0, Some(&reset_2m), Some(300))],
            now
        ));
        // 이미 100% = 여유 제로 — 리셋이 코앞이고 이력이 없어도 보류하지 않는다
        assert!(!saturated_windows_reset_soon(
            &[mk(100.0, Some(&reset_2m), None)],
            now
        ));
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

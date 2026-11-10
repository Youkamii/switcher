//! Per-connection monthly USD budgets. Unknown costs are never reported as zero.
//! Administrative credentials stay in the OS vault, separate from inference keys.
use super::*;
use chrono::{DateTime, Datelike, Utc};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

#[cfg(test)]
thread_local! { static FAIL_STORE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

fn default_budget() -> f64 {
    195.0
}
#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    #[default]
    Manual,
    Automatic,
}
#[derive(Clone, Serialize, Deserialize)]
struct Report {
    usd: f64,
    checked_at: u64,
    provider_updated_at: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct State {
    budget_usd: f64,
    source: Source,
    api_key_id: String,
    admin_key_stored: bool,
    revision: String,
    reports: BTreeMap<String, Report>,
    last_attempt: u64,
    attempt_month: String,
    attempt_id: String,
    error: Option<String>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            budget_usd: default_budget(),
            source: Source::Manual,
            api_key_id: String::new(),
            admin_key_stored: false,
            revision: String::new(),
            reports: BTreeMap::new(),
            last_attempt: 0,
            attempt_month: String::new(),
            attempt_id: String::new(),
            error: None,
        }
    }
}
#[derive(Serialize, Clone)]
pub struct Usage {
    pub month: String,
    pub budget_usd: f64,
    pub spent_usd: Option<f64>,
    pub remaining_usd: Option<f64>,
    pub percent: Option<f64>,
    pub source: Source,
    pub api_key_id: String,
    pub admin_key_stored: bool,
    pub checked_at: Option<u64>,
    pub provider_updated_at: Option<String>,
    pub error: Option<String>,
}
#[derive(Deserialize)]
pub struct Settings {
    pub budget_usd: f64,
    pub source: Source,
    pub spent_usd: Option<f64>,
    pub api_key_id: Option<String>,
    pub admin_key: Option<String>,
}
fn credential(provider: Provider, name: &str) -> String {
    format!(
        "Switcher-Usage-{}-{}",
        provider.dir_name(),
        name.to_ascii_lowercase()
    )
}
fn period(now: u64) -> (String, i64, String, String) {
    let date = DateTime::<Utc>::from_timestamp(now as i64, 0).unwrap();
    let first = date
        .date_naive()
        .with_day(1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    (
        date.format("%Y-%m").to_string(),
        first.timestamp(),
        first.to_rfc3339(),
        date.to_rfc3339(),
    )
}
fn load(dir: &std::path::Path) -> Result<State, String> {
    match fs::read(dir.join("usage.json")) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|_| "API 사용량 설정을 읽지 못했습니다".into())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
        Err(_) => Err("API 사용량 파일을 읽지 못했습니다".into()),
    }
}
fn store(dir: &std::path::Path, state: &State) -> Result<(), String> {
    #[cfg(test)]
    if FAIL_STORE.with(|fail| fail.replace(false)) {
        return Err("fixture write failure".into());
    }
    atomic_write(
        &dir.join("usage.json"),
        &serde_json::to_vec(state).map_err(|_| "API 사용량 저장 실패")?,
    )
}
fn view(state: &State, now: u64) -> Usage {
    let month = period(now).0;
    let report = state.reports.get(&month);
    let spent = report.map(|report| report.usd);
    Usage {
        month,
        budget_usd: state.budget_usd,
        spent_usd: spent,
        remaining_usd: spent.map(|used| state.budget_usd - used),
        percent: spent.map(|used| used / state.budget_usd * 100.0),
        source: state.source,
        api_key_id: state.api_key_id.clone(),
        admin_key_stored: state.admin_key_stored,
        checked_at: report.map(|report| report.checked_at),
        provider_updated_at: report.and_then(|report| report.provider_updated_at.clone()),
        error: state.error.clone(),
    }
}
pub fn read(env: &Env, provider: &str, name: &str) -> Result<Usage, String> {
    let _guard = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
    let (provider, dir) = checked(env, provider, name)?;
    read_connection(&dir, provider, name)?;
    Ok(view(&load(&dir)?, crate::accounts::now()))
}
pub fn save(env: &Env, provider: &str, name: &str, settings: Settings) -> Result<Usage, String> {
    let _guard = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
    let (provider, dir) = checked(env, provider, name)?;
    read_connection(&dir, provider, name)?;
    save_with_store(
        &dir,
        &OsSecrets(env),
        provider,
        name,
        settings,
        crate::accounts::now(),
    )
}
fn save_with_store(
    dir: &std::path::Path,
    secrets: &impl Secrets,
    provider: Provider,
    name: &str,
    settings: Settings,
    now: u64,
) -> Result<Usage, String> {
    if !settings.budget_usd.is_finite() || !(0.01..=1_000_000_000.0).contains(&settings.budget_usd)
    {
        return Err("월 예산은 $0.01~$1,000,000,000 범위로 입력하세요".into());
    }
    if settings
        .spent_usd
        .is_some_and(|value| !value.is_finite() || !(0.0..=1_000_000_000.0).contains(&value))
    {
        return Err("사용 금액은 0 이상의 유효한 USD 금액이어야 합니다".into());
    }
    let mut state = load(dir)?;
    let id = settings.api_key_id.unwrap_or_default().trim().to_owned();
    let new_key = settings.admin_key.filter(|key| !key.trim().is_empty());
    if settings.source == Source::Automatic {
        if id.is_empty()
            || id.len() > 200
            || id.starts_with("sk-")
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err("조회할 API 키의 ID를 입력하세요 — 비밀 키(sk-…)와 다릅니다".into());
        }
        if new_key.as_deref().is_some_and(|key| !valid_key(key.trim())) {
            return Err("조회용 관리자 API 키 형식을 확인하세요".into());
        }
        if new_key.is_none() && !state.admin_key_stored {
            return Err("자동 조회용 관리자 API 키를 등록하세요".into());
        }
    }
    let service = credential(provider, name);
    let old_key = if state.admin_key_stored {
        Some(secrets.get(&service)?)
    } else {
        None
    };
    if state.source != settings.source || state.api_key_id != id || new_key.is_some() {
        // Never label an old scope or manually entered number as fetched API data.
        state.reports.clear();
    }
    state.budget_usd = settings.budget_usd;
    state.source = settings.source;
    state.api_key_id = if settings.source == Source::Automatic {
        id
    } else {
        String::new()
    };
    state.error = None;
    state.last_attempt = 0;
    state.revision = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    match settings.source {
        Source::Automatic => {
            if let Some(key) = new_key {
                secrets.put(&service, key.trim().as_bytes())?;
            }
            state.admin_key_stored = true;
        }
        Source::Manual => {
            if state.admin_key_stored {
                secrets.remove(&service)?;
            }
            state.admin_key_stored = false;
            if let Some(usd) = settings.spent_usd {
                state.reports.insert(
                    period(now).0,
                    Report {
                        usd,
                        checked_at: now,
                        provider_updated_at: None,
                    },
                );
            }
        }
    }
    while state.reports.len() > 24 {
        state.reports.pop_first();
    }
    if let Err(error) = store(dir, &state) {
        let restored = match old_key {
            Some(key) => secrets.put(&service, &key),
            None => secrets.remove(&service),
        };
        restored.map_err(|_| "사용량 설정 저장·조회용 키 복구 실패")?;
        return Err(error);
    }
    Ok(view(&state, now))
}
pub(super) fn remove_locked(env: &Env, provider: Provider, name: &str) -> Result<(), String> {
    let (_, dir) = checked(env, provider.dir_name(), name)?;
    if load(&dir)?.admin_key_stored {
        OsSecrets(env).remove(&credential(provider, name))?;
    }
    Ok(())
}

fn response_error(status: u16) -> String {
    match status {
        401 | 403 => "비용 조회 권한이 없습니다 — 조회용 관리자 API 키와 API 키 ID를 확인하세요",
        429 => "비용 조회 요청이 제한됐습니다 — 잠시 후 다시 시도하세요",
        _ => "API 비용을 조회하지 못했습니다 — 공급자 상태와 네트워크를 확인하세요",
    }
    .into()
}
fn amount(value: &Value) -> Result<f64, String> {
    let result = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.parse::<f64>().ok()));
    result
        .filter(|n| n.is_finite() && n.abs() <= 1_000_000_000.0)
        .ok_or("API 비용 응답 금액 형식이 올바르지 않습니다".into())
}
fn currency(value: &Value) -> Result<(), String> {
    if value
        .as_str()
        .is_some_and(|text| text.eq_ignore_ascii_case("usd"))
    {
        Ok(())
    } else {
        Err("USD가 아닌 비용 응답은 합산하지 않습니다".into())
    }
}
fn page_total(provider: Provider, body: &Value) -> Result<f64, String> {
    let data = body
        .get("data")
        .and_then(Value::as_array)
        .ok_or("API 비용 응답 형식이 올바르지 않습니다")?;
    let mut total = 0.0;
    for row in data {
        match provider {
            Provider::Claude => {
                currency(&row["cost"]["currency"])?;
                // Analytics reports fractional cents, not dollars.
                total += amount(&row["cost"]["amount"])? / 100.0;
            }
            Provider::Codex => {
                for result in row
                    .get("results")
                    .and_then(Value::as_array)
                    .ok_or("OpenAI 비용 버킷 형식이 올바르지 않습니다")?
                {
                    currency(&result["amount"]["currency"])?;
                    total += amount(&result["amount"]["value"])?;
                }
            }
        }
    }
    if !total.is_finite() || total.abs() > 1_000_000_000.0 {
        return Err("API 비용 합계가 유효 범위를 벗어났습니다".into());
    }
    Ok(total)
}
async fn fetch_at(
    provider: Provider,
    key: &str,
    key_id: &str,
    now: u64,
    url: &str,
) -> Result<Report, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|_| "비용 조회 초기화 실패")?;
    let (_, start, start_iso, end_iso) = period(now);
    let mut query = vec![
        ("api_key_ids[]".to_owned(), key_id.to_owned()),
        ("limit".to_owned(), "31".to_owned()),
    ];
    match provider {
        Provider::Claude => {
            query.push(("starting_at".into(), start_iso));
            query.push(("ending_at".into(), end_iso));
        }
        Provider::Codex => {
            query.push(("start_time".into(), start.to_string()));
            query.push(("end_time".into(), now.to_string()));
        }
    }
    let mut page = None;
    let mut cursors = HashSet::new();
    let mut total = 0.0;
    let mut refreshed = None;
    for _ in 0..100 {
        let mut request = client.get(url).query(&query);
        if let Some(ref page) = page {
            request = request.query(&[("page", page)]);
        }
        request = match provider {
            Provider::Claude => request
                .header("x-api-key", key)
                .header("anthropic-version", "2026-10-21")
                .header("anthropic-beta", "analytics-usage-cost-2026-08-24"),
            Provider::Codex => request.bearer_auth(key),
        };
        let response = request.send().await.map_err(|_| response_error(0))?;
        if !response.status().is_success() {
            return Err(response_error(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|size| size > 2_000_000)
        {
            return Err("API 비용 응답이 너무 큽니다".into());
        }
        let bytes = response.bytes().await.map_err(|_| response_error(0))?;
        if bytes.len() > 2_000_000 {
            return Err("API 비용 응답이 너무 큽니다".into());
        }
        let body: Value =
            serde_json::from_slice(&bytes).map_err(|_| "API 비용 응답 형식이 올바르지 않습니다")?;
        if provider == Provider::Claude {
            let value = body
                .get("data_refreshed_at")
                .and_then(Value::as_str)
                .ok_or("공급자가 이번 달 비용을 아직 집계하지 않았습니다")?;
            DateTime::parse_from_rfc3339(value)
                .map_err(|_| "비용 집계 시각 형식이 올바르지 않습니다")?;
            refreshed = Some(value.to_owned());
        }
        total += page_total(provider, &body)?;
        if !total.is_finite() || total.abs() > 1_000_000_000.0 {
            return Err("API 비용 합계가 유효 범위를 벗어났습니다".into());
        }
        match body.get("has_more").and_then(Value::as_bool) {
            Some(false) => {
                return Ok(Report {
                    usd: total,
                    checked_at: now,
                    provider_updated_at: refreshed,
                })
            }
            Some(true) => {
                let next = body
                    .get("next_page")
                    .and_then(Value::as_str)
                    .filter(|page| !page.is_empty() && page.len() <= 2048)
                    .ok_or("비용 조회 다음 페이지가 없습니다")?;
                if !cursors.insert(next.to_owned()) {
                    return Err("비용 조회 페이지가 반복됐습니다".into());
                }
                page = Some(next.to_owned());
            }
            None => return Err("비용 조회 페이지 정보가 없습니다".into()),
        }
    }
    Err("비용 조회 페이지 한도를 넘었습니다".into())
}
pub async fn refresh(env: &Env, provider: &str, name: &str, force: bool) -> Result<Usage, String> {
    let (provider, dir) = checked(env, provider, name)?;
    let now = crate::accounts::now();
    let month = period(now).0;
    let (snapshot, connection_revision, key) = {
        let _guard = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
        let connection = read_connection(&dir, provider, name)?;
        let mut state = load(&dir)?;
        if state.source == Source::Manual
            || (!force
                && state.attempt_month == month
                && now.saturating_sub(state.last_attempt) < 300)
        {
            return Ok(view(&state, now));
        }
        let key = OsSecrets(env)
            .get(&credential(provider, name))
            .map_err(|_| "자동 조회용 관리자 API 키를 다시 등록하세요")?;
        state.last_attempt = now;
        state.attempt_month = month.clone();
        state.attempt_id = format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        store(&dir, &state)?;
        (state, connection.revision, key)
    };
    let key = std::str::from_utf8(&key).map_err(|_| "조회용 키 저장 형식 오류")?;
    let url = match provider {
        Provider::Claude => "https://api.anthropic.com/v1/organizations/analytics/cost",
        Provider::Codex => "https://api.openai.com/v1/organization/costs",
    };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        fetch_at(provider, key, &snapshot.api_key_id, now, url),
    )
    .await
    .unwrap_or_else(|_| Err("비용 조회 시간이 초과됐습니다".into()));
    land(
        env,
        provider,
        name,
        &snapshot,
        &connection_revision,
        month,
        result,
    )
}
fn land(
    env: &Env,
    provider: Provider,
    name: &str,
    snapshot: &State,
    connection_revision: &str,
    month: String,
    result: Result<Report, String>,
) -> Result<Usage, String> {
    let _guard = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
    let (_, dir) = checked(env, provider.dir_name(), name)?;
    let connection = read_connection(&dir, provider, name)?;
    let mut state = load(&dir)?;
    if connection.revision != connection_revision
        || state.revision != snapshot.revision
        || state.attempt_id != snapshot.attempt_id
    {
        return Err("조회 중 연결 또는 사용량 설정이 바뀌었습니다 — 다시 조회하세요".into());
    }
    match result {
        Ok(report) => {
            state.reports.insert(month, report);
            state.error = None;
        }
        Err(error) => state.error = Some(error), // Keep the last complete cost report.
    }
    while state.reports.len() > 24 {
        state.reports.pop_first();
    }
    store(&dir, &state)?;
    Ok(view(&state, crate::accounts::now()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{cell::RefCell, collections::HashMap};
    #[derive(Default)]
    struct Memory(RefCell<HashMap<String, Vec<u8>>>);
    impl Secrets for Memory {
        fn put(&self, id: &str, key: &[u8]) -> Result<(), String> {
            self.0.borrow_mut().insert(id.into(), key.into());
            Ok(())
        }
        fn get(&self, id: &str) -> Result<Vec<u8>, String> {
            self.0
                .borrow()
                .get(id)
                .cloned()
                .ok_or("missing fixture".into())
        }
        fn remove(&self, id: &str) -> Result<(), String> {
            self.0.borrow_mut().remove(id);
            Ok(())
        }
    }
    fn time(value: &str) -> u64 {
        DateTime::parse_from_rfc3339(value).unwrap().timestamp() as u64
    }
    fn manual(budget: f64, spent: Option<f64>) -> Settings {
        Settings {
            budget_usd: budget,
            source: Source::Manual,
            spent_usd: spent,
            api_key_id: None,
            admin_key: None,
        }
    }
    fn automatic(key: Option<&str>) -> Settings {
        Settings {
            budget_usd: 195.0,
            source: Source::Automatic,
            spent_usd: None,
            api_key_id: Some("apikey_fixture".into()),
            admin_key: key.map(str::to_owned),
        }
    }
    #[test]
    fn budgets_are_per_connection_and_unknown_is_not_zero() {
        let env = crate::accounts::test_support::test_env("api-cost-defaults");
        let secrets = Memory::default();
        let first = env.store.join("one");
        let second = env.store.join("two");
        let now = time("2026-01-31T23:59:59Z");
        let unknown = view(&load(&first).unwrap(), now);
        assert_eq!(unknown.budget_usd, 195.0);
        assert_eq!(unknown.spent_usd, None);
        assert_eq!(unknown.percent, None);
        let saved = save_with_store(
            &first,
            &secrets,
            Provider::Claude,
            "one",
            manual(250.0, Some(78.45)),
            now,
        )
        .unwrap();
        assert_eq!(saved.spent_usd, Some(78.45));
        assert!((saved.remaining_usd.unwrap() - 171.55).abs() < 1e-9);
        assert_eq!(view(&load(&second).unwrap(), now).budget_usd, 195.0);
        let next = view(&load(&first).unwrap(), now + 1);
        assert_eq!(next.month, "2026-02");
        assert_eq!(next.budget_usd, 250.0);
        assert_eq!(next.spent_usd, None);
        assert_eq!(load(&first).unwrap().reports.len(), 1);
        let zero = save_with_store(
            &first,
            &secrets,
            Provider::Claude,
            "one",
            manual(195.0, Some(0.0)),
            now + 1,
        )
        .unwrap();
        assert_eq!(zero.spent_usd, Some(0.0));
        assert_eq!(zero.remaining_usd, Some(195.0));
    }
    #[test]
    fn budget_overage_and_invalid_values_are_not_silently_clamped() {
        let env = crate::accounts::test_support::test_env("api-cost-validation");
        let secrets = Memory::default();
        let now = time("2026-11-10T12:00:00Z");
        let dir = env.store.join("work");
        let over = save_with_store(
            &dir,
            &secrets,
            Provider::Claude,
            "work",
            manual(195.0, Some(210.0)),
            now,
        )
        .unwrap();
        assert_eq!(over.remaining_usd, Some(-15.0));
        assert!(over.percent.unwrap() > 100.0);
        for settings in [
            manual(0.0, None),
            manual(f64::NAN, None),
            manual(195.0, Some(-1.0)),
            manual(195.0, Some(f64::INFINITY)),
        ] {
            assert!(
                save_with_store(&dir, &secrets, Provider::Claude, "work", settings, now).is_err()
            );
        }
        assert_eq!(view(&load(&dir).unwrap(), now).spent_usd, Some(210.0));
        assert_eq!(period(time("2024-02-29T23:59:59Z")).0, "2024-02");
        assert_eq!(period(time("2024-02-29T23:59:59Z") + 1).0, "2024-03");
    }
    #[test]
    fn reporting_secrets_are_separate_and_roll_back_on_write_failure() {
        let env = crate::accounts::test_support::test_env("api-cost-vault");
        let secrets = Memory::default();
        let dir = env.store.join("work");
        let now = time("2026-11-10T12:00:00Z");
        let first = "sk-dummy-reporting-first-1234";
        let second = "sk-dummy-reporting-second-1234";
        save_with_store(
            &dir,
            &secrets,
            Provider::Claude,
            "work",
            automatic(Some(first)),
            now,
        )
        .unwrap();
        assert!(!fs::read_to_string(dir.join("usage.json"))
            .unwrap()
            .contains(first));
        assert!(secrets.get(&service(Provider::Claude, "work")).is_err());
        assert_eq!(
            secrets.get(&credential(Provider::Claude, "work")).unwrap(),
            first.as_bytes()
        );
        FAIL_STORE.with(|fail| fail.set(true));
        assert!(save_with_store(
            &dir,
            &secrets,
            Provider::Claude,
            "work",
            automatic(Some(second)),
            now
        )
        .is_err());
        assert_eq!(
            secrets.get(&credential(Provider::Claude, "work")).unwrap(),
            first.as_bytes()
        );
        let updated = save_with_store(
            &dir,
            &secrets,
            Provider::Claude,
            "work",
            manual(195.0, Some(35.0)),
            now,
        )
        .unwrap();
        assert!(!updated.admin_key_stored);
        assert!(secrets.get(&credential(Provider::Claude, "work")).is_err());
        assert!(save_with_store(
            &dir,
            &secrets,
            Provider::Claude,
            "work",
            automatic(None),
            now
        )
        .is_err());
        let mut invalid = automatic(Some(first));
        invalid.api_key_id = Some(first.into());
        assert!(save_with_store(&dir, &secrets, Provider::Claude, "work", invalid, now).is_err());
    }
    fn mock_server(responses: Vec<(u16, Value)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (code, body) in responses {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|s| s == b"\r\n\r\n") {
                    let n = socket.read(&mut buffer).unwrap();
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..n]);
                }
                requests.push(String::from_utf8(request).unwrap());
                let body = body.to_string();
                write!(socket,"HTTP/1.1 {code} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nLocation: http://127.0.0.1:1/never\r\n\r\n{body}",body.len()).unwrap();
            }
            requests
        });
        (format!("http://{addr}/costs"), worker)
    }
    fn cost_page(provider: Provider, amount: f64, more: bool) -> Value {
        match provider {
            Provider::Claude => {
                json!({"data":[{"cost":{"amount":amount.to_string(),"currency":"USD"}}],"has_more":more,"next_page":if more{Some("next")}else{None},"data_refreshed_at":"2026-11-10T08:00:00Z"})
            }
            Provider::Codex => {
                json!({"data":[{"results":[{"amount":{"value":amount,"currency":"usd"}}]}],"has_more":more,"next_page":if more{Some("next")}else{None}})
            }
        }
    }
    #[tokio::test]
    async fn providers_filter_by_key_and_sum_all_pages_in_correct_units() {
        let now = time("2026-11-10T12:00:00Z");
        for provider in [Provider::Claude, Provider::Codex] {
            let factor = if provider == Provider::Claude {
                100.0
            } else {
                1.0
            };
            let (url, server) = mock_server(vec![
                (200, cost_page(provider, 12.345 * factor, true)),
                (200, cost_page(provider, 7.655 * factor, false)),
            ]);
            let report = fetch_at(
                provider,
                "sk-dummy-reporting-123456",
                "apikey_fixture",
                now,
                &url,
            )
            .await
            .unwrap();
            assert!((report.usd - 20.0).abs() < 1e-9);
            let requests = server.join().unwrap();
            assert_eq!(requests.len(), 2);
            assert!(requests
                .iter()
                .all(|r| r.contains("api_key_ids%5B%5D=apikey_fixture")));
            assert!(requests[1].contains("page=next"));
            if provider == Provider::Claude {
                assert!(requests[0].contains("anthropic-beta: analytics-usage-cost-2026-08-24"));
                assert!(requests[0].contains("starting_at=2026-11-01"));
            } else {
                assert!(requests[0].contains("authorization: Bearer sk-dummy-reporting-123456"));
                assert!(requests[0].contains("start_time=1793491200"));
            }
        }
    }
    #[tokio::test]
    async fn failures_partial_pages_and_unknown_exports_never_become_zero_cost() {
        let now = time("2026-11-10T12:00:00Z");
        for code in [401, 403, 429, 503, 302] {
            let (url, server) = mock_server(vec![(code, json!({"secret":"fixture-do-not-echo"}))]);
            let error = fetch_at(
                Provider::Codex,
                "sk-dummy-reporting-123456",
                "apikey_fixture",
                now,
                &url,
            )
            .await
            .err()
            .unwrap();
            assert!(!error.contains("fixture-do-not-echo"));
            server.join().unwrap();
        }
        let (url, server) = mock_server(vec![
            (200, cost_page(Provider::Codex, 12.0, true)),
            (503, json!({})),
        ]);
        assert!(fetch_at(
            Provider::Codex,
            "sk-dummy-reporting-123456",
            "apikey_fixture",
            now,
            &url
        )
        .await
        .is_err());
        server.join().unwrap();
        let empty = json!({"data":[],"has_more":false,"next_page":null,"data_refreshed_at":null});
        let (url, server) = mock_server(vec![(200, empty.clone())]);
        assert!(fetch_at(
            Provider::Claude,
            "sk-dummy-reporting-123456",
            "apikey_fixture",
            now,
            &url
        )
        .await
        .is_err());
        server.join().unwrap();
        let mut known = empty;
        known["data_refreshed_at"] = json!("2026-11-10T08:00:00Z");
        let (url, server) = mock_server(vec![(200, known)]);
        assert_eq!(
            fetch_at(
                Provider::Claude,
                "sk-dummy-reporting-123456",
                "apikey_fixture",
                now,
                &url
            )
            .await
            .unwrap()
            .usd,
            0.0
        );
        server.join().unwrap();
        assert!(page_total(Provider::Codex, &json!({"data":[{}]})).is_err());
        assert!(page_total(
            Provider::Claude,
            &json!({"data":[{"cost":{"amount":"NaN","currency":"USD"}}]})
        )
        .is_err());
        assert!(page_total(
            Provider::Claude,
            &json!({"data":[{"cost":{"amount":"100","currency":"EUR"}}]})
        )
        .is_err());
    }
    #[test]
    fn late_reports_cannot_overwrite_edits_or_recreate_deleted_connections() {
        let env = crate::accounts::test_support::test_env("api-cost-race");
        let secrets = Memory::default();
        let provider = Provider::Claude;
        super::super::save_with(
            &env,
            &secrets,
            "claude",
            "work",
            "sk-dummy-inference-123456",
        )
        .unwrap();
        let (_, dir) = checked(&env, "claude", "work").unwrap();
        let connection = read_connection(&dir, provider, "work").unwrap();
        let now = crate::accounts::now();
        save_with_store(
            &dir,
            &secrets,
            provider,
            "work",
            manual(195.0, Some(50.0)),
            now,
        )
        .unwrap();
        let old = load(&dir).unwrap();
        let failed = land(
            &env,
            provider,
            "work",
            &old,
            &connection.revision,
            period(now).0,
            Err("fixture unavailable".into()),
        )
        .unwrap();
        assert_eq!(failed.spent_usd, Some(50.0));
        assert!(failed.error.is_some());
        save_with_store(
            &dir,
            &secrets,
            provider,
            "work",
            manual(250.0, Some(75.0)),
            now,
        )
        .unwrap();
        assert!(land(
            &env,
            provider,
            "work",
            &old,
            &connection.revision,
            period(now).0,
            Ok(Report {
                usd: 90.0,
                checked_at: now,
                provider_updated_at: None
            })
        )
        .is_err());
        assert_eq!(view(&load(&dir).unwrap(), now).spent_usd, Some(75.0));
        let old = load(&dir).unwrap();
        let mut current = old.clone();
        current.attempt_id = "newer-request".into();
        store(&dir, &current).unwrap();
        assert!(land(
            &env,
            provider,
            "work",
            &old,
            &connection.revision,
            period(now).0,
            Err("late".into())
        )
        .is_err());
        fs::remove_file(dir.join("connection.json")).unwrap();
        assert!(land(
            &env,
            provider,
            "work",
            &current,
            &connection.revision,
            period(now).0,
            Err("deleted".into())
        )
        .is_err());
        assert!(!dir.join("connection.json").exists());
    }
}

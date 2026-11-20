//! A reading copied by the user from their Console dashboard. The webview sends
//! only the verified organization label and USD cents, never the clipboard text.
use super::*;
use crate::accounts::atomic_write_existing_parent;

#[derive(Serialize, Deserialize)]
struct Saved {
    organization_id: String,
    revision: String,
    credit: Credit,
}

pub(super) fn load(dir: &Path, org: Uuid, revision: &str) -> Option<Credit> {
    let bytes = std::fs::read(dir.join("console-credit.json")).ok()?;
    let saved: Saved = serde_json::from_slice(&bytes).ok()?;
    (saved.organization_id == org.to_string()
        && saved.revision == revision
        && saved.credit.currency == "USD"
        && saved.credit.checked_at > 0
        && saved.credit.amount_minor.unsigned_abs() <= 9_007_199_254_740_991)
        .then_some(saved.credit)
}

#[derive(Deserialize)]
pub struct ConsoleReading {
    pub organization: String,
    pub import_ref: String,
    pub amount_minor: i64,
}

pub async fn import(env: &Env, name: &str, reading: ConsoleReading) -> Result<Balance, String> {
    import_with(env, name, reading, &OsSecrets(env), API).await
}

pub(super) async fn import_with(
    env: &Env,
    name: &str,
    reading: ConsoleReading,
    secrets: &impl Secrets,
    base: &str,
) -> Result<Balance, String> {
    if reading.amount_minor.unsigned_abs() > 9_007_199_254_740_991 {
        return Err("Console 금액 형식을 확인하세요".into());
    }
    read_with(env, "claude", name, false, secrets, base).await?;
    let (provider, dir) = checked(env, "claude", name)?;
    let slot = SLOTS
        .lock()
        .map_err(|_| "API 잔액 잠금 실패")?
        .get(&dir)
        .cloned()
        .ok_or("API 연결이 변경됐습니다. 다시 확인하세요.")?;
    let mut stored = slot.lock().await;
    let cached = stored.as_mut().ok_or("API 연결을 확인하지 못했습니다")?;
    let org = cached
        .org
        .ok_or("먼저 API 키의 Console 조직을 확인해야 합니다")?;
    // A name alone never chooses a connection. The UUID and current key revision
    // come from the backend and the pasted label must match this connection.
    let normalize = |label: &str| label.trim().replace('’', "'");
    let matched = cached
        .balance
        .organization
        .as_deref()
        .is_some_and(|label| normalize(label) == normalize(&reading.organization));
    if !matched || cached.balance.import_ref.as_ref() != Some(&reading.import_ref) {
        return Err("이 API 키를 발급한 Console의 대시보드를 복사하세요".into());
    }
    let _lock = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
    if read_connection(&dir, provider, name)?.revision != cached.revision {
        return Err("API 키가 바뀌었습니다. 다시 확인하세요.".into());
    }
    let credit = Credit {
        amount_minor: reading.amount_minor,
        currency: "USD".into(),
        checked_at: now(),
    };
    let snapshot = Saved {
        revision: cached.revision.clone(),
        organization_id: org.to_string(),
        credit: credit.clone(),
    };
    atomic_write_existing_parent(
        &dir.join("console-credit.json"),
        &serde_json::to_vec(&snapshot).map_err(|_| "Console 확인값을 저장하지 못했습니다")?,
    )?;
    cached.balance.console_credit = Some(credit);
    Ok(cached.balance.clone())
}

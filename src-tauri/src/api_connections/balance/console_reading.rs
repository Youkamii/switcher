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

fn usable(credit: &Credit) -> bool {
    credit.currency == "USD" && credit.checked_at > 0 && credit.amount_minor.unsigned_abs() <= 9_007_199_254_740_991
}

fn saved_in(dir: &Path) -> Option<Saved> {
    serde_json::from_slice(&std::fs::read(dir.join("console-credit.json")).ok()?).ok()
}

pub(super) fn load(dir: &Path, org: Uuid, revision: &str) -> Option<Credit> {
    let saved = saved_in(dir)?;
    (saved.organization_id == org.to_string() && saved.revision == revision && usable(&saved.credit))
        .then_some(saved.credit)
}

// ── 다른 PC로 옮기기 (#189) ────────────────────────────────────────
// 이 확인값은 사용자가 Console 대시보드를 붙여넣어 만든 것이다. 저절로 다시 생기지 않는다 — 키만
// 옮기면 받는 PC에서는 남은 금액 자리가 비어 나온다(실제로 그렇게 배포했다가 지적받았다:
// "남은 api 사용량 왜 안나와"). 그래서 키와 함께 옮긴다. 키의 revision은 PC마다 다르므로 싣지 않고,
// 받는 쪽이 자기 연결의 revision에 묶는다.

/// 옮기는 확인값 — 어느 조직의 얼마를 언제 확인했는가
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(in crate::api_connections) struct TransferReading {
    organization_id: String,
    credit: Credit,
}

/// 지금 키(revision)에 묶여 저장된 확인값. 옛 키의 것은 내놓지 않는다.
pub(in crate::api_connections) fn export_reading(dir: &Path, revision: &str) -> Option<TransferReading> {
    let saved = saved_in(dir)?;
    (saved.revision == revision && Uuid::try_parse(&saved.organization_id).is_ok() && usable(&saved.credit)).then_some(
        TransferReading {
            organization_id: saved.organization_id,
            credit: saved.credit,
        },
    )
}

/// 옮겨 온 확인값을 이 연결(revision)에 묶어 저장한다. 같은 조직을 더 최근에 확인한 값이 이미 있으면
/// 그대로 둔다. 저장했으면 true. 묶음은 바깥 입력이다 — 조직 ID와 금액의 모양을 본 뒤에만 쓴다.
pub(in crate::api_connections) fn import_reading(dir: &Path, revision: &str, reading: &TransferReading) -> Result<bool, String> {
    let Ok(org) = Uuid::try_parse(&reading.organization_id) else {
        return Ok(false);
    };
    if !usable(&reading.credit) {
        return Ok(false);
    }
    let organization_id = org.to_string();
    let newer_here = saved_in(dir).is_some_and(|saved| {
        saved.revision == revision
            && saved.organization_id == organization_id
            && usable(&saved.credit)
            && saved.credit.checked_at >= reading.credit.checked_at
    });
    if newer_here {
        return Ok(false);
    }
    let snapshot = Saved {
        organization_id,
        revision: revision.to_owned(),
        credit: reading.credit.clone(),
    };
    atomic_write_existing_parent(
        &dir.join("console-credit.json"),
        &serde_json::to_vec(&snapshot).map_err(|_| "Console 확인값을 저장하지 못했습니다")?,
    )?;
    // 열려 있는 화면이 옛 조회 결과를 들고 있지 않게
    forget(dir);
    Ok(true)
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

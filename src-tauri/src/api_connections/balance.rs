//! Claude prepaid credit, in the provider's currency and minor units.
use super::{checked, read_connection, service, Env, OsSecrets, Provider, Secrets, LOCK};
use crate::accounts::{
    claude_oauth_block, now, profile_dirs, read_json, read_meta, validate_name, MUTATION_LOCK,
};
use reqwest::{Client, RequestBuilder, Response};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;

pub(super) const API: &str = "https://api.anthropic.com";
const FRESH: Duration = Duration::from_secs(300);
const RETRY: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue {
    Unsupported,
    LoginRequired,
    LoginExpired,
    KeyRejected,
    Forbidden,
    RateLimited,
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Credit {
    pub amount_minor: i64,
    pub currency: String,
    pub checked_at: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Balance {
    pub credit: Option<Credit>,
    pub issue: Option<Issue>,
    pub organization: Option<String>,
    pub console_credit: Option<Credit>,
    pub import_ref: Option<String>,
}

impl Balance {
    fn failed(issue: Issue) -> Self {
        Self {
            credit: None,
            issue: Some(issue),
            organization: None,
            console_credit: None,
            import_ref: None,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
enum Login {
    Widget {
        account: Uuid,
        profile: Option<String>,
    },
    Console(console_profile::Profile),
}

// Local files are hints for finding the right token. Confirm the account and
// organization with the issuer before requesting financial data.
fn login_for(
    block: &Value,
    org: Uuid,
    owner: Option<&str>,
    profile: Option<String>,
) -> Option<Login> {
    if Uuid::try_parse(block.get("organizationUuid")?.as_str()?).ok()? != org {
        return None;
    }
    let account = Uuid::try_parse(block.get("accountUuid")?.as_str()?).ok()?;
    if owner.is_some_and(|id| Uuid::try_parse(id).ok() != Some(account)) {
        return None;
    }
    Some(Login::Widget { account, profile })
}

fn logins_for_org(env: &Env, org: Uuid, owner: Option<&str>) -> Result<Vec<Login>, Issue> {
    let _lock = MUTATION_LOCK.lock().map_err(|_| Issue::Unavailable)?;
    let mut logins: Vec<_> = console_profile::active(&env.anthropic_config_dir, org)
        .map(Login::Console)
        .into_iter()
        .collect();
    if let Some(live) = claude_oauth_block(env)
        .ok()
        .flatten()
        .and_then(|b| login_for(&b, org, owner, None))
    {
        logins.push(live);
    }
    for (name, dir) in profile_dirs(env, Provider::Claude).map_err(|_| Issue::Unavailable)? {
        if validate_name(&name).is_err() {
            continue;
        }
        let Ok(block) = read_json(&dir.join("oauth_account.json")) else {
            continue;
        };
        let Some(login) = login_for(&block, org, owner, Some(name)) else {
            continue;
        };
        if matches!(&login, Login::Widget { account, .. } if read_meta(&dir).and_then(|meta| Uuid::try_parse(&meta.id).ok()) == Some(*account))
        {
            logins.push(login);
        }
    }
    Ok(logins)
}

/// 키를 발급한 조직의 이름 — 계정을 고르지 않은 등록(#187)이 주인 계정을 찾는 단서다.
/// 이름을 확인하지 못하면(키 거절·권한·네트워크·이름 없는 응답) None이고, 호출자는 추정하지 않고 묻는다.
pub(super) async fn key_organization_name(key: &str, base: &str) -> Option<String> {
    let api = Api::new(base).ok()?;
    let org = api.organization(Auth::Key(key)).await.ok()?;
    org.named.then_some(org.label)
}

struct Api<'a> {
    base: &'a str, // constant in production; loopback in fixture tests.
    client: Client,
}

#[derive(Clone, Copy)]
enum Auth<'a> {
    Key(&'a str),
    Console(&'a str),
}

struct Organization {
    id: Uuid,
    label: String,
    named: bool,
}

impl Organization {
    fn new(id: Uuid, name: Option<&str>) -> Self {
        let verified_name = name
            .map(str::trim)
            .filter(|s| {
                !s.is_empty() && s.chars().count() <= 128 && !s.chars().any(char::is_control)
            })
            .map(str::to_owned);
        Self {
            id,
            named: verified_name.is_some(),
            label: verified_name.unwrap_or_else(|| format!("…{}", &id.to_string()[28..])),
        }
    }
}

impl<'a> Api<'a> {
    fn new(base: &'a str) -> Result<Self, Issue> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(9))
            .build()
            .map_err(|_| Issue::Unavailable)?;
        Ok(Self { base, client })
    }

    async fn get(request: RequestBuilder, unauthorized: Issue) -> Result<Response, Issue> {
        let response = request.send().await.map_err(|_| Issue::Unavailable)?;
        match response.status().as_u16() {
            200 => Ok(response),
            401 => Err(unauthorized),
            403 => Err(Issue::Forbidden),
            429 => Err(Issue::RateLimited),
            _ => Err(Issue::Unavailable),
        }
    }

    async fn json<T: DeserializeOwned>(mut response: Response) -> Result<T, Issue> {
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Issue::Unavailable)? {
            if bytes.len() + chunk.len() > 128 * 1024 {
                return Err(Issue::Unavailable);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| Issue::Unavailable)
    }

    async fn organization(&self, auth: Auth<'_>) -> Result<Organization, Issue> {
        let request = |path: &str| {
            let request = self
                .client
                .get(format!("{}{path}", self.base))
                .header("anthropic-version", "2026-10-21");
            match auth {
                Auth::Key(key) => request.header("x-api-key", key),
                Auth::Console(token) => request.bearer_auth(token),
            }
        };
        let unauthorized = match auth {
            Auth::Key(_) => Issue::KeyRejected,
            Auth::Console(_) => Issue::LoginExpired,
        };
        #[derive(Deserialize)]
        struct CurrentOrg {
            id: String,
            name: Option<String>,
        }
        match Self::get(request("/v1/organizations/me"), unauthorized).await {
            Ok(response) => {
                let org: CurrentOrg = Self::json(response).await?;
                let id = Uuid::try_parse(&org.id).map_err(|_| Issue::Unavailable)?;
                return Ok(Organization::new(id, org.name.as_deref()));
            }
            Err(Issue::KeyRejected | Issue::LoginExpired | Issue::Forbidden) => {}
            Err(issue) => return Err(issue),
        }
        // Workspace-scoped keys can carry the org in the standard model response header.
        let response = Self::get(request("/v1/models?limit=1"), unauthorized).await?;
        let id = response
            .headers()
            .get("anthropic-organization-id")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| Uuid::try_parse(value).ok())
            .ok_or(Issue::Unavailable)?;
        Ok(Organization::new(id, None))
    }

    async fn credit(&self, token: &str, account: Uuid, org: Uuid) -> Result<Credit, Issue> {
        let profile = self
            .client
            .get(format!("{}/api/oauth/profile", self.base))
            .bearer_auth(token)
            .header("anthropic-beta", "oauth-2025-04-20");
        let identity: Value = Self::json(Self::get(profile, Issue::LoginExpired).await?).await?;
        let id = |path: &str| identity.pointer(path)?.as_str()?.parse::<Uuid>().ok();
        if id("/account/uuid") != Some(account) || id("/organization/uuid") != Some(org) {
            return Err(Issue::LoginRequired);
        }
        // Claude Code uses this route for subscription extra usage as well. Only
        // accept the two Console organization types recognized by the official CLI.
        if !matches!(
            identity
                .pointer("/organization/organization_type")
                .and_then(Value::as_str),
            Some("api_individual" | "api_team")
        ) {
            return Err(Issue::LoginRequired);
        }
        self.prepaid(token, org).await
    }

    async fn prepaid(&self, token: &str, org: Uuid) -> Result<Credit, Issue> {
        let request = self
            .client
            .get(format!(
                "{}/api/oauth/organizations/{org}/prepaid/credits",
                self.base
            ))
            .bearer_auth(token)
            .header("x-organization-uuid", org.to_string())
            .header("anthropic-version", "2023-06-01");
        #[derive(Deserialize)]
        struct Prepaid {
            amount: i64,
            currency: String,
        }
        let prepaid: Prepaid = Self::json(Self::get(request, Issue::LoginExpired).await?).await?;
        // Amounts cross a JavaScript boundary. Reject fractional or unrepresentable minor units.
        if prepaid.amount.unsigned_abs() > 9_007_199_254_740_991
            || prepaid.currency.len() != 3
            || !prepaid.currency.bytes().all(|c| c.is_ascii_alphabetic())
        {
            return Err(Issue::Unavailable);
        }
        Ok(Credit {
            amount_minor: prepaid.amount,
            currency: prepaid.currency.to_ascii_uppercase(),
            checked_at: now(),
        })
    }
}

struct Cached {
    revision: String,
    at: Instant,
    org: Option<Uuid>,
    logins: Vec<Login>,
    balance: Balance,
}

type Slot = Arc<tokio::sync::Mutex<Option<Cached>>>;
static SLOTS: LazyLock<Mutex<HashMap<PathBuf, Slot>>> = LazyLock::new(Mutex::default);

pub(super) fn forget(dir: &Path) {
    if let Ok(mut slots) = SLOTS.lock() {
        slots.remove(dir);
    }
}

async fn lookup(env: &Env, api: &Api<'_>, logins: &[Login], org: Uuid) -> Result<Credit, Issue> {
    let mut issue = Issue::LoginRequired;
    for login in logins {
        let result = match login {
            Login::Widget { account, profile } => match crate::usage::claude_balance_token(
                env,
                profile.as_deref(),
                &account.to_string(),
            )
            .await
            {
                Some(token) => api.credit(&token, *account, org).await,
                None => Err(Issue::LoginExpired),
            },
            Login::Console(profile) => profile.credit(api, org).await,
        };
        match result {
            Err(reason @ (Issue::LoginExpired | Issue::LoginRequired | Issue::Forbidden)) => {
                issue = reason
            }
            result => return result,
        }
    }
    Err(issue)
}

pub async fn read(env: &Env, provider: &str, name: &str, force: bool) -> Result<Balance, String> {
    read_with(env, provider, name, force, &OsSecrets(env), API).await
}

async fn read_with(
    env: &Env,
    provider: &str,
    name: &str,
    force: bool,
    secrets: &impl Secrets,
    base: &str,
) -> Result<Balance, String> {
    let started = Instant::now();
    let (provider, dir) = checked(env, provider, name)?;
    let slot = SLOTS
        .lock()
        .map_err(|_| "API 잔액 잠금 실패")?
        .entry(dir.clone())
        .or_default()
        .clone();
    let mut cached = slot.lock().await;
    let connection = {
        let _lock = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
        read_connection(&dir, provider, name)?
    };
    let revision = connection.revision;
    let owner = connection.account_id.as_deref();
    if provider == Provider::Codex {
        return Ok(Balance::failed(Issue::Unsupported));
    }
    let prior = cached.as_ref().filter(|c| c.revision == revision);
    let mut org = prior.and_then(|c| c.org);
    let mut logins = Vec::new();
    if let Some(id) = org {
        logins = logins_for_org(env, id, owner).map_err(|_| "Claude 로그인 계정을 확인하지 못했습니다")?;
    }
    if let Some(c) = prior {
        let ttl = match c.balance.issue {
            None | Some(Issue::LoginRequired | Issue::Forbidden) => FRESH,
            _ => RETRY,
        };
        if c.logins == logins && (c.at >= started || (!force && c.at.elapsed() < ttl)) {
            return Ok(c.balance.clone());
        }
    }
    // A previous organization is a cache hint only. Revalidate the execution key
    // on every network refresh, including after an external revocation.
    org = None;
    logins.clear();
    let mut organization = None;
    let mut import_ref = None;
    let result = async {
        let api = Api::new(base)?;
        let key = {
            let _lock = LOCK.lock().map_err(|_| Issue::Unavailable)?;
            secrets
                .get(&service(provider, name))
                .map_err(|_| Issue::KeyRejected)?
        };
        let verified = api
            .organization(Auth::Key(
                std::str::from_utf8(&key).map_err(|_| Issue::KeyRejected)?,
            ))
            .await?;
        let id = verified.id;
        if verified.named {
            use sha2::{Digest, Sha256};
            import_ref = Some(format!("{:x}", Sha256::digest(format!("{revision}/{id}"))));
        }
        organization = Some(verified.label);
        org = Some(id);
        logins = logins_for_org(env, id, owner)?;
        let credit = lookup(env, &api, &logins, id).await?;
        if logins_for_org(env, id, owner)? != logins {
            return Err(Issue::LoginRequired);
        }
        Ok(credit)
    }
    .await;
    let (credit, issue) = match result {
        Ok(credit) => (Some(credit), None),
        Err(issue) => (None, Some(issue)),
    };
    let balance = Balance {
        credit,
        issue,
        organization,
        // 조직을 지금 확인했으면 그 조직의 확인값만 보여 준다. 확인하지 못했으면(네트워크·일시 오류)
        // 이 키에 묶인 확인값을 그대로 보여 준다 — 예전에는 그때 금액이 통째로 사라져, 다른 PC로
        // 옮긴 키에서 "조직 크레딧이 안 뜬다"고 보였다(#189, Windows 사용자 보고). 키 자체가 거절되면
        // 보여 주지 않는다.
        console_credit: match org {
            Some(id) => console_reading::load(&dir, id, &revision),
            None if issue != Some(Issue::KeyRejected) => console_reading::load_for_key(&dir, &revision),
            None => None,
        },
        import_ref,
    };
    let _lock = LOCK.lock().map_err(|_| "API 연결 잠금 실패")?;
    let current = read_connection(&dir, provider, name)?;
    if current.revision != revision || current.account_id != connection.account_id {
        return Err("조회 중 API 키 또는 연결 계정이 바뀌었습니다. 다시 조회하세요.".into());
    }
    *cached = Some(Cached {
        revision,
        at: Instant::now(),
        org,
        logins,
        balance: balance.clone(),
    });
    Ok(balance)
}

mod console_profile;
mod console_reading;
pub use console_reading::{import, ConsoleReading};
pub(super) use console_reading::{export_reading, import_reading, TransferReading};

#[cfg(test)]
mod tests;

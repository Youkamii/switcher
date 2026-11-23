use super::*;
use crate::accounts::{atomic_write, test_support::test_env, write_profile_parts, LiveIdentity};
use crate::api_connections::{save_with, tests::Memory};
use serde_json::json;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    thread,
};

const ORG: &str = "00000000-0000-4000-8000-000000000001";
const ACCOUNT: &str = "00000000-0000-4000-8000-000000000002";
const OTHER: &str = "00000000-0000-4000-8000-000000000003";
const KEY: &str = "sk-dummy-local-credit-test";
const LOGIN: &str = "synthetic-local-login";

fn fixture(tag: &str, login_org: Option<&str>) -> (Env, Memory) {
    let env = test_env(tag);
    let memory = Memory::default();
    save_with(&env, &memory, "claude", "work", KEY).unwrap();
    if let Some(org) = login_org {
        let identity = LiveIdentity {
            id: ACCOUNT.into(),
            email: None,
        };
        let credentials =
            json!({"claudeAiOauth": {"accessToken": LOGIN, "expiresAt": (now() + 86400) * 1000}});
        write_profile_parts(
            &env,
            Provider::Claude,
            "fixture",
            &identity,
            &serde_json::to_vec(&credentials).unwrap(),
            Some(&json!({"accountUuid": ACCOUNT, "organizationUuid":org})),
        )
        .unwrap();
    }
    (env, memory)
}

struct Reply {
    path: String,
    status: u16,
    header: String,
    body: String,
    bearer: bool,
}
impl Reply {
    fn new(path: &str, status: u16, body: Value) -> Self {
        Self {
            path: path.into(),
            status,
            header: String::new(),
            body: body.to_string(),
            bearer: false,
        }
    }
    fn organization() -> Self {
        Self::new(
            "/v1/organizations/me",
            200,
            json!({"type":"organization","id":ORG,"name":"Fixture Studio"}),
        )
    }
    fn models() -> Self {
        let mut r = Self::new("/v1/models?limit=1", 200, json!({"data":[]}));
        r.header = format!("anthropic-organization-id: {ORG}\r\n");
        r
    }
    fn profile(org: &str, account: &str) -> Self {
        Self::new(
            "/api/oauth/profile",
            200,
            json!({"organization":{"uuid":org,"organization_type":"api_team"}, "account":{"uuid":account}}),
        )
    }
    fn credit(status: u16, body: Value) -> Self {
        Self::new(
            &format!("/api/oauth/organizations/{ORG}/prepaid/credits"),
            status,
            body,
        )
    }
}

// Stops accepting after the expected requests. A missing or extra call fails the test.
fn serve(
    replies: Vec<Reply>,
    on_request: impl Fn(&str) + Send + 'static,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = thread::spawn(move || {
        for reply in replies {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("missing fixture request to {}: {e}", reply.path),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(&stream);
            let mut headers = String::new();
            while reader.read_line(&mut headers).unwrap() > 0 && !headers.ends_with("\r\n\r\n") {}
            let headers = headers.to_ascii_lowercase();
            assert!(headers.starts_with(&format!("get {} http/", reply.path)));
            if reply.path.starts_with("/v1/") && !reply.bearer {
                assert!(headers.contains(&format!("x-api-key: {KEY}\r\n")));
                assert!(!headers.contains("authorization:"));
            } else {
                assert!(headers.contains(&format!("authorization: bearer {LOGIN}\r\n")));
                assert!(!headers.contains("x-api-key:"));
                if reply.path.ends_with("/credits") {
                    assert!(headers.contains(&format!("x-organization-uuid: {ORG}\r\n")));
                }
            }
            on_request(&reply.path);
            write!(stream, "HTTP/1.1 {} Test\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", reply.status, reply.header, reply.body.len(), reply.body).unwrap();
        }
    });
    (url, task)
}

#[tokio::test]
async fn linked_key_does_not_borrow_another_saved_accounts_login_in_the_same_org() {
    let (env, secrets) = fixture("balance-linked-owner", Some(ORG));
    crate::api_connections::tests::add_account(&env, Provider::Claude, "second", OTHER);
    crate::api_connections::attach(&env, "claude", "work", OTHER).unwrap();
    let (url, server) = serve(vec![Reply::organization()], |_| {});
    let balance = read_with(&env, "claude", "work", false, &secrets, &url)
        .await
        .unwrap();
    assert!(matches!(balance.issue, Some(Issue::LoginRequired)));
    assert!(balance.credit.is_none());
    server.join().unwrap();
}

#[tokio::test]
async fn verified_prepaid_amount_is_cached_and_force_can_return_a_real_zero() {
    let (env, secrets) = fixture("prepaid-values", Some(ORG));
    let (_, dir) = checked(&env, "claude", "work").unwrap();
    fs::write(
        dir.join("usage.json"),
        br#"{"budget_usd":195,"reports":{}}"#,
    )
    .unwrap();
    let (url, server) = serve(
        vec![
            Reply::organization(),
            Reply::profile(ORG, ACCOUNT),
            Reply::credit(200, json!({"amount":7845,"currency":"usd"})),
            Reply::new("/v1/organizations/me", 403, json!({})),
            Reply::models(),
            Reply::profile(ORG, ACCOUNT),
            Reply::credit(200, json!({"amount":0,"currency":"JPY"})),
        ],
        |_| {},
    );
    for _ in 0..2 {
        let credit = read_with(&env, "claude", "work", false, &secrets, &url)
            .await
            .unwrap()
            .credit
            .unwrap();
        assert_eq!(
            (credit.amount_minor, credit.currency.as_str()),
            (7845, "USD")
        );
        assert!(credit.checked_at > 0);
    }
    let result = read_with(&env, "claude", "work", true, &secrets, &url)
        .await
        .unwrap();
    assert_eq!(result.issue, None);
    assert_eq!(result.organization.as_deref(), Some("…00000001"));
    let zero = result.credit.unwrap();
    assert_eq!((zero.amount_minor, zero.currency.as_str()), (0, "JPY"));
    server.join().unwrap();
}

#[tokio::test]
async fn a_different_local_organization_never_prompts_a_billing_request() {
    let (env, secrets) = fixture("prepaid-no-login", Some(OTHER));
    let (url, server) = serve(vec![Reply::organization()], |_| {});
    let result = read_with(&env, "claude", "work", false, &secrets, &url)
        .await
        .unwrap();
    assert_eq!(result.issue, Some(Issue::LoginRequired));
    assert!(result.credit.is_none());
    assert_eq!(result.organization.as_deref(), Some("Fixture Studio"));
    server.join().unwrap();
}

#[tokio::test]
async fn subscription_or_unknown_wallet_is_never_displayed_as_api_credits() {
    let (env, secrets) = fixture("prepaid-ledger", Some(ORG));
    for kind in [json!("claude_max"), json!("claude_enterprise"), Value::Null] {
        let profile = Reply::new(
            "/api/oauth/profile",
            200,
            json!({"organization":{"uuid":ORG,"organization_type":kind},"account":{"uuid":ACCOUNT}}),
        );
        let (url, server) = serve(vec![Reply::organization(), profile], |_| {});
        let result = read_with(&env, "claude", "work", true, &secrets, &url)
            .await
            .unwrap();
        assert_eq!(result.issue, Some(Issue::LoginRequired));
        assert!(result.credit.is_none());
        server.join().unwrap();
    }
}

#[tokio::test]
async fn stale_local_hints_require_verified_account_and_organization() {
    let (env, secrets) = fixture("prepaid-stale-metadata", Some(ORG));
    for (org, account) in [(ORG, OTHER), (OTHER, ACCOUNT)] {
        let (url, server) = serve(
            vec![Reply::organization(), Reply::profile(org, account)],
            |_| {},
        );
        let result = read_with(&env, "claude", "work", true, &secrets, &url)
            .await
            .unwrap();
        assert_eq!(result.issue, Some(Issue::LoginRequired));
        assert!(result.credit.is_none());
        server.join().unwrap();
    }
}

#[tokio::test]
async fn revoked_keys_http_errors_and_invalid_amounts_never_turn_into_zero() {
    let (env, secrets) = fixture("prepaid-errors", Some(ORG));
    for (status, body, issue) in [
        (200, json!({"currency":"USD"}), Issue::Unavailable),
        (
            200,
            json!({"amount":7.1,"currency":"USD"}),
            Issue::Unavailable,
        ),
        (
            200,
            json!({"amount":9007199254740992_i64,"currency":"USD"}),
            Issue::Unavailable,
        ),
        (200, json!({"amount":0,"currency":"?"}), Issue::Unavailable),
        (401, json!({}), Issue::LoginExpired),
        (403, json!({}), Issue::Forbidden),
        (429, json!({}), Issue::RateLimited),
        (503, json!({}), Issue::Unavailable),
    ] {
        let (url, server) = serve(
            vec![
                Reply::organization(),
                Reply::profile(ORG, ACCOUNT),
                Reply::credit(status, body),
            ],
            |_| {},
        );
        let result = read_with(&env, "claude", "work", true, &secrets, &url)
            .await
            .unwrap();
        assert_eq!(result.issue, Some(issue));
        assert!(result.credit.is_none());
        server.join().unwrap();
    }
    let (url, server) = serve(
        vec![
            Reply::new("/v1/organizations/me", 401, json!({})),
            Reply::new("/v1/models?limit=1", 401, json!({})),
        ],
        |_| {},
    );
    let denied = read_with(&env, "claude", "work", true, &secrets, &url)
        .await
        .unwrap();
    assert_eq!(denied.issue, Some(Issue::KeyRejected));
    assert!(denied.credit.is_none());
    server.join().unwrap();
}

#[tokio::test]
async fn changes_during_lookup_discard_a_previously_authorized_response() {
    let (env, secrets) = fixture("prepaid-race", Some(ORG));
    for rotate in [true, false] {
        let (_, dir) = checked(&env, "claude", "work").unwrap();
        let oauth = env
            .profiles_dir(Provider::Claude)
            .join("fixture/oauth_account.json");
        let (url, server) = serve(
            vec![
                Reply::organization(),
                Reply::profile(ORG, ACCOUNT),
                Reply::credit(200, json!({"amount":5,"currency":"USD"})),
            ],
            move |path| {
                if !path.ends_with("/credits") {
                    return;
                }
                if rotate {
                    let mut updated = read_connection(&dir, Provider::Claude, "work").unwrap();
                    updated.revision = "replacement".into();
                    atomic_write(
                        &dir.join("connection.json"),
                        &serde_json::to_vec(&updated).unwrap(),
                    )
                    .unwrap();
                } else {
                    fs::remove_file(&oauth).unwrap();
                }
            },
        );
        let result = read_with(&env, "claude", "work", true, &secrets, &url).await;
        if rotate {
            assert!(result.is_err());
        } else {
            assert_eq!(result.unwrap().issue, Some(Issue::LoginRequired));
        }
        server.join().unwrap();
    }
}

#[tokio::test]
async fn openai_needs_no_secret_or_network_for_the_billing_link() {
    let (env, secrets) = fixture("prepaid-openai", None);
    save_with(&env, &secrets, "codex", "work", KEY).unwrap();
    secrets.remove(&service(Provider::Codex, "work")).unwrap();
    let result = read_with(&env, "codex", "work", true, &secrets, "http://127.0.0.1:1")
        .await
        .unwrap();
    assert!(result.credit.is_none());
    assert_eq!(result.issue, Some(Issue::Unsupported));
}

#[tokio::test]
#[ignore = "requires a locally registered Claude key; only reports the outcome"]
async fn real_registered_claude_balance_status() {
    let env = Env::real().unwrap();
    let name = std::env::var("SWITCHER_BALANCE_CONNECTION")
        .ok()
        .or_else(|| {
            super::super::list(&env, "claude")
                .ok()?
                .first()
                .map(|c| c.name.clone())
        })
        .expect("requires a registered API connection");
    let result = read(&env, "claude", &name, true).await.unwrap();
    if let Ok(expected) = std::env::var("SWITCHER_EXPECTED_BILLING_ORG") {
        let normalize = |name: &str| name.trim().replace('’', "'");
        let matches = result
            .organization
            .as_deref()
            .is_some_and(|name| normalize(name) == normalize(&expected));
        eprintln!("Console display name matches the registered API key: {matches}");
    }
    eprintln!(
        "Claude prepaid balance status: {}",
        match (result.credit.is_some(), result.issue) {
            (true, None) => "confirmed",
            (false, Some(Issue::LoginRequired)) => "billing organization sign-in required",
            (false, Some(Issue::LoginExpired)) => "sign-in expired",
            (false, Some(Issue::Forbidden)) => "permission required",
            (false, Some(Issue::KeyRejected)) => "key rejected",
            (false, Some(Issue::RateLimited)) => "rate limited",
            (false, Some(Issue::Unavailable)) => "unavailable",
            _ => panic!("inconsistent Claude balance status"),
        }
    );
}

#[tokio::test]
async fn console_reading_is_dated_and_scoped_to_the_registered_key() {
    let (env, secrets) = fixture("console-credit-copy", None);
    let (url, server) = serve(vec![Reply::organization()], |_| {});
    let initial = read_with(&env, "claude", "work", false, &secrets, &url)
        .await
        .unwrap();
    let reference = initial.import_ref.unwrap();
    let reading = |org: &str, value, reference: &str| ConsoleReading {
        organization: org.into(),
        amount_minor: value,
        import_ref: reference.into(),
    };
    assert!(console_reading::import_with(
        &env,
        "work",
        reading("Other Studio", 42, &reference),
        &secrets,
        &url
    )
    .await
    .is_err());
    assert!(console_reading::import_with(
        &env,
        "work",
        reading("Fixture Studio", 42, "old"),
        &secrets,
        &url
    )
    .await
    .is_err());
    let saved = console_reading::import_with(
        &env,
        "work",
        reading("Fixture Studio", 6742, &reference),
        &secrets,
        &url,
    )
    .await
    .unwrap();
    assert!(saved.credit.is_none());
    assert_eq!(saved.console_credit.unwrap().amount_minor, 6742);
    let (_, dir) = checked(&env, "claude", "work").unwrap();
    let file = fs::read_to_string(dir.join("console-credit.json")).unwrap();
    assert!(!file.contains("Fixture Studio") && !file.contains(KEY) && !file.contains(LOGIN));
    let revision = read_connection(&dir, Provider::Claude, "work")
        .unwrap()
        .revision;
    assert!(console_reading::load(&dir, OTHER.parse().unwrap(), &revision).is_none());
    assert!(console_reading::load(&dir, ORG.parse().unwrap(), "rotated").is_none());
    forget(&dir);
    let (new_url, new_server) = serve(vec![Reply::organization()], |_| {});
    let restored = read_with(&env, "claude", "work", false, &secrets, &new_url)
        .await
        .unwrap();
    assert_eq!(restored.console_credit.unwrap().amount_minor, 6742);
    new_server.join().unwrap();
    server.join().unwrap();
}

#[tokio::test]
#[ignore = "requires an explicitly supplied local Console reading and registered API connection"]
async fn real_record_requested_console_reading() {
    let (Ok(name), Ok(label), Ok(cents)) = (
        std::env::var("SWITCHER_BALANCE_CONNECTION"),
        std::env::var("SWITCHER_EXPECTED_BILLING_ORG"),
        std::env::var("SWITCHER_CONSOLE_CREDIT_CENTS"),
    ) else {
        return;
    };
    let env = Env::real().unwrap();
    let amount: i64 = cents.parse().unwrap();
    let verified = read(&env, "claude", &name, true).await.unwrap();
    let reading = ConsoleReading {
        organization: label,
        amount_minor: amount,
        import_ref: verified
            .import_ref
            .expect("requires a verified Console organization"),
    };
    let saved = import(&env, &name, reading)
        .await
        .unwrap()
        .console_credit
        .expect("missing Console reading");
    assert!(saved.amount_minor == amount && saved.currency == "USD");
    let (_, dir) = checked(&env, "claude", &name).unwrap();
    forget(&dir);
    let restored = read(&env, "claude", &name, true)
        .await
        .unwrap()
        .console_credit
        .expect("reading lost after reopening");
    assert!(restored == saved && restored.checked_at > 0);
}

#[tokio::test]
async fn active_platform_login_reads_only_the_organization_issued_to_its_token() {
    let (env, secrets) = fixture("prepaid-platform-profile", Some(OTHER));
    let dir = &env.anthropic_config_dir;
    let name = std::env::var("ANTHROPIC_PROFILE").unwrap_or_else(|_| "personal".into());
    assert!(
        name.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            && !name.contains("..")
    );
    fs::create_dir_all(dir.join("configs")).unwrap();
    fs::create_dir_all(dir.join("credentials")).unwrap();
    fs::write(dir.join("active_config"), &name).unwrap();
    fs::write(
        dir.join("configs").join(format!("{name}.json")),
        serde_json::to_vec(&json!({
            "organization_id":ORG,"authentication":{"type":"user_oauth"}
        }))
        .unwrap(),
    )
    .unwrap();
    let path = dir.join("credentials").join(format!("{name}.json"));
    let put =
        |org: &str, exp: u64| {
            fs::write(&path,serde_json::to_vec(&json!({
        "type":"oauth_token", "organization_uuid":org,"expires_at":exp,"access_token":LOGIN
    })).unwrap()).unwrap()
        };
    put(ORG, now() + 86400);
    for issuer in [ORG, OTHER] {
        let mut proof = Reply::new("/v1/organizations/me", 200, json!({"id":issuer}));
        proof.bearer = true;
        let mut replies = vec![Reply::organization(), proof];
        if issuer == ORG {
            replies.push(Reply::credit(200, json!({"amount":845,"currency":"USD"})));
        }
        let (url, server) = serve(replies, |_| {});
        let result = read_with(&env, "claude", "work", true, &secrets, &url)
            .await
            .unwrap();
        if issuer == ORG {
            assert_eq!(result.credit.unwrap().amount_minor, 845);
        } else {
            assert_eq!(result.issue, Some(Issue::LoginRequired));
        }
        server.join().unwrap();
    }
    for (token_org, expiry, issue) in [
        (OTHER, now() + 86400, Issue::LoginRequired),
        (ORG, 1, Issue::LoginExpired),
    ] {
        put(token_org, expiry);
        let (url, server) = serve(vec![Reply::organization()], |_| {});
        let result = read_with(&env, "claude", "work", true, &secrets, &url)
            .await
            .unwrap();
        assert_eq!(result.issue, Some(issue));
        server.join().unwrap();
    }
}

// ── 계정을 고르지 않은 등록 (#187) ─────────────────────────────────
use crate::api_connections::{save_detecting_with, stored_connections, AutoSave, OwnerMatch};

fn named_account(env: &Env, profile: &str, id: &str, full_name: &str) {
    write_profile_parts(
        env,
        Provider::Claude,
        profile,
        &LiveIdentity {
            id: id.into(),
            email: None,
        },
        b"{}",
        Some(&json!({"accountUuid": id, "fullName": full_name})),
    )
    .unwrap();
}

fn organization_named(name: &str) -> Reply {
    Reply::new(
        "/v1/organizations/me",
        200,
        json!({"type":"organization","id":ORG,"name":name}),
    )
}

#[tokio::test]
async fn key_added_without_an_account_lands_under_the_account_its_organization_names() {
    let env = test_env("auto-owner-by-organization");
    let secrets = Memory::default();
    named_account(&env, "first", ACCOUNT, "First Person");
    named_account(&env, "second", OTHER, "Second Person");
    let (url, server) = serve(vec![organization_named("Second Person’s Individual Org")], |_| {});
    let saved = save_detecting_with(&env, &secrets, "claude", None, "  ", KEY, &url)
        .await
        .unwrap();
    server.join().unwrap();
    let AutoSave::Saved {
        connection,
        account_id,
        account,
        matched_by,
    } = saved
    else {
        panic!("the organization names exactly one saved account");
    };
    assert_eq!((account_id.as_str(), account.as_str()), (OTHER, "second"));
    assert_eq!(matched_by, OwnerMatch::OrganizationName);
    assert_eq!(connection.account_id.as_deref(), Some(OTHER));
    assert_eq!(connection.display_name(), "api");
    let stored = stored_connections(&env, Provider::Claude).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].account_id.as_deref(), Some(OTHER));
    // 키 원문은 보안 저장소에만 있다 — 메타데이터에는 없다
    assert_eq!(secrets.get(&service(Provider::Claude, &connection.name)).unwrap(), KEY.as_bytes());
    let (_, dir) = checked(&env, "claude", &connection.name).unwrap();
    assert!(!fs::read_to_string(dir.join("connection.json")).unwrap().contains(KEY));
}

#[tokio::test]
async fn key_whose_owner_cannot_be_told_is_not_stored_until_the_user_names_the_account() {
    let env = test_env("auto-owner-unknown");
    let secrets = Memory::default();
    named_account(&env, "first", ACCOUNT, "First Person");
    named_account(&env, "second", OTHER, "Second Person");
    let (url, server) = serve(
        vec![
            // 회사 이름은 계정을 가리키지 않는다
            organization_named("Demo Studio"),
            // 조직을 확인하지 못해도(키 거절) 추정하지 않는다
            Reply::new("/v1/organizations/me", 401, json!({})),
            Reply::new("/v1/models?limit=1", 401, json!({})),
            // 이름 없는 응답(워크스페이스 키)도 마찬가지다
            Reply::new("/v1/organizations/me", 403, json!({})),
            Reply::models(),
        ],
        |_| {},
    );
    for _ in 0..3 {
        let outcome = save_detecting_with(&env, &secrets, "claude", None, "work", KEY, &url)
            .await
            .unwrap();
        assert!(matches!(outcome, AutoSave::NeedsAccount));
        assert!(stored_connections(&env, Provider::Claude).unwrap().is_empty());
        assert!(secrets.get(&service(Provider::Claude, "work")).is_err());
    }
    // 사용자가 계정을 고르면 공급자에게 다시 묻지 않고 그 계정에 넣는다 (가짜 서버는 요청을 더 받지 않는다)
    let chosen = save_detecting_with(&env, &secrets, "claude", Some(ACCOUNT), "work", KEY, &url)
        .await
        .unwrap();
    server.join().unwrap();
    assert!(matches!(
        &chosen,
        AutoSave::Saved { account_id, matched_by: OwnerMatch::Chosen, connection, .. }
            if account_id == ACCOUNT && connection.display_name() == "work"
    ));
    // 없는 계정·틀린 입력은 저장하지 않는다
    for (account, name, key) in [
        (Some("missing"), "other", KEY),
        (None, "bad name", KEY),
        (None, "other", "not-a-key"),
    ] {
        assert!(save_detecting_with(&env, &secrets, "claude", account, name, key, &url)
            .await
            .is_err());
    }
    assert_eq!(stored_connections(&env, Provider::Claude).unwrap().len(), 1);
}

#[tokio::test]
async fn single_accounts_and_codex_keys_are_placed_without_asking_the_provider() {
    let env = test_env("auto-owner-offline");
    let secrets = Memory::default();
    // 요청을 하나도 받지 않는 가짜 서버 — 조회가 나가면 연결 거부로 드러나지 않고 None이 되므로,
    // 저장 결과(근거)로 조회가 없었음을 확인한다
    let (url, server) = serve(Vec::new(), |_| {});
    server.join().unwrap();
    // 계정이 없으면 넣을 곳이 없다
    assert!(save_detecting_with(&env, &secrets, "claude", None, "", KEY, &url)
        .await
        .is_err());
    named_account(&env, "only", ACCOUNT, "Only Person");
    let saved = save_detecting_with(&env, &secrets, "claude", None, "", KEY, &url)
        .await
        .unwrap();
    assert!(matches!(
        &saved,
        AutoSave::Saved { account_id, account, matched_by: OwnerMatch::OnlyAccount, .. }
            if account_id == ACCOUNT && account == "only"
    ));
    // Codex는 조직 이름을 묻지 않는다: 계정이 하나면 그 계정, 여럿이면 사용자에게 묻는다
    crate::api_connections::tests::add_account(&env, Provider::Codex, "codex-one", "codex-alpha");
    let codex = save_detecting_with(&env, &secrets, "codex", None, "", KEY, &url)
        .await
        .unwrap();
    assert!(matches!(&codex, AutoSave::Saved { matched_by: OwnerMatch::OnlyAccount, .. }));
    crate::api_connections::tests::add_account(&env, Provider::Codex, "codex-two", "codex-beta");
    let asked = save_detecting_with(&env, &secrets, "codex", None, "", KEY, &url)
        .await
        .unwrap();
    assert!(matches!(asked, AutoSave::NeedsAccount));
    assert_eq!(stored_connections(&env, Provider::Codex).unwrap().len(), 1);
}

/// #189 — "남은 api 사용량 왜 안나와": 다른 PC에서 옮겨 온 키는 받는 PC의 화면이 잔액을 읽는
/// 바로 그 길에서도 남은 금액을 보여 줘야 한다. v2.2.0은 키만 옮겨서 이 자리가 비어 있었다.
#[tokio::test]
async fn a_transferred_key_shows_its_remaining_credit_on_the_receiving_pc() {
    // 보내는 PC: 키를 등록하고, Console 대시보드에서 남은 금액을 가져와 둔 상태
    let (from, from_secrets) = fixture("transfer-credit-from", None);
    let (url, server) = serve(vec![Reply::organization()], |_| {});
    let initial = read_with(&from, "claude", "work", false, &from_secrets, &url)
        .await
        .unwrap();
    let on_sender = console_reading::import_with(
        &from,
        "work",
        ConsoleReading {
            organization: "Fixture Studio".into(),
            amount_minor: 6742,
            import_ref: initial.import_ref.unwrap(),
        },
        &from_secrets,
        &url,
    )
    .await
    .unwrap()
    .console_credit
    .unwrap();
    server.join().unwrap();
    let keys = crate::api_connections::export_keys_with(&from, &from_secrets).unwrap();

    // 받는 PC: 아무것도 없는 상태에서 키를 받는다
    let to = test_env("transfer-credit-to");
    let to_secrets = Memory::default();
    crate::api_connections::import_keys_with(&to, &to_secrets, &keys).unwrap();
    // 화면이 잔액을 읽는 길: 키로 조직을 확인한 뒤, 이 PC에 저장된 확인값을 내놓는다
    let (url, server) = serve(vec![Reply::organization()], |_| {});
    let shown = read_with(&to, "claude", "work", false, &to_secrets, &url)
        .await
        .unwrap();
    server.join().unwrap();
    assert_eq!(shown.organization.as_deref(), Some("Fixture Studio"));
    // 금액과 확인한 시각이 보내는 PC와 같다 (받는 PC에서 새로 확인한 것처럼 꾸미지 않는다)
    assert_eq!(shown.console_credit, Some(on_sender));
    assert!(shown.credit.is_none() && shown.console_credit.unwrap().amount_minor == 6742);
}

/// #189 — 키의 조직을 지금 확인하지 못해도(네트워크·일시 오류) 이 키에 묶인 Console 확인값은 보인다.
/// 키가 거절되거나, 확인한 조직이 다르면 보이지 않는다.
#[tokio::test]
async fn a_saved_reading_stays_visible_when_the_organization_cannot_be_checked_now() {
    let (env, secrets) = fixture("console-credit-offline", None);
    let (_, dir) = checked(&env, "claude", "work").unwrap();
    let revision = read_connection(&dir, Provider::Claude, "work").unwrap().revision;
    let reading = |org: &str, revision: &str| {
        fs::write(
            dir.join("console-credit.json"),
            json!({"organization_id": org, "revision": revision, "credit": {"amount_minor": 6742, "currency": "USD", "checked_at": 1_795_000_000}}).to_string(),
        )
        .unwrap();
        forget(&dir);
    };
    reading(ORG, &revision);
    // 서버에 닿지 못함: 조직은 모르지만 이 키의 확인값을 보여 준다
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let closed = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let offline = read_with(&env, "claude", "work", true, &secrets, &closed).await.unwrap();
    assert_eq!(offline.issue, Some(Issue::Unavailable));
    assert_eq!(offline.console_credit.map(|c| c.amount_minor), Some(6742));
    // 서버가 일시 오류: 마찬가지
    forget(&dir);
    let (url, server) = serve(vec![Reply::new("/v1/organizations/me", 503, json!({}))], |_| {});
    let busy = read_with(&env, "claude", "work", true, &secrets, &url).await.unwrap();
    server.join().unwrap();
    assert_eq!(busy.console_credit.map(|c| c.amount_minor), Some(6742));
    // 키가 거절됨: 보이지 않는다
    forget(&dir);
    let (url, server) = serve(
        vec![
            Reply::new("/v1/organizations/me", 401, json!({})),
            Reply::new("/v1/models?limit=1", 401, json!({})),
        ],
        |_| {},
    );
    let rejected = read_with(&env, "claude", "work", true, &secrets, &url).await.unwrap();
    server.join().unwrap();
    assert_eq!(rejected.issue, Some(Issue::KeyRejected));
    assert!(rejected.console_credit.is_none());
    // 옛 키에 묶인 확인값은 서버에 닿지 못해도 보이지 않는다
    reading(ORG, "an-older-key");
    let stale = read_with(&env, "claude", "work", true, &secrets, &closed).await.unwrap();
    assert!(stale.console_credit.is_none());
    // 조직을 확인했는데 확인값의 조직과 다르면 보이지 않는다 (예전 그대로)
    reading(OTHER, &revision);
    let (url, server) = serve(vec![Reply::organization()], |_| {});
    let other = read_with(&env, "claude", "work", true, &secrets, &url).await.unwrap();
    server.join().unwrap();
    assert!(other.console_credit.is_none());
}

/// #189 — Windows 실제 보안 저장소(DPAPI)로 보내는 PC → 받는 PC를 거친 키도 남은 금액이 보인다.
/// 사용자가 "윈도우 최신에선 남은 조직 크레딧 양이 안 떠"라고 보고한 길을 Windows에서 그대로 밟는다.
#[cfg(windows)]
#[tokio::test]
async fn windows_a_key_moved_through_the_os_store_shows_its_remaining_credit() {
    let from = test_env("win-transfer-credit-from");
    save_with(&from, &OsSecrets(&from), "claude", "work", KEY).unwrap();
    let (_, from_dir) = checked(&from, "claude", "work").unwrap();
    let revision = read_connection(&from_dir, Provider::Claude, "work").unwrap().revision;
    fs::write(
        from_dir.join("console-credit.json"),
        json!({"organization_id": ORG, "revision": revision, "credit": {"amount_minor": 6742, "currency": "USD", "checked_at": 1_795_000_000}}).to_string(),
    )
    .unwrap();
    let keys: Vec<crate::api_connections::TransferKey> = serde_json::from_slice(
        &serde_json::to_vec(&crate::api_connections::export_keys_with(&from, &OsSecrets(&from)).unwrap()).unwrap(),
    )
    .unwrap();
    let to = test_env("win-transfer-credit-to");
    crate::api_connections::import_keys_with(&to, &OsSecrets(&to), &keys).unwrap();
    let (url, server) = serve(vec![Reply::organization()], |_| {});
    let shown = read_with(&to, "claude", "work", false, &OsSecrets(&to), &url).await.unwrap();
    server.join().unwrap();
    assert_eq!(shown.console_credit.map(|c| (c.amount_minor, c.checked_at)), Some((6742, 1_795_000_000)));
    // 받은 PC에서 서버에 닿지 못해도 보인다
    let (_, to_dir) = checked(&to, "claude", "work").unwrap();
    forget(&to_dir);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let closed = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let offline = read_with(&to, "claude", "work", true, &OsSecrets(&to), &closed).await.unwrap();
    assert_eq!(offline.console_credit.map(|c| c.amount_minor), Some(6742));
    let _ = fs::remove_dir_all(&from.home);
    let _ = fs::remove_dir_all(&to.home);
}

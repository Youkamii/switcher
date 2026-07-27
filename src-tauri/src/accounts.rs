//! 계정 프로필 저장·전환 코어.
//!
//! 불변 규칙 (CLAUDE.md 금기 항목과 동일):
//! - 전환 순서: 활성 파일을 현재 계정 프로필에 백업한 뒤에만 대상 프로필을 복사한다.
//!   토큰이 수시로 자동 갱신되므로 순서를 바꾸면 최신 토큰이 유실된다.
//! - 어떤 경로에서도 토큰 내용을 로그·에러 메시지에 싣지 않는다.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub fn parse(s: &str) -> Result<Provider, String> {
        match s {
            "claude" => Ok(Provider::Claude),
            "codex" => Ok(Provider::Codex),
            _ => Err(format!("알 수 없는 provider: {s}")),
        }
    }

    fn dir_name(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
        }
    }

    /// 프로필 폴더 안에 저장되는 토큰 파일 이름 (원본 파일명과 동일하게 유지)
    pub(crate) fn credential_file_name(self) -> &'static str {
        match self {
            Provider::Claude => "credentials.json",
            Provider::Codex => "auth.json",
        }
    }
}

/// 홈·보관소 경로 묶음. 테스트에서는 임시 디렉토리를 주입한다.
#[derive(Clone)]
pub struct Env {
    pub home: PathBuf,
    pub store: PathBuf,
}

impl Env {
    pub fn real() -> Result<Env, String> {
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .ok_or("홈 디렉토리를 찾을 수 없습니다")?;
        let store = home.join(".switcher");
        Ok(Env { home, store })
    }

    pub(crate) fn profiles_dir(&self, provider: Provider) -> PathBuf {
        self.store.join(provider.dir_name()).join("profiles")
    }

    pub(crate) fn live_credential_path(&self, provider: Provider) -> PathBuf {
        match provider {
            Provider::Claude => self.home.join(".claude").join(".credentials.json"),
            Provider::Codex => self.home.join(".codex").join("auth.json"),
        }
    }

    fn claude_json_path(&self) -> PathBuf {
        self.home.join(".claude.json")
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Meta {
    pub id: String,
    pub email: Option<String>,
    pub saved_at: u64,
}

#[derive(Serialize, Clone)]
pub struct LiveIdentity {
    pub id: String,
    pub email: Option<String>,
}

#[derive(Serialize)]
pub struct ProfileInfo {
    pub name: String,
    pub id: String,
    pub email: Option<String>,
    pub saved_at: u64,
    pub active: bool,
}

#[derive(Serialize)]
pub struct Snapshot {
    pub profiles: Vec<ProfileInfo>,
    pub live: Option<LiveIdentity>,
    /// 현재 로그인 계정이 어느 프로필에도 저장되어 있지 않으면 false
    pub live_saved: bool,
}

#[derive(Serialize)]
pub struct SwitchResult {
    pub backed_up_to: Option<String>,
    pub switched_to: String,
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 프로필 이름은 경로에 들어가므로 엄격히 제한한다 (경로 탈출 방지).
fn validate_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err("프로필 이름은 영문·숫자·하이픈·언더스코어 1~32자만 가능합니다".into())
    }
}

/// 임시 파일에 쓴 뒤 rename — 읽는 쪽이 절대 반쪽짜리 파일을 보지 않게 한다.
fn atomic_write(path: &Path, data: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("경로 오류: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|e| format!("폴더 생성 실패 {}: {e}", parent.display()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("경로 오류: {}", path.display()))?
        .to_string_lossy()
        .to_string();
    let tmp = parent.join(format!("{file_name}.switcher-tmp"));
    fs::write(&tmp, data).map_err(|e| format!("쓰기 실패 {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("교체 실패 {}: {e}", path.display()))
}

pub(crate) fn read_json(path: &Path) -> Result<Value, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("읽기 실패 {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("JSON 파싱 실패 {}: {e}", path.display()))
}

/// JWT payload에서 특정 문자열 claim을 꺼낸다 (서명 검증 없음 — 표시용 신원 확인 목적).
fn jwt_claim(token: &str, claim: &str) -> Option<String> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value.get(claim)?.as_str().map(String::from)
}

/// 현재 로그인된 계정의 신원. 파일이 없거나 식별 불가면 Ok(None).
fn live_identity(env: &Env, provider: Provider) -> Result<Option<LiveIdentity>, String> {
    match provider {
        Provider::Claude => {
            let path = env.claude_json_path();
            if !path.exists() {
                return Ok(None);
            }
            let root = read_json(&path)?;
            let Some(acc) = root.get("oauthAccount") else {
                return Ok(None);
            };
            let Some(id) = acc.get("accountUuid").and_then(|v| v.as_str()) else {
                return Ok(None);
            };
            let email = acc
                .get("emailAddress")
                .and_then(|v| v.as_str())
                .map(String::from);
            Ok(Some(LiveIdentity {
                id: id.to_string(),
                email,
            }))
        }
        Provider::Codex => {
            let path = env.live_credential_path(Provider::Codex);
            if !path.exists() {
                return Ok(None);
            }
            let root = read_json(&path)?;
            let Some(tokens) = root.get("tokens") else {
                return Ok(None);
            };
            let id_token = tokens.get("id_token").and_then(|v| v.as_str());
            let id = tokens
                .get("account_id")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| id_token.and_then(|t| jwt_claim(t, "sub")));
            let Some(id) = id else {
                return Ok(None);
            };
            let email = id_token.and_then(|t| jwt_claim(t, "email"));
            Ok(Some(LiveIdentity { id, email }))
        }
    }
}

/// ~/.claude.json의 oauthAccount 블록 (프로필에 함께 보관해 전환 시 복원)
fn claude_oauth_block(env: &Env) -> Result<Option<Value>, String> {
    let path = env.claude_json_path();
    if !path.exists() {
        return Ok(None);
    }
    Ok(read_json(&path)?.get("oauthAccount").cloned())
}

/// 현재 활성 파일들을 지정 이름의 프로필로 저장한다 (덮어쓰기 허용).
fn write_profile(
    env: &Env,
    provider: Provider,
    name: &str,
    ident: &LiveIdentity,
) -> Result<(), String> {
    let dir = env.profiles_dir(provider).join(name);
    let live = env.live_credential_path(provider);
    let data = fs::read(&live).map_err(|e| format!("읽기 실패 {}: {e}", live.display()))?;
    atomic_write(&dir.join(provider.credential_file_name()), &data)?;

    if provider == Provider::Claude {
        if let Some(block) = claude_oauth_block(env)? {
            let bytes = serde_json::to_vec_pretty(&block).map_err(|e| e.to_string())?;
            atomic_write(&dir.join("oauth_account.json"), &bytes)?;
        }
    }

    let meta = Meta {
        id: ident.id.clone(),
        email: ident.email.clone(),
        saved_at: now(),
    };
    let bytes = serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?;
    atomic_write(&dir.join("meta.json"), &bytes)
}

fn read_meta(dir: &Path) -> Option<Meta> {
    let path = dir.join("meta.json");
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn profile_dirs(env: &Env, provider: Provider) -> Result<Vec<(String, PathBuf)>, String> {
    let root = env.profiles_dir(provider);
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let entries =
        fs::read_dir(&root).map_err(|e| format!("읽기 실패 {}: {e}", root.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.push((entry.file_name().to_string_lossy().to_string(), path));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn find_profile_by_id(
    env: &Env,
    provider: Provider,
    id: &str,
) -> Result<Option<String>, String> {
    for (name, dir) in profile_dirs(env, provider)? {
        if let Some(meta) = read_meta(&dir) {
            if meta.id == id {
                return Ok(Some(name));
            }
        }
    }
    Ok(None)
}

/// 이메일 앞부분(또는 계정 id 앞 8자)으로 자동 프로필 이름을 만든다.
fn auto_name(env: &Env, provider: Provider, ident: &LiveIdentity) -> String {
    let base: String = ident
        .email
        .as_deref()
        .and_then(|e| e.split('@').next())
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(20)
        .collect::<String>()
        .to_lowercase();
    let mut name = if base.is_empty() {
        format!("account-{}", ident.id.chars().take(8).collect::<String>())
    } else {
        base
    };
    // 같은 이름의 프로필이 다른 계정 것이면 id 앞 4자를 붙여 구분
    let dir = env.profiles_dir(provider).join(&name);
    if let Some(meta) = read_meta(&dir) {
        if meta.id != ident.id {
            let suffix: String = ident.id.chars().take(4).collect();
            name = format!("{name}-{suffix}");
        }
    }
    name
}

/// 대상 프로필의 oauth_account.json을 ~/.claude.json에 반영한다.
/// 실행 중인 클로드 세션과의 쓰기 경합 창을 줄이기 위해 반영 직전에 새로 읽는다.
fn claude_apply_oauth_block(env: &Env, profile_dir: &Path) -> Result<(), String> {
    let block_path = profile_dir.join("oauth_account.json");
    let cj = env.claude_json_path();
    if !block_path.exists() || !cj.exists() {
        return Ok(());
    }
    let block = read_json(&block_path)?;
    let mut root = read_json(&cj)?;
    let Some(obj) = root.as_object_mut() else {
        return Ok(());
    };
    obj.insert("oauthAccount".to_string(), block);
    let bytes = serde_json::to_vec_pretty(&root).map_err(|e| e.to_string())?;
    atomic_write(&cj, &bytes)
}

/// 현재 로그인 계정을 이름 붙여 프로필로 저장
pub fn save_current(env: &Env, provider: Provider, name: &str) -> Result<(), String> {
    validate_name(name)?;
    let live = env.live_credential_path(provider);
    if !live.exists() {
        return Err(format!(
            "로그인 파일이 없습니다: {} — 먼저 해당 CLI에서 로그인하세요",
            live.display()
        ));
    }
    let ident = live_identity(env, provider)?
        .ok_or("현재 로그인 계정을 식별할 수 없습니다 (로그인 직후 다시 시도)")?;
    // 같은 계정이 이미 다른 이름으로 저장돼 있으면 중복 프로필을 막는다
    if let Some(existing) = find_profile_by_id(env, provider, &ident.id)? {
        if existing != name {
            return Err(format!(
                "이 계정은 이미 '{existing}' 프로필로 저장되어 있습니다"
            ));
        }
    }
    write_profile(env, provider, name, &ident)
}

/// 계정 전환. 순서 불변: 1) 현재 활성 파일 백업 → 2) 대상 프로필 복사
pub fn switch(env: &Env, provider: Provider, name: &str) -> Result<SwitchResult, String> {
    validate_name(name)?;
    let profile_dir = env.profiles_dir(provider).join(name);
    let target_cred = profile_dir.join(provider.credential_file_name());
    if !target_cred.exists() {
        return Err(format!("프로필 '{name}'에 저장된 토큰이 없습니다"));
    }

    // 1) 백업 — 현재 활성 계정을 자기 프로필(없으면 자동 생성)에 저장
    let mut backed_up_to = None;
    let live_path = env.live_credential_path(provider);
    if live_path.exists() {
        match live_identity(env, provider)? {
            Some(live) => {
                let back_name = match find_profile_by_id(env, provider, &live.id)? {
                    Some(existing) => existing,
                    None => auto_name(env, provider, &live),
                };
                write_profile(env, provider, &back_name, &live)?;
                backed_up_to = Some(back_name);
            }
            None => {
                // 신원 불명이어도 토큰을 버리지 않는다 — 구조용 프로필로 보관
                let rescue = format!("rescue-{}", now());
                let ident = LiveIdentity {
                    id: format!("unknown-{}", now()),
                    email: None,
                };
                write_profile(env, provider, &rescue, &ident)?;
                backed_up_to = Some(rescue);
            }
        }
    }

    // 2) 대상 프로필을 활성 위치로 복사
    let data = fs::read(&target_cred)
        .map_err(|e| format!("읽기 실패 {}: {e}", target_cred.display()))?;
    atomic_write(&live_path, &data)?;
    if provider == Provider::Claude {
        claude_apply_oauth_block(env, &profile_dir)?;
    }

    Ok(SwitchResult {
        backed_up_to,
        switched_to: name.to_string(),
    })
}

/// 프로필 목록 + 현재 로그인 계정 상태
pub fn list(env: &Env, provider: Provider) -> Result<Snapshot, String> {
    let live = live_identity(env, provider)?;
    let live_id = live.as_ref().map(|l| l.id.clone());
    let mut profiles = Vec::new();
    for (name, dir) in profile_dirs(env, provider)? {
        if let Some(meta) = read_meta(&dir) {
            profiles.push(ProfileInfo {
                active: live_id.as_deref() == Some(meta.id.as_str()),
                name,
                id: meta.id,
                email: meta.email,
                saved_at: meta.saved_at,
            });
        }
    }
    let live_saved = profiles.iter().any(|p| p.active);
    Ok(Snapshot {
        profiles,
        live,
        live_saved,
    })
}

/// 프로필 삭제 (보관함에서만 지운다 — 활성 로그인은 건드리지 않음)
pub fn delete(env: &Env, provider: Provider, name: &str) -> Result<(), String> {
    validate_name(name)?;
    let dir = env.profiles_dir(provider).join(name);
    if !dir.exists() {
        return Err(format!("프로필 '{name}'이 없습니다"));
    }
    fs::remove_dir_all(&dir).map_err(|e| format!("삭제 실패 {}: {e}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 실토큰이 아닌 픽스처로만 검증한다 (CLAUDE.md 금기)
    fn test_env(tag: &str) -> Env {
        let base = std::env::temp_dir().join(format!("switcher-test-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join(".claude")).unwrap();
        Env {
            home: base.clone(),
            store: base.join(".switcher"),
        }
    }

    fn login_claude(env: &Env, uuid: &str, email: &str, token: &str) {
        fs::write(
            env.home.join(".claude").join(".credentials.json"),
            format!(r#"{{"claudeAiOauth":{{"accessToken":"{token}"}}}}"#),
        )
        .unwrap();
        fs::write(
            env.claude_json_path(),
            format!(
                r#"{{"numStartups":1,"oauthAccount":{{"accountUuid":"{uuid}","emailAddress":"{email}"}}}}"#
            ),
        )
        .unwrap();
    }

    fn live_token(env: &Env) -> String {
        fs::read_to_string(env.live_credential_path(Provider::Claude)).unwrap()
    }

    #[test]
    fn save_then_list_marks_active() {
        let env = test_env("save-list");
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        save_current(&env, Provider::Claude, "main").unwrap();

        let snap = list(&env, Provider::Claude).unwrap();
        assert_eq!(snap.profiles.len(), 1);
        assert_eq!(snap.profiles[0].name, "main");
        assert!(snap.profiles[0].active);
        assert!(snap.live_saved);
        assert_eq!(snap.profiles[0].email.as_deref(), Some("alice@test.dev"));
    }

    #[test]
    fn switch_backs_up_current_then_swaps() {
        let env = test_env("switch");
        // 계정 B를 먼저 저장해두고
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b1");
        save_current(&env, Provider::Claude, "second").unwrap();
        // 계정 A로 로그인된 상태(토큰이 그새 갱신됐다고 가정: tok-a2)
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a2");
        save_current(&env, Provider::Claude, "main").unwrap();
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a3"); // 저장 후 또 갱신됨

        let result = switch(&env, Provider::Claude, "second").unwrap();

        // 백업이 먼저: main 프로필에 최신 토큰(tok-a3)이 저장돼 있어야 한다
        assert_eq!(result.backed_up_to.as_deref(), Some("main"));
        let backed = fs::read_to_string(
            env.profiles_dir(Provider::Claude)
                .join("main")
                .join("credentials.json"),
        )
        .unwrap();
        assert!(backed.contains("tok-a3"));

        // 활성 파일은 B의 토큰으로 교체
        assert!(live_token(&env).contains("tok-b1"));
        // ~/.claude.json의 oauthAccount도 B로 반영
        let root = read_json(&env.claude_json_path()).unwrap();
        assert_eq!(
            root["oauthAccount"]["accountUuid"].as_str(),
            Some("uuid-b")
        );
        // 다른 키는 보존
        assert_eq!(root["numStartups"].as_i64(), Some(1));

        let snap = list(&env, Provider::Claude).unwrap();
        let active: Vec<_> = snap.profiles.iter().filter(|p| p.active).collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].name, "second");
    }

    #[test]
    fn switch_auto_saves_unsaved_account() {
        let env = test_env("auto-save");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b1");
        save_current(&env, Provider::Claude, "second").unwrap();
        // 프로필로 저장한 적 없는 계정 A가 로그인된 상태에서 바로 전환
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");

        let result = switch(&env, Provider::Claude, "second").unwrap();

        // A 계정이 이메일 기반 자동 프로필로 구조됨
        assert_eq!(result.backed_up_to.as_deref(), Some("alice"));
        let backed = fs::read_to_string(
            env.profiles_dir(Provider::Claude)
                .join("alice")
                .join("credentials.json"),
        )
        .unwrap();
        assert!(backed.contains("tok-a1"));
        assert!(live_token(&env).contains("tok-b1"));
    }

    #[test]
    fn save_rejects_duplicate_account_under_new_name() {
        let env = test_env("dup");
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        save_current(&env, Provider::Claude, "main").unwrap();
        let err = save_current(&env, Provider::Claude, "other").unwrap_err();
        assert!(err.contains("main"));
    }

    #[test]
    fn name_validation_blocks_path_escape() {
        let env = test_env("names");
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        assert!(save_current(&env, Provider::Claude, "../evil").is_err());
        assert!(save_current(&env, Provider::Claude, "a b").is_err());
        assert!(save_current(&env, Provider::Claude, "").is_err());
        assert!(switch(&env, Provider::Claude, "..").is_err());
    }

    /// 가짜 JWT (서명 없음) — email claim만 담는다
    fn fake_jwt(email: &str) -> String {
        use base64::Engine;
        let enc = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        format!(
            "{}.{}.{}",
            enc(r#"{"alg":"none"}"#),
            enc(&format!(r#"{{"email":"{email}","sub":"sub-x"}}"#)),
            enc("sig")
        )
    }

    fn login_codex(env: &Env, account_id: &str, email: &str, token: &str) {
        fs::create_dir_all(env.home.join(".codex")).unwrap();
        fs::write(
            env.home.join(".codex").join("auth.json"),
            format!(
                r#"{{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{{"id_token":"{}","access_token":"{token}","refresh_token":"r-{token}","account_id":"{account_id}"}},"last_refresh":"2026-01-01T00:00:00Z"}}"#,
                fake_jwt(email)
            ),
        )
        .unwrap();
    }

    #[test]
    fn codex_switch_backs_up_then_swaps() {
        let env = test_env("codex");
        login_codex(&env, "acct-b", "bob@test.dev", "ctok-b1");
        save_current(&env, Provider::Codex, "personal").unwrap();
        login_codex(&env, "acct-a", "alice@test.dev", "ctok-a1");
        save_current(&env, Provider::Codex, "work").unwrap();
        login_codex(&env, "acct-a", "alice@test.dev", "ctok-a2"); // 갱신 가정

        let result = switch(&env, Provider::Codex, "personal").unwrap();

        assert_eq!(result.backed_up_to.as_deref(), Some("work"));
        let backed = fs::read_to_string(
            env.profiles_dir(Provider::Codex)
                .join("work")
                .join("auth.json"),
        )
        .unwrap();
        assert!(backed.contains("ctok-a2"));
        let live =
            fs::read_to_string(env.live_credential_path(Provider::Codex)).unwrap();
        assert!(live.contains("ctok-b1"));

        let snap = list(&env, Provider::Codex).unwrap();
        let active: Vec<_> = snap.profiles.iter().filter(|p| p.active).collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].name, "personal");
        assert_eq!(active[0].email.as_deref(), Some("bob@test.dev"));
    }

    #[test]
    fn codex_and_claude_profiles_are_isolated() {
        let env = test_env("isolation");
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        save_current(&env, Provider::Claude, "main").unwrap();
        login_codex(&env, "acct-a", "alice@test.dev", "ctok-a1");
        save_current(&env, Provider::Codex, "main").unwrap();

        // 같은 이름이라도 프로바이더별로 분리 보관
        assert!(env
            .profiles_dir(Provider::Claude)
            .join("main")
            .join("credentials.json")
            .exists());
        assert!(env
            .profiles_dir(Provider::Codex)
            .join("main")
            .join("auth.json")
            .exists());
    }

    #[test]
    fn delete_removes_only_stored_profile() {
        let env = test_env("delete");
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        save_current(&env, Provider::Claude, "main").unwrap();
        delete(&env, Provider::Claude, "main").unwrap();
        assert!(list(&env, Provider::Claude).unwrap().profiles.is_empty());
        // 활성 로그인 파일은 그대로
        assert!(env.live_credential_path(Provider::Claude).exists());
    }
}

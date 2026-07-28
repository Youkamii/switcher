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

    pub(crate) fn dir_name(self) -> &'static str {
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

#[derive(Serialize, Deserialize)]
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
    /// 구독 레벨 (Max·Pro·Plus 등) — 토큰 파일에서 추출한 표시용 정보
    pub plan: Option<String>,
    /// 클로드 Max의 배수 (rateLimitTier "..._max_20x" → 20). 코덱스는 None.
    pub plan_tier: Option<u32>,
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

#[derive(Serialize, Debug)]
pub struct SwitchResult {
    pub backed_up_to: Option<String>,
    pub switched_to: String,
}

/// 저장·전환·삭제·임포트는 파일을 옮기는 다단계 작업이라 동시에 두 개가 돌면
/// 백업과 교체가 교차해 계정이 어긋날 수 있다 — 이 잠금으로 직렬화한다.
pub(crate) static MUTATION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 프로필 이름은 경로에 들어가므로 엄격히 제한한다 (경로 탈출 방지).
pub(crate) fn validate_name(name: &str) -> Result<(), String> {
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
pub(crate) fn atomic_write(path: &Path, data: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("경로 오류: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|e| format!("폴더 생성 실패 {}: {e}", parent.display()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("경로 오류: {}", path.display()))?
        .to_string_lossy()
        .to_string();
    // 동시 쓰기 경합 시 임시 파일이 겹치지 않게 일련번호를 붙인다
    static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = parent.join(format!("{file_name}.switcher-tmp{seq}"));
    fs::write(&tmp, data).map_err(|e| format!("쓰기 실패 {}: {e}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, path).map_err(|e| {
        // 실패 시 평문 토큰이 담긴 임시 파일을 남기지 않는다
        let _ = fs::remove_file(&tmp);
        format!("교체 실패 {}: {e}", path.display())
    })
}

pub(crate) fn read_json(path: &Path) -> Result<Value, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("읽기 실패 {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("JSON 파싱 실패 {}: {e}", path.display()))
}

/// 다른 프로그램(실행 중인 CLI)이 쓰는 도중의 반쪽짜리 파일을 읽을 수 있으므로
/// 파싱 실패 시 짧게 기다렸다가 재시도한다.
fn read_json_retry(path: &Path) -> Result<Value, String> {
    let mut last_err = String::new();
    for attempt in 0..3 {
        match read_json(path) {
            Ok(value) => return Ok(value),
            Err(e) => {
                last_err = e;
                if attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_millis(80));
                }
            }
        }
    }
    Err(last_err)
}

/// JWT payload를 디코딩한다 (서명 검증 없음 — 표시용 신원·만료 확인 목적).
pub(crate) fn jwt_payload(token: &str) -> Option<Value> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn jwt_claim(token: &str, claim: &str) -> Option<String> {
    jwt_payload(token)?.get(claim)?.as_str().map(String::from)
}

/// 파싱된 JSON에서 계정 신원을 뽑는다.
/// 클로드는 ~/.claude.json 루트를, 코덱스는 auth.json 루트를 받는다.
/// 활성 파일 판독과 격리 로그인 결과 임포트가 같은 파서를 쓴다 (한쪽만 고치는 사고 방지).
pub(crate) fn identity_from_value(provider: Provider, root: &Value) -> Option<LiveIdentity> {
    match provider {
        Provider::Claude => {
            let acc = root.get("oauthAccount")?;
            let id = acc.get("accountUuid").and_then(|v| v.as_str())?.to_string();
            let email = acc
                .get("emailAddress")
                .and_then(|v| v.as_str())
                .map(String::from);
            Some(LiveIdentity { id, email })
        }
        Provider::Codex => {
            let tokens = root.get("tokens")?;
            let id_token = tokens.get("id_token").and_then(|v| v.as_str());
            let id = tokens
                .get("account_id")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| id_token.and_then(|t| jwt_claim(t, "sub")))?;
            let email = id_token.and_then(|t| jwt_claim(t, "email"));
            Some(LiveIdentity { id, email })
        }
    }
}

/// 토큰 파일에서 구독 레벨을 뽑는다 (표시용).
/// 클로드: claudeAiOauth.subscriptionType ("max"), 코덱스: JWT auth claim의 chatgpt_plan_type ("pro").
fn plan_from_credential(provider: Provider, root: &Value) -> Option<String> {
    let raw = match provider {
        Provider::Claude => root
            .pointer("/claudeAiOauth/subscriptionType")?
            .as_str()?
            .to_string(),
        Provider::Codex => {
            let token = root.pointer("/tokens/id_token")?.as_str()?;
            jwt_payload(token)?
                .get("https://api.openai.com/auth")?
                .get("chatgpt_plan_type")?
                .as_str()?
                .to_string()
        }
    };
    // 첫 글자만 대문자로 (max → Max)
    let mut chars = raw.chars();
    Some(match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => return None,
    })
}

/// 클로드 Max의 배수를 뽑는다 ("default_claude_max_20x" → 20)
fn tier_from_credential(provider: Provider, root: &Value) -> Option<u32> {
    if provider != Provider::Claude {
        return None;
    }
    let tier = root.pointer("/claudeAiOauth/rateLimitTier")?.as_str()?;
    tier.strip_suffix('x')?.rsplit('_').next()?.parse().ok()
}

/// 현재 로그인된 계정의 신원. 파일이 없거나 식별 불가면 Ok(None).
pub(crate) fn live_identity(env: &Env, provider: Provider) -> Result<Option<LiveIdentity>, String> {
    let path = match provider {
        Provider::Claude => env.claude_json_path(),
        Provider::Codex => env.live_credential_path(Provider::Codex),
    };
    if !path.exists() {
        return Ok(None);
    }
    let root = read_json_retry(&path)?;
    Ok(identity_from_value(provider, &root))
}

/// ~/.claude.json의 oauthAccount 블록 (프로필에 함께 보관해 전환 시 복원)
fn claude_oauth_block(env: &Env) -> Result<Option<Value>, String> {
    let path = env.claude_json_path();
    if !path.exists() {
        return Ok(None);
    }
    Ok(read_json_retry(&path)?.get("oauthAccount").cloned())
}

/// 토큰 내용과 계정 정보를 직접 받아 프로필로 저장한다.
/// 활성 파일에서 저장할 때도, 격리 로그인 결과를 임포트할 때도 이 경로를 쓴다.
pub(crate) fn write_profile_parts(
    env: &Env,
    provider: Provider,
    name: &str,
    ident: &LiveIdentity,
    cred: &[u8],
    oauth_block: Option<&Value>,
) -> Result<(), String> {
    let dir = env.profiles_dir(provider).join(name);
    // 기존 토큰을 덮어쓰기 전에 한 세대 .bak으로 남긴다 — 잘못된 덮어쓰기의 최후 안전망
    let cred_path = dir.join(provider.credential_file_name());
    if cred_path.exists() {
        let _ = fs::copy(&cred_path, cred_path.with_extension("json.bak"));
    }
    atomic_write(&cred_path, cred)?;

    if let Some(block) = oauth_block {
        let bytes = serde_json::to_vec_pretty(block).map_err(|e| e.to_string())?;
        atomic_write(&dir.join("oauth_account.json"), &bytes)?;
    }

    let meta = Meta {
        id: ident.id.clone(),
        email: ident.email.clone(),
        saved_at: now(),
    };
    let bytes = serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?;
    atomic_write(&dir.join("meta.json"), &bytes)
}

/// 현재 활성 파일들을 지정 이름의 프로필로 저장한다 (덮어쓰기 허용).
fn write_profile(
    env: &Env,
    provider: Provider,
    name: &str,
    ident: &LiveIdentity,
) -> Result<(), String> {
    let live = env.live_credential_path(provider);
    let data = fs::read(&live).map_err(|e| format!("읽기 실패 {}: {e}", live.display()))?;
    let block = if provider == Provider::Claude {
        claude_oauth_block(env)?
    } else {
        None
    };
    write_profile_parts(env, provider, name, ident, &data, block.as_ref())
}

/// name 프로필이 이미 다른 계정의 것이면 에러 — 다른 계정 토큰을 덮어쓰지 않는다
pub(crate) fn ensure_name_not_owned_by_other(
    env: &Env,
    provider: Provider,
    name: &str,
    ident: &LiveIdentity,
) -> Result<(), String> {
    let dir = env.profiles_dir(provider).join(name);
    if let Some(meta) = read_meta(&dir) {
        if meta.id != ident.id {
            let owner = meta.email.unwrap_or(meta.id);
            return Err(format!(
                "'{name}'은 이미 다른 계정({owner})의 프로필입니다 — 다른 이름을 쓰세요"
            ));
        }
    }
    Ok(())
}

pub(crate) fn read_meta(dir: &Path) -> Option<Meta> {
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

pub(crate) fn find_profile_by_id(
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
/// id·이메일은 외부 입력(JWT 클레임 등)이므로 허용 문자만 남긴다 — 결과는 항상 validate_name을 통과한다.
/// 다른 계정이 쓰는 이름과 절대 겹치지 않을 때까지 숫자를 올린다 (한 단계 접미사로는
/// 제3 계정이 기존 프로필을 덮어쓰는 사고가 났었다 — red-review 2라운드).
pub(crate) fn auto_name(env: &Env, provider: Provider, ident: &LiveIdentity) -> String {
    let clean = |s: &str, limit: usize| -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(limit)
            .collect::<String>()
            .to_lowercase()
    };
    let base = {
        let from_email = clean(
            ident.email.as_deref().and_then(|e| e.split('@').next()).unwrap_or(""),
            20,
        );
        if from_email.is_empty() {
            let id_part = clean(&ident.id, 8);
            if id_part.is_empty() {
                "account".to_string()
            } else {
                format!("account-{id_part}")
            }
        } else {
            from_email
        }
    };
    let mut candidate = base.clone();
    let mut n = 1;
    loop {
        match read_meta(&env.profiles_dir(provider).join(&candidate)) {
            None => return candidate,                          // 빈 이름
            Some(meta) if meta.id == ident.id => return candidate, // 이미 이 계정의 프로필
            Some(_) => {
                n += 1;
                if n > 99 {
                    return format!("account-{}", now());
                }
                candidate = format!("{base}-{n}");
            }
        }
    }
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
    let mut root = read_json_retry(&cj)?;
    let Some(obj) = root.as_object_mut() else {
        return Ok(());
    };
    obj.insert("oauthAccount".to_string(), block);
    let bytes = serde_json::to_vec_pretty(&root).map_err(|e| e.to_string())?;
    atomic_write(&cj, &bytes)
}

/// 현재 로그인 계정을 이름 붙여 프로필로 저장
pub fn save_current(env: &Env, provider: Provider, name: &str) -> Result<(), String> {
    // 변이 함수가 스스로 잠근다 — 호출자가 잠금을 잊을 수 없게 (관례 단일화)
    let _guard = MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
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
    // 다른 계정이 쓰는 이름을 덮어써 그 계정 토큰을 파괴하는 것을 막는다
    ensure_name_not_owned_by_other(env, provider, name, &ident)?;
    write_profile(env, provider, name, &ident)
}

/// 계정 전환. 순서 불변: 1) 현재 활성 파일 백업 → 2) 대상 프로필 복사
pub fn switch(env: &Env, provider: Provider, name: &str) -> Result<SwitchResult, String> {
    let _guard = MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    validate_name(name)?;
    let profile_dir = env.profiles_dir(provider).join(name);
    let target_cred = profile_dir.join(provider.credential_file_name());
    if !target_cred.exists() {
        return Err(format!("프로필 '{name}'에 저장된 토큰이 없습니다"));
    }
    // 계정 정보 없이 토큰만 있는 프로필(구조용 백업)로 전환하면 이후 신원 판정이
    // 어긋나 다음 전환의 백업이 엉뚱한 프로필을 덮어쓸 수 있다 — 전환 대상에서 제외
    if provider == Provider::Claude && !profile_dir.join("oauth_account.json").exists() {
        return Err(format!(
            "프로필 '{name}'에는 계정 정보가 없어 전환할 수 없습니다 (구조용 백업) — 해당 계정으로 CLI 로그인 후 다시 저장하세요"
        ));
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
    // 표시용 목록은 신원 읽기가 일시적으로 실패해도 화면 전체를 깨뜨리지 않는다
    // (전환·저장 경로는 여전히 엄격하게 실패한다)
    let live = live_identity(env, provider).unwrap_or(None);
    let live_id = live.as_ref().map(|l| l.id.clone());
    let mut profiles = Vec::new();
    for (name, dir) in profile_dirs(env, provider)? {
        if let Some(meta) = read_meta(&dir) {
            let cred = read_json(&dir.join(provider.credential_file_name())).ok();
            let plan = cred
                .as_ref()
                .and_then(|root| plan_from_credential(provider, root));
            let plan_tier = cred
                .as_ref()
                .and_then(|root| tier_from_credential(provider, root));
            profiles.push(ProfileInfo {
                active: live_id.as_deref() == Some(meta.id.as_str()),
                name,
                id: meta.id,
                email: meta.email,
                plan,
                plan_tier,
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
    let _guard = MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    validate_name(name)?;
    let dir = env.profiles_dir(provider).join(name);
    if !dir.exists() {
        return Err(format!("프로필 '{name}'이 없습니다"));
    }
    fs::remove_dir_all(&dir).map_err(|e| format!("삭제 실패 {}: {e}", dir.display()))
}

/// 형제 모듈(usage·login) 테스트와 공유하는 픽스처 헬퍼.
/// 실토큰이 아닌 픽스처로만 검증한다 (CLAUDE.md 금기).
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub(crate) fn test_env(tag: &str) -> Env {
        let base = std::env::temp_dir().join(format!("switcher-test-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join(".claude")).unwrap();
        Env {
            home: base.clone(),
            store: base.join(".switcher"),
        }
    }

    /// 가짜 JWT (서명 없음) — claims JSON을 그대로 payload로 쓴다
    pub(crate) fn fake_jwt(claims_json: &str) -> String {
        use base64::Engine;
        let enc = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        format!(
            "{}.{}.{}",
            enc(r#"{"alg":"none"}"#),
            enc(claims_json),
            enc("sig")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{fake_jwt, test_env};
    use super::*;

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

        // 백업이 기존 사본을 덮어쓸 때 한 세대 .bak이 남아야 한다
        assert!(env
            .profiles_dir(Provider::Claude)
            .join("main")
            .join("credentials.json.bak")
            .exists());

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
    fn save_rejects_name_owned_by_other_account() {
        let env = test_env("foreign-name");
        login_claude(&env, "uuid-b", "bob@test.dev", "tok-b1");
        save_current(&env, Provider::Claude, "main").unwrap();
        // 다른 계정 A로 로그인한 뒤 같은 이름 "main"으로 저장 시도 → 거부돼야 한다
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        let err = save_current(&env, Provider::Claude, "main").unwrap_err();
        assert!(err.contains("다른 계정"));
        // B의 토큰이 파괴되지 않고 그대로 보존
        let kept = fs::read_to_string(
            env.profiles_dir(Provider::Claude)
                .join("main")
                .join("credentials.json"),
        )
        .unwrap();
        assert!(kept.contains("tok-b1"));
    }

    #[test]
    fn switch_refuses_claude_profile_without_account_info() {
        let env = test_env("no-oauth-block");
        login_claude(&env, "uuid-a", "alice@test.dev", "tok-a1");
        save_current(&env, Provider::Claude, "main").unwrap();
        // 구조용 백업처럼 계정 정보(oauth_account.json) 없는 프로필
        let dir = env.profiles_dir(Provider::Claude).join("rescue1");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"tok-x"}}"#,
        )
        .unwrap();
        fs::write(
            dir.join("meta.json"),
            r#"{"id":"unknown-1","email":null,"saved_at":0}"#,
        )
        .unwrap();
        let err = switch(&env, Provider::Claude, "rescue1").unwrap_err();
        assert!(err.contains("계정 정보"));
        assert!(live_token(&env).contains("tok-a1"), "활성 토큰은 불변");
    }

    #[test]
    fn plan_is_extracted_from_credentials() {
        let claude: Value = serde_json::from_str(
            r#"{"claudeAiOauth":{"accessToken":"t","subscriptionType":"max","rateLimitTier":"default_claude_max_20x"}}"#,
        )
        .unwrap();
        assert_eq!(
            plan_from_credential(Provider::Claude, &claude).as_deref(),
            Some("Max")
        );
        assert_eq!(tier_from_credential(Provider::Claude, &claude), Some(20));
        let five: Value = serde_json::from_str(
            r#"{"claudeAiOauth":{"rateLimitTier":"default_claude_max_5x"}}"#,
        )
        .unwrap();
        assert_eq!(tier_from_credential(Provider::Claude, &five), Some(5));
        let jwt = fake_jwt(
            r#"{"email":"x@test.dev","https://api.openai.com/auth":{"chatgpt_plan_type":"pro"}}"#,
        );
        let codex: Value = serde_json::from_str(&format!(
            r#"{{"tokens":{{"id_token":"{jwt}","account_id":"a"}}}}"#
        ))
        .unwrap();
        assert_eq!(
            plan_from_credential(Provider::Codex, &codex).as_deref(),
            Some("Pro")
        );
    }

    #[test]
    fn auto_name_sanitizes_untrusted_id() {
        let env = test_env("autoname");
        let ident = LiveIdentity {
            id: "auth0|../..evil".to_string(),
            email: None,
        };
        let name = auto_name(&env, Provider::Codex, &ident);
        assert!(validate_name(&name).is_ok(), "생성된 이름: {name}");
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

    fn login_codex(env: &Env, account_id: &str, email: &str, token: &str) {
        fs::create_dir_all(env.home.join(".codex")).unwrap();
        fs::write(
            env.home.join(".codex").join("auth.json"),
            format!(
                r#"{{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{{"id_token":"{}","access_token":"{token}","refresh_token":"r-{token}","account_id":"{account_id}"}},"last_refresh":"2026-01-01T00:00:00Z"}}"#,
                fake_jwt(&format!(r#"{{"email":"{email}","sub":"sub-x"}}"#))
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

    /// 실계정 파일 검증: 현재 계정을 프로필로 저장 → 자기 자신으로 전환(무해한 왕복).
    /// 계정이 바뀌지 않아야 하고 토큰 파일 내용도 보존되어야 한다.
    /// CI에서는 돌지 않는다: `cargo test -- --ignored` 로만 실행.
    fn real_self_switch(provider: Provider) {
        let env = Env::real().unwrap();
        let live_path = env.live_credential_path(provider);
        if !live_path.exists() {
            panic!("로그인 파일이 없어 실환경 검증 불가");
        }
        let before_cred = fs::read(&live_path).unwrap();
        let before_ident = live_identity(&env, provider).unwrap().unwrap();

        let name = match find_profile_by_id(&env, provider, &before_ident.id).unwrap() {
            Some(existing) => existing,
            None => {
                // 제품과 같은 규칙으로 이름을 정한다 (하드코딩 이름은 다른 계정 소유일 수 있어
                // ensure_name_not_owned_by_other에 막힌다 — 실제로 막힌 사례 있음)
                let name = auto_name(&env, provider, &before_ident);
                save_current(&env, provider, &name).unwrap();
                name
            }
        };

        let result = switch(&env, provider, &name).unwrap();
        assert_eq!(result.switched_to, name);
        assert_eq!(result.backed_up_to.as_deref(), Some(name.as_str()));

        let after_ident = live_identity(&env, provider).unwrap().unwrap();
        assert_eq!(before_ident.id, after_ident.id, "계정이 바뀌면 안 된다");
        let after_cred = fs::read(&live_path).unwrap();
        assert_eq!(before_cred, after_cred, "토큰 파일이 보존되어야 한다");

        let snap = list(&env, provider).unwrap();
        assert!(snap.live_saved, "전환 후 활성 계정이 프로필과 매칭되어야 한다");
    }

    #[test]
    #[ignore]
    fn real_self_switch_claude() {
        real_self_switch(Provider::Claude);
    }

    #[test]
    #[ignore]
    fn real_self_switch_codex() {
        real_self_switch(Provider::Codex);
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

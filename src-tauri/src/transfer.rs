//! 다른 PC로 옮기기 (#189, 사용자 지시 2026-11-23).
//!
//! A PC의 구독 계정 로그인과 API 키를 파일 하나에 담아 B PC에 넣는다. "옮기는 게 귀찮아서"
//! 쓰는 기능이다 — 받는 쪽은 **전부 덮어쓴다**(사용자 결정):
//!
//! - 같은 계정(계정 ID)이 있으면 그 프로필의 로그인을 파일 것으로 바꾼다. 그 계정이 지금 CLI에
//!   로그인된 계정이면 그 로그인도 함께 바꾼다 — 재로그인(login.rs `import_inner`)과 같은 순서다.
//!   그래야 받는 PC의 만료된 로그인이 살아난다.
//! - 같은 계정의 같은 이름 API 키는 덮어쓴다. 지금 사용 중이어도 덮어쓰고 새 키로 계속 사용한다.
//! - 파일에 없는 계정·키는 그대로 둔다. 지우는 것은 없다.
//!
//! **옮기지 않는 것**: 설정·화면 설정·메모(사용자 결정 — 두 PC를 다른 용도로 쓰면 받는 쪽 것이
//! 날아간다. 처음 판에 내가 묻지 않고 넣었다가 뺐다. 다시 넣지 말 것), 잔액 기록, 사용량 캐시,
//! 런처, `.bak`·손상 사본.
//!
//! 파일에는 로그인 토큰과 API 키 원문이 들어가므로 통째로 잠근다:
//! - 열쇠는 사용자의 비밀번호에서 만든다 (Argon2id — 일부러 느리고 메모리를 많이 써서
//!   비밀번호를 하나씩 대입해 보는 공격을 비싸게 한다). 소금은 파일마다 새로 뽑는다.
//! - 묶음은 XChaCha20-Poly1305로 잠근다. 머리말(형식·소금·열쇠 만들기 값)까지 인증에 넣어
//!   한 바이트라도 바뀌면 풀리지 않는다 — 틀린 비밀번호와 변조는 같은 오류가 된다.
//! - 받는 쪽은 다 풀고 검사한 뒤에야 쓴다. 이름은 경로가 되므로 저장할 때와 같은 규칙으로 거른다.
//!
//! 알아 둘 것: 구독 로그인은 갱신될 때마다 이전 것이 무효가 된다(usage.rs의 회전 주석). 같은 로그인을
//! 두 PC가 나눠 쓰면 먼저 갱신한 쪽만 남고 다른 쪽은 다시 로그인해야 한다 — 사용자는 알고 골랐다.
//! 금기: 비밀번호·열쇠·묶음 내용을 로그나 오류 문구에 넣지 말 것.

use crate::accounts::{self, atomic_write, validate_name, Env, LiveIdentity, Provider};
use crate::api_connections::{self, KeyImport, TransferKey};
use base64::Engine;
use chacha20poly1305::{aead::Aead, aead::Payload, KeyInit, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use zeroize::Zeroize;

const MAGIC: &[u8; 8] = b"SWXFER1\n";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
/// 머리말: 형식 표시 + 열쇠 만들기 값(메모리 KiB·반복·병렬, 각 u32 LE) + 소금 + 논스
const HEADER_LEN: usize = MAGIC.len() + 12 + SALT_LEN + NONCE_LEN;
/// 열쇠 만들기 기본값 — 64MiB·3회. 2026년의 보통 PC에서 0.2~0.5초.
const KDF_MEMORY_KIB: u32 = 64 * 1024;
const KDF_ITERATIONS: u32 = 3;
const KDF_LANES: u32 = 1;
/// 받는 쪽이 허용하는 상한 — 조작된 파일이 메모리를 끝없이 쓰게 하지 못한다
const KDF_MEMORY_MAX_KIB: u32 = 1024 * 1024;
const KDF_ITERATIONS_MAX: u32 = 16;
const KDF_LANES_MAX: u32 = 8;
pub const PASSWORD_MIN: usize = 8;
const FILE_MAX: u64 = 64 * 1024 * 1024;
const PROFILE_FILE_MAX: usize = 1024 * 1024;
pub const FILE_EXT: &str = "swx";
/// 묶음 형식. 1은 처음 판(설정·메모까지 담고 키 항목 모양이 달랐다) — 받지 않는다.
const BUNDLE_VERSION: u64 = 2;
const BAD_FILE: &str = "옮기기 파일이 아닙니다";
const BAD_PASSWORD: &str = "비밀번호가 다르거나 파일이 손상됐습니다";

/// 프로필 폴더에서 옮기는 파일 — 이 목록에 없는 이름은 내보내지도, 받아 쓰지도 않는다
fn profile_files(provider: Provider) -> &'static [&'static str] {
    match provider {
        Provider::Claude => &["credentials.json", "oauth_account.json", "meta.json", "plan.json"],
        Provider::Codex => &["auth.json", "meta.json"],
    }
}

// Debug를 달지 말 것 — 토큰과 키 원문이 들어 있다
#[derive(Serialize, Deserialize)]
struct Bundle {
    version: u64,
    created_at: u64,
    source_os: String,
    profiles: Vec<ProfileEntry>,
    api_keys: Vec<TransferKey>,
}

#[derive(Serialize, Deserialize)]
struct ProfileEntry {
    provider: String,
    name: String,
    /// 파일 이름 → 내용(base64)
    files: BTreeMap<String, String>,
}

#[derive(Serialize, Debug, Default, PartialEq, Eq)]
pub struct ExportResult {
    pub path: String,
    pub accounts: usize,
    pub api_keys: usize,
}

#[derive(Serialize, Debug, Default, PartialEq, Eq)]
pub struct ImportResult {
    pub accounts_added: usize,
    pub accounts_replaced: usize,
    /// 덮어쓴 계정 가운데 지금 CLI에 로그인된 계정이라 그 로그인까지 바꾼 수
    pub live_replaced: usize,
    pub api_keys_added: usize,
    pub api_keys_replaced: usize,
    pub api_keys_unchanged: usize,
    /// 형식이 맞지 않아 넣지 않은 항목 수
    pub rejected: usize,
    /// 넣지 못했거나 마무리하지 못한 것 — 사용자에게 그대로 보여 준다 (토큰·키는 들어가지 않는다)
    pub problems: Vec<String>,
    pub source_os: String,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub path: String,
    pub name: String,
    pub modified: u64,
}

// ── 잠그기·풀기 ────────────────────────────────────────────────────

fn derive_key(password: &[u8], salt: &[u8], memory_kib: u32, iterations: u32, lanes: u32) -> Result<[u8; 32], String> {
    let params = argon2::Params::new(memory_kib, iterations, lanes, Some(32)).map_err(|_| BAD_FILE)?;
    let kdf = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = [0u8; 32];
    kdf.hash_password_into(password, salt, &mut key)
        .map_err(|_| "열쇠를 만들지 못했습니다 (메모리 부족일 수 있습니다)")?;
    Ok(key)
}

fn seal_with(password: &str, plain: &[u8], memory_kib: u32, iterations: u32) -> Result<Vec<u8>, String> {
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    for value in [memory_kib, iterations, KDF_LANES] {
        header.extend_from_slice(&value.to_le_bytes());
    }
    let mut random = [0u8; SALT_LEN + NONCE_LEN];
    getrandom::getrandom(&mut random).map_err(|_| "난수를 만들지 못했습니다")?;
    header.extend_from_slice(&random);
    let (salt, nonce) = random.split_at(SALT_LEN);
    let mut key = derive_key(password.as_bytes(), salt, memory_kib, iterations, KDF_LANES)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    key.zeroize();
    let sealed = cipher
        .encrypt(XNonce::from_slice(nonce), Payload { msg: plain, aad: &header })
        .map_err(|_| "잠그지 못했습니다")?;
    header.extend_from_slice(&sealed);
    Ok(header)
}

fn seal(password: &str, plain: &[u8]) -> Result<Vec<u8>, String> {
    seal_with(password, plain, KDF_MEMORY_KIB, KDF_ITERATIONS)
}

fn is_transfer_file(bytes: &[u8]) -> bool {
    bytes.len() > HEADER_LEN && bytes.starts_with(MAGIC)
}

fn open(password: &str, file: &[u8]) -> Result<Vec<u8>, String> {
    if !is_transfer_file(file) {
        return Err(BAD_FILE.into());
    }
    let (header, sealed) = file.split_at(HEADER_LEN);
    let number = |at: usize| u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]]);
    let (memory_kib, iterations, lanes) = (number(8), number(12), number(16));
    if memory_kib > KDF_MEMORY_MAX_KIB || iterations > KDF_ITERATIONS_MAX || lanes > KDF_LANES_MAX {
        return Err(BAD_FILE.into());
    }
    let salt = &header[20..20 + SALT_LEN];
    let nonce = &header[20 + SALT_LEN..];
    let mut key = derive_key(password.as_bytes(), salt, memory_kib, iterations, lanes)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    key.zeroize();
    cipher
        .decrypt(XNonce::from_slice(nonce), Payload { msg: sealed, aad: header })
        .map_err(|_| BAD_PASSWORD.into())
}

// ── 모으기 ─────────────────────────────────────────────────────────

/// 지금 CLI에 로그인된 구독 계정의 로그인 — (계정 ID, 자격증명, Claude의 계정 표시 블록)
fn live_login(env: &Env, provider: Provider) -> Option<(String, Vec<u8>, Option<Value>)> {
    let ident = accounts::live_identity(env, provider).ok()??;
    let cred = accounts::read_live_cred(env, provider).ok()?;
    // 구독 로그인이 맞는지 본다 — Codex가 API 키로 돌고 있으면 auth.json은 로그인 정보가 아니다
    let root: Value = serde_json::from_slice(&cred).ok()?;
    let is_login = match provider {
        Provider::Claude => root.get("claudeAiOauth").is_some(),
        Provider::Codex => root.get("tokens").is_some(),
    };
    if !is_login {
        return None;
    }
    let block = match provider {
        Provider::Claude => accounts::claude_oauth_block(env).ok().flatten(),
        Provider::Codex => None,
    };
    Some((ident.id, cred, block))
}

fn collect(env: &Env, api_keys: Vec<TransferKey>) -> Result<Bundle, String> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut profiles = Vec::new();
    for provider in [Provider::Claude, Provider::Codex] {
        // 지금 로그인된 계정은 활성 저장소의 것이 가장 새 로그인이다. 프로필 사본은 마지막 전환 때의
        // 것이라, 그 뒤 CLI가 로그인을 갱신했으면 이미 못 쓰는 값이다 — 그대로 담으면 죽은 로그인이 간다.
        let live = live_login(env, provider);
        for (name, dir) in accounts::profile_dirs(env, provider)? {
            // 이름 변경 잔해 같은 폴더는 계정이 아니다
            let Some(meta) = accounts::read_meta(&dir).filter(|_| validate_name(&name).is_ok()) else {
                continue;
            };
            let mut files = BTreeMap::new();
            for file in profile_files(provider) {
                match fs::read(dir.join(file)) {
                    Ok(bytes) => {
                        files.insert((*file).to_string(), b64.encode(bytes));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err(format!("계정 '{name}'의 파일을 읽지 못했습니다")),
                }
            }
            if let Some((_, cred, block)) = live.as_ref().filter(|(id, ..)| *id == meta.id) {
                files.insert(provider.credential_file_name().to_string(), b64.encode(cred));
                if let Some(bytes) = block.as_ref().and_then(|block| serde_json::to_vec_pretty(block).ok()) {
                    files.insert("oauth_account.json".to_string(), b64.encode(bytes));
                }
            }
            if files.contains_key(provider.credential_file_name()) {
                profiles.push(ProfileEntry { provider: provider.dir_name().into(), name, files });
            }
        }
    }
    Ok(Bundle {
        version: BUNDLE_VERSION,
        created_at: accounts::now(),
        source_os: std::env::consts::OS.into(),
        profiles,
        api_keys,
    })
}

fn export_dir(env: &Env) -> PathBuf {
    let downloads = env.home.join("Downloads");
    if downloads.is_dir() {
        downloads
    } else {
        env.home.clone()
    }
}

fn check_password(password: &str) -> Result<(), String> {
    if password.chars().count() < PASSWORD_MIN {
        return Err(format!("비밀번호는 {PASSWORD_MIN}자 이상으로 정하세요"));
    }
    Ok(())
}

fn write_export(env: &Env, password: &str, bundle: &Bundle, stamp: &str) -> Result<ExportResult, String> {
    let mut plain = serde_json::to_vec(bundle).map_err(|_| "묶음을 만들지 못했습니다")?;
    let sealed = seal(password, &plain);
    plain.zeroize();
    let path = export_dir(env).join(format!("switcher-transfer-{stamp}.{FILE_EXT}"));
    atomic_write(&path, &sealed?)?;
    Ok(ExportResult {
        path: path.to_string_lossy().into_owned(),
        accounts: bundle.profiles.len(),
        api_keys: bundle.api_keys.len(),
    })
}

pub fn export(env: &Env, password: &str) -> Result<ExportResult, String> {
    check_password(password)?;
    let _mutation = accounts::MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let bundle = collect(env, api_connections::export_keys(env)?)?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    write_export(env, password, &bundle, &stamp)
}

// ── 되살리기 ───────────────────────────────────────────────────────

fn parse(password: &str, file: &[u8]) -> Result<Bundle, String> {
    let mut plain = open(password, file)?;
    let value: Result<Value, _> = serde_json::from_slice(&plain);
    plain.zeroize();
    let value = value.map_err(|_| BAD_FILE)?;
    match value.get("version").and_then(Value::as_u64) {
        Some(BUNDLE_VERSION) => {}
        Some(version) if version < BUNDLE_VERSION => {
            return Err("이 파일은 이전 판의 위젯에서 만든 것입니다 — 보내는 PC의 위젯을 업데이트하고 다시 내보내세요".into());
        }
        Some(_) => return Err("이 파일은 더 새 버전의 위젯에서 만든 것입니다 — 이 PC의 위젯을 업데이트하세요".into()),
        None => return Err(BAD_FILE.into()),
    }
    serde_json::from_value(value).map_err(|_| BAD_FILE.into())
}

/// 받아 쓸 수 있는 모양으로 푼 프로필
struct IncomingProfile {
    provider: Provider,
    ident: LiveIdentity,
    credential: Vec<u8>,
    oauth_block: Option<Value>,
    plan: Option<Vec<u8>>,
}

fn checked_profile(entry: &ProfileEntry) -> Option<IncomingProfile> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let provider = Provider::parse(&entry.provider).ok()?;
    validate_name(&entry.name).ok()?;
    // 허용 목록에 있는 이름만 꺼낸다 — 묶음에 든 다른 이름은 어디에도 쓰이지 않는다
    let file = |name: &str| -> Option<Option<Vec<u8>>> {
        if !profile_files(provider).contains(&name) {
            return Some(None);
        }
        match entry.files.get(name) {
            None => Some(None),
            Some(encoded) => {
                let bytes = b64.decode(encoded).ok()?;
                (bytes.len() <= PROFILE_FILE_MAX).then_some(Some(bytes))
            }
        }
    };
    let meta: accounts::Meta = serde_json::from_slice(&file("meta.json")??).ok()?;
    if meta.id.is_empty() || meta.id.len() > 128 || meta.id.chars().any(char::is_control) {
        return None;
    }
    let credential = file(provider.credential_file_name())??;
    // 자격증명은 그 공급자의 로그인 JSON이어야 한다 — 다른 것을 프로필·활성 자리에 쓰지 않는다
    let root: Value = serde_json::from_slice(&credential).ok()?;
    let is_login = match provider {
        Provider::Claude => root.get("claudeAiOauth").is_some_and(Value::is_object),
        Provider::Codex => root.get("tokens").is_some_and(Value::is_object),
    };
    if !is_login {
        return None;
    }
    let oauth_block = match file("oauth_account.json")? {
        Some(bytes) => Some(serde_json::from_slice::<Value>(&bytes).ok().filter(Value::is_object)?),
        None => None,
    };
    let plan = match file("plan.json")? {
        Some(bytes) => Some(bytes).filter(|bytes| serde_json::from_slice::<Value>(bytes).is_ok()),
        None => None,
    };
    Some(IncomingProfile {
        provider,
        ident: LiveIdentity { id: meta.id, email: meta.email },
        credential,
        oauth_block,
        plan,
    })
}

/// 계정 하나를 넣는다 — (이미 있던 계정인가, 지금 로그인된 계정이라 그 로그인도 바꿨는가).
/// MUTATION_LOCK 안에서 부를 것.
fn import_profile(env: &Env, incoming: &IncomingProfile, wanted: &str) -> Result<(bool, bool), String> {
    let provider = incoming.provider;
    // 계정은 ID로 찾는다 — 받는 PC에서 다른 이름으로 저장돼 있어도 그 프로필을 덮어쓴다
    let existing = accounts::find_profile_by_id(env, provider, &incoming.ident.id)?;
    let replaced = existing.is_some();
    let name = match existing {
        Some(name) => name,
        None if !env.profiles_dir(provider).join(wanted).exists() => wanted.to_string(),
        // 같은 이름을 다른 계정이 쓰고 있다 — 그 계정을 덮지 않고 새 이름으로 넣는다
        None => accounts::auto_name(env, provider, &incoming.ident),
    };
    accounts::ensure_name_not_owned_by_other(env, provider, &name, &incoming.ident)?;
    let dir = env.profiles_dir(provider).join(&name);
    // 재로그인과 같은 순서(login.rs import_inner): 지금 로그인된 계정이면 활성 로그인을 먼저 바꾼다.
    // 프로필에만 쓰면 다음 전환의 "활성 → 프로필 백업"이 옛 로그인으로 새 로그인을 덮는다.
    let updates_live = accounts::live_identity(env, provider)
        .ok()
        .flatten()
        .is_some_and(|live| live.id == incoming.ident.id);
    if updates_live {
        accounts::write_live_cred(env, provider, &incoming.credential)?;
    }
    accounts::write_profile_parts(
        env,
        provider,
        &name,
        &incoming.ident,
        &incoming.credential,
        incoming.oauth_block.as_ref(),
    )?;
    if let Some(plan) = &incoming.plan {
        // 표시용 플랜 정보 — 못 써도 로그인은 들어왔다 (다음 사용량 조회가 다시 채운다)
        let _ = atomic_write(&dir.join("plan.json"), plan);
    }
    if updates_live && provider == Provider::Claude {
        accounts::claude_apply_oauth_block(env, &dir)?;
    }
    // 새 로그인을 받은 계정은 이전 "다시 로그인 필요" 판정을 물려받지 않는다 — 재로그인과 같다 (#122)
    crate::usage::clear_profile_backoff(env, provider, &name);
    Ok((replaced, updates_live))
}

fn apply(
    env: &Env,
    bundle: Bundle,
    import_keys: impl FnOnce(&Env, &[TransferKey]) -> Result<KeyImport, String>,
) -> Result<ImportResult, String> {
    let _mutation = accounts::MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let mut result = ImportResult { source_os: bundle.source_os.clone(), ..Default::default() };
    for entry in &bundle.profiles {
        let Some(incoming) = checked_profile(entry) else {
            result.rejected += 1;
            continue;
        };
        // 한 계정이 실패해도 나머지는 넣는다 — 무엇이 안 들어갔는지는 결과에 적는다
        match import_profile(env, &incoming, &entry.name) {
            Ok((replaced, live)) => {
                if replaced {
                    result.accounts_replaced += 1;
                } else {
                    result.accounts_added += 1;
                }
                if live {
                    result.live_replaced += 1;
                }
            }
            Err(error) => result.problems.push(format!("계정 '{}': {error}", entry.name)),
        }
    }
    match import_keys(env, &bundle.api_keys) {
        Ok(keys) => {
            result.api_keys_added = keys.added;
            result.api_keys_replaced = keys.replaced;
            result.api_keys_unchanged = keys.unchanged;
            result.rejected += keys.rejected;
            for name in keys.reapply_failed {
                result
                    .problems
                    .push(format!("API 키 '{name}': 덮어썼지만 사용 중 상태를 새 키로 맞추지 못했습니다 — 사용을 다시 눌러 주세요"));
            }
        }
        Err(error) => result.problems.push(format!("API 키: {error}")),
    }
    Ok(result)
}

fn read_transfer_file(path: &Path) -> Result<Vec<u8>, String> {
    let meta = fs::metadata(path).map_err(|_| "파일을 찾을 수 없습니다")?;
    if !meta.is_file() || meta.len() > FILE_MAX {
        return Err(BAD_FILE.into());
    }
    fs::read(path).map_err(|_| "파일을 읽지 못했습니다".into())
}

/// `path`의 파일이나 화면이 읽어 준 내용(`bytes`)을 푼다 — 다 풀고 검사한 뒤에야 쓴다
pub fn import(env: &Env, password: &str, path: Option<&str>, bytes: Option<Vec<u8>>) -> Result<ImportResult, String> {
    let file = match (bytes, path) {
        (Some(bytes), _) => bytes,
        (None, Some(path)) => read_transfer_file(Path::new(path))?,
        (None, None) => return Err("파일을 고르세요".into()),
    };
    let bundle = parse(password, &file)?;
    apply(env, bundle, api_connections::import_keys)
}

/// 흔히 받아 두는 곳(다운로드·바탕화면·홈)에서 옮기기 파일을 찾는다 — 최근 것부터
pub fn candidates(env: &Env) -> Vec<Candidate> {
    let mut found = Vec::new();
    for dir in [env.home.join("Downloads"), env.home.join("Desktop"), env.home.clone()] {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some(FILE_EXT) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() || meta.len() > FILE_MAX || meta.len() <= HEADER_LEN as u64 {
                continue;
            }
            let mut head = [0u8; 8];
            let is_ours = fs::File::open(&path)
                .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut head))
                .is_ok()
                && &head == MAGIC;
            if !is_ours {
                continue;
            }
            let modified = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|age| age.as_secs())
                .unwrap_or(0);
            found.push(Candidate {
                path: path.to_string_lossy().into_owned(),
                name: entry.file_name().to_string_lossy().into_owned(),
                modified,
            });
        }
    }
    found.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.path.cmp(&b.path)));
    found.truncate(10);
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::test_support::test_env;
    use crate::api_connections::tests::{add_account, save_fixture, Memory};

    // 검사는 가벼운 열쇠 만들기 값으로 돈다 — 기본값(64MiB)은 한 번만 왕복해 본다
    fn seal_fast(password: &str, plain: &[u8]) -> Vec<u8> {
        seal_with(password, plain, 64, 1).unwrap()
    }

    fn claude_cred(tag: &str) -> Vec<u8> {
        format!(r#"{{"claudeAiOauth":{{"accessToken":"fake-access-{tag}","refreshToken":"fake-refresh-{tag}","expiresAt":9999999999999}}}}"#).into_bytes()
    }
    fn codex_cred(tag: &str, account: &str) -> Vec<u8> {
        format!(r#"{{"auth_mode":"chatgpt","tokens":{{"access_token":"fake-access-{tag}","refresh_token":"fake-refresh-{tag}","account_id":"{account}"}}}}"#).into_bytes()
    }
    /// 저장된 계정 하나 (프로필 사본만 — CLI에 로그인된 상태는 아니다)
    fn account(env: &Env, provider: Provider, name: &str, id: &str, tag: &str) {
        add_account(env, provider, name, id);
        let cred = match provider {
            Provider::Claude => claude_cred(tag),
            Provider::Codex => codex_cred(tag, id),
        };
        fs::write(env.profiles_dir(provider).join(name).join(provider.credential_file_name()), cred).unwrap();
    }
    /// 그 계정으로 CLI에 로그인된 상태를 만든다
    fn log_in(env: &Env, provider: Provider, id: &str, tag: &str) {
        match provider {
            Provider::Claude => {
                accounts::write_live_cred(env, provider, &claude_cred(tag)).unwrap();
                atomic_write(
                    &env.home.join(".claude.json"),
                    format!(r#"{{"oauthAccount":{{"accountUuid":"{id}","emailAddress":"{id}@example.test"}},"theme":"dark"}}"#).as_bytes(),
                )
                .unwrap();
            }
            Provider::Codex => accounts::write_live_cred(env, provider, &codex_cred(tag, id)).unwrap(),
        }
    }
    fn profile_cred(env: &Env, provider: Provider, name: &str) -> String {
        fs::read_to_string(env.profiles_dir(provider).join(name).join(provider.credential_file_name())).unwrap()
    }
    fn live_cred(env: &Env, provider: Provider) -> String {
        String::from_utf8(accounts::read_live_cred(env, provider).unwrap()).unwrap()
    }

    /// 계정 둘과 키 하나가 있는 보내는 PC. 설정·메모도 있지만 옮기는 대상이 아니다.
    fn source(tag: &str) -> (Env, Memory) {
        let env = test_env(tag);
        let secrets = Memory::default();
        account(&env, Provider::Claude, "alpha", "acct-alpha", "a-alpha");
        account(&env, Provider::Codex, "delta", "acct-delta", "a-delta");
        let alpha = env.profiles_dir(Provider::Claude).join("alpha");
        fs::write(alpha.join("credentials.json.bak"), b"old-generation").unwrap();
        fs::write(alpha.join("plan.json"), br#"{"tier":20}"#).unwrap();
        save_fixture(&env, &secrets, "claude", "acct-alpha", "work", "sk-fake-transfer-key-000001");
        fs::write(env.store.join("settings.json"), br#"{"accent_theme":"violet","tfsd":true}"#).unwrap();
        fs::write(env.store.join("memo.json"), br#"{"active":0,"tabs":["memo-from-a"]}"#).unwrap();
        (env, secrets)
    }

    fn bundle_of(env: &Env, secrets: &Memory) -> Bundle {
        collect(env, api_connections::export_keys_with(env, secrets).unwrap()).unwrap()
    }
    fn bring(to: &Env, to_secrets: &Memory, bundle: Bundle) -> ImportResult {
        apply(to, bundle, |env, keys| api_connections::import_keys_with(env, to_secrets, keys)).unwrap()
    }

    #[test]
    fn the_default_lock_round_trips_and_a_wrong_password_opens_nothing() {
        let sealed = seal("correct horse", b"secret payload").unwrap();
        assert!(is_transfer_file(&sealed));
        assert_eq!(open("correct horse", &sealed).unwrap(), b"secret payload");
        assert_eq!(open("correct horsf", &sealed).unwrap_err(), BAD_PASSWORD);
    }

    #[test]
    fn the_file_shows_nothing_and_any_changed_byte_refuses_to_open() {
        let plain = br#"{"key":"sk-fake-transfer-key-000001"}"#;
        let sealed = seal_fast("password-1", plain);
        // 같은 내용을 두 번 잠가도 같은 파일이 나오지 않는다 (소금·논스가 매번 새로 뽑힌다)
        assert_ne!(sealed, seal_fast("password-1", plain));
        assert!(!sealed.windows(7).any(|window| window == b"sk-fake"));
        for at in [8, 12, 20, 30, HEADER_LEN - 1, HEADER_LEN, sealed.len() - 1] {
            let mut changed = sealed.clone();
            changed[at] ^= 1;
            assert!(open("password-1", &changed).is_err(), "byte {at} must be authenticated");
        }
        // 잘린 파일, 다른 파일, 터무니없는 열쇠 만들기 값
        assert_eq!(open("password-1", &sealed[..HEADER_LEN]).unwrap_err(), BAD_FILE);
        assert_eq!(open("password-1", b"not a transfer file at all, just some bytes that are long enough to pass").unwrap_err(), BAD_FILE);
        let mut greedy = sealed.clone();
        greedy[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(open("password-1", &greedy).unwrap_err(), BAD_FILE);
    }

    #[test]
    fn logins_and_keys_arrive_on_an_empty_pc_and_nothing_else_does() {
        let (from, from_secrets) = source("transfer-from");
        let bundle = bundle_of(&from, &from_secrets);
        // 묶음에는 로그인과 키만 있다 — 설정·메모는 들어가지 않는다
        let plain = String::from_utf8(serde_json::to_vec(&bundle).unwrap()).unwrap();
        assert!(!plain.contains("memo-from-a") && !plain.contains("violet") && !plain.contains("old-generation"));
        let file = seal_fast("password-1", plain.as_bytes());
        let to = test_env("transfer-to");
        let to_secrets = Memory::default();
        let result = bring(&to, &to_secrets, parse("password-1", &file).unwrap());
        assert_eq!((result.accounts_added, result.accounts_replaced, result.live_replaced), (2, 0, 0));
        assert_eq!((result.api_keys_added, result.api_keys_replaced, result.rejected), (1, 0, 0));
        assert!(result.problems.is_empty());
        // 계정: 같은 이름·같은 ID·같은 로그인. .bak은 오지 않는다.
        let alpha = to.profiles_dir(Provider::Claude).join("alpha");
        assert_eq!(accounts::read_meta(&alpha).unwrap().id, "acct-alpha");
        assert_eq!(profile_cred(&to, Provider::Claude, "alpha"), profile_cred(&from, Provider::Claude, "alpha"));
        assert!(alpha.join("plan.json").is_file() && !alpha.join("credentials.json.bak").exists());
        assert!(profile_cred(&to, Provider::Codex, "delta").contains("fake-refresh-a-delta"));
        // 키: 받는 PC의 보안 저장소에 들어가고, 같은 계정에 붙어 있고, 사용 중으로 켜지지 않는다
        let listed = api_connections::list(&to, "claude").unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].account_id.as_deref(), Some("acct-alpha"));
        assert!(!listed[0].active);
        assert_eq!(api_connections::export_keys_with(&to, &to_secrets).unwrap().len(), 1);
        // 설정·메모는 생기지 않는다. 받는 PC에 로그인된 CLI가 없으면 로그인 자리도 그대로 비어 있다.
        assert!(!to.store.join("settings.json").exists() && !to.store.join("memo.json").exists());
        assert!(!to.home.join(".claude/.credentials.json").exists() && !to.codex_home.join("auth.json").exists());
    }

    /// 사용자 결정: "그냥 완전히 덮어쓰게 해줘. 구독로그인도 api도."
    #[test]
    fn an_existing_pc_is_overwritten_but_loses_nothing_that_is_not_in_the_file() {
        let (from, from_secrets) = source("transfer-over-from");
        let to = test_env("transfer-over-to");
        let to_secrets = Memory::default();
        // 받는 PC: 같은 계정이 다른 이름(mine)으로 있고, alpha라는 이름은 다른 계정이 쓰고, 파일에 없는 beta도 있다
        account(&to, Provider::Claude, "mine", "acct-alpha", "b-old");
        account(&to, Provider::Claude, "alpha", "someone-else", "b-other");
        account(&to, Provider::Claude, "beta", "acct-beta", "b-beta");
        save_fixture(&to, &to_secrets, "claude", "acct-alpha", "work", "sk-fake-b-old-work-key-0001");
        save_fixture(&to, &to_secrets, "claude", "acct-beta", "work", "sk-fake-b-beta-work-key-001");
        fs::write(to.store.join("settings.json"), br#"{"language":"en","accent_theme":"blue"}"#).unwrap();
        fs::write(to.store.join("memo.json"), br#"{"tabs":["memo-on-b"]}"#).unwrap();
        let result = bring(&to, &to_secrets, bundle_of(&from, &from_secrets));
        assert_eq!((result.accounts_added, result.accounts_replaced, result.live_replaced), (1, 1, 0));
        assert_eq!((result.api_keys_added, result.api_keys_replaced), (0, 1));
        assert!(result.problems.is_empty() && result.rejected == 0);
        // 같은 계정: 받는 PC의 이름 그대로, 로그인만 파일 것으로. 옛 것은 한 세대 남는다.
        assert!(profile_cred(&to, Provider::Claude, "mine").contains("fake-refresh-a-alpha"));
        assert!(to.profiles_dir(Provider::Claude).join("mine/credentials.json.bak").is_file());
        // 이름만 같은 다른 계정과 파일에 없는 계정은 그대로다
        assert_eq!(accounts::read_meta(&to.profiles_dir(Provider::Claude).join("alpha")).unwrap().id, "someone-else");
        assert!(profile_cred(&to, Provider::Claude, "alpha").contains("fake-refresh-b-other"));
        assert!(profile_cred(&to, Provider::Claude, "beta").contains("fake-refresh-b-beta"));
        // 키: 같은 계정의 같은 이름만 덮어쓴다. 다른 계정의 같은 이름 키는 그대로다.
        let keys = api_connections::export_keys_with(&to, &to_secrets).unwrap();
        let plain = String::from_utf8(serde_json::to_vec(&keys).unwrap()).unwrap();
        assert!(plain.contains("sk-fake-transfer-key-000001") && plain.contains("sk-fake-b-beta-work-key-001") && !plain.contains("sk-fake-b-old-work-key-0001"));
        assert_eq!(keys.len(), 2);
        // 설정과 메모는 한 글자도 바뀌지 않는다
        assert_eq!(fs::read(to.store.join("settings.json")).unwrap(), br#"{"language":"en","accent_theme":"blue"}"#);
        assert_eq!(fs::read(to.store.join("memo.json")).unwrap(), br#"{"tabs":["memo-on-b"]}"#);
        // 같은 파일을 한 번 더 넣어도 계정도 키도 늘지 않는다
        let again = bring(&to, &to_secrets, bundle_of(&from, &from_secrets));
        assert_eq!((again.accounts_added, again.accounts_replaced, again.api_keys_added, again.api_keys_unchanged), (0, 2, 0, 1));
        assert_eq!(accounts::profile_dirs(&to, Provider::Claude).unwrap().len(), 3);
    }

    /// 받는 PC에서 지금 CLI에 로그인된 계정이 파일에 있으면 그 로그인도 바꾼다 — 만료된 로그인이 살아난다
    #[test]
    fn the_account_logged_in_on_the_receiving_pc_gets_the_new_login_too() {
        let (from, from_secrets) = source("transfer-live-from");
        let to = test_env("transfer-live-to");
        let to_secrets = Memory::default();
        account(&to, Provider::Claude, "alpha", "acct-alpha", "b-expired");
        log_in(&to, Provider::Claude, "acct-alpha", "b-expired");
        // Codex에는 파일에 없는 다른 계정이 로그인돼 있다 — 건드리지 않는다
        log_in(&to, Provider::Codex, "acct-other", "b-other");
        let result = bring(&to, &to_secrets, bundle_of(&from, &from_secrets));
        assert_eq!((result.accounts_added, result.accounts_replaced, result.live_replaced), (1, 1, 1));
        assert!(result.problems.is_empty());
        assert!(live_cred(&to, Provider::Claude).contains("fake-refresh-a-alpha"));
        assert!(profile_cred(&to, Provider::Claude, "alpha").contains("fake-refresh-a-alpha"));
        // 계정 표시 정보만 손대고 ~/.claude.json의 다른 설정은 남는다
        let claude_json: Value = serde_json::from_slice(&fs::read(to.home.join(".claude.json")).unwrap()).unwrap();
        assert_eq!(claude_json["theme"], "dark");
        assert_eq!(claude_json["oauthAccount"]["accountUuid"], "acct-alpha");
        assert!(live_cred(&to, Provider::Codex).contains("fake-refresh-b-other"));
        assert!(profile_cred(&to, Provider::Codex, "delta").contains("fake-refresh-a-delta"));
    }

    /// 보내는 PC에서 지금 로그인된 계정은 프로필 사본이 아니라 활성 로그인을 담는다 —
    /// 사본은 마지막 전환 때의 것이라 그 뒤 갱신됐으면 이미 못 쓰는 값이다
    #[test]
    fn the_account_logged_in_on_the_sending_pc_is_exported_with_its_current_login() {
        let (from, from_secrets) = source("transfer-fresh-from");
        log_in(&from, Provider::Claude, "acct-alpha", "a-refreshed");
        let to = test_env("transfer-fresh-to");
        let to_secrets = Memory::default();
        bring(&to, &to_secrets, bundle_of(&from, &from_secrets));
        let cred = profile_cred(&to, Provider::Claude, "alpha");
        assert!(cred.contains("fake-refresh-a-refreshed") && !cred.contains("fake-refresh-a-alpha"));
        // 보내는 PC는 읽기만 한다
        assert!(profile_cred(&from, Provider::Claude, "alpha").contains("fake-refresh-a-alpha"));
    }

    #[test]
    fn a_crafted_bundle_cannot_write_outside_the_profile_folder() {
        let to = test_env("transfer-crafted");
        let b64 = base64::engine::general_purpose::STANDARD;
        let meta = b64.encode(br#"{"id":"x","email":null,"saved_at":1}"#);
        let entry = |name: &str, files: &[(&str, &str)]| ProfileEntry {
            provider: "claude".into(),
            name: name.into(),
            files: files.iter().map(|(file, body)| (file.to_string(), body.to_string())).collect(),
        };
        let cred = b64.encode(claude_cred("crafted"));
        let not_a_login = b64.encode(b"{}");
        let bundle = Bundle {
            version: BUNDLE_VERSION,
            created_at: 0,
            source_os: "test".into(),
            profiles: vec![
                entry("../escape", &[("credentials.json", &cred), ("meta.json", &meta)]),
                entry("nul", &[("credentials.json", &cred), ("meta.json", &meta)]),
                entry("nocred", &[("meta.json", &meta)]),
                entry("notlogin", &[("credentials.json", &not_a_login), ("meta.json", &meta)]),
                entry("extra", &[("credentials.json", &cred), ("meta.json", &meta), ("../../evil.sh", &cred), ("run.sh", &cred), ("auth.json", &cred)]),
            ],
            api_keys: serde_json::from_value(serde_json::json!([
                {"provider": "claude", "name": "../up", "key": "sk-fake-transfer-key-000001"},
                {"provider": "claude", "name": "ok", "key": "not-a-key"}
            ]))
            .unwrap(),
        };
        let secrets = Memory::default();
        let result = bring(&to, &secrets, bundle);
        assert_eq!((result.accounts_added, result.rejected), (1, 6));
        let extra = to.profiles_dir(Provider::Claude).join("extra");
        let mut names: Vec<_> = fs::read_dir(&extra).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["credentials.json", "meta.json"]);
        assert!(!to.store.join("evil.sh").exists() && !to.home.join("evil.sh").exists());
        assert_eq!(accounts::profile_dirs(&to, Provider::Claude).unwrap().len(), 1);
        assert!(api_connections::list(&to, "claude").unwrap().is_empty());
    }

    #[test]
    fn export_writes_one_locked_file_and_import_finds_it() {
        let (from, from_secrets) = source("transfer-file");
        fs::create_dir_all(from.home.join("Downloads")).unwrap();
        assert!(check_password("short").is_err());
        let done = write_export(&from, "password-1", &bundle_of(&from, &from_secrets), "20261123-101500").unwrap();
        assert_eq!((done.accounts, done.api_keys), (2, 1));
        assert!(done.path.ends_with("switcher-transfer-20261123-101500.swx"));
        let bytes = fs::read(&done.path).unwrap();
        for secret in [&b"sk-fake"[..], b"fake-access", b"fake-refresh", b"acct-alpha"] {
            assert!(!bytes.windows(secret.len()).any(|window| window == secret));
        }
        // 찾기: 우리 파일만, 확장자만 같은 다른 파일은 아니다
        fs::write(from.home.join("Downloads/other.swx"), vec![7u8; 200]).unwrap();
        let found = candidates(&from);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, done.path);
        assert!(parse("password-1", &read_transfer_file(Path::new(&done.path)).unwrap()).is_ok());
        assert_eq!(parse("password-2", &bytes).err().unwrap(), BAD_PASSWORD);
        // 처음 판의 묶음은 받지 않는다 — 무엇을 하면 되는지 말해 준다
        let old = seal_fast("password-1", br#"{"version":1,"created_at":0,"source_os":"macos","profiles":[],"api_keys":[],"memo":{"tabs":["x"]}}"#);
        assert!(parse("password-1", &old).err().unwrap().contains("다시 내보내세요"));
    }
}

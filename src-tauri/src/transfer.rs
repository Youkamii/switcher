//! 다른 PC로 옮기기 (#189, 사용자 지시 2026-11-23).
//!
//! A PC의 계정 프로필·API 키·메모·설정을 파일 하나에 담아 B PC에서 그대로 되살린다.
//! 파일에는 로그인 토큰과 API 키 원문이 들어가므로 통째로 잠근다:
//!
//! - 열쇠는 사용자의 비밀번호에서 만든다 (Argon2id — 일부러 느리고 메모리를 많이 써서
//!   비밀번호를 하나씩 대입해 보는 공격을 비싸게 한다). 소금은 파일마다 새로 뽑는다.
//! - 묶음은 XChaCha20-Poly1305로 잠근다. 머리말(형식·소금·열쇠 만들기 값)까지 인증에 넣어
//!   한 바이트라도 바뀌면 풀리지 않는다 — 틀린 비밀번호와 변조는 같은 오류가 된다.
//! - 받는 쪽은 다 풀고 검사한 뒤에야 쓴다. 이름은 경로가 되므로 저장할 때와 같은 규칙으로 거른다.
//!
//! 옮기지 않는 것: 지금 CLI에 로그인된 상태(받는 PC의 것은 그대로 둔다), 사용량 캐시, 런처,
//! `.bak`·손상 사본. 금기: 비밀번호·열쇠·묶음 내용을 로그나 오류 문구에 넣지 말 것.

use crate::accounts::{self, atomic_write, validate_name, Env, Provider};
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
    version: u32,
    created_at: u64,
    source_os: String,
    profiles: Vec<ProfileEntry>,
    api_keys: Vec<TransferKey>,
    #[serde(default)]
    settings: Option<Value>,
    #[serde(default)]
    memo: Option<Value>,
    /// 화면 쪽 설정(보기 모드·순서 등) — 프론트가 주고 프론트가 되살린다
    #[serde(default)]
    ui: BTreeMap<String, String>,
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
    /// 같은 이름이 다른 계정의 것이거나 그 계정이 다른 이름으로 이미 있어 넣지 않은 프로필
    pub accounts_skipped: Vec<String>,
    pub api_keys_added: usize,
    pub api_keys_replaced: usize,
    pub rejected: usize,
    pub settings: bool,
    pub memo: bool,
    pub ui: BTreeMap<String, String>,
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

fn read_json_file(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn collect(env: &Env, api_keys: Vec<TransferKey>, ui: BTreeMap<String, String>) -> Result<Bundle, String> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut profiles = Vec::new();
    for provider in [Provider::Claude, Provider::Codex] {
        for (name, dir) in accounts::profile_dirs(env, provider)? {
            // 이름 변경 잔해 같은 폴더는 계정이 아니다
            if validate_name(&name).is_err() || accounts::read_meta(&dir).is_none() {
                continue;
            }
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
            if files.contains_key(provider.credential_file_name()) {
                profiles.push(ProfileEntry { provider: provider.dir_name().into(), name, files });
            }
        }
    }
    Ok(Bundle {
        version: 1,
        created_at: accounts::now(),
        source_os: std::env::consts::OS.into(),
        profiles,
        api_keys,
        settings: read_json_file(&env.store.join("settings.json")),
        memo: read_json_file(&env.store.join("memo.json")),
        ui,
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

pub fn export(env: &Env, password: &str, ui: BTreeMap<String, String>) -> Result<ExportResult, String> {
    check_password(password)?;
    let _mutation = accounts::MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let bundle = collect(env, api_connections::export_keys(env)?, ui)?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    write_export(env, password, &bundle, &stamp)
}

// ── 되살리기 ───────────────────────────────────────────────────────

fn parse(password: &str, file: &[u8]) -> Result<Bundle, String> {
    let mut plain = open(password, file)?;
    let bundle: Result<Bundle, _> = serde_json::from_slice(&plain);
    plain.zeroize();
    let bundle = bundle.map_err(|_| BAD_FILE)?;
    if bundle.version != 1 {
        return Err("이 파일은 더 새 버전의 위젯에서 만든 것입니다 — 위젯을 업데이트하세요".into());
    }
    Ok(bundle)
}

/// 받아 쓸 수 있는 프로필인지 본다 — (공급자, 계정 ID, 파일 이름 → 내용)
fn checked_profile(entry: &ProfileEntry) -> Option<(Provider, String, Vec<(&'static str, Vec<u8>)>)> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let provider = Provider::parse(&entry.provider).ok()?;
    validate_name(&entry.name).ok()?;
    let mut files = Vec::new();
    // 허용 목록을 기준으로 돈다 — 묶음에 든 다른 이름은 경로로 쓰이지 않는다
    for allowed in profile_files(provider) {
        if let Some(encoded) = entry.files.get(*allowed) {
            let bytes = b64.decode(encoded).ok()?;
            if bytes.len() > PROFILE_FILE_MAX {
                return None;
            }
            files.push((*allowed, bytes));
        }
    }
    let meta = files.iter().find(|(name, _)| *name == "meta.json")?;
    let meta: accounts::Meta = serde_json::from_slice(&meta.1).ok()?;
    let has_credential = files.iter().any(|(name, _)| *name == provider.credential_file_name());
    (has_credential && !meta.id.is_empty()).then_some((provider, meta.id, files))
}

fn merge_settings(env: &Env, incoming: &Value) -> Result<bool, String> {
    let Some(incoming) = incoming.as_object() else {
        return Ok(false);
    };
    let path = env.store.join("settings.json");
    let mut current = match read_json_file(&path) {
        Some(Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    };
    for (key, value) in incoming {
        current.insert(key.clone(), value.clone());
    }
    let bytes = serde_json::to_vec_pretty(&Value::Object(current)).map_err(|_| "설정 저장 실패")?;
    atomic_write(&path, &bytes)?;
    Ok(true)
}

fn apply(
    env: &Env,
    bundle: Bundle,
    import_keys: impl FnOnce(&Env, &[TransferKey]) -> Result<KeyImport, String>,
) -> Result<ImportResult, String> {
    let _mutation = accounts::MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let mut result = ImportResult { source_os: bundle.source_os.clone(), ..Default::default() };
    for entry in &bundle.profiles {
        let Some((provider, id, files)) = checked_profile(entry) else {
            result.rejected += 1;
            continue;
        };
        let dir = env.profiles_dir(provider).join(&entry.name);
        let existing = accounts::read_meta(&dir);
        // 다른 계정의 프로필을 덮지 않는다. 같은 계정이 다른 이름으로 있으면 둘로 만들지 않는다.
        let taken = dir.exists() && existing.as_ref().map(|meta| meta.id != id).unwrap_or(true);
        let elsewhere = matches!(accounts::find_profile_by_id(env, provider, &id)?, Some(name) if name != entry.name);
        if taken || elsewhere {
            result.accounts_skipped.push(entry.name.clone());
            continue;
        }
        let credential = dir.join(provider.credential_file_name());
        if credential.exists() {
            // 덮어쓰기 전 한 세대 — 프로필 저장과 같은 안전망
            let _ = fs::copy(&credential, credential.with_extension("json.bak"));
        }
        // meta.json을 마지막에 쓴다 — 중간에 실패한 폴더는 계정으로 보이지 않는다
        for (name, bytes) in files.iter().filter(|(name, _)| *name != "meta.json") {
            atomic_write(&dir.join(name), bytes)?;
        }
        if let Some((name, bytes)) = files.iter().find(|(name, _)| *name == "meta.json") {
            atomic_write(&dir.join(name), bytes)?;
        }
        if existing.is_some() {
            result.accounts_replaced += 1;
        } else {
            result.accounts_added += 1;
        }
    }
    let keys = import_keys(env, &bundle.api_keys)?;
    result.api_keys_added = keys.added;
    result.api_keys_replaced = keys.replaced;
    result.rejected += keys.rejected;
    if let Some(settings) = &bundle.settings {
        result.settings = merge_settings(env, settings)?;
    }
    if let Some(memo) = &bundle.memo {
        let path = env.store.join("memo.json");
        if path.exists() {
            // 받는 PC의 메모를 말없이 지우지 않는다
            let _ = fs::copy(&path, env.store.join("memo.before-transfer.json"));
        }
        let bytes = serde_json::to_vec_pretty(memo).map_err(|_| "메모 저장 실패")?;
        atomic_write(&path, &bytes)?;
        result.memo = true;
    }
    result.ui = bundle.ui;
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
    use crate::api_connections::tests::{add_account, Memory};

    // 검사는 가벼운 열쇠 만들기 값으로 돈다 — 기본값(64MiB)은 한 번만 왕복해 본다
    fn seal_fast(password: &str, plain: &[u8]) -> Vec<u8> {
        seal_with(password, plain, 64, 1).unwrap()
    }

    fn sample_ui() -> BTreeMap<String, String> {
        BTreeMap::from([("switcher.viewmode".to_string(), "edge".to_string())])
    }

    /// 계정 둘과 키 하나, 메모·설정이 있는 보내는 PC
    fn source(tag: &str) -> (Env, Memory) {
        let env = test_env(tag);
        let secrets = Memory::default();
        add_account(&env, Provider::Claude, "alpha", "acct-alpha");
        add_account(&env, Provider::Codex, "delta", "acct-delta");
        let alpha = env.profiles_dir(Provider::Claude).join("alpha");
        fs::write(alpha.join("credentials.json"), br#"{"claudeAiOauth":{"accessToken":"fake-access-alpha"}}"#).unwrap();
        fs::write(alpha.join("credentials.json.bak"), b"old-generation").unwrap();
        fs::write(alpha.join("plan.json"), br#"{"tier":20}"#).unwrap();
        api_connections::tests::save_fixture(&env, &secrets, "claude", "acct-alpha", "work", "sk-fake-transfer-key-000001");
        fs::write(env.store.join("settings.json"), br#"{"accent_theme":"violet","tfsd":true}"#).unwrap();
        fs::write(env.store.join("memo.json"), br#"{"active":0,"tabs":["hello"]}"#).unwrap();
        (env, secrets)
    }

    fn bundle_of(env: &Env, secrets: &Memory) -> Bundle {
        collect(env, api_connections::export_keys_with(env, secrets).unwrap(), sample_ui()).unwrap()
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
    fn everything_arrives_on_an_empty_pc() {
        let (from, from_secrets) = source("transfer-from");
        let file = seal_fast("password-1", &serde_json::to_vec(&bundle_of(&from, &from_secrets)).unwrap());
        let to = test_env("transfer-to");
        let to_secrets = Memory::default();
        let result = apply(&to, parse("password-1", &file).unwrap(), |env, keys| {
            api_connections::import_keys_with(env, &to_secrets, keys)
        })
        .unwrap();
        assert_eq!((result.accounts_added, result.accounts_replaced, result.api_keys_added), (2, 0, 1));
        assert!(result.accounts_skipped.is_empty() && result.rejected == 0 && result.settings && result.memo);
        assert_eq!(result.ui, sample_ui());
        // 계정: 같은 이름·같은 ID·같은 토큰. .bak은 오지 않는다.
        let alpha = to.profiles_dir(Provider::Claude).join("alpha");
        assert_eq!(accounts::read_meta(&alpha).unwrap().id, "acct-alpha");
        assert_eq!(
            fs::read(alpha.join("credentials.json")).unwrap(),
            fs::read(from.profiles_dir(Provider::Claude).join("alpha/credentials.json")).unwrap()
        );
        assert!(alpha.join("plan.json").is_file() && !alpha.join("credentials.json.bak").exists());
        assert!(to.profiles_dir(Provider::Codex).join("delta/auth.json").is_file());
        // 키: 받는 PC의 보안 저장소에 들어가고, 같은 계정에 붙어 있고, 쓰는 중으로 표시되지 않는다
        let keys = api_connections::export_keys_with(&to, &to_secrets).unwrap();
        assert_eq!(keys.len(), 1);
        let listed = api_connections::list(&to, "claude").unwrap();
        assert_eq!(listed[0].account_id.as_deref(), Some("acct-alpha"));
        assert!(!listed[0].active);
        assert_eq!(read_json_file(&to.store.join("memo.json")).unwrap()["tabs"][0], "hello");
        assert_eq!(read_json_file(&to.store.join("settings.json")).unwrap()["accent_theme"], "violet");
        // 받는 PC의 CLI 로그인 상태는 건드리지 않는다
        assert!(!to.home.join(".claude/.credentials.json").exists());
        assert!(!to.codex_home.join("auth.json").exists());
    }

    #[test]
    fn an_existing_pc_keeps_what_is_not_in_the_file_and_other_peoples_profiles() {
        let (from, from_secrets) = source("transfer-merge-from");
        let bundle = bundle_of(&from, &from_secrets);
        let to = test_env("transfer-merge-to");
        let to_secrets = Memory::default();
        // 받는 PC: 같은 계정 alpha(옛 토큰), 이름만 같은 다른 계정 delta, 파일에 없는 계정 beta
        add_account(&to, Provider::Claude, "alpha", "acct-alpha");
        add_account(&to, Provider::Claude, "beta", "acct-beta");
        add_account(&to, Provider::Codex, "delta", "someone-else");
        fs::write(to.store.join("settings.json"), br#"{"language":"en","accent_theme":"blue"}"#).unwrap();
        fs::write(to.store.join("memo.json"), br#"{"tabs":["mine"]}"#).unwrap();
        let result = apply(&to, bundle, |env, keys| api_connections::import_keys_with(env, &to_secrets, keys)).unwrap();
        assert_eq!((result.accounts_added, result.accounts_replaced), (0, 1));
        assert_eq!(result.accounts_skipped, vec!["delta".to_string()]);
        let alpha = to.profiles_dir(Provider::Claude).join("alpha");
        assert!(fs::read_to_string(alpha.join("credentials.json")).unwrap().contains("fake-access-alpha"));
        assert!(alpha.join("credentials.json.bak").is_file(), "the replaced token keeps one generation");
        assert_eq!(accounts::read_meta(&to.profiles_dir(Provider::Codex).join("delta")).unwrap().id, "someone-else");
        assert!(to.profiles_dir(Provider::Claude).join("beta").is_dir());
        let settings = read_json_file(&to.store.join("settings.json")).unwrap();
        assert_eq!((settings["language"].as_str(), settings["accent_theme"].as_str()), (Some("en"), Some("violet")));
        assert_eq!(read_json_file(&to.store.join("memo.before-transfer.json")).unwrap()["tabs"][0], "mine");
        // 같은 파일을 한 번 더 넣어도 계정이 둘이 되지 않는다
        let again = apply(&to, bundle_of(&from, &from_secrets), |env, keys| {
            api_connections::import_keys_with(env, &to_secrets, keys)
        })
        .unwrap();
        assert_eq!((again.accounts_added, again.accounts_replaced, again.api_keys_added, again.api_keys_replaced), (0, 1, 0, 1));
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
        let cred = b64.encode(b"{}");
        let bundle = Bundle {
            version: 1,
            created_at: 0,
            source_os: "test".into(),
            profiles: vec![
                entry("../escape", &[("credentials.json", &cred), ("meta.json", &meta)]),
                entry("nul", &[("credentials.json", &cred), ("meta.json", &meta)]),
                entry("nocred", &[("meta.json", &meta)]),
                entry("extra", &[("credentials.json", &cred), ("meta.json", &meta), ("../../evil.sh", &cred), ("run.sh", &cred)]),
            ],
            api_keys: serde_json::from_value(serde_json::json!([
                {"connection": {"provider": "claude", "name": "../up"}, "key": "sk-fake-transfer-key-000001"},
                {"connection": {"provider": "claude", "name": "ok"}, "key": "not-a-key"}
            ]))
            .unwrap(),
            settings: None,
            memo: None,
            ui: BTreeMap::new(),
        };
        let secrets = Memory::default();
        let result = apply(&to, bundle, |env, keys| api_connections::import_keys_with(env, &secrets, keys)).unwrap();
        assert_eq!((result.accounts_added, result.rejected), (1, 5));
        let extra = to.profiles_dir(Provider::Claude).join("extra");
        let mut names: Vec<_> = fs::read_dir(&extra).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["credentials.json", "meta.json"]);
        assert!(!to.store.join("evil.sh").exists() && !to.home.join("evil.sh").exists());
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
        for secret in [&b"sk-fake"[..], b"fake-access", b"acct-alpha", b"hello"] {
            assert!(!bytes.windows(secret.len()).any(|window| window == secret));
        }
        // 찾기: 우리 파일만, 확장자만 같은 다른 파일은 아니다
        fs::write(from.home.join("Downloads/other.swx"), vec![7u8; 200]).unwrap();
        let found = candidates(&from);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, done.path);
        assert!(parse("password-1", &read_transfer_file(Path::new(&done.path)).unwrap()).is_ok());
        assert_eq!(parse("password-2", &bytes).err().unwrap(), BAD_PASSWORD);
    }
}

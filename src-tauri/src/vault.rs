//! 선택한 계정 프로필을 암호화 파일로 옮기는 플랫폼 공통 코어.
//!
//! 포맷은 Windows 전용 저장소(DPAPI)나 macOS 키체인 형식에 기대지 않는다.
//! 같은 Argon2id + AES-256-GCM 파일을 두 OS에서 그대로 읽고 쓸 수 있게 모든
//! 플랫폼 종속 처리는 `accounts`의 활성 자격증명 관문 뒤에 둔다.

use crate::accounts::{self, Env, Provider, MUTATION_LOCK};
use aes_gcm::aead::{Aead, KeyInit, Payload as AeadPayload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::hash_map::RandomState;
use std::collections::HashSet;
use std::fs;
use std::hash::BuildHasher;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const MAGIC: &[u8; 8] = b"SWVAULT\0";
const FORMAT_VERSION: u8 = 1;
const PAYLOAD_VERSION: u32 = 1;
const HEADER_MAX_BYTES: usize = 4 * 1024;
const VAULT_MAX_BYTES: usize = 64 * 1024 * 1024;
const PAYLOAD_MAX_BYTES: usize = 48 * 1024 * 1024;
const CREDENTIAL_MAX_BYTES: usize = 4 * 1024 * 1024;
const OAUTH_MAX_BYTES: usize = 256 * 1024;
const PENDING_MAX_BYTES: usize = 4 * 1024 * 1024;
const RAW_TOTAL_MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_ENTRIES: usize = 256;
const TAG_BYTES: usize = 16;
const JS_SAFE_U64_MASK: u64 = (1 << 53) - 1;

const ARGON_MEMORY_KIB: u32 = 65_536;
const ARGON_ITERATIONS: u32 = 3;
const ARGON_PARALLELISM: u32 = 1;
const ARGON_OUTPUT_BYTES: usize = 32;

const CRYPTO_ERROR: &str = "인증정보 파일 또는 복구 코드가 올바르지 않습니다";

struct OperationState {
    active: bool,
    pending_recovery: Option<Zeroizing<String>>,
    shutdown_reserved: bool,
}

static OPERATION_STATE: Mutex<OperationState> = Mutex::new(OperationState {
    active: false,
    pending_recovery: None,
    shutdown_reserved: false,
});

static PROFILE_REVISION_HASHER: LazyLock<RandomState> = LazyLock::new(RandomState::new);

pub(crate) struct OperationGuard;

impl Drop for OperationGuard {
    fn drop(&mut self) {
        // 잠금이 오염돼도 작업 중 상태를 영구히 남기지 않는다. 다른 진입점은
        // 오염을 오류로 처리하지만 Drop에서는 복구 가능한 상태만 정리한다.
        let mut state = OPERATION_STATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.active = false;
    }
}

pub(crate) fn begin_operation() -> Result<OperationGuard, String> {
    let mut state = OPERATION_STATE
        .lock()
        .map_err(|_| "내부 잠금 오류".to_string())?;
    if state.shutdown_reserved {
        return Err("앱 종료 또는 재시작을 준비하고 있습니다".into());
    }
    if state.pending_recovery.is_some() {
        return Err("복구 코드를 화면에 표시하는 중입니다".into());
    }
    if state.active {
        return Err("다른 인증정보 이동 작업이 진행 중입니다".into());
    }
    state.active = true;
    Ok(OperationGuard)
}

pub(crate) fn operation_busy() -> bool {
    OPERATION_STATE
        .lock()
        .map(|state| state.active || state.pending_recovery.is_some() || state.shutdown_reserved)
        .unwrap_or(true)
}

pub(crate) fn try_reserve_shutdown() -> bool {
    let Ok(mut state) = OPERATION_STATE.lock() else {
        return false;
    };
    if state.active || state.pending_recovery.is_some() || state.shutdown_reserved {
        return false;
    }
    state.shutdown_reserved = true;
    true
}

pub(crate) fn release_shutdown_reservation() {
    let mut state = OPERATION_STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.shutdown_reserved = false;
}

pub(crate) fn hold_recovery_for_delivery(recovery_code: &str) -> Result<(), String> {
    let mut state = OPERATION_STATE
        .lock()
        .map_err(|_| "내부 잠금 오류".to_string())?;
    state.pending_recovery = Some(Zeroizing::new(recovery_code.to_string()));
    Ok(())
}

pub(crate) fn pending_recovery() -> Result<Option<RecoveryCode>, String> {
    OPERATION_STATE
        .lock()
        .map(|state| {
            state
                .pending_recovery
                .as_ref()
                .map(|code| RecoveryCode::new(code.as_str().to_owned()))
        })
        .map_err(|_| "내부 잠금 오류".to_string())
}

pub(crate) fn ack_recovery_stored(recovery_code: String) -> Result<bool, String> {
    let recovery_code = Zeroizing::new(recovery_code);
    let mut state = OPERATION_STATE
        .lock()
        .map_err(|_| "내부 잠금 오류".to_string())?;
    match state.pending_recovery.as_ref() {
        None => Ok(true), // 응답만 유실된 재시도도 성공으로 합친다.
        Some(expected) if expected.as_str() == recovery_code.as_str() => {
            state.pending_recovery = None;
            Ok(true)
        }
        Some(_) => Ok(false),
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VaultSelection {
    pub provider: String,
    pub name: String,
    pub revision: u64,
    #[serde(default)]
    pub hide_email: bool,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct VaultProfile {
    pub provider: String,
    pub name: String,
    pub active: bool,
    pub revision: u64,
}

pub struct RecoveryCode(Zeroizing<String>);

impl RecoveryCode {
    fn new(code: String) -> Self {
        Self(Zeroizing::new(code))
    }

    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl PartialEq for RecoveryCode {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for RecoveryCode {}

impl std::fmt::Debug for RecoveryCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted]")
    }
}

impl Serialize for RecoveryCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

#[derive(Serialize, PartialEq, Eq)]
pub struct VaultExportResult {
    pub recovery_code: RecoveryCode,
    pub exported: usize,
}

impl std::fmt::Debug for VaultExportResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VaultExportResult")
            .field("recovery_code", &"[redacted]")
            .field("exported", &self.exported)
            .finish()
    }
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct VaultImportResult {
    pub imported: usize,
    pub skipped: usize,
    pub cleanup_pending: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VaultHeader {
    cipher: String,
    kdf: KdfHeader,
    salt: String,
    nonce: String,
    payload_len: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KdfHeader {
    algorithm: String,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    output_len: u32,
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
struct VaultPayload {
    format_version: u32,
    entries: Vec<VaultEntry>,
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
struct VaultEntry {
    provider: String,
    name: String,
    hide_email: bool,
    id: String,
    email: Option<String>,
    /// base64url 문자열로 두어 JSON의 `Vec<u8>` 숫자 배열 폭증을 피한다.
    credential: String,
    oauth_account: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
struct ClaudeCredentialProbe {
    claude_ai_oauth: ClaudeTokenProbe,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
struct ClaudeTokenProbe {
    access_token: String,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
struct ClaudeOauthProbe {
    account_uuid: String,
    email_address: Option<String>,
}

struct ParsedVault {
    payload: VaultPayload,
}

struct ClaudeLiveSnapshot {
    credential: Zeroizing<Vec<u8>>,
    oauth_account: serde_json::Value,
    identity: accounts::LiveIdentity,
}

pub fn list_profiles(env: &Env) -> Result<Vec<VaultProfile>, String> {
    let mut result = Vec::new();
    for provider in [Provider::Claude, Provider::Codex] {
        let live_id = accounts::live_identity(env, provider)
            .unwrap_or(None)
            .map(|identity| identity.id);
        for (name, dir) in accounts::profile_dirs(env, provider)? {
            if accounts::validate_name(&name).is_err() {
                continue;
            }
            let Some(meta) = accounts::read_meta(&dir) else {
                continue;
            };
            if !dir.join(provider.credential_file_name()).is_file() {
                continue;
            }
            if provider == Provider::Claude && !dir.join("oauth_account.json").is_file() {
                continue;
            }
            result.push(VaultProfile {
                provider: provider.dir_name().to_string(),
                name,
                active: live_id.as_deref() == Some(meta.id.as_str()),
                revision: profile_revision(provider, &meta.id),
            });
        }
    }
    Ok(result)
}

fn profile_revision(provider: Provider, id: &str) -> u64 {
    // RandomState의 키는 프로세스마다 새로 생긴다. 원본 id를 프론트에 내보내지
    // 않으면서 같은 실행 안에서만 identity 교체를 구분한다.
    PROFILE_REVISION_HASHER.hash_one((provider.dir_name(), id)) & JS_SAFE_U64_MASK
}

pub fn export(
    env: &Env,
    path: &Path,
    selections: Vec<VaultSelection>,
) -> Result<VaultExportResult, String> {
    ensure_export_destination_is_safe(env, path)?;
    validate_selections(&selections)?;
    let entries = capture_entries(env, &selections)?;
    let exported = entries.len();
    let payload = VaultPayload {
        format_version: PAYLOAD_VERSION,
        entries,
    };
    validate_payload(&payload)
        .map_err(|_| "선택한 프로필에 중복되거나 올바르지 않은 항목이 있습니다")?;

    let plaintext = Zeroizing::new(
        serde_json::to_vec(&payload).map_err(|_| "인증정보 묶음을 만들 수 없습니다")?,
    );
    if plaintext.len() > PAYLOAD_MAX_BYTES {
        return Err("내보낼 인증정보가 너무 큽니다".into());
    }

    let mut recovery_secret = Zeroizing::new([0u8; 32]);
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 12];
    OsRng
        .try_fill_bytes(&mut *recovery_secret)
        .and_then(|_| OsRng.try_fill_bytes(&mut salt))
        .and_then(|_| OsRng.try_fill_bytes(&mut nonce))
        .map_err(|_| "안전한 난수를 만들 수 없습니다")?;

    let header = VaultHeader {
        cipher: "AES-256-GCM".into(),
        kdf: KdfHeader {
            algorithm: "Argon2id".into(),
            memory_kib: ARGON_MEMORY_KIB,
            iterations: ARGON_ITERATIONS,
            parallelism: ARGON_PARALLELISM,
            output_len: ARGON_OUTPUT_BYTES as u32,
        },
        salt: URL_SAFE_NO_PAD.encode(salt),
        nonce: URL_SAFE_NO_PAD.encode(nonce),
        payload_len: plaintext.len() as u64,
    };
    let header_bytes = serde_json::to_vec(&header).map_err(|_| CRYPTO_ERROR.to_string())?;
    if header_bytes.len() > HEADER_MAX_BYTES {
        return Err(CRYPTO_ERROR.into());
    }
    let aad = make_aad(&header_bytes)?;
    let key = derive_key(&recovery_secret[..], &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key[..]).map_err(|_| CRYPTO_ERROR.to_string())?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            AeadPayload {
                msg: &plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| CRYPTO_ERROR.to_string())?;

    let total_len = aad
        .len()
        .checked_add(ciphertext.len())
        .filter(|length| *length <= VAULT_MAX_BYTES)
        .ok_or_else(|| "내보낼 인증정보가 너무 큽니다".to_string())?;
    let mut file_bytes = Vec::with_capacity(total_len);
    file_bytes.extend_from_slice(&aad);
    file_bytes.extend_from_slice(&ciphertext);
    // 암호화가 도는 동안 저장 부모가 바뀌는 실수를 줄이기 위해 실제 교체 직전
    // 보호 경로를 다시 확인한다. 같은 사용자 권한의 악성 junction 공격은 별도 OS
    // 경계가 아니지만, 활성 auth 파일을 실수로 고르는 경로는 두 번 막는다.
    ensure_export_destination_is_safe(env, path)?;
    accounts::atomic_write(path, &file_bytes)
        .map_err(|_| "암호화 파일을 저장할 수 없습니다".to_string())?;

    Ok(VaultExportResult {
        recovery_code: RecoveryCode::new(URL_SAFE_NO_PAD.encode(&recovery_secret[..])),
        exported,
    })
}

fn ensure_export_destination_is_safe(env: &Env, path: &Path) -> Result<(), String> {
    let destination = resolved_path(path)?;
    let store = resolved_path(&env.store)?;
    let protected = [
        env.live_credential_path(Provider::Claude),
        env.live_credential_path(Provider::Codex),
        env.home.join(".claude.json"),
    ];
    if path_is_within(&destination, &store)
        || protected
            .iter()
            .filter_map(|path| resolved_path(path).ok())
            .any(|path| same_path(&destination, &path))
    {
        return Err("인증정보 원본 폴더 밖의 다른 위치를 선택하세요".into());
    }
    Ok(())
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    #[cfg(any(windows, target_os = "macos"))]
    {
        let path = path.to_string_lossy().to_ascii_lowercase();
        let mut root = root.to_string_lossy().to_ascii_lowercase();
        while root.ends_with('\\') || root.ends_with('/') {
            root.pop();
        }
        path == root
            || path
                .strip_prefix(&root)
                .is_some_and(|rest| rest.starts_with('\\') || rest.starts_with('/'))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        path.starts_with(root)
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    #[cfg(any(windows, target_os = "macos"))]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        left == right
    }
}

fn resolved_path(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        return fs::canonicalize(path).map_err(|_| "저장 경로를 확인할 수 없습니다".into());
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        if parent.exists() {
            return fs::canonicalize(parent)
                .map(|parent| parent.join(name))
                .map_err(|_| "저장 경로를 확인할 수 없습니다".into());
        }
    }
    std::path::absolute(path).map_err(|_| "저장 경로를 확인할 수 없습니다".into())
}

pub fn import(env: &Env, path: &Path, recovery_code: String) -> Result<VaultImportResult, String> {
    let recovery_code = Zeroizing::new(recovery_code);
    let parsed = decrypt_file(path, &recovery_code)?;
    commit_payload(env, &parsed.payload, |_, _| Ok(()))
}

fn validate_selections(selections: &[VaultSelection]) -> Result<(), String> {
    if selections.is_empty() {
        return Err("옮길 프로필을 하나 이상 선택하세요".into());
    }
    if selections.len() > MAX_ENTRIES {
        return Err("한 번에 옮길 수 있는 프로필 수를 넘었습니다".into());
    }
    let mut seen = HashSet::new();
    for selection in selections {
        let provider = Provider::parse(&selection.provider)?;
        accounts::validate_name(&selection.name)?;
        if !seen.insert(name_key(provider, &selection.name)) {
            return Err("같은 프로필이 두 번 선택되었습니다".into());
        }
    }
    Ok(())
}

fn capture_entries(env: &Env, selections: &[VaultSelection]) -> Result<Vec<VaultEntry>, String> {
    let mut lock_targets: Vec<(String, Provider, &str)> = selections
        .iter()
        .map(|selection| {
            let provider = Provider::parse(&selection.provider)?;
            Ok((
                accounts::refresh_key(env, provider, &selection.name),
                provider,
                selection.name.as_str(),
            ))
        })
        .collect::<Result<_, String>>()?;
    lock_targets.sort_by(|a, b| a.0.cmp(&b.0));

    let mut profile_guards = Vec::with_capacity(lock_targets.len());
    for (key, _, _) in &lock_targets {
        profile_guards.push(accounts::profile_exclusive_begin(
            key.clone(),
            std::time::Duration::from_secs(20),
        )?);
    }

    let captured = {
        let _mutation = MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
        let mut entries = Vec::with_capacity(selections.len());
        let mut raw_total = 0usize;
        for selection in selections {
            let provider = Provider::parse(&selection.provider)?;
            let dir = env.profiles_dir(provider).join(&selection.name);
            let meta = accounts::read_meta(&dir)
                .ok_or_else(|| format!("프로필 '{}'을 찾을 수 없습니다", selection.name))?;
            if profile_revision(provider, &meta.id) != selection.revision {
                return Err(format!(
                    "프로필 '{}'이 선택한 뒤 변경되었습니다 — 목록을 새로 확인하세요",
                    selection.name
                ));
            }
            let live = accounts::live_identity(env, provider)?;
            let active = live.as_ref().is_some_and(|identity| identity.id == meta.id);
            let stable_claude = if active && provider == Provider::Claude {
                let snapshot = read_stable_claude_snapshot(env)?;
                if snapshot.identity.id != meta.id {
                    return Err(format!(
                        "프로필 '{}'을 읽는 동안 Claude 로그인이 변경되었습니다",
                        selection.name
                    ));
                }
                Some(snapshot)
            } else {
                None
            };
            let (identity_id, identity_email) = if let Some(snapshot) = stable_claude.as_ref() {
                (
                    snapshot.identity.id.clone(),
                    snapshot.identity.email.clone(),
                )
            } else {
                match live.as_ref() {
                    Some(identity) if active => (identity.id.clone(), identity.email.clone()),
                    _ => (meta.id, meta.email),
                }
            };

            let credential_path = dir.join(provider.credential_file_name());
            let base_credential = Zeroizing::new(if let Some(snapshot) = stable_claude.as_ref() {
                snapshot.credential.as_slice().to_vec()
            } else if active {
                read_live_bounded(env, provider)?
            } else {
                accounts::normalize_cred(
                    read_bounded_file(&credential_path, CREDENTIAL_MAX_BYTES).map_err(|_| {
                        format!("프로필 '{}'의 인증정보를 읽을 수 없습니다", selection.name)
                    })?,
                )
            });
            let pending_path = crate::usage::pending_path(&credential_path);
            let pending = if pending_path.is_file() {
                Some(Zeroizing::new(
                    read_bounded_file(&pending_path, PENDING_MAX_BYTES).map_err(|_| {
                        format!(
                            "프로필 '{}'의 갱신 복구 정보를 읽을 수 없습니다",
                            selection.name
                        )
                    })?,
                ))
            } else {
                None
            };
            let credential = Zeroizing::new(
                crate::usage::merge_pending_snapshot(
                    provider,
                    &base_credential,
                    pending.as_deref().map(|bytes| bytes.as_slice()),
                )
                .map_err(|_| {
                    format!(
                        "프로필 '{}'의 갱신 복구 정보가 올바르지 않습니다",
                        selection.name
                    )
                })?,
            );
            if credential.is_empty() || credential.len() > CREDENTIAL_MAX_BYTES {
                return Err(format!(
                    "프로필 '{}'의 인증정보 크기가 올바르지 않습니다",
                    selection.name
                ));
            }
            add_raw_total(&mut raw_total, credential.len())?;

            let oauth_account = if provider == Provider::Claude {
                if let Some(snapshot) = stable_claude.as_ref() {
                    Some(serde_json::to_vec(&snapshot.oauth_account).map_err(|_| {
                        format!(
                            "프로필 '{}'의 계정 정보가 올바르지 않습니다",
                            selection.name
                        )
                    })?)
                } else if active {
                    let block = accounts::claude_oauth_block(env)?.ok_or_else(|| {
                        format!("프로필 '{}'의 계정 정보가 없습니다", selection.name)
                    })?;
                    Some(serde_json::to_vec(&block).map_err(|_| {
                        format!(
                            "프로필 '{}'의 계정 정보가 올바르지 않습니다",
                            selection.name
                        )
                    })?)
                } else {
                    Some(
                        read_bounded_file(&dir.join("oauth_account.json"), OAUTH_MAX_BYTES)
                            .map_err(|_| {
                                format!(
                                    "프로필 '{}'의 계정 정보를 읽을 수 없습니다",
                                    selection.name
                                )
                            })?,
                    )
                }
            } else {
                None
            }
            .map(Zeroizing::new);

            if let Some(oauth) = oauth_account.as_ref() {
                add_raw_total(&mut raw_total, oauth.len())?;
            }

            validate_entry_bytes(
                provider,
                &selection.name,
                &identity_id,
                identity_email.as_deref(),
                &credential,
                oauth_account.as_deref().map(|bytes| bytes.as_slice()),
            )
            .map_err(|_| {
                format!(
                    "프로필 '{}'의 인증정보와 계정 정보가 일치하지 않습니다",
                    selection.name
                )
            })?;

            let entry = VaultEntry {
                provider: provider.dir_name().into(),
                name: selection.name.clone(),
                hide_email: selection.hide_email,
                id: identity_id,
                email: identity_email,
                credential: URL_SAFE_NO_PAD.encode(&credential[..]),
                oauth_account: oauth_account
                    .as_deref()
                    .map(|oauth| URL_SAFE_NO_PAD.encode(oauth.as_slice())),
            };
            entries.push(entry);
        }
        entries
    };
    drop(profile_guards);
    Ok(captured)
}

fn read_live_bounded(env: &Env, provider: Provider) -> Result<Vec<u8>, String> {
    let data = accounts::read_live_cred(env, provider)
        .map_err(|_| "활성 인증정보를 읽을 수 없습니다".to_string())?;
    if data.is_empty() || data.len() > CREDENTIAL_MAX_BYTES {
        return Err("활성 인증정보 크기가 올바르지 않습니다".into());
    }
    Ok(data)
}

fn read_claude_snapshot_once(env: &Env) -> Result<ClaudeLiveSnapshot, String> {
    let oauth_before = accounts::claude_oauth_block(env)?
        .ok_or_else(|| "활성 Claude 계정 정보가 없습니다".to_string())?;
    let credential = Zeroizing::new(read_live_bounded(env, Provider::Claude)?);
    let oauth_after = accounts::claude_oauth_block(env)?
        .ok_or_else(|| "활성 Claude 계정 정보가 없습니다".to_string())?;
    if oauth_before != oauth_after {
        return Err("Claude 로그인이 변경되는 중이라 내보내기를 중단했습니다".into());
    }
    let root = serde_json::json!({ "oauthAccount": oauth_after.clone() });
    let identity = accounts::identity_from_value(Provider::Claude, &root)
        .ok_or_else(|| "활성 Claude 계정 정보가 올바르지 않습니다".to_string())?;
    Ok(ClaudeLiveSnapshot {
        credential,
        oauth_account: oauth_after,
        identity,
    })
}

/// Claude CLI는 토큰과 oauthAccount를 서로 다른 저장소에 쓴다. 외부 로그인이
/// 동시에 진행되면 두 세대가 섞일 수 있으므로 두 값이 연속 세 번 같은 때만 쓴다.
fn read_stable_claude_snapshot(env: &Env) -> Result<ClaudeLiveSnapshot, String> {
    read_stable_claude_snapshot_with(
        || read_claude_snapshot_once(env),
        || std::thread::sleep(std::time::Duration::from_millis(120)),
    )
}

fn read_stable_claude_snapshot_with<R, P>(
    mut read: R,
    mut pause: P,
) -> Result<ClaudeLiveSnapshot, String>
where
    R: FnMut() -> Result<ClaudeLiveSnapshot, String>,
    P: FnMut(),
{
    let mut previous = read()?;
    let mut stable_intervals = 0usize;
    for _ in 0..5 {
        pause();
        let next = read()?;
        if previous.credential.as_slice() == next.credential.as_slice()
            && previous.oauth_account == next.oauth_account
        {
            stable_intervals += 1;
            if stable_intervals >= 2 {
                return Ok(next);
            }
        } else {
            stable_intervals = 0;
        }
        previous = next;
    }
    Err("Claude 로그인이 변경되는 중이라 내보내기를 중단했습니다 — 로그인이 끝난 뒤 다시 시도하세요".into())
}

fn read_bounded_file(path: &Path, limit: usize) -> Result<Vec<u8>, ()> {
    let file = fs::File::open(path).map_err(|_| ())?;
    let mut bytes = Vec::new();
    file.take((limit as u64) + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.is_empty() || bytes.len() > limit {
        return Err(());
    }
    Ok(bytes)
}

fn make_aad(header: &[u8]) -> Result<Vec<u8>, String> {
    let header_len = u32::try_from(header.len()).map_err(|_| CRYPTO_ERROR.to_string())?;
    let mut aad = Vec::with_capacity(MAGIC.len() + 1 + 4 + header.len());
    aad.extend_from_slice(MAGIC);
    aad.push(FORMAT_VERSION);
    aad.extend_from_slice(&header_len.to_be_bytes());
    aad.extend_from_slice(header);
    Ok(aad)
}

#[cfg(test)]
std::thread_local! {
    static KDF_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static DECODE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn kdf_call_count() -> usize {
    KDF_CALLS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn decode_call_count() -> usize {
    DECODE_CALLS.with(std::cell::Cell::get)
}

fn derive_key(secret: &[u8], salt: &[u8; 16]) -> Result<Zeroizing<[u8; 32]>, String> {
    #[cfg(test)]
    KDF_CALLS.with(|calls| calls.set(calls.get() + 1));
    let params = Params::new(
        ARGON_MEMORY_KIB,
        ARGON_ITERATIONS,
        ARGON_PARALLELISM,
        Some(ARGON_OUTPUT_BYTES),
    )
    .map_err(|_| CRYPTO_ERROR.to_string())?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into(secret, salt, &mut *key)
        .map_err(|_| CRYPTO_ERROR.to_string())?;
    Ok(key)
}

fn decrypt_file(path: &Path, recovery_code: &str) -> Result<ParsedVault, String> {
    let recovery_code = recovery_code.trim();
    if recovery_code.len() != 43
        || !recovery_code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(CRYPTO_ERROR.into());
    }
    let bytes = read_bounded_file(path, VAULT_MAX_BYTES).map_err(|_| CRYPTO_ERROR.to_string())?;
    let fixed = MAGIC.len() + 1 + 4;
    if bytes.len() < fixed + TAG_BYTES || &bytes[..MAGIC.len()] != MAGIC {
        return Err(CRYPTO_ERROR.into());
    }
    if bytes[MAGIC.len()] != FORMAT_VERSION {
        return Err(CRYPTO_ERROR.into());
    }
    let length_start = MAGIC.len() + 1;
    let header_len = u32::from_be_bytes(
        bytes[length_start..length_start + 4]
            .try_into()
            .map_err(|_| CRYPTO_ERROR.to_string())?,
    ) as usize;
    if header_len == 0 || header_len > HEADER_MAX_BYTES {
        return Err(CRYPTO_ERROR.into());
    }
    let header_end = fixed
        .checked_add(header_len)
        .filter(|end| *end <= bytes.len().saturating_sub(TAG_BYTES))
        .ok_or_else(|| CRYPTO_ERROR.to_string())?;
    let header: VaultHeader =
        serde_json::from_slice(&bytes[fixed..header_end]).map_err(|_| CRYPTO_ERROR.to_string())?;
    validate_header(&header, bytes.len() - header_end)?;

    let salt_vec = URL_SAFE_NO_PAD
        .decode(&header.salt)
        .map_err(|_| CRYPTO_ERROR.to_string())?;
    let nonce_vec = URL_SAFE_NO_PAD
        .decode(&header.nonce)
        .map_err(|_| CRYPTO_ERROR.to_string())?;
    let salt: [u8; 16] = salt_vec
        .as_slice()
        .try_into()
        .map_err(|_| CRYPTO_ERROR.to_string())?;
    let nonce: [u8; 12] = nonce_vec
        .as_slice()
        .try_into()
        .map_err(|_| CRYPTO_ERROR.to_string())?;

    let secret = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(recovery_code)
            .map_err(|_| CRYPTO_ERROR.to_string())?,
    );
    if secret.len() != 32 {
        return Err(CRYPTO_ERROR.into());
    }
    let key = derive_key(&secret, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key[..]).map_err(|_| CRYPTO_ERROR.to_string())?;
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                AeadPayload {
                    msg: &bytes[header_end..],
                    aad: &bytes[..header_end],
                },
            )
            .map_err(|_| CRYPTO_ERROR.to_string())?,
    );
    if plaintext.len() != header.payload_len as usize {
        return Err(CRYPTO_ERROR.into());
    }
    let payload: VaultPayload =
        serde_json::from_slice(&plaintext).map_err(|_| CRYPTO_ERROR.to_string())?;
    validate_payload(&payload).map_err(|_| CRYPTO_ERROR.to_string())?;
    Ok(ParsedVault { payload })
}

fn validate_header(header: &VaultHeader, ciphertext_len: usize) -> Result<(), String> {
    let payload_len = usize::try_from(header.payload_len).map_err(|_| CRYPTO_ERROR.to_string())?;
    let expected_ciphertext = payload_len
        .checked_add(TAG_BYTES)
        .ok_or_else(|| CRYPTO_ERROR.to_string())?;
    let valid = header.cipher == "AES-256-GCM"
        && header.kdf.algorithm == "Argon2id"
        && header.kdf.memory_kib == ARGON_MEMORY_KIB
        && header.kdf.iterations == ARGON_ITERATIONS
        && header.kdf.parallelism == ARGON_PARALLELISM
        && header.kdf.output_len == ARGON_OUTPUT_BYTES as u32
        && payload_len <= PAYLOAD_MAX_BYTES
        && ciphertext_len == expected_ciphertext;
    if valid {
        Ok(())
    } else {
        Err(CRYPTO_ERROR.into())
    }
}

fn validate_payload(payload: &VaultPayload) -> Result<(), ()> {
    if payload.format_version != PAYLOAD_VERSION
        || payload.entries.is_empty()
        || payload.entries.len() > MAX_ENTRIES
    {
        return Err(());
    }
    // Base64 디코드보다 먼저 각 필드와 누적 문자열 크기를 검사한다. 공격 파일이
    // 큰 문자열 여러 개로 디코더 할당을 유도하지 못하게 한다.
    let encoded_total_limit = encoded_max_len(RAW_TOTAL_MAX_BYTES)
        .checked_add(MAX_ENTRIES * 2)
        .ok_or(())?;
    let mut encoded_total = 0usize;
    let mut names = HashSet::new();
    let mut identities = HashSet::new();
    for entry in &payload.entries {
        let provider = validate_entry_metadata(entry)?;
        validate_encoded_field(&entry.credential, CREDENTIAL_MAX_BYTES)?;
        if provider == Provider::Claude {
            validate_encoded_field(entry.oauth_account.as_deref().ok_or(())?, OAUTH_MAX_BYTES)?;
        } else if entry.oauth_account.is_some() {
            return Err(());
        }
        encoded_total = encoded_total
            .checked_add(entry.credential.len())
            .and_then(|total| {
                entry
                    .oauth_account
                    .as_ref()
                    .map_or(Some(total), |oauth| total.checked_add(oauth.len()))
            })
            .filter(|total| *total <= encoded_total_limit)
            .ok_or(())?;
        if !names.insert(name_key(provider, &entry.name))
            || !identities.insert((provider.dir_name(), entry.id.as_str()))
        {
            return Err(());
        }
    }

    let mut raw_total = 0usize;
    for entry in &payload.entries {
        let provider = Provider::parse(&entry.provider).map_err(|_| ())?;
        let credential = decode_field(&entry.credential, CREDENTIAL_MAX_BYTES)?;
        let oauth = entry
            .oauth_account
            .as_deref()
            .map(|encoded| decode_field(encoded, OAUTH_MAX_BYTES))
            .transpose()?;
        add_raw_total_unit(&mut raw_total, credential.len())?;
        if let Some(oauth) = oauth.as_ref() {
            add_raw_total_unit(&mut raw_total, oauth.len())?;
        }
        validate_entry_bytes(
            provider,
            &entry.name,
            &entry.id,
            entry.email.as_deref(),
            &credential,
            oauth.as_deref().map(|bytes| bytes.as_slice()),
        )?;
    }
    Ok(())
}

fn validate_entry_metadata(entry: &VaultEntry) -> Result<Provider, ()> {
    let provider = Provider::parse(&entry.provider).map_err(|_| ())?;
    accounts::validate_name(&entry.name).map_err(|_| ())?;
    if entry.id.is_empty()
        || entry.id.len() > 512
        || entry.id.chars().any(char::is_control)
        || entry.email.as_ref().is_some_and(|email| {
            email.is_empty() || email.len() > 512 || email.chars().any(char::is_control)
        })
    {
        return Err(());
    }
    Ok(provider)
}

fn validate_entry_bytes(
    provider: Provider,
    name: &str,
    id: &str,
    email: Option<&str>,
    credential: &[u8],
    oauth_account: Option<&[u8]>,
) -> Result<(), ()> {
    accounts::validate_name(name).map_err(|_| ())?;
    if credential.is_empty() || credential.len() > CREDENTIAL_MAX_BYTES {
        return Err(());
    }

    match provider {
        Provider::Claude => {
            let credential: ClaudeCredentialProbe =
                serde_json::from_slice(credential).map_err(|_| ())?;
            if credential.claude_ai_oauth.access_token.is_empty() {
                return Err(());
            }
            let oauth: ClaudeOauthProbe =
                serde_json::from_slice(oauth_account.ok_or(())?).map_err(|_| ())?;
            if oauth.account_uuid != id || oauth.email_address.as_deref() != email {
                return Err(());
            }
        }
        Provider::Codex => {
            if oauth_account.is_some() {
                return Err(());
            }
            let root: serde_json::Value = serde_json::from_slice(credential).map_err(|_| ())?;
            if root
                .pointer("/tokens/access_token")
                .and_then(serde_json::Value::as_str)
                .is_none_or(str::is_empty)
            {
                return Err(());
            }
            // 계정 판정 정책은 accounts의 정본을 그대로 쓴다: account_id 우선,
            // 없을 때만 JWT sub. 둘이 달라도 기존 프로필 판정과 동일하게 허용한다.
            let identity = accounts::identity_from_value(Provider::Codex, &root).ok_or(())?;
            if identity.id != id || identity.email.as_deref() != email {
                return Err(());
            }
        }
    }
    Ok(())
}

fn encoded_max_len(raw_limit: usize) -> usize {
    raw_limit.saturating_mul(4).saturating_add(2) / 3
}

fn validate_encoded_field(encoded: &str, raw_limit: usize) -> Result<(), ()> {
    if encoded.is_empty()
        || encoded.len() > encoded_max_len(raw_limit)
        || !encoded
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(());
    }
    Ok(())
}

fn decode_field(encoded: &str, raw_limit: usize) -> Result<Zeroizing<Vec<u8>>, ()> {
    validate_encoded_field(encoded, raw_limit)?;
    #[cfg(test)]
    DECODE_CALLS.with(|calls| calls.set(calls.get() + 1));
    let bytes = Zeroizing::new(URL_SAFE_NO_PAD.decode(encoded).map_err(|_| ())?);
    if bytes.is_empty() || bytes.len() > raw_limit {
        return Err(());
    }
    Ok(bytes)
}

fn add_raw_total(total: &mut usize, add: usize) -> Result<(), String> {
    add_raw_total_unit(total, add)
        .map_err(|_| "한 번에 내보낼 인증정보 전체 크기를 넘었습니다".to_string())
}

fn add_raw_total_unit(total: &mut usize, add: usize) -> Result<(), ()> {
    *total = total
        .checked_add(add)
        .filter(|total| *total <= RAW_TOTAL_MAX_BYTES)
        .ok_or(())?;
    Ok(())
}

struct PreparedImport {
    entry_index: usize,
    provider: Provider,
    stage_dir: PathBuf,
    final_dir: PathBuf,
    credential: Zeroizing<Vec<u8>>,
    oauth_account: Option<Zeroizing<Vec<u8>>>,
}

const IMPORT_JOURNAL_FILE: &str = "journal.json";
const IMPORT_COMMITTED_FILE: &str = "committed.json";
const IMPORT_MARKER_FILE: &str = accounts::PROFILE_IMPORT_MARKER;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportJournal {
    version: u32,
    id: String,
    state: String,
    entries: Vec<ImportJournalEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportJournalEntry {
    provider: String,
    name: String,
}

fn commit_payload<F>(
    env: &Env,
    payload: &VaultPayload,
    mut before_rename: F,
) -> Result<VaultImportResult, String>
where
    F: FnMut(usize, &Path) -> Result<(), String>,
{
    let _mutation = MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    recover_stale_imports_locked(env)?;

    let mut existing_ids: HashSet<(String, String)> = HashSet::new();
    let mut occupied_names: HashSet<(String, String)> = HashSet::new();
    for provider in [Provider::Claude, Provider::Codex] {
        if accounts::live_cred_exists(env, provider)? {
            let live = accounts::live_identity(env, provider)?
                .ok_or("현재 로그인 계정을 식별할 수 없습니다 (로그인 직후 다시 시도)")?;
            existing_ids.insert((provider.dir_name().to_string(), live.id));
        }
        for (name, dir) in accounts::profile_dirs(env, provider)? {
            occupied_names.insert(name_key(provider, &name));
            if let Some(meta) = accounts::read_meta(&dir) {
                existing_ids.insert((provider.dir_name().to_string(), meta.id));
            }
        }
    }

    let mut planned: Vec<(usize, Provider, String)> = Vec::new();
    let mut skipped = 0usize;
    for (entry_index, entry) in payload.entries.iter().enumerate() {
        let provider = Provider::parse(&entry.provider).map_err(|_| CRYPTO_ERROR.to_string())?;
        let identity_key = (provider.dir_name().to_string(), entry.id.clone());
        if existing_ids.contains(&identity_key) {
            skipped += 1;
            continue;
        }
        let name = available_name(provider, &entry.name, &occupied_names)?;
        occupied_names.insert(name_key(provider, &name));
        existing_ids.insert(identity_key);
        planned.push((entry_index, provider, name));
    }

    if planned.is_empty() {
        return Ok(VaultImportResult {
            imported: 0,
            skipped,
            cleanup_pending: false,
        });
    }

    let (stage_root, import_id) = create_stage_root(env)?;
    let mut journal = ImportJournal {
        version: 1,
        id: import_id.clone(),
        state: "staging".into(),
        entries: planned
            .iter()
            .map(|(_, provider, name)| ImportJournalEntry {
                provider: provider.dir_name().into(),
                name: name.clone(),
            })
            .collect(),
    };
    write_import_journal(&stage_root, &journal)?;
    let mut prepared = Vec::with_capacity(planned.len());
    let stage_result: Result<(), String> = (|| {
        for (entry_index, provider, name) in &planned {
            let entry = &payload.entries[*entry_index];
            let credential = decode_field(&entry.credential, CREDENTIAL_MAX_BYTES)
                .map_err(|_| "가져오기 준비에 실패했습니다")?;
            let oauth_account = entry
                .oauth_account
                .as_deref()
                .map(|encoded| decode_field(encoded, OAUTH_MAX_BYTES))
                .transpose()
                .map_err(|_| "가져오기 준비에 실패했습니다")?;

            // stage에는 무작위 소유 표식과 journal만 둔다. 토큰·계정 정보 평문은
            // 절대 임시 폴더에 쓰지 않고, 이 marked 폴더를 최종 위치로 옮긴 뒤 쓴다.
            let stage_dir = stage_root.join(provider.dir_name()).join(name);
            fs::create_dir_all(&stage_dir).map_err(|_| "가져오기 준비에 실패했습니다")?;
            accounts::atomic_write(&stage_dir.join(IMPORT_MARKER_FILE), import_id.as_bytes())
                .map_err(|_| "가져오기 준비에 실패했습니다")?;

            let final_parent = env.profiles_dir(*provider);
            fs::create_dir_all(&final_parent).map_err(|_| "가져오기 준비에 실패했습니다")?;
            prepared.push(PreparedImport {
                entry_index: *entry_index,
                provider: *provider,
                stage_dir,
                final_dir: final_parent.join(name),
                credential,
                oauth_account,
            });
        }
        Ok(())
    })();
    if let Err(error) = stage_result {
        return if remove_tree_retry(&stage_root) {
            Err(error)
        } else {
            Err("가져오기 준비 실패 뒤 임시 작업 폴더를 정리하지 못했습니다".into())
        };
    }

    let mut committed: Vec<PathBuf> = Vec::new();
    for (index, item) in prepared.iter().enumerate() {
        let result = (|| {
            before_rename(index, &item.final_dir)?;
            if item.final_dir.exists() {
                return Err("가져오기 중 프로필 이름이 충돌했습니다".into());
            }
            fs::rename(&item.stage_dir, &item.final_dir)
                .map_err(|_| "가져오기 반영에 실패했습니다".to_string())?;
            // rename 뒤부터는 journal+marker가 이 최종 폴더의 소유권을 증명한다.
            // 먼저 rollback 목록에 넣어 이후 어떤 쓰기가 실패해도 반드시 지운다.
            committed.push(item.final_dir.clone());
            let entry = &payload.entries[item.entry_index];
            let identity = accounts::LiveIdentity {
                id: entry.id.clone(),
                email: entry.email.clone(),
            };
            accounts::write_new_marked_profile_bundle_to_dir(
                &item.final_dir,
                item.provider,
                &identity,
                &item.credential,
                item.oauth_account.as_deref().map(|bytes| bytes.as_slice()),
                entry.hide_email,
            )
            .map_err(|_| "가져오기 반영에 실패했습니다".to_string())?;
            Ok(())
        })();
        if let Err(error) = result {
            let mut rollback_ok = true;
            for path in committed.iter().rev() {
                rollback_ok &= remove_import_profile_retry(path, &import_id);
            }
            // 하나라도 최종 폴더가 남으면 소유권 journal도 반드시 보존해 다음
            // 시작에서 다시 원복한다.
            if rollback_ok {
                rollback_ok = remove_tree_retry(&stage_root);
            }
            return if rollback_ok {
                Err(error)
            } else {
                Err("가져오기 실패 뒤 새 프로필을 모두 원복하지 못했습니다".into())
            };
        }
    }

    // 이 원자적 상태 변경이 crash 경계다. staging이면 다음 시작에서 모두 원복하고,
    // committed이면 최종 프로필은 보존한 채 표식만 청소한다.
    journal.state = "committed".into();
    if write_import_journal(&stage_root, &journal).is_err() {
        let mut rollback_ok = true;
        for path in committed.iter().rev() {
            rollback_ok &= remove_import_profile_retry(path, &import_id);
        }
        if rollback_ok {
            rollback_ok = remove_tree_retry(&stage_root);
        }
        return Err(if rollback_ok {
            "가져오기 완료 상태를 기록하지 못해 새 프로필을 원복했습니다".into()
        } else {
            "가져오기 완료 상태 기록과 원복에 실패했습니다".into()
        });
    }
    // journal은 staging→committed 전환의 기준이고, committed 복제본은 marker를
    // 일부 지운 뒤 journal을 잠시 읽지 못해도 전체 항목을 계속 보이게 한다.
    let cleanup_pending = if write_committed_import(&stage_root, &journal).is_err() {
        true
    } else {
        !cleanup_committed_import(&stage_root, &journal)
    };
    Ok(VaultImportResult {
        imported: committed.len(),
        skipped,
        cleanup_pending,
    })
}

fn write_import_journal(stage_root: &Path, journal: &ImportJournal) -> Result<(), String> {
    let bytes = serde_json::to_vec(journal).map_err(|_| "가져오기 기록 생성에 실패했습니다")?;
    accounts::atomic_write(&stage_root.join(IMPORT_JOURNAL_FILE), &bytes)
        .map_err(|_| "가져오기 기록 저장에 실패했습니다".to_string())
}

fn write_committed_import(stage_root: &Path, journal: &ImportJournal) -> Result<(), String> {
    if journal.state != "committed" {
        return Err("완료되지 않은 가져오기 기록입니다".into());
    }
    let bytes =
        serde_json::to_vec(journal).map_err(|_| "가져오기 완료 기록 생성에 실패했습니다")?;
    accounts::atomic_write(&stage_root.join(IMPORT_COMMITTED_FILE), &bytes)
        .map_err(|_| "가져오기 완료 기록 저장에 실패했습니다".to_string())
}

fn cleanup_committed_import(stage_root: &Path, journal: &ImportJournal) -> bool {
    let Some(store) = stage_root.parent() else {
        return false;
    };
    let mut markers = Vec::new();
    for entry in &journal.entries {
        let Ok(provider) = Provider::parse(&entry.provider) else {
            return false;
        };
        let final_dir = store
            .join(provider.dir_name())
            .join("profiles")
            .join(&entry.name);
        let marker = final_dir.join(IMPORT_MARKER_FILE);
        match fs::symlink_metadata(&marker) {
            Ok(metadata)
                if metadata.file_type().is_file() && marker_matches(&marker, &journal.id) =>
            {
                markers.push(marker);
            }
            Ok(_) => return false,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return false,
        }
    }

    // 모든 marker를 먼저 검증해, 뒤쪽 marker 하나가 잠겼다고 앞쪽 항목만
    // 보이는 중간 상태를 만들지 않는다.
    for marker in markers {
        if !remove_file_retry(&marker) {
            return false;
        }
    }
    remove_tree_retry(stage_root)
}

pub(crate) fn recover_stale_imports(env: &Env) -> Result<(), String> {
    let _mutation = MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    recover_stale_imports_locked(env)
}

pub(crate) fn recover_stale_imports_with_wait<F>(env: &Env, mut wait: F) -> Result<(), String>
where
    F: FnMut(std::time::Duration),
{
    let mut last_error = match recover_stale_imports(env) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    for delay_ms in [250, 1_000, 4_000, 15_000] {
        wait(std::time::Duration::from_millis(delay_ms));
        match recover_stale_imports(env) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

fn recover_stale_imports_locked(env: &Env) -> Result<(), String> {
    if !env.store.is_dir() {
        return Ok(());
    }
    let entries =
        fs::read_dir(&env.store).map_err(|_| "이전 인증정보 가져오기 상태를 확인할 수 없습니다")?;
    let mut recovery_error = None;
    let mut stage_roots = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                recovery_error.get_or_insert_with(|| {
                    "이전 인증정보 가져오기 상태를 확인할 수 없습니다".to_string()
                });
                continue;
            }
        };
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => {
                recovery_error.get_or_insert_with(|| {
                    "이전 인증정보 가져오기 상태를 확인할 수 없습니다".to_string()
                });
                continue;
            }
        };
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(import_id) = name.strip_prefix(".vault-import-") else {
            continue;
        };
        if !valid_import_id(import_id) {
            continue;
        }
        stage_roots.push((entry.path(), import_id.to_string()));
    }
    stage_roots.sort_by(|a, b| a.0.cmp(&b.0));
    for (stage_root, import_id) in stage_roots {
        if let Err(error) = recover_stage_root(env, &stage_root, &import_id) {
            recovery_error.get_or_insert(error);
        }
    }
    match recovery_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn recover_stage_root(env: &Env, stage_root: &Path, import_id: &str) -> Result<(), String> {
    let journal = read_import_record(stage_root, IMPORT_JOURNAL_FILE, import_id, false);
    let committed = read_import_record(stage_root, IMPORT_COMMITTED_FILE, import_id, true);

    // 복제본은 journal committed 기록 뒤에만 생긴다. 둘이 충돌하거나 journal을
    // 읽지 못할 때도 이 내구성 있는 완료 기록을 우선한다.
    if let ImportRecord::Valid(committed) = &committed {
        return if cleanup_committed_import(stage_root, committed) {
            Ok(())
        } else {
            Err("완료된 가져오기 표식을 정리하지 못했습니다".into())
        };
    }

    let journal = match journal {
        ImportRecord::Valid(journal) => journal,
        ImportRecord::Missing if matches!(&committed, ImportRecord::Missing) => {
            return if remove_tree_retry(stage_root) {
                Ok(())
            } else {
                Err("이전 가져오기 임시 폴더를 정리하지 못했습니다".into())
            };
        }
        ImportRecord::Missing | ImportRecord::Invalid => {
            return Err("이전 가져오기 기록을 읽을 수 없습니다".into());
        }
    };

    if journal.state == "committed" {
        write_committed_import(stage_root, &journal)?;
        return if cleanup_committed_import(stage_root, &journal) {
            Ok(())
        } else {
            Err("완료된 가져오기 표식을 정리하지 못했습니다".into())
        };
    }

    // staging journal 옆에 읽을 수 없는 committed 파일이 있으면 어느 상태가
    // 마지막인지 단정할 수 없으므로 원복하지 않고 그대로 보존한다.
    if matches!(&committed, ImportRecord::Invalid) {
        return Err("이전 가져오기 완료 기록을 읽을 수 없습니다".into());
    }

    let mut rollback_ok = true;
    for entry in journal.entries.iter().rev() {
        let provider = Provider::parse(&entry.provider)
            .map_err(|_| "이전 가져오기 기록이 올바르지 않습니다")?;
        let final_dir = env.profiles_dir(provider).join(&entry.name);
        let marker = final_dir.join(IMPORT_MARKER_FILE);
        if marker_matches(&marker, &journal.id) {
            rollback_ok &= remove_import_profile_retry(&final_dir, &journal.id);
        } else if marker.exists() {
            // 표식은 있는데 읽지 못하거나 다른 값이면 이 journal을 지워선 안 된다.
            rollback_ok = false;
        } else if final_dir.is_dir()
            && fs::read_dir(&final_dir)
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(false)
        {
            // marker를 마지막에 지운 직후 process가 죽으면 빈 최종 폴더만 남는다.
            // 이 한 경우만 소유권 표식 없이도 안전하게 정리할 수 있다.
            rollback_ok &= remove_empty_dir_retry(&final_dir);
        } else if final_dir.exists() {
            // 표식 없는 비어 있지 않은 폴더는 이 journal 소유인지 증명할 수 없다.
            // 폴더와 소유권 기록을 모두 보존해 다음 복구에서 다시 판단하게 한다.
            rollback_ok = false;
        }
    }
    if rollback_ok {
        rollback_ok = remove_tree_retry(stage_root);
    }
    if rollback_ok {
        Ok(())
    } else {
        Err("중단된 인증정보 가져오기를 모두 원복하지 못했습니다".into())
    }
}

fn valid_import_journal(journal: &ImportJournal, import_id: &str) -> bool {
    journal.version == 1
        && journal.id == import_id
        && matches!(journal.state.as_str(), "staging" | "committed")
        && !journal.entries.is_empty()
        && journal.entries.len() <= MAX_ENTRIES
        && journal.entries.iter().all(|entry| {
            Provider::parse(&entry.provider).is_ok() && accounts::validate_name(&entry.name).is_ok()
        })
}

enum ImportRecord {
    Missing,
    Valid(ImportJournal),
    Invalid,
}

fn read_import_record(
    stage_root: &Path,
    file_name: &str,
    import_id: &str,
    committed_only: bool,
) -> ImportRecord {
    let path = stage_root.join(file_name);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ImportRecord::Missing;
        }
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) | Err(_) => return ImportRecord::Invalid,
    }
    let Ok(bytes) = read_bounded_file(&path, 256 * 1024) else {
        return ImportRecord::Invalid;
    };
    let Ok(journal) = serde_json::from_slice::<ImportJournal>(&bytes) else {
        return ImportRecord::Invalid;
    };
    if !valid_import_journal(&journal, import_id)
        || (committed_only && journal.state != "committed")
    {
        return ImportRecord::Invalid;
    }
    ImportRecord::Valid(journal)
}

fn valid_import_id(import_id: &str) -> bool {
    import_id.len() == 24
        && import_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn committed_stage_contains(
    stage_root: &Path,
    import_id: &str,
    provider: Provider,
    name: &str,
) -> bool {
    let contains = |journal: &ImportJournal| {
        journal.state == "committed"
            && journal
                .entries
                .iter()
                .any(|entry| entry.provider == provider.dir_name() && entry.name == name)
    };
    match read_import_record(stage_root, IMPORT_COMMITTED_FILE, import_id, true) {
        ImportRecord::Valid(committed) => contains(&committed),
        ImportRecord::Missing | ImportRecord::Invalid => {
            match read_import_record(stage_root, IMPORT_JOURNAL_FILE, import_id, false) {
                ImportRecord::Valid(journal) => contains(&journal),
                ImportRecord::Missing | ImportRecord::Invalid => false,
            }
        }
    }
}

pub(crate) fn profile_import_blocked(env: &Env, provider: Provider, name: &str) -> bool {
    let marker = env
        .profiles_dir(provider)
        .join(name)
        .join(IMPORT_MARKER_FILE);
    match fs::symlink_metadata(&marker) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Ok(_) | Err(_) => {}
    }

    let marker_id = read_bounded_file(&marker, 64)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .filter(|import_id| valid_import_id(import_id));
    if let Some(import_id) = marker_id.as_deref() {
        if committed_stage_contains(
            &env.store.join(format!(".vault-import-{import_id}")),
            import_id,
            provider,
            name,
        ) {
            return false;
        }
    }

    let committed = fs::read_dir(&env.store)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            let import_name = entry.file_name().to_string_lossy().to_string();
            let Some(import_id) = import_name.strip_prefix(".vault-import-") else {
                return false;
            };
            valid_import_id(import_id)
                && committed_stage_contains(&entry.path(), import_id, provider, name)
        });
    !committed
}

fn marker_matches(marker: &Path, expected: &str) -> bool {
    read_bounded_file(marker, 64)
        .ok()
        .is_some_and(|bytes| bytes == expected.as_bytes())
}

fn remove_file_retry(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    for attempt in 0..4 {
        match fs::remove_file(path) {
            Ok(()) => return true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) if attempt < 3 => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return false,
        }
    }
    false
}

/// rollback 대상 프로필은 marker를 마지막까지 남겨 일반 목록·전환에서 숨긴다.
/// Windows에서 토큰 파일이 잠겨 삭제가 실패해도 marker가 먼저 사라지지 않는다.
fn remove_import_profile_retry(path: &Path, import_id: &str) -> bool {
    if !path.exists() {
        return true;
    }
    let marker = path.join(IMPORT_MARKER_FILE);
    if !marker_matches(&marker, import_id) {
        return false;
    }

    for attempt in 0..4 {
        let Ok(entries) = fs::read_dir(path) else {
            return false;
        };
        let mut children_removed = true;
        for entry in entries {
            let Ok(entry) = entry else {
                children_removed = false;
                break;
            };
            if entry.file_name() == std::ffi::OsStr::new(IMPORT_MARKER_FILE) {
                continue;
            }
            let child = entry.path();
            let removed = entry
                .file_type()
                .map(|kind| {
                    if kind.is_dir() {
                        remove_tree_retry(&child)
                    } else {
                        remove_file_retry(&child)
                    }
                })
                .unwrap_or(false);
            children_removed &= removed;
        }
        if !children_removed {
            if attempt < 3 {
                std::thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
            return false;
        }
        if !directory_contains_only_import_marker(path) {
            if attempt < 3 {
                std::thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
            return false;
        }

        if !remove_file_retry(&marker) {
            return false;
        }
        match fs::remove_dir(path) {
            Ok(()) => return true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => {
                // 빈 디렉터리 삭제만 실패했으면 marker를 즉시 복원해 숨김을 유지한다.
                if accounts::atomic_write(&marker, import_id.as_bytes()).is_err() {
                    return false;
                }
                if attempt < 3 {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
    }
    false
}

fn directory_contains_only_import_marker(path: &Path) -> bool {
    let Ok(entries) = fs::read_dir(path) else {
        return false;
    };
    let mut saw_marker = false;
    for entry in entries {
        let Ok(entry) = entry else {
            return false;
        };
        if entry.file_name() != std::ffi::OsStr::new(IMPORT_MARKER_FILE) {
            return false;
        }
        saw_marker = true;
    }
    saw_marker
}

fn remove_empty_dir_retry(path: &Path) -> bool {
    for attempt in 0..4 {
        match fs::remove_dir(path) {
            Ok(()) => return true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) if attempt < 3 => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return false,
        }
    }
    false
}

fn remove_tree_retry(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    for attempt in 0..4 {
        match fs::remove_dir_all(path) {
            Ok(()) => return true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) if attempt < 3 => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return false,
        }
    }
    false
}

fn available_name(
    provider: Provider,
    requested: &str,
    occupied: &HashSet<(String, String)>,
) -> Result<String, String> {
    if !occupied.contains(&name_key(provider, requested)) {
        return Ok(requested.to_string());
    }
    for suffix_number in 2u32..=999_999 {
        let suffix = format!("-{suffix_number}");
        let base_len = 32usize.saturating_sub(suffix.len());
        if base_len == 0 {
            break;
        }
        let base = &requested[..requested.len().min(base_len)];
        let candidate = format!("{base}{suffix}");
        if !occupied.contains(&name_key(provider, &candidate)) {
            return Ok(candidate);
        }
    }
    Err("가져올 프로필 이름을 만들 수 없습니다".into())
}

/// Windows와 일반적인 macOS 볼륨은 파일 이름의 ASCII 대소문자를 구분하지 않는다.
/// 암호 파일을 두 플랫폼에서 같은 결과로 가져오도록 더 엄격한 쪽에 맞춘다.
fn name_key(provider: Provider, name: &str) -> (String, String) {
    (provider.dir_name().to_string(), name.to_ascii_lowercase())
}

fn create_stage_root(env: &Env) -> Result<(PathBuf, String), String> {
    fs::create_dir_all(&env.store).map_err(|_| "가져오기 준비에 실패했습니다")?;
    for _ in 0..8 {
        let mut random = [0u8; 12];
        OsRng
            .try_fill_bytes(&mut random)
            .map_err(|_| "가져오기 준비에 실패했습니다")?;
        let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let path = env.store.join(format!(".vault-import-{suffix}"));
        match fs::create_dir(&path) {
            Ok(()) => return Ok((path, suffix)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("가져오기 준비에 실패했습니다".into()),
        }
    }
    Err("가져오기 준비에 실패했습니다".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::test_support::test_env;
    use crate::accounts::{atomic_write, LiveIdentity};
    #[cfg(target_os = "macos")]
    use crate::accounts::ClaudeLiveStore;
    use serde_json::Value;

    #[cfg(target_os = "macos")]
    static KEYCHAIN_TEST_SEQ: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);

    #[cfg(target_os = "macos")]
    struct KeychainFixture {
        service: String,
        account: String,
    }

    #[cfg(target_os = "macos")]
    impl Drop for KeychainFixture {
        fn drop(&mut self) {
            let _ = accounts::keychain::delete_item(&self.service, &self.account);
        }
    }

    #[cfg(target_os = "macos")]
    fn keychain_test_env(tag: &str) -> (Env, KeychainFixture) {
        use std::sync::atomic::Ordering;

        let mut env = test_env(tag);
        let sequence = KEYCHAIN_TEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let service = format!(
            "switcher-vault-selftest-{}-{sequence}",
            std::process::id()
        );
        let account = accounts::keychain::username();
        let legacy_file = env.live_credential_path(Provider::Claude);
        env.claude_live = ClaudeLiveStore::Keychain {
            service: service.clone(),
            account: account.clone(),
            legacy_file,
        };
        (env, KeychainFixture { service, account })
    }

    #[cfg(target_os = "macos")]
    fn write_keychain_fixture(fixture: &KeychainFixture, bytes: &[u8]) {
        accounts::keychain::write_item(
            &fixture.service,
            &fixture.account,
            bytes,
        )
        .unwrap();
    }

    #[cfg(target_os = "macos")]
    fn lower_hex(bytes: &[u8]) -> Vec<u8> {
        bytes
            .iter()
            .flat_map(|byte| format!("{byte:02x}").into_bytes())
            .collect()
    }

    fn jwt(id: &str, email: &str) -> String {
        let payload = URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&serde_json::json!({"sub": id, "email": email})).unwrap());
        format!("e30.{payload}.fixture")
    }

    fn add_claude(env: &Env, name: &str, id: &str, email: &str, token: &str) {
        accounts::write_profile_parts(
            env,
            Provider::Claude,
            name,
            &LiveIdentity {
                id: id.into(),
                email: Some(email.into()),
            },
            format!(r#"{{"claudeAiOauth":{{"accessToken":"{token}"}}}}"#).as_bytes(),
            Some(&serde_json::json!({"accountUuid": id, "emailAddress": email})),
        )
        .unwrap();
    }

    fn add_codex(env: &Env, name: &str, id: &str, email: &str, token: &str) {
        let credential = serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": jwt(id, email),
                "access_token": token,
                "refresh_token": format!("refresh-{token}"),
                "account_id": id
            }
        });
        accounts::write_profile_parts(
            env,
            Provider::Codex,
            name,
            &LiveIdentity {
                id: id.into(),
                email: Some(email.into()),
            },
            &serde_json::to_vec(&credential).unwrap(),
            None,
        )
        .unwrap();
    }

    fn selection(env: &Env, provider: &str, name: &str, hide_email: bool) -> VaultSelection {
        let parsed_provider = Provider::parse(provider).unwrap();
        let meta = accounts::read_meta(&env.profiles_dir(parsed_provider).join(name)).unwrap();
        VaultSelection {
            provider: provider.into(),
            name: name.into(),
            revision: profile_revision(parsed_provider, &meta.id),
            hide_email,
        }
    }

    fn vault_path(env: &Env, name: &str) -> PathBuf {
        env.home.join(name)
    }

    fn encoded(bytes: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(bytes)
    }

    fn journal(id: &str, state: &str, provider: Provider, name: &str) -> ImportJournal {
        ImportJournal {
            version: 1,
            id: id.into(),
            state: state.into(),
            entries: vec![ImportJournalEntry {
                provider: provider.dir_name().into(),
                name: name.into(),
            }],
        }
    }

    fn add_marked_claude(
        env: &Env,
        name: &str,
        identity: &str,
        email: &str,
        import_id: &str,
    ) -> PathBuf {
        let dir = env.profiles_dir(Provider::Claude).join(name);
        accounts::write_profile_bundle_to_dir(
            &dir,
            Provider::Claude,
            &LiveIdentity {
                id: identity.into(),
                email: Some(email.into()),
            },
            br#"{"claudeAiOauth":{"accessToken":"fixture-import-token"}}"#,
            Some(&serde_json::json!({
                "accountUuid": identity,
                "emailAddress": email
            })),
            false,
        )
        .unwrap();
        atomic_write(&dir.join(IMPORT_MARKER_FILE), import_id.as_bytes()).unwrap();
        dir
    }

    #[test]
    fn vault_profile_revision_is_stable_opaque_and_identity_specific() {
        let env = test_env("vault-profile-revision");
        add_claude(
            &env,
            "first",
            "first-private-id",
            "first-private@example.test",
            "first-token",
        );
        add_claude(
            &env,
            "second",
            "second-private-id",
            "second-private@example.test",
            "second-token",
        );

        let first_read = list_profiles(&env).unwrap();
        let second_read = list_profiles(&env).unwrap();
        let revision = |profiles: &[VaultProfile], name: &str| {
            profiles
                .iter()
                .find(|profile| profile.name == name)
                .unwrap()
                .revision
        };
        assert_eq!(
            revision(&first_read, "first"),
            revision(&second_read, "first")
        );
        assert_ne!(
            revision(&first_read, "first"),
            revision(&first_read, "second")
        );
        assert!(first_read
            .iter()
            .all(|profile| profile.revision <= JS_SAFE_U64_MASK));

        let serialized = serde_json::to_string(&first_read).unwrap();
        for private in [
            "first-private-id",
            "second-private-id",
            "first-private@example.test",
            "second-private@example.test",
        ] {
            assert!(!serialized.contains(private));
        }
        for profile in serde_json::to_value(&first_read)
            .unwrap()
            .as_array()
            .unwrap()
        {
            let fields = profile.as_object().unwrap();
            assert_eq!(fields.len(), 4);
            assert!(fields.contains_key("provider"));
            assert!(fields.contains_key("name"));
            assert!(fields.contains_key("active"));
            assert!(fields.contains_key("revision"));
        }
    }

    #[test]
    fn export_rejects_an_alias_replaced_after_selection() {
        let env = test_env("vault-profile-replaced-after-selection");
        add_claude(
            &env,
            "work",
            "selected-identity",
            "selected@example.test",
            "selected-token",
        );
        let selected = selection(&env, "claude", "work", false);
        add_claude(
            &env,
            "work",
            "replacement-identity",
            "replacement@example.test",
            "replacement-token",
        );

        let error = export(
            &env,
            &vault_path(&env, "replaced.switcher-vault"),
            vec![selected],
        )
        .unwrap_err();

        assert!(error.contains("선택한 뒤 변경되었습니다"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_path_guards_follow_case_insensitive_apfs_semantics() {
        let store = Path::new("/Users/test/.switcher");
        assert!(path_is_within(
            Path::new("/Users/test/.SWITCHER/export.switcher-vault"),
            store
        ));
        assert!(!path_is_within(
            Path::new("/Users/test/.switcher-backup/export.switcher-vault"),
            store
        ));
        assert!(same_path(
            Path::new("/Users/test/.CLAUDE/.Credentials.JSON"),
            Path::new("/Users/test/.claude/.credentials.json")
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_active_claude_export_normalizes_raw_and_hex_keychain_without_mutation() {
        let (source, keychain) = keychain_test_env("vault-macos-keychain-export");
        let identity = "mac-keychain-id";
        let email = "mac-keychain@example.test";
        add_claude(
            &source,
            "active",
            identity,
            email,
            "stale-profile-token",
        );
        atomic_write(
            &source.home.join(".claude.json"),
            serde_json::to_vec(&serde_json::json!({
                "oauthAccount": {
                    "accountUuid": identity,
                    "emailAddress": email
                }
            }))
            .unwrap()
            .as_slice(),
        )
        .unwrap();

        let live = br#"{"claudeAiOauth":{"accessToken":"mac-live-keychain-token"}}"#;
        for (suffix, stored) in [("raw", live.to_vec()), ("hex", lower_hex(live))] {
            write_keychain_fixture(&keychain, &stored);
            let before = accounts::keychain::read_item(&keychain.service, &keychain.account)
                .unwrap()
                .unwrap();
            let path = vault_path(&source, &format!("keychain-{suffix}.switcher-vault"));
            let result = export(
                &source,
                &path,
                vec![selection(&source, "claude", "active", false)],
            )
            .unwrap();
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600,
                "vault 파일은 소유자만 읽고 쓸 수 있어야 한다"
            );
            assert_eq!(
                accounts::keychain::read_item(&keychain.service, &keychain.account)
                    .unwrap()
                    .unwrap(),
                before,
                "내보내기는 키체인 원본을 바꾸면 안 된다"
            );

            let parsed = decrypt_file(&path, result.recovery_code.as_str()).unwrap();
            let exported = URL_SAFE_NO_PAD
                .decode(&parsed.payload.entries[0].credential)
                .unwrap();
            assert_eq!(exported, live, "{suffix} 키체인 값은 같은 JSON으로 정규화");
        }
        let service = keychain.service.clone();
        let account = keychain.account.clone();
        drop(keychain);
        assert!(accounts::keychain::read_item(&service, &account)
            .unwrap()
            .is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_active_claude_export_falls_back_to_legacy_file_without_mutation() {
        let (source, keychain) = keychain_test_env("vault-macos-legacy-export");
        let identity = "mac-legacy-id";
        let email = "mac-legacy@example.test";
        add_claude(
            &source,
            "active",
            identity,
            email,
            "stale-profile-token",
        );
        atomic_write(
            &source.home.join(".claude.json"),
            serde_json::to_vec(&serde_json::json!({
                "oauthAccount": {
                    "accountUuid": identity,
                    "emailAddress": email
                }
            }))
            .unwrap()
            .as_slice(),
        )
        .unwrap();

        let legacy = source.live_credential_path(Provider::Claude);
        let live = br#"{"claudeAiOauth":{"accessToken":"mac-legacy-live-token"}}"#;
        atomic_write(&legacy, live).unwrap();
        let before = fs::read(&legacy).unwrap();
        assert!(
            accounts::keychain::read_item(&keychain.service, &keychain.account)
                .unwrap()
                .is_none()
        );

        let path = vault_path(&source, "legacy.switcher-vault");
        let result = export(
            &source,
            &path,
            vec![selection(&source, "claude", "active", false)],
        )
        .unwrap();
        assert_eq!(fs::read(&legacy).unwrap(), before);
        assert!(
            accounts::keychain::read_item(&keychain.service, &keychain.account)
                .unwrap()
                .is_none(),
            "legacy 폴백 내보내기가 키체인 항목을 만들면 안 된다"
        );

        let parsed = decrypt_file(&path, result.recovery_code.as_str()).unwrap();
        let exported = URL_SAFE_NO_PAD
            .decode(&parsed.payload.entries[0].credential)
            .unwrap();
        assert_eq!(exported, live);
        let service = keychain.service.clone();
        let account = keychain.account.clone();
        drop(keychain);
        assert!(accounts::keychain::read_item(&service, &account)
            .unwrap()
            .is_none());
    }

    fn claude_snapshot(
        credential_generation: usize,
        identity_generation: usize,
    ) -> ClaudeLiveSnapshot {
        let id = format!("identity-{identity_generation}");
        let email = format!("generation-{identity_generation}@example.test");
        let oauth_account = serde_json::json!({
            "accountUuid": id,
            "emailAddress": email
        });
        ClaudeLiveSnapshot {
            credential: Zeroizing::new(
                format!(
                    r#"{{"claudeAiOauth":{{"accessToken":"token-{credential_generation}"}}}}"#
                )
                .into_bytes(),
            ),
            oauth_account,
            identity: LiveIdentity {
                id,
                email: Some(email),
            },
        }
    }

    #[test]
    fn changing_claude_generations_never_form_an_export_snapshot() {
        for change_oauth_account in [false, true] {
            let mut generation = 0usize;
            let result = read_stable_claude_snapshot_with(
                || {
                    generation += 1;
                    let credential_generation =
                        if change_oauth_account { 0 } else { generation };
                    let identity_generation =
                        if change_oauth_account { generation } else { 0 };
                    Ok(claude_snapshot(
                        credential_generation,
                        identity_generation,
                    ))
                },
                || {},
            );
            let error = match result {
                Ok(_) => panic!("변경 중인 Claude 세대를 안정된 스냅샷으로 받아들이면 안 된다"),
                Err(error) => error,
            };
            assert_eq!(
                error,
                "Claude 로그인이 변경되는 중이라 내보내기를 중단했습니다 — 로그인이 끝난 뒤 다시 시도하세요"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_import_never_writes_active_keychain_or_auth_files() {
        let source = test_env("vault-macos-import-source");
        add_claude(
            &source,
            "claude-import",
            "incoming-claude-id",
            "incoming-claude@example.test",
            "incoming-claude-token",
        );
        add_codex(
            &source,
            "codex-import",
            "incoming-codex-id",
            "incoming-codex@example.test",
            "incoming-codex-token",
        );
        let path = vault_path(&source, "mac-import.switcher-vault");
        let exported = export(
            &source,
            &path,
            vec![
                selection(&source, "claude", "claude-import", true),
                selection(&source, "codex", "codex-import", false),
            ],
        )
        .unwrap();

        let (target, keychain) = keychain_test_env("vault-macos-import-target");
        let keychain_before = br#"{"claudeAiOauth":{"accessToken":"existing-keychain-token"}}"#;
        write_keychain_fixture(&keychain, keychain_before);
        let legacy = target.live_credential_path(Provider::Claude);
        let claude_json = target.home.join(".claude.json");
        let codex_auth = target.live_credential_path(Provider::Codex);
        atomic_write(&legacy, b"existing-legacy-claude-file").unwrap();
        atomic_write(&claude_json, b"existing-claude-account-file").unwrap();
        atomic_write(&codex_auth, b"existing-codex-auth-file").unwrap();
        let legacy_before = fs::read(&legacy).unwrap();
        let claude_json_before = fs::read(&claude_json).unwrap();
        let codex_before = fs::read(&codex_auth).unwrap();

        let result = import(&target, &path, exported.recovery_code.as_str().to_owned()).unwrap();
        assert_eq!(result.imported, 2);
        assert_eq!(result.skipped, 0);
        assert_eq!(
            accounts::keychain::read_item(&keychain.service, &keychain.account)
                .unwrap()
                .unwrap(),
            keychain_before
        );
        assert_eq!(fs::read(&legacy).unwrap(), legacy_before);
        assert_eq!(fs::read(&claude_json).unwrap(), claude_json_before);
        assert_eq!(fs::read(&codex_auth).unwrap(), codex_before);
        assert!(
            target
                .profiles_dir(Provider::Claude)
                .join("claude-import/credentials.json")
                .is_file()
        );
        assert!(
            target
                .profiles_dir(Provider::Codex)
                .join("codex-import/auth.json")
                .is_file()
        );
        let service = keychain.service.clone();
        let account = keychain.account.clone();
        drop(keychain);
        assert!(accounts::keychain::read_item(&service, &account)
            .unwrap()
            .is_none());
    }

    #[test]
    fn lifecycle_state_serializes_operations_recovery_and_shutdown() {
        assert!(!operation_busy());
        let guard = begin_operation().unwrap();
        assert!(operation_busy());
        assert!(std::thread::spawn(operation_busy).join().unwrap());
        assert!(begin_operation().is_err());
        let code = URL_SAFE_NO_PAD.encode([9u8; 32]);
        hold_recovery_for_delivery(&code).unwrap();
        drop(guard);

        assert!(operation_busy());
        let pending = pending_recovery().unwrap();
        assert_eq!(
            pending.as_ref().map(RecoveryCode::as_str),
            Some(code.as_str())
        );
        assert!(begin_operation().is_err());
        assert!(!ack_recovery_stored(URL_SAFE_NO_PAD.encode([8u8; 32])).unwrap());
        assert!(operation_busy());
        assert!(ack_recovery_stored(code.clone()).unwrap());
        assert!(
            ack_recovery_stored(code).unwrap(),
            "응답 유실 재시도는 멱등이다"
        );
        assert!(!operation_busy());
        assert_eq!(pending_recovery().unwrap(), None);

        assert!(try_reserve_shutdown());
        assert!(operation_busy());
        assert!(!try_reserve_shutdown());
        assert!(begin_operation().is_err());
        release_shutdown_reservation();
        assert!(!operation_busy());

        let guard = begin_operation().unwrap();
        assert!(!try_reserve_shutdown());
        drop(guard);
        assert!(!operation_busy());
    }

    #[test]
    fn recovery_code_serializes_without_exposing_debug_output() {
        let code = RecoveryCode::new("fixture-recovery-code".to_string());

        assert_eq!(
            serde_json::to_string(&code).unwrap(),
            r#""fixture-recovery-code""#
        );
        assert_eq!(format!("{code:?}"), "[redacted]");
    }

    #[test]
    fn roundtrip_selected_only_and_hide_email_persists() {
        let source = test_env("vault-roundtrip-source");
        add_claude(
            &source,
            "one",
            "id-one",
            "one@example.test",
            "fake-token-one",
        );
        add_codex(
            &source,
            "two",
            "id-two",
            "two@example.test",
            "fake-token-two",
        );
        let path = vault_path(&source, "selected.switcher-vault");
        let exported = export(
            &source,
            &path,
            vec![selection(&source, "claude", "one", true)],
        )
        .unwrap();
        assert_eq!(exported.exported, 1);

        let target = test_env("vault-roundtrip-target");
        let imported = import(&target, &path, exported.recovery_code.as_str().to_owned()).unwrap();
        assert_eq!(
            imported,
            VaultImportResult {
                imported: 1,
                skipped: 0,
                cleanup_pending: false,
            }
        );
        assert!(target.profiles_dir(Provider::Claude).join("one").exists());
        assert!(!target.profiles_dir(Provider::Codex).join("two").exists());
        assert!(
            accounts::read_meta(&target.profiles_dir(Provider::Claude).join("one"))
                .unwrap()
                .hide_email
        );
        assert!(!target
            .profiles_dir(Provider::Claude)
            .join("one")
            .join(IMPORT_MARKER_FILE)
            .exists());
        assert_eq!(
            accounts::list(&target, Provider::Claude).unwrap().profiles[0].email,
            None
        );

        // 이후 토큰 갱신/재로그인이 같은 쓰기 관문을 지나도 표시 설정은 유지된다.
        accounts::write_profile_parts(
            &target,
            Provider::Claude,
            "one",
            &LiveIdentity {
                id: "id-one".into(),
                email: Some("one@example.test".into()),
            },
            br#"{"claudeAiOauth":{"accessToken":"updated-fake-token"}}"#,
            Some(&serde_json::json!({"accountUuid":"id-one","emailAddress":"one@example.test"})),
        )
        .unwrap();
        assert!(
            accounts::read_meta(&target.profiles_dir(Provider::Claude).join("one"))
                .unwrap()
                .hide_email
        );
    }

    #[test]
    fn wrong_code_tamper_and_truncation_share_one_error() {
        let source = test_env("vault-crypto-errors");
        add_codex(
            &source,
            "codex",
            "acct",
            "codex@example.test",
            "fake-access-secret",
        );
        let path = vault_path(&source, "crypto.switcher-vault");
        let result = export(
            &source,
            &path,
            vec![selection(&source, "codex", "codex", false)],
        )
        .unwrap();
        let target = test_env("vault-crypto-errors-target");
        let wrong = URL_SAFE_NO_PAD.encode([7u8; 32]);
        assert_eq!(import(&target, &path, wrong).unwrap_err(), CRYPTO_ERROR);

        let original = fs::read(&path).unwrap();
        let mut tampered = original.clone();
        *tampered.last_mut().unwrap() ^= 1;
        atomic_write(&path, &tampered).unwrap();
        assert_eq!(
            import(&target, &path, result.recovery_code.as_str().to_owned()).unwrap_err(),
            CRYPTO_ERROR
        );

        atomic_write(&path, &original[..original.len() - 3]).unwrap();
        assert_eq!(
            import(&target, &path, result.recovery_code.as_str().to_owned()).unwrap_err(),
            CRYPTO_ERROR
        );
    }

    #[test]
    fn rejects_kdf_bomb_before_running_argon() {
        let source = test_env("vault-kdf-bomb");
        add_claude(
            &source,
            "claude",
            "id",
            "mail@example.test",
            "fake-secret-kdf",
        );
        let path = vault_path(&source, "bomb.switcher-vault");
        let result = export(
            &source,
            &path,
            vec![selection(&source, "claude", "claude", false)],
        )
        .unwrap();
        let mut bytes = fs::read(&path).unwrap();
        let needle = b"\"memory_kib\":65536";
        let position = bytes
            .windows(needle.len())
            .position(|part| part == needle)
            .unwrap();
        bytes[position + needle.len() - 1] = b'7';
        atomic_write(&path, &bytes).unwrap();
        let before = kdf_call_count();
        let target = test_env("vault-kdf-bomb-target");
        assert_eq!(
            import(&target, &path, result.recovery_code.as_str().to_owned()).unwrap_err(),
            CRYPTO_ERROR
        );
        assert_eq!(kdf_call_count(), before);
    }

    #[test]
    fn rejects_recovery_code_shape_before_kdf() {
        let source = test_env("vault-code-shape");
        add_codex(
            &source,
            "codex",
            "account-id",
            "codex@example.test",
            "fixture-access",
        );
        let path = vault_path(&source, "code-shape.switcher-vault");
        export(
            &source,
            &path,
            vec![selection(&source, "codex", "codex", false)],
        )
        .unwrap();

        let before = kdf_call_count();
        let target = test_env("vault-code-shape-target");
        assert_eq!(
            import(&target, &path, "A".repeat(1_000_000)).unwrap_err(),
            CRYPTO_ERROR
        );
        assert_eq!(kdf_call_count(), before);
    }

    #[test]
    fn encrypted_file_contains_no_profile_or_plaintext_identity() {
        let source = test_env("vault-no-plaintext");
        let token = "fixture-super-distinct-access-token-not-in-header-987654";
        let email = "distinct-address-987654@example.test";
        add_claude(
            &source,
            "distinct-profile",
            "distinct-id-987654",
            email,
            token,
        );
        let path = vault_path(&source, "opaque.switcher-vault");
        export(
            &source,
            &path,
            vec![selection(&source, "claude", "distinct-profile", true)],
        )
        .unwrap();
        let bytes = fs::read(path).unwrap();
        for forbidden in [
            token,
            email,
            "distinct-id-987654",
            "distinct-profile",
            "claude",
        ] {
            assert!(!bytes
                .windows(forbidden.len())
                .any(|part| part == forbidden.as_bytes()));
        }
    }

    #[test]
    fn export_refuses_to_overwrite_any_profile_or_active_account_file() {
        let source = test_env("vault-protected-destination");
        add_claude(
            &source,
            "protected",
            "protected-id",
            "protected@example.test",
            "protected-fake-token",
        );
        let profile_credential = source
            .profiles_dir(Provider::Claude)
            .join("protected/credentials.json");
        let before = fs::read(&profile_credential).unwrap();
        let error = export(
            &source,
            &profile_credential,
            vec![selection(&source, "claude", "protected", false)],
        )
        .unwrap_err();
        assert_eq!(error, "인증정보 원본 폴더 밖의 다른 위치를 선택하세요");
        assert_eq!(fs::read(profile_credential).unwrap(), before);

        atomic_write(
            &source.live_credential_path(Provider::Codex),
            b"active-codex-bytes",
        )
        .unwrap();
        let active_before = fs::read(source.live_credential_path(Provider::Codex)).unwrap();
        assert!(export(
            &source,
            &source.live_credential_path(Provider::Codex),
            vec![selection(&source, "claude", "protected", false)],
        )
        .is_err());
        assert_eq!(
            fs::read(source.live_credential_path(Provider::Codex)).unwrap(),
            active_before
        );
    }

    #[test]
    fn active_profile_exports_live_credential_and_current_oauth() {
        let source = test_env("vault-active-source");
        add_claude(
            &source,
            "active",
            "live-id",
            "live@example.test",
            "stored-old-token",
        );
        atomic_write(
            &source.live_credential_path(Provider::Claude),
            br#"{"claudeAiOauth":{"accessToken":"live-new-token"}}"#,
        )
        .unwrap();
        atomic_write(
            &source.home.join(".claude.json"),
            br#"{"oauthAccount":{"accountUuid":"live-id","emailAddress":"live@example.test"}}"#,
        )
        .unwrap();
        let path = vault_path(&source, "active.switcher-vault");
        let result = export(
            &source,
            &path,
            vec![selection(&source, "claude", "active", false)],
        )
        .unwrap();

        let target = test_env("vault-active-target");
        import(&target, &path, result.recovery_code.as_str().to_owned()).unwrap();
        let imported: Value = serde_json::from_slice(
            &fs::read(
                target
                    .profiles_dir(Provider::Claude)
                    .join("active/credentials.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            imported.pointer("/claudeAiOauth/accessToken").unwrap(),
            "live-new-token"
        );
    }

    #[test]
    fn active_codex_export_uses_live_auth_without_mutating_it() {
        let source = test_env("vault-active-codex-source");
        add_codex(
            &source,
            "active",
            "live-codex-id",
            "live-codex@example.test",
            "stale-profile-token",
        );
        let live_auth = serde_json::to_vec(&serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": jwt("live-codex-id", "live-codex@example.test"),
                "access_token": "current-live-codex-token",
                "refresh_token": "current-live-codex-refresh",
                "account_id": "live-codex-id"
            }
        }))
        .unwrap();
        let live_path = source.live_credential_path(Provider::Codex);
        atomic_write(&live_path, &live_auth).unwrap();

        let path = vault_path(&source, "active-codex.switcher-vault");
        let result = export(
            &source,
            &path,
            vec![selection(&source, "codex", "active", false)],
        )
        .unwrap();
        assert_eq!(fs::read(&live_path).unwrap(), live_auth);
        let parsed = decrypt_file(&path, result.recovery_code.as_str()).unwrap();
        let exported: Value = serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(&parsed.payload.entries[0].credential)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            exported.pointer("/tokens/access_token").unwrap(),
            "current-live-codex-token"
        );
    }

    #[test]
    fn export_merges_pending_in_memory_and_never_mutates_source_files() {
        let source = test_env("vault-pending-source");
        add_claude(
            &source,
            "active",
            "pending-id",
            "pending@example.test",
            "stored-token",
        );
        let profile_credential = source
            .profiles_dir(Provider::Claude)
            .join("active/credentials.json");
        atomic_write(
            &profile_credential,
            br#"{"claudeAiOauth":{"accessToken":"stored-old","refreshToken":"refresh-old","expiresAt":1000}}"#,
        )
        .unwrap();
        atomic_write(
            &source.live_credential_path(Provider::Claude),
            br#"{"claudeAiOauth":{"accessToken":"live-old","refreshToken":"refresh-old","expiresAt":1000}}"#,
        )
        .unwrap();
        atomic_write(
            &source.home.join(".claude.json"),
            br#"{"oauthAccount":{"accountUuid":"pending-id","emailAddress":"pending@example.test"}}"#,
        )
        .unwrap();
        let pending = crate::usage::pending_path(&profile_credential);
        atomic_write(
            &pending,
            br#"{"old_refresh":"refresh-old","response":{"access_token":"merged-access","refresh_token":"refresh-new","expires_in":28800},"saved_at":1}"#,
        )
        .unwrap();

        let source_paths = [
            profile_credential.clone(),
            source.live_credential_path(Provider::Claude),
            source.home.join(".claude.json"),
            pending.clone(),
        ];
        let before: Vec<Vec<u8>> = source_paths
            .iter()
            .map(|path| fs::read(path).unwrap())
            .collect();
        let path = vault_path(&source, "pending.switcher-vault");
        let result = export(
            &source,
            &path,
            vec![selection(&source, "claude", "active", false)],
        )
        .unwrap();
        for (path, expected) in source_paths.iter().zip(&before) {
            assert_eq!(&fs::read(path).unwrap(), expected);
        }

        let target = test_env("vault-pending-target");
        import(&target, &path, result.recovery_code.as_str().to_owned()).unwrap();
        let imported: Value = serde_json::from_slice(
            &fs::read(
                target
                    .profiles_dir(Provider::Claude)
                    .join("active/credentials.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            imported.pointer("/claudeAiOauth/accessToken").unwrap(),
            "merged-access"
        );

        atomic_write(&pending, b"broken pending fixture").unwrap();
        let failure_before: Vec<Vec<u8>> = source_paths
            .iter()
            .map(|path| fs::read(path).unwrap())
            .collect();
        assert!(export(
            &source,
            &vault_path(&source, "pending-failure.switcher-vault"),
            vec![selection(&source, "claude", "active", false)],
        )
        .is_err());
        for (path, expected) in source_paths.iter().zip(&failure_before) {
            assert_eq!(&fs::read(path).unwrap(), expected);
        }
    }

    #[test]
    fn collision_suffix_same_identity_skip_and_active_files_unchanged() {
        let source = test_env("vault-collision-source");
        add_codex(
            &source,
            "same",
            "incoming-id",
            "incoming@example.test",
            "incoming-token",
        );
        add_claude(
            &source,
            "duplicate",
            "existing-id",
            "existing@example.test",
            "duplicate-token",
        );
        let path = vault_path(&source, "collision.switcher-vault");
        let result = export(
            &source,
            &path,
            vec![
                selection(&source, "codex", "same", false),
                selection(&source, "claude", "duplicate", false),
            ],
        )
        .unwrap();

        let target = test_env("vault-collision-target");
        add_codex(
            &target,
            "SAME",
            "other-id",
            "other@example.test",
            "other-token",
        );
        add_claude(
            &target,
            "already",
            "existing-id",
            "existing@example.test",
            "existing-token",
        );
        atomic_write(
            &target.live_credential_path(Provider::Codex),
            &serde_json::to_vec(&serde_json::json!({
                "auth_mode": "chatgpt",
                "tokens": {
                    "id_token": jwt("active-codex-id", "active-codex@example.test"),
                    "access_token": "active-codex-token",
                    "refresh_token": "active-codex-refresh",
                    "account_id": "active-codex-id"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        atomic_write(
            &target.home.join(".claude.json"),
            br#"{"oauthAccount":{"accountUuid":"active-claude-id","emailAddress":"active-claude@example.test"}}"#,
        )
        .unwrap();
        atomic_write(
            &target.live_credential_path(Provider::Claude),
            br#"{"claudeAiOauth":{"accessToken":"active-claude-token"}}"#,
        )
        .unwrap();
        let codex_before = fs::read(target.live_credential_path(Provider::Codex)).unwrap();
        let claude_before = fs::read(target.home.join(".claude.json")).unwrap();
        let claude_credential_before =
            fs::read(target.live_credential_path(Provider::Claude)).unwrap();

        let imported = import(&target, &path, result.recovery_code.as_str().to_owned()).unwrap();
        assert_eq!(
            imported,
            VaultImportResult {
                imported: 1,
                skipped: 1,
                cleanup_pending: false,
            }
        );
        assert!(target.profiles_dir(Provider::Codex).join("same-2").exists());
        assert_eq!(
            fs::read(target.live_credential_path(Provider::Codex)).unwrap(),
            codex_before
        );
        assert_eq!(
            fs::read(target.home.join(".claude.json")).unwrap(),
            claude_before
        );
        assert_eq!(
            fs::read(target.live_credential_path(Provider::Claude)).unwrap(),
            claude_credential_before
        );
    }

    #[test]
    fn import_skips_unsaved_live_identities_without_activating_them() {
        let source = test_env("vault-live-identity-source");
        add_claude(
            &source,
            "claude-incoming",
            "claude-live-id",
            "claude-live@example.test",
            "claude-incoming-token",
        );
        add_codex(
            &source,
            "codex-incoming",
            "codex-live-id",
            "codex-live@example.test",
            "codex-incoming-token",
        );
        let path = vault_path(&source, "live-identities.switcher-vault");
        let exported = export(
            &source,
            &path,
            vec![
                selection(&source, "claude", "claude-incoming", false),
                selection(&source, "codex", "codex-incoming", false),
            ],
        )
        .unwrap();

        let target = test_env("vault-live-identity-target");
        let claude_live = br#"{"claudeAiOauth":{"accessToken":"claude-live-token"}}"#;
        let claude_account = br#"{"oauthAccount":{"accountUuid":"claude-live-id","emailAddress":"claude-live@example.test"}}"#;
        let codex_live = serde_json::to_vec(&serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": jwt("codex-live-id", "codex-live@example.test"),
                "access_token": "codex-live-token",
                "refresh_token": "codex-live-refresh",
                "account_id": "codex-live-id"
            }
        }))
        .unwrap();
        atomic_write(
            &target.live_credential_path(Provider::Claude),
            claude_live,
        )
        .unwrap();
        atomic_write(&target.home.join(".claude.json"), claude_account).unwrap();
        atomic_write(
            &target.live_credential_path(Provider::Codex),
            &codex_live,
        )
        .unwrap();
        let before = [
            fs::read(target.live_credential_path(Provider::Claude)).unwrap(),
            fs::read(target.home.join(".claude.json")).unwrap(),
            fs::read(target.live_credential_path(Provider::Codex)).unwrap(),
        ];

        let imported = import(&target, &path, exported.recovery_code.as_str().to_owned()).unwrap();

        assert_eq!(
            imported,
            VaultImportResult {
                imported: 0,
                skipped: 2,
                cleanup_pending: false,
            }
        );
        for provider in [Provider::Claude, Provider::Codex] {
            let snapshot = accounts::list(&target, provider).unwrap();
            assert!(snapshot.profiles.is_empty());
            assert!(!snapshot.live_saved);
        }
        assert_eq!(
            fs::read(target.live_credential_path(Provider::Claude)).unwrap(),
            before[0]
        );
        assert_eq!(
            fs::read(target.home.join(".claude.json")).unwrap(),
            before[1]
        );
        assert_eq!(
            fs::read(target.live_credential_path(Provider::Codex)).unwrap(),
            before[2]
        );
    }

    #[test]
    fn import_rejects_unidentifiable_active_credentials_before_creating_profiles() {
        let payload = VaultPayload {
            format_version: PAYLOAD_VERSION,
            entries: vec![VaultEntry {
                provider: "codex".into(),
                name: "incoming".into(),
                hide_email: false,
                id: "incoming-id".into(),
                email: Some("incoming@example.test".into()),
                credential: encoded(
                    br#"{"auth_mode":"chatgpt","tokens":{"id_token":"fixture"}}"#,
                ),
                oauth_account: None,
            }],
        };

        for (tag, active_credential) in [
            ("vault-invalid-live-identity", b"not-json".as_slice()),
            (
                "vault-missing-live-identity",
                br#"{"auth_mode":"api_key"}"#.as_slice(),
            ),
        ] {
            let target = test_env(tag);
            atomic_write(
                &target.live_credential_path(Provider::Codex),
                active_credential,
            )
            .unwrap();

            assert!(commit_payload(&target, &payload, |_, _| Ok(())).is_err());
            assert!(accounts::profile_dirs(&target, Provider::Codex)
                .unwrap()
                .is_empty());
            assert!(!target.store.exists());
        }
    }

    #[test]
    fn import_rejects_active_credential_presence_errors_before_creating_profiles() {
        let payload = VaultPayload {
            format_version: PAYLOAD_VERSION,
            entries: vec![VaultEntry {
                provider: "claude".into(),
                name: "incoming".into(),
                hide_email: false,
                id: "incoming-id".into(),
                email: Some("incoming@example.test".into()),
                credential: encoded(br#"{"claudeAiOauth":{"accessToken":"fixture"}}"#),
                oauth_account: Some(encoded(
                    br#"{"accountUuid":"incoming-id","emailAddress":"incoming@example.test"}"#,
                )),
            }],
        };
        let mut target = test_env("vault-live-presence-error");
        target.claude_live =
            accounts::ClaudeLiveStore::File(PathBuf::from("invalid\0active-credential"));

        let error = commit_payload(&target, &payload, |_, _| Ok(())).unwrap_err();

        assert!(error.contains("활성 인증정보 확인 실패"));
        assert!(!target.store.exists());
    }

    #[test]
    fn commit_failure_rolls_back_final_and_staging_directories() {
        let target = test_env("vault-rollback");
        let payload = VaultPayload {
            format_version: PAYLOAD_VERSION,
            entries: vec![
                VaultEntry {
                    provider: "claude".into(),
                    name: "first".into(),
                    hide_email: false,
                    id: "first-id".into(),
                    email: Some("first@example.test".into()),
                    credential: encoded(br#"{"claudeAiOauth":{"accessToken":"first-fake-token"}}"#),
                    oauth_account: Some(encoded(
                        br#"{"accountUuid":"first-id","emailAddress":"first@example.test"}"#,
                    )),
                },
                VaultEntry {
                    provider: "claude".into(),
                    name: "second".into(),
                    hide_email: false,
                    id: "second-id".into(),
                    email: Some("second@example.test".into()),
                    credential: encoded(
                        br#"{"claudeAiOauth":{"accessToken":"second-fake-token"}}"#,
                    ),
                    oauth_account: Some(encoded(
                        br#"{"accountUuid":"second-id","emailAddress":"second@example.test"}"#,
                    )),
                },
            ],
        };
        validate_payload(&payload).unwrap();
        let error = commit_payload(&target, &payload, |index, _| {
            if index == 0 {
                let stage_root = fs::read_dir(&target.store)
                    .unwrap()
                    .flatten()
                    .map(|entry| entry.path())
                    .find(|path| {
                        path.file_name().is_some_and(|name| {
                            name.to_string_lossy().starts_with(".vault-import-")
                        })
                    })
                    .unwrap();
                let mut pending = vec![stage_root];
                let mut staged_bytes = Vec::new();
                let mut staged_names = Vec::new();
                while let Some(path) = pending.pop() {
                    for entry in fs::read_dir(path).unwrap().flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            pending.push(path);
                        } else {
                            staged_names.push(entry.file_name().to_string_lossy().to_string());
                            staged_bytes.extend(fs::read(path).unwrap());
                        }
                    }
                }
                let staged_text = String::from_utf8_lossy(&staged_bytes);
                for secret in [
                    "first-fake-token",
                    "second-fake-token",
                    "first-id",
                    "second-id",
                    "first@example.test",
                    "second@example.test",
                ] {
                    assert!(!staged_text.contains(secret));
                }
                assert!(staged_names
                    .iter()
                    .all(|name| matches!(name.as_str(), IMPORT_JOURNAL_FILE | IMPORT_MARKER_FILE)));
            }
            if index == 1 {
                let first = target.profiles_dir(Provider::Claude).join("first");
                assert!(first.exists());
                assert!(fs::read_dir(&first)
                    .unwrap()
                    .flatten()
                    .all(|entry| { !entry.file_name().to_string_lossy().contains(".tmp") }));
                Err("fixture commit failure".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(error, "fixture commit failure");
        assert!(!target.profiles_dir(Provider::Claude).join("first").exists());
        assert!(!target
            .profiles_dir(Provider::Claude)
            .join("second")
            .exists());
        let leftovers = fs::read_dir(&target.store)
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".vault-import-")
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn stale_import_recovery_handles_orphan_staging_and_both_crash_states() {
        let target = test_env("vault-stale-recovery");

        let (orphan, _) = create_stage_root(&target).unwrap();
        atomic_write(&orphan.join("plaintext-fixture"), b"fake-token-only").unwrap();
        recover_stale_imports(&target).unwrap();
        assert!(!orphan.exists());

        let (staging_root, staging_id) = create_stage_root(&target).unwrap();
        let staging_journal = journal(&staging_id, "staging", Provider::Claude, "partial");
        write_import_journal(&staging_root, &staging_journal).unwrap();
        let partial = add_marked_claude(
            &target,
            "partial",
            "partial-id",
            "partial@example.test",
            &staging_id,
        );
        recover_stale_imports(&target).unwrap();
        assert!(!partial.exists());
        assert!(!staging_root.exists());

        add_claude(
            &target,
            "existing",
            "existing-id",
            "existing@example.test",
            "existing-token",
        );
        let existing = target.profiles_dir(Provider::Claude).join("existing");
        let existing_before = fs::read(existing.join("credentials.json")).unwrap();
        let (safe_root, safe_id) = create_stage_root(&target).unwrap();
        write_import_journal(
            &safe_root,
            &journal(&safe_id, "staging", Provider::Claude, "existing"),
        )
        .unwrap();
        assert!(recover_stale_imports(&target).is_err());
        assert_eq!(
            fs::read(existing.join("credentials.json")).unwrap(),
            existing_before
        );
        assert!(safe_root.exists(), "소유권 journal을 지우면 안 된다");
        fs::remove_dir_all(&safe_root).unwrap();

        let (empty_root, empty_id) = create_stage_root(&target).unwrap();
        write_import_journal(
            &empty_root,
            &journal(&empty_id, "staging", Provider::Claude, "empty-crash"),
        )
        .unwrap();
        let empty_final = target.profiles_dir(Provider::Claude).join("empty-crash");
        fs::create_dir_all(&empty_final).unwrap();
        recover_stale_imports(&target).unwrap();
        assert!(!empty_final.exists());
        assert!(!empty_root.exists());

        let (committed_root, committed_id) = create_stage_root(&target).unwrap();
        let committed_journal = journal(&committed_id, "committed", Provider::Claude, "complete");
        write_import_journal(&committed_root, &committed_journal).unwrap();
        let complete = add_marked_claude(
            &target,
            "complete",
            "complete-id",
            "complete@example.test",
            &committed_id,
        );
        recover_stale_imports(&target).unwrap();
        assert!(complete.exists());
        assert!(!complete.join(IMPORT_MARKER_FILE).exists());
        assert!(!committed_root.exists());
    }

    #[test]
    fn committed_marker_cleanup_failure_stays_visible_and_retries_safely() {
        let target = test_env("vault-marker-cleanup-retry");
        let (stage_root, import_id) = create_stage_root(&target).unwrap();
        let journal = ImportJournal {
            version: 1,
            id: import_id.clone(),
            state: "committed".into(),
            entries: vec![
                ImportJournalEntry {
                    provider: Provider::Claude.dir_name().into(),
                    name: "first".into(),
                },
                ImportJournalEntry {
                    provider: Provider::Claude.dir_name().into(),
                    name: "second".into(),
                },
            ],
        };
        write_import_journal(&stage_root, &journal).unwrap();
        write_committed_import(&stage_root, &journal).unwrap();
        let first = add_marked_claude(
            &target,
            "first",
            "first-id",
            "first@example.test",
            &import_id,
        );
        let second = add_marked_claude(
            &target,
            "second",
            "second-id",
            "second@example.test",
            &import_id,
        );
        let first_marker = first.join(IMPORT_MARKER_FILE);
        let second_marker = second.join(IMPORT_MARKER_FILE);
        fs::remove_file(&second_marker).unwrap();
        fs::create_dir(&second_marker).unwrap();
        atomic_write(&stage_root.join(IMPORT_JOURNAL_FILE), b"broken journal").unwrap();

        assert!(recover_stale_imports(&target).is_err());
        assert!(stage_root.exists());
        assert!(
            first_marker.exists(),
            "marker를 모두 검증하기 전에 지우면 안 된다"
        );
        assert!(second_marker.is_dir());
        let visible = accounts::profile_dirs(&target, Provider::Claude)
            .unwrap()
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>();
        assert_eq!(visible, vec!["first", "second"]);
        assert!(!profile_import_blocked(&target, Provider::Claude, "first"));
        assert!(!profile_import_blocked(&target, Provider::Claude, "second"));

        atomic_write(&first_marker, b"000000000000000000000000").unwrap();
        assert!(
            !profile_import_blocked(&target, Provider::Claude, "first"),
            "잘못된 valid marker id가 committed 전체 가시성을 깨면 안 된다"
        );
        atomic_write(&first_marker, import_id.as_bytes()).unwrap();

        // journal 자체를 읽을 수 없어도 committed 복제본으로 완료 상태와
        // 두 항목의 소속을 판정해야 한다.
        fs::remove_file(stage_root.join(IMPORT_JOURNAL_FILE)).unwrap();
        fs::create_dir(stage_root.join(IMPORT_JOURNAL_FILE)).unwrap();
        assert!(!profile_import_blocked(&target, Provider::Claude, "first"));
        assert!(!profile_import_blocked(&target, Provider::Claude, "second"));

        let mut waits = Vec::new();
        recover_stale_imports_with_wait(&target, |delay| {
            waits.push(delay);
            if waits.len() == 1 {
                fs::remove_dir(&second_marker).unwrap();
                atomic_write(&second_marker, import_id.as_bytes()).unwrap();
            }
        })
        .unwrap();
        assert_eq!(waits, vec![std::time::Duration::from_millis(250)]);
        assert!(first.exists());
        assert!(second.exists());
        assert!(!second_marker.exists());
        assert!(!stage_root.exists());
    }

    #[test]
    fn unreadable_journal_without_committed_copy_is_preserved() {
        let target = test_env("vault-unreadable-journal");
        let (stage_root, _) = create_stage_root(&target).unwrap();
        atomic_write(&stage_root.join(IMPORT_JOURNAL_FILE), b"broken journal").unwrap();

        assert!(recover_stale_imports(&target).is_err());
        assert!(stage_root.exists());
    }

    #[test]
    fn one_broken_stage_does_not_block_other_stage_recovery() {
        let target = test_env("vault-independent-stage-recovery");
        fs::create_dir_all(&target.store).unwrap();
        let broken_root = target.store.join(".vault-import-000000000000000000000001");
        fs::create_dir(&broken_root).unwrap();
        atomic_write(
            &broken_root.join(IMPORT_JOURNAL_FILE),
            b"broken journal fixture",
        )
        .unwrap();

        let recoverable_id = "000000000000000000000002".to_string();
        let recoverable_root = target.store.join(format!(".vault-import-{recoverable_id}"));
        fs::create_dir(&recoverable_root).unwrap();
        write_import_journal(
            &recoverable_root,
            &journal(&recoverable_id, "staging", Provider::Claude, "recoverable"),
        )
        .unwrap();
        let recoverable = add_marked_claude(
            &target,
            "recoverable",
            "recoverable-id",
            "recoverable@example.test",
            &recoverable_id,
        );

        assert!(recover_stale_imports(&target).is_err());
        assert!(broken_root.exists(), "판단할 수 없는 stage는 보존해야 한다");
        assert!(
            !recoverable.exists(),
            "독립된 staging 프로필은 원복해야 한다"
        );
        assert!(!recoverable_root.exists());
    }

    #[cfg(unix)]
    #[test]
    fn dangling_import_marker_is_blocked_fail_closed() {
        use std::os::unix::fs::symlink;

        let target = test_env("vault-dangling-import-marker");
        let profile = target.profiles_dir(Provider::Claude).join("partial");
        fs::create_dir_all(&profile).unwrap();
        symlink("missing-marker-target", profile.join(IMPORT_MARKER_FILE)).unwrap();

        assert!(profile_import_blocked(&target, Provider::Claude, "partial"));
    }

    #[test]
    fn committed_copy_write_failure_returns_cleanup_pending_and_recovers() {
        let target = test_env("vault-committed-copy-retry");
        let payload = VaultPayload {
            format_version: PAYLOAD_VERSION,
            entries: vec![VaultEntry {
                provider: "claude".into(),
                name: "complete".into(),
                hide_email: false,
                id: "complete-id".into(),
                email: Some("complete@example.test".into()),
                credential: encoded(br#"{"claudeAiOauth":{"accessToken":"fixture-import-token"}}"#),
                oauth_account: Some(encoded(
                    br#"{"accountUuid":"complete-id","emailAddress":"complete@example.test"}"#,
                )),
            }],
        };
        let mut stage_root = None;
        let result = commit_payload(&target, &payload, |_, _| {
            let root = fs::read_dir(&target.store)
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    path.file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with(".vault-import-"))
                })
                .unwrap();
            fs::create_dir(root.join(IMPORT_COMMITTED_FILE)).unwrap();
            stage_root = Some(root);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            result,
            VaultImportResult {
                imported: 1,
                skipped: 0,
                cleanup_pending: true,
            }
        );
        let stage_root = stage_root.unwrap();
        let profile = target.profiles_dir(Provider::Claude).join("complete");
        assert!(profile.exists());
        assert!(!profile_import_blocked(
            &target,
            Provider::Claude,
            "complete"
        ));

        fs::remove_dir(stage_root.join(IMPORT_COMMITTED_FILE)).unwrap();
        recover_stale_imports(&target).unwrap();
        assert!(profile.exists());
        assert!(!profile.join(IMPORT_MARKER_FILE).exists());
        assert!(!stage_root.exists());
    }

    #[test]
    fn staging_marker_read_failure_preserves_journal_for_retry() {
        let target = test_env("vault-staging-marker-retry");
        let (stage_root, import_id) = create_stage_root(&target).unwrap();
        let journal = journal(&import_id, "staging", Provider::Claude, "partial");
        write_import_journal(&stage_root, &journal).unwrap();
        let final_dir = target.profiles_dir(Provider::Claude).join("partial");
        accounts::write_profile_bundle_to_dir(
            &final_dir,
            Provider::Claude,
            &LiveIdentity {
                id: "partial-id".into(),
                email: Some("partial@example.test".into()),
            },
            br#"{"claudeAiOauth":{"accessToken":"fixture-token"}}"#,
            Some(&serde_json::json!({
                "accountUuid": "partial-id",
                "emailAddress": "partial@example.test"
            })),
            false,
        )
        .unwrap();
        let marker = final_dir.join(IMPORT_MARKER_FILE);
        fs::create_dir(&marker).unwrap();

        assert!(recover_stale_imports(&target).is_err());
        assert!(stage_root.exists(), "소유권 journal을 지우면 안 된다");
        assert!(final_dir.exists());
        assert!(accounts::profile_dirs(&target, Provider::Claude)
            .unwrap()
            .is_empty());

        fs::remove_dir(&marker).unwrap();
        atomic_write(&marker, import_id.as_bytes()).unwrap();
        recover_stale_imports(&target).unwrap();
        assert!(!final_dir.exists());
        assert!(!stage_root.exists());
    }

    #[cfg(windows)]
    #[test]
    fn locked_token_rollback_keeps_marker_and_journal_until_retry() {
        use std::os::windows::fs::OpenOptionsExt;

        let target = test_env("vault-locked-token-retry");
        let (stage_root, import_id) = create_stage_root(&target).unwrap();
        let journal = journal(&import_id, "staging", Provider::Claude, "partial");
        write_import_journal(&stage_root, &journal).unwrap();
        let final_dir = add_marked_claude(
            &target,
            "partial",
            "partial-id",
            "partial@example.test",
            &import_id,
        );
        let marker = final_dir.join(IMPORT_MARKER_FILE);
        let locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(final_dir.join("credentials.json"))
            .unwrap();

        assert!(recover_stale_imports(&target).is_err());
        assert!(marker_matches(&marker, &import_id));
        assert!(stage_root.exists());
        assert!(accounts::profile_dirs(&target, Provider::Claude)
            .unwrap()
            .is_empty());

        drop(locked);
        recover_stale_imports(&target).unwrap();
        assert!(!final_dir.exists());
        assert!(!stage_root.exists());
    }

    #[test]
    fn encoded_field_preflight_and_raw_total_stop_before_decode() {
        let before = decode_call_count();
        let malformed = VaultPayload {
            format_version: PAYLOAD_VERSION,
            entries: vec![VaultEntry {
                provider: "codex".into(),
                name: "profile".into(),
                hide_email: false,
                id: "account-id".into(),
                email: None,
                credential: "%%%".into(),
                oauth_account: None,
            }],
        };
        assert!(validate_payload(&malformed).is_err());
        assert_eq!(decode_call_count(), before);

        let mut total = RAW_TOTAL_MAX_BYTES - 1;
        add_raw_total_unit(&mut total, 1).unwrap();
        assert!(add_raw_total_unit(&mut total, 1).is_err());
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn export_replaces_existing_destination_on_desktop_platforms() {
        let source = test_env("vault-desktop-overwrite");
        add_codex(
            &source,
            "codex",
            "account-id",
            "codex@example.test",
            "fixture-token",
        );
        let path = vault_path(&source, "replace.switcher-vault");
        let first = export(
            &source,
            &path,
            vec![selection(&source, "codex", "codex", false)],
        )
        .unwrap();
        let first_bytes = fs::read(&path).unwrap();
        let second = export(
            &source,
            &path,
            vec![selection(&source, "codex", "codex", false)],
        )
        .unwrap();
        assert_ne!(fs::read(&path).unwrap(), first_bytes);
        assert_ne!(first.recovery_code, second.recovery_code);

        let target = test_env("vault-desktop-overwrite-target");
        assert_eq!(
            import(&target, &path, second.recovery_code.as_str().to_owned())
                .unwrap()
                .imported,
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn imported_profile_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let source = test_env("vault-unix-mode-source");
        add_claude(
            &source,
            "claude",
            "account-id",
            "claude@example.test",
            "fixture-token",
        );
        let path = vault_path(&source, "mode.switcher-vault");
        let exported = export(
            &source,
            &path,
            vec![selection(&source, "claude", "claude", false)],
        )
        .unwrap();
        let target = test_env("vault-unix-mode-target");
        import(&target, &path, exported.recovery_code.as_str().to_owned()).unwrap();
        let dir = target.profiles_dir(Provider::Claude).join("claude");
        for file in ["credentials.json", "oauth_account.json", "meta.json"] {
            let mode = fs::metadata(dir.join(file)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{file} mode was {mode:o}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn import_journal_and_marker_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let target = test_env("vault-private-import-control-files");
        let (stage_root, import_id) = create_stage_root(&target).unwrap();
        let record = journal(&import_id, "staging", Provider::Claude, "private");
        write_import_journal(&stage_root, &record).unwrap();
        let profile_stage = stage_root.join("claude/private");
        fs::create_dir_all(&profile_stage).unwrap();
        let marker = profile_stage.join(IMPORT_MARKER_FILE);
        accounts::atomic_write(&marker, import_id.as_bytes()).unwrap();

        for path in [stage_root.join(IMPORT_JOURNAL_FILE), marker] {
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{} mode was {mode:o}", path.display());
        }
        assert!(remove_tree_retry(&stage_root));
    }

    #[test]
    fn errors_never_echo_recovery_code_or_secret() {
        let source = test_env("vault-errors-secret");
        add_claude(
            &source,
            "profile",
            "identity",
            "hidden@example.test",
            "never-echo-token-value",
        );
        let path = vault_path(&source, "secret-error.switcher-vault");
        let result = export(
            &source,
            &path,
            vec![selection(&source, "claude", "profile", true)],
        )
        .unwrap();
        let target = test_env("vault-errors-secret-target");
        let wrong = "this-recovery-code-must-never-appear".to_string();
        let error = import(&target, &path, wrong.clone()).unwrap_err();
        assert!(!error.contains(&wrong));
        assert!(!error.contains("never-echo-token-value"));
        assert!(!error.contains("hidden@example.test"));
        assert!(!result.recovery_code.as_str().is_empty());
    }

    #[test]
    fn codex_account_id_keeps_accounts_policy_when_jwt_subject_differs() {
        let entry = VaultEntry {
            provider: "codex".into(),
            name: "mismatch".into(),
            hide_email: true,
            id: "account-a".into(),
            email: Some("a@example.test".into()),
            credential: encoded(
                &serde_json::to_vec(&serde_json::json!({
                    "tokens": {
                        "id_token": jwt("account-b", "a@example.test"),
                        "access_token": "fixture-access-token",
                        "account_id": "account-a"
                    }
                }))
                .unwrap(),
            ),
            oauth_account: None,
        };
        assert!(validate_payload(&VaultPayload {
            format_version: PAYLOAD_VERSION,
            entries: vec![entry],
        })
        .is_ok());
    }
}

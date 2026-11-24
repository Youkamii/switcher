//! 다른 PC로 옮기기 (#189, 사용자 지시·결정 2026-11-23).
//!
//! A PC의 구독 계정 로그인과 API 키를 잠가서 B PC에 넣는다. "옮기는 게 귀찮아서" 쓰는 기능이다.
//!
//! **나르는 방법은 둘 다 있다** (사용자 지시: "둘 다 있으면 되는 거 아냐?"): 복사해서 붙여넣는 한 줄
//! 글과, 그 글을 그대로 담은 파일(`.swx`). 형식은 하나다 — `SWX2.<글자 수>.<base64>`.
//! 새 방법을 넣는다고 있던 방법을 없애지 말 것.
//!
//! **만든 지 10분이 지나면 쓸 수 없다** (사용자 지시). 만든 시각은 머리말에 있고 머리말은 잠금의 인증
//! 대상이라, 시각을 고치면 풀리지 않는다. 검사는 받는 PC의 시계로 한다. 서버가 없으므로 비밀번호까지
//! 아는 사람이 자기 PC의 시계를 되돌리거나 직접 푸는 프로그램을 쓰는 것은 막지 못한다 — 이 한계는
//! 화면과 문서에 적혀 있다. 비밀번호를 모르는 사람은 10분 전이든 후든 열지 못한다.
//!
//! **받는 쪽은 전부 덮어쓴다** (사용자 결정):
//! - 같은 계정(계정 ID)이 있으면 그 프로필의 로그인을 받은 것으로 바꾼다. 그 계정이 지금 CLI에
//!   로그인된 계정이면 그 로그인도 함께 바꾼다 — 재로그인(login.rs `import_inner`)과 같은 순서다.
//!   그래야 받는 PC의 만료된 로그인이 살아난다.
//! - 같은 계정의 같은 이름 API 키는 덮어쓴다. 지금 사용 중이어도 덮어쓰고 새 키로 계속 사용한다.
//! - 받은 것에 없는 계정·키는 그대로 둔다. 지우는 것은 없다.
//!
//! **옮기지 않는 것**: 설정·화면 설정·메모(사용자 결정 — 두 PC를 다른 용도로 쓰면 받는 쪽 것이
//! 날아간다. 처음 판에 내가 묻지 않고 넣었다가 뺐다. 다시 넣지 말 것), 사용량 캐시, 런처,
//! `.bak`·손상 사본.
//!
//! **API 키는 옮기지 않는다** — API 키 기능을 뺐다 (#191, 사용자 지시 2026-11-24 "api 걍 빼버려").
//! v2.2.x가 만든 묶음에 들어 있는 키는 읽지 않고 버린다.
//!
//! 로그인 토큰이 들어가므로 통째로 잠근다:
//! - 열쇠는 사용자의 비밀번호에서 만든다 (Argon2id — 일부러 느리고 메모리를 많이 써서
//!   비밀번호를 하나씩 대입해 보는 공격을 비싸게 한다). 소금은 만들 때마다 새로 뽑는다.
//! - 묶음은 줄인(deflate) 뒤 XChaCha20-Poly1305로 잠근다. 머리말(형식·열쇠 만들기 값·만든 시각·
//!   소금·논스)까지 인증에 넣어 한 바이트라도 바뀌면 풀리지 않는다.
//! - 받는 쪽은 다 풀고 검사한 뒤에야 쓴다. 이름은 경로가 되므로 저장할 때와 같은 규칙으로 거른다.
//!
//! 알아 둘 것: 구독 로그인은 갱신될 때마다 이전 것이 무효가 된다(usage.rs의 회전 주석). 같은 로그인을
//! 두 PC가 나눠 쓰면 먼저 갱신한 쪽만 남고 다른 쪽은 다시 로그인해야 한다 — 사용자는 알고 골랐다.
//! 금기: 비밀번호·열쇠·묶음 내용을 로그나 오류 문구에 넣지 말 것.

use crate::accounts::{self, atomic_write, validate_name, Env, LiveIdentity, Provider};
use crate::api_connections;
use base64::Engine;
use chacha20poly1305::{aead::Aead, aead::Payload, KeyInit, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use zeroize::Zeroize;

const MAGIC: &[u8; 8] = b"SWXFER2\n";
/// 처음 판의 파일(잠근 바이트 그대로)이 시작하는 글자 — 받지 않고, 다시 만들라고 말한다
const OLD_MAGIC: &str = "SWXFER1";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
/// 머리말: 형식 표시 + 열쇠 만들기 값(메모리 KiB·반복·병렬, 각 u32 LE) + 만든 시각(u64 LE, 초) + 소금 + 논스
const CREATED_AT: usize = MAGIC.len() + 12;
const SALT_AT: usize = CREATED_AT + 8;
const HEADER_LEN: usize = SALT_AT + SALT_LEN + NONCE_LEN;
/// 열쇠 만들기 기본값 — 64MiB·3회. 2026년의 보통 PC에서 0.2~0.5초.
const KDF_MEMORY_KIB: u32 = 64 * 1024;
const KDF_ITERATIONS: u32 = 3;
const KDF_LANES: u32 = 1;
/// 받는 쪽이 허용하는 상한 — 조작된 입력이 메모리를 끝없이 쓰게 하지 못한다
const KDF_MEMORY_MAX_KIB: u32 = 1024 * 1024;
const KDF_ITERATIONS_MAX: u32 = 16;
const KDF_LANES_MAX: u32 = 8;
pub const PASSWORD_MIN: usize = 8;
/// 만든 뒤 이 시간(초) 안에만 가져올 수 있다 (사용자 지시: 10분)
pub const VALID_SECS: u64 = 10 * 60;
/// 두 PC의 시계가 달라 "아직 만들어지지 않은" 것이 올 수 있다 — 이만큼까지는 받는다
const CLOCK_SKEW_SECS: u64 = 2 * 60;
/// 글·파일의 상한(계정 수백 개 분량)과, 풀어서 펼친 크기의 상한
const TEXT_MAX: usize = 8 * 1024 * 1024;
const PLAIN_MAX: usize = 32 * 1024 * 1024;
const PROFILE_FILE_MAX: usize = 1024 * 1024;
pub const FILE_EXT: &str = "swx";
/// 복사해 옮기는 글의 머리 — 뒤에 본문 글자 수와 본문(base64)이 온다
const TEXT_MARK: &str = "SWX2.";
/// 묶음 형식. 1·2는 배포되지 않은 처음 판들이다(설정·메모까지 담았거나 파일 내용을 base64로 담았다).
const BUNDLE_VERSION: u64 = 3;
const BAD_TEXT: &str = "옮기기용 글이나 파일이 아닙니다 — 보내는 PC에서 만든 것 전체를 넣으세요";
const BAD_PASSWORD: &str = "비밀번호가 다르거나 내용이 손상됐습니다";
const OLD_FORMAT: &str = "이전 판의 위젯에서 만든 것입니다 — 보내는 PC의 위젯을 업데이트하고 다시 만드세요";

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
    source_os: String,
    profiles: Vec<ProfileEntry>,
    /// v2.2.x가 넣던 API 키 자리 — 이제 늘 비워 보내고, 받은 것은 쓰지 않는다 (#191).
    /// 자리를 남겨 두는 것은 v2.2.x와 서로 주고받을 수 있게 하려는 것이다.
    #[serde(default)]
    api_keys: Vec<serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
struct ProfileEntry {
    provider: String,
    name: String,
    /// 파일 이름 → 내용. 전부 JSON 글이라 글 그대로 담는다 (base64로 싸면 복사할 글이 길어진다)
    files: BTreeMap<String, String>,
}

/// 내보낸 결과. `text`는 잠긴 글이라 화면에 보여도 된다.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct ExportResult {
    pub text: String,
    pub chars: usize,
    pub accounts: usize,
    /// 이 시각(유닉스 초)까지만 가져올 수 있다
    pub expires_at: u64,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct SavedFile {
    pub path: String,
    pub name: String,
}

#[derive(Serialize, Debug, Default, PartialEq, Eq)]
pub struct ImportResult {
    pub accounts_added: usize,
    pub accounts_replaced: usize,
    /// 덮어쓴 계정 가운데 지금 CLI에 로그인된 계정이라 그 로그인까지 바꾼 수
    pub live_replaced: usize,
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
    let params = argon2::Params::new(memory_kib, iterations, lanes, Some(32)).map_err(|_| BAD_TEXT)?;
    let kdf = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = [0u8; 32];
    kdf.hash_password_into(password, salt, &mut key)
        .map_err(|_| "열쇠를 만들지 못했습니다 (메모리 부족일 수 있습니다)")?;
    Ok(key)
}

fn seal_with(password: &str, plain: &[u8], created_at: u64, memory_kib: u32, iterations: u32) -> Result<Vec<u8>, String> {
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    for value in [memory_kib, iterations, KDF_LANES] {
        header.extend_from_slice(&value.to_le_bytes());
    }
    header.extend_from_slice(&created_at.to_le_bytes());
    let mut random = [0u8; SALT_LEN + NONCE_LEN];
    getrandom::getrandom(&mut random).map_err(|_| "난수를 만들지 못했습니다")?;
    header.extend_from_slice(&random);
    let (salt, nonce) = random.split_at(SALT_LEN);
    let mut key = derive_key(password.as_bytes(), salt, memory_kib, iterations, KDF_LANES)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    key.zeroize();
    // 글이 짧아야 복사해 옮기기 쉽다 — 잠그기 전에 줄인다 (잠근 것은 줄지 않는다)
    let mut packed = miniz_oxide::deflate::compress_to_vec(plain, 9);
    let sealed = cipher.encrypt(XNonce::from_slice(nonce), Payload { msg: &packed, aad: &header });
    packed.zeroize();
    header.extend_from_slice(&sealed.map_err(|_| "잠그지 못했습니다")?);
    Ok(header)
}

fn seal(password: &str, plain: &[u8], created_at: u64) -> Result<Vec<u8>, String> {
    seal_with(password, plain, created_at, KDF_MEMORY_KIB, KDF_ITERATIONS)
}

/// 머리말에 적힌 만든 시각. 잠금을 풀기 전에는 참고값이고, 풀리면 인증된 값이다.
fn created_at_of(sealed: &[u8]) -> Result<u64, String> {
    if sealed.len() <= HEADER_LEN || !sealed.starts_with(MAGIC) {
        return Err(BAD_TEXT.into());
    }
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&sealed[CREATED_AT..CREATED_AT + 8]);
    Ok(u64::from_le_bytes(bytes))
}

fn age_text(secs: u64) -> String {
    match secs {
        0..=3599 => format!("{}분", secs / 60),
        3600..=172_799 => format!("{}시간", secs / 3600),
        _ => format!("{}일", secs / 86_400),
    }
}

/// 만든 지 10분 안인가. 받는 PC의 시계로 잰다.
fn check_fresh(created_at: u64, now: u64) -> Result<(), String> {
    if created_at > now.saturating_add(CLOCK_SKEW_SECS) {
        return Err(format!(
            "두 PC의 시계가 {} 넘게 다릅니다 — 시계를 맞춘 뒤 보내는 PC에서 다시 만드세요",
            age_text(created_at - now)
        ));
    }
    let age = now.saturating_sub(created_at);
    if age > VALID_SECS {
        return Err(format!(
            "만든 지 {}이 지나 쓸 수 없습니다 (10분 안에만 가져올 수 있습니다) — 보내는 PC에서 다시 만드세요",
            age_text(age)
        ));
    }
    Ok(())
}

fn open(password: &str, sealed: &[u8], now: u64) -> Result<Vec<u8>, String> {
    // 만료는 비밀번호를 보기 전에 가른다. 만든 시각을 고쳐 여기를 지나가도 아래 잠금 풀기에서 걸린다.
    check_fresh(created_at_of(sealed)?, now)?;
    let (header, body) = sealed.split_at(HEADER_LEN);
    let number = |at: usize| u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]]);
    let (memory_kib, iterations, lanes) = (number(8), number(12), number(16));
    if memory_kib > KDF_MEMORY_MAX_KIB || iterations > KDF_ITERATIONS_MAX || lanes > KDF_LANES_MAX {
        return Err(BAD_TEXT.into());
    }
    let salt = &header[SALT_AT..SALT_AT + SALT_LEN];
    let nonce = &header[SALT_AT + SALT_LEN..];
    let mut key = derive_key(password.as_bytes(), salt, memory_kib, iterations, lanes)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    key.zeroize();
    let mut packed = cipher
        .decrypt(XNonce::from_slice(nonce), Payload { msg: body, aad: header })
        .map_err(|_| BAD_PASSWORD)?;
    let plain = miniz_oxide::inflate::decompress_to_vec_with_limit(&packed, PLAIN_MAX);
    packed.zeroize();
    plain.map_err(|_| BAD_TEXT.into())
}

// ── 복사해 옮기는 글 ───────────────────────────────────────────────
// `SWX2.<본문 글자 수>.<본문>` 한 줄. 본문은 표준 base64(`A-Z a-z 0-9 + /`, 패딩 없음)다 —
// 메신저가 서식 기호로 먹는 글자(`_ * ~ -`)가 없다. 글자 수를 적어 두는 것은 긴 글이 메신저에서
// 잘렸을 때 "비밀번호가 틀렸다"가 아니라 "잘렸다"고 말하기 위해서고, 뒤에 딸려 온 글을 버리기 위해서다.

fn text_codec() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD_NO_PAD
}

fn armor(sealed: &[u8]) -> String {
    let body = text_codec().encode(sealed);
    format!("{TEXT_MARK}{}.{body}", body.len())
}

/// 붙여넣는 길에 끼어든 것(줄바꿈·공백·보이지 않는 글자)을 걷어낸다
fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !c.is_whitespace()
                && !matches!(c, '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}')
        })
        .collect()
}

/// 글 머리(`SWX2.<글자 수>.`) 뒤의 본문과, 적혀 있는 본문 글자 수
fn text_body(cleaned: &str) -> Result<(usize, &str), String> {
    let start = cleaned.find(TEXT_MARK).ok_or(BAD_TEXT)?;
    let rest = &cleaned[start + TEXT_MARK.len()..];
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 9 {
        return Err(BAD_TEXT.into());
    }
    let expected: usize = rest[..digits].parse().map_err(|_| BAD_TEXT)?;
    match rest.as_bytes().get(digits) {
        Some(b'.') => Ok((expected, &rest[digits + 1..])),
        // 글자 수까지만 오고 끊겼다
        None => Ok((expected, "")),
        Some(_) => Err(BAD_TEXT.into()),
    }
}

fn dearmor(text: &str) -> Result<Vec<u8>, String> {
    if text.len() > TEXT_MAX {
        return Err(BAD_TEXT.into());
    }
    if text.trim_start().starts_with(OLD_MAGIC) {
        return Err(OLD_FORMAT.into());
    }
    let cleaned = clean(text);
    let (expected, rest) = text_body(&cleaned)?;
    if expected > TEXT_MAX {
        return Err(BAD_TEXT.into());
    }
    // 뒤에 딸려 온 글(메신저의 이름·시각 등)은 버린다
    let body: String = rest.chars().take(expected).collect();
    let got = body.chars().count();
    if got < expected {
        return Err(format!(
            "복사한 글이 중간에 잘렸습니다 ({expected}자 중 {got}자만 왔습니다) — 전체를 다시 복사하거나 파일로 옮기세요"
        ));
    }
    text_codec()
        .decode(body)
        .map_err(|_| "복사한 글이 중간에 바뀌었습니다 — 다시 복사하거나 파일로 옮기세요".into())
}

/// 글의 앞부분만 보고 만든 시각을 읽는다 (파일 목록에서 못 쓰는 것을 거르는 데 쓴다)
fn peek_created_at(head: &str) -> Option<u64> {
    // 머리말을 담는 base64 글자 수 — 4의 배수로 올려 그만큼만 푼다
    const HEAD_CHARS: usize = HEADER_LEN.div_ceil(3) * 4;
    let cleaned = clean(head);
    let (_, rest) = text_body(&cleaned).ok()?;
    let bytes = text_codec().decode(rest.get(..HEAD_CHARS)?).ok()?;
    if !bytes.starts_with(MAGIC) {
        return None;
    }
    let mut created = [0u8; 8];
    created.copy_from_slice(bytes.get(CREATED_AT..CREATED_AT + 8)?);
    Some(u64::from_le_bytes(created))
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

fn collect(env: &Env) -> Result<Bundle, String> {
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
                    // JSON 글이 아닌 파일은 계정 정보가 아니다 — 담지 않는다
                    Ok(bytes) => {
                        if let Ok(text) = String::from_utf8(bytes) {
                            files.insert((*file).to_string(), text);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err(format!("계정 '{name}'의 파일을 읽지 못했습니다")),
                }
            }
            if let Some((_, cred, block)) = live.as_ref().filter(|(id, ..)| *id == meta.id) {
                if let Ok(text) = String::from_utf8(cred.clone()) {
                    files.insert(provider.credential_file_name().to_string(), text);
                }
                if let Some(text) = block.as_ref().and_then(|block| serde_json::to_string_pretty(block).ok()) {
                    files.insert("oauth_account.json".to_string(), text);
                }
            }
            if files.contains_key(provider.credential_file_name()) && files.contains_key("meta.json") {
                profiles.push(ProfileEntry { provider: provider.dir_name().into(), name, files });
            }
        }
    }
    Ok(Bundle {
        version: BUNDLE_VERSION,
        source_os: std::env::consts::OS.into(),
        profiles,
        api_keys: Vec::new(),
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

/// 옮기기 파일을 두거나 찾는 곳 — 다운로드·바탕화면·홈
fn transfer_dirs(env: &Env) -> [PathBuf; 3] {
    [env.home.join("Downloads"), env.home.join("Desktop"), env.home.clone()]
}

/// 우리가 다루는 자리에 있는 옮기기 파일인가 (화면이 넘긴 경로를 그대로 믿지 않는다)
pub fn is_transfer_location(env: &Env, path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some(FILE_EXT)
        && path.is_file()
        && path.parent().is_some_and(|parent| transfer_dirs(env).iter().any(|dir| dir == parent))
}

fn check_password(password: &str) -> Result<(), String> {
    if password.chars().count() < PASSWORD_MIN {
        return Err(format!("비밀번호는 {PASSWORD_MIN}자 이상으로 정하세요"));
    }
    Ok(())
}

fn make_text(password: &str, bundle: &Bundle, now: u64) -> Result<ExportResult, String> {
    let mut plain = serde_json::to_vec(bundle).map_err(|_| "묶음을 만들지 못했습니다")?;
    let sealed = seal(password, &plain, now);
    plain.zeroize();
    let text = armor(&sealed?);
    Ok(ExportResult {
        chars: text.chars().count(),
        text,
        accounts: bundle.profiles.len(),
        expires_at: now + VALID_SECS,
    })
}

/// 복사해 옮길 글을 만든다. 파일로도 남기려면 이 글을 `save_file`에 넘긴다.
pub fn export(env: &Env, password: &str) -> Result<ExportResult, String> {
    check_password(password)?;
    let _mutation = accounts::MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let bundle = collect(env)?;
    make_text(password, &bundle, accounts::now())
}

fn write_file(env: &Env, sealed: &[u8], stamp: &str) -> Result<SavedFile, String> {
    let name = format!("switcher-transfer-{stamp}.{FILE_EXT}");
    let path = export_dir(env).join(&name);
    atomic_write(&path, format!("{}\n", armor(sealed)).as_bytes())?;
    Ok(SavedFile { path: path.to_string_lossy().into_owned(), name })
}

/// 만들어 둔 글을 다운로드 폴더에 파일로 남긴다 — 내용은 같은 글이다. 우리 형식이 아니면 쓰지 않는다.
pub fn save_file(env: &Env, text: &str) -> Result<SavedFile, String> {
    let sealed = dearmor(text)?;
    created_at_of(&sealed)?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    write_file(env, &sealed, &stamp)
}

// ── 되살리기 ───────────────────────────────────────────────────────

fn parse(password: &str, sealed: &[u8], now: u64) -> Result<Bundle, String> {
    let mut plain = open(password, sealed, now)?;
    let value: Result<Value, _> = serde_json::from_slice(&plain);
    plain.zeroize();
    let value = value.map_err(|_| BAD_TEXT)?;
    match value.get("version").and_then(Value::as_u64) {
        Some(BUNDLE_VERSION) => {}
        Some(version) if version < BUNDLE_VERSION => return Err(OLD_FORMAT.into()),
        Some(_) => return Err("더 새 버전의 위젯에서 만든 것입니다 — 이 PC의 위젯을 업데이트하세요".into()),
        None => return Err(BAD_TEXT.into()),
    }
    serde_json::from_value(value).map_err(|_| BAD_TEXT.into())
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
    let provider = Provider::parse(&entry.provider).ok()?;
    validate_name(&entry.name).ok()?;
    // 허용 목록에 있는 이름만 꺼낸다 — 묶음에 든 다른 이름은 어디에도 쓰이지 않는다
    let file = |name: &str| -> Option<Option<Vec<u8>>> {
        if !profile_files(provider).contains(&name) {
            return Some(None);
        }
        match entry.files.get(name) {
            None => Some(None),
            Some(text) => (text.len() <= PROFILE_FILE_MAX).then(|| Some(text.as_bytes().to_vec())),
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
    // 이전 버전에서 Codex를 API 키로 켜 둔 채라면 그 계정의 구독 로그인은 활성 자리가 아니라 복원
    // 지점에 보관돼 있다. 그것도 바꾼다 — 아니면 해제될 때 옛 로그인이 돌아와 받은 로그인을 덮는다.
    let updates_held = !updates_live
        && api_connections::replace_protected_oauth(env, provider, &incoming.ident.id, &incoming.credential)?;
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
    Ok((replaced, updates_live || updates_held))
}

fn apply(env: &Env, bundle: Bundle) -> Result<ImportResult, String> {
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
    Ok(result)
}

fn read_transfer_file(path: &Path) -> Result<String, String> {
    let meta = fs::metadata(path).map_err(|_| "파일을 찾을 수 없습니다")?;
    if !meta.is_file() || meta.len() > TEXT_MAX as u64 {
        return Err(BAD_TEXT.into());
    }
    let bytes = fs::read(path).map_err(|_| "파일을 읽지 못했습니다")?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// 붙여넣은 글(또는 화면이 읽어 준 파일 내용)이 먼저고, 없으면 찾아 둔 파일이다
fn read_source(env: &Env, text: Option<&str>, path: Option<&str>) -> Result<Vec<u8>, String> {
    match (text.map(str::trim).filter(|text| !text.is_empty()), path) {
        (Some(text), _) => dearmor(text),
        (None, Some(path)) => {
            let path = Path::new(path);
            if !is_transfer_location(env, path) {
                return Err("파일을 찾을 수 없습니다".into());
            }
            dearmor(&read_transfer_file(path)?)
        }
        (None, None) => Err("복사한 글을 붙여넣거나 파일을 고르세요".into()),
    }
}

fn import_at(
    env: &Env,
    password: &str,
    text: Option<&str>,
    path: Option<&str>,
    now: u64,
) -> Result<ImportResult, String> {
    let sealed = read_source(env, text, path)?;
    // 다 풀고 검사한 뒤에야 쓴다 — 만료·틀린 비밀번호·변조는 여기서 끝난다
    let bundle = parse(password, &sealed, now)?;
    apply(env, bundle)
}

pub fn import(env: &Env, password: &str, text: Option<&str>, path: Option<&str>) -> Result<ImportResult, String> {
    import_at(env, password, text, path, accounts::now())
}

/// 흔히 받아 두는 곳에서 지금 가져올 수 있는 옮기기 파일을 찾는다 — 최근 것부터.
/// 만든 지 10분이 지난 것은 내놓지 않는다(골라도 못 쓴다).
fn candidates_at(env: &Env, now: u64) -> Vec<Candidate> {
    let mut found = Vec::new();
    for dir in transfer_dirs(env) {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some(FILE_EXT) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() || meta.len() > TEXT_MAX as u64 {
                continue;
            }
            let mut head = [0u8; 256];
            let Ok(read) = fs::File::open(&path).and_then(|mut file| std::io::Read::read(&mut file, &mut head)) else {
                continue;
            };
            let fresh = peek_created_at(&String::from_utf8_lossy(&head[..read]))
                .is_some_and(|created| check_fresh(created, now).is_ok());
            if !fresh {
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
    found.sort_by(|a, b| b.modified.cmp(&a.modified).then(b.name.cmp(&a.name)));
    found.truncate(10);
    found
}

pub fn candidates(env: &Env) -> Vec<Candidate> {
    candidates_at(env, accounts::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::test_support::test_env;
    use crate::api_connections::tests::{add_account, save_fixture, use_fixture, Memory};

    /// 검사에서 "만든 시각"으로 쓰는 값
    const T0: u64 = 1_795_400_000;

    // 검사는 가벼운 열쇠 만들기 값으로 돈다 — 기본값(64MiB)은 한 번만 왕복해 본다
    fn seal_fast(password: &str, plain: &[u8]) -> Vec<u8> {
        seal_with(password, plain, T0, 64, 1).unwrap()
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

    /// 계정 둘이 있는 보내는 PC. 설정·메모도 있지만 옮기는 대상이 아니다.
    fn source(tag: &str) -> Env {
        let env = test_env(tag);
        account(&env, Provider::Claude, "alpha", "acct-alpha", "a-alpha");
        account(&env, Provider::Codex, "delta", "acct-delta", "a-delta");
        let alpha = env.profiles_dir(Provider::Claude).join("alpha");
        fs::write(alpha.join("credentials.json.bak"), b"old-generation").unwrap();
        fs::write(alpha.join("plan.json"), br#"{"tier":20}"#).unwrap();
        fs::write(env.store.join("settings.json"), br#"{"accent_theme":"violet","tfsd":true}"#).unwrap();
        fs::write(env.store.join("memo.json"), br#"{"active":0,"tabs":["memo-from-a"]}"#).unwrap();
        env
    }

    fn bundle_of(env: &Env) -> Bundle {
        collect(env).unwrap()
    }
    fn bring(to: &Env, bundle: Bundle) -> ImportResult {
        apply(to, bundle).unwrap()
    }
    /// 보내는 PC에서 만든 글 (가벼운 열쇠 만들기 값, 만든 시각 T0)
    fn text_of(env: &Env, password: &str) -> String {
        armor(&seal_fast(password, &serde_json::to_vec(&bundle_of(env)).unwrap()))
    }

    #[test]
    fn the_default_lock_round_trips_and_a_wrong_password_opens_nothing() {
        let sealed = seal("correct horse", b"secret payload", T0).unwrap();
        assert_eq!(open("correct horse", &sealed, T0).unwrap(), b"secret payload");
        assert_eq!(open("correct horsf", &sealed, T0).unwrap_err(), BAD_PASSWORD);
    }

    #[test]
    fn the_locked_bytes_show_nothing_and_any_changed_byte_refuses_to_open() {
        let plain = br#"{"key":"sk-fake-transfer-key-000001","more":"sk-fake-transfer-key-000001 sk-fake-transfer-key-000001"}"#;
        let sealed = seal_fast("password-1", plain);
        // 같은 내용을 두 번 잠가도 같은 것이 나오지 않는다 (소금·논스가 매번 새로 뽑힌다)
        assert_ne!(sealed, seal_fast("password-1", plain));
        assert!(!sealed.windows(7).any(|window| window == b"sk-fake"));
        // 잠그기 전에 줄인다 — 복사할 글이 짧아진다
        assert!(sealed.len() < plain.len() + HEADER_LEN);
        for at in [8, 12, 16, CREATED_AT, SALT_AT, SALT_AT + SALT_LEN, HEADER_LEN - 1, HEADER_LEN, sealed.len() - 1] {
            let mut changed = sealed.clone();
            changed[at] ^= 1;
            assert!(open("password-1", &changed, T0).is_err(), "byte {at} must be authenticated");
        }
        // 잘린 것, 다른 것, 터무니없는 열쇠 만들기 값
        assert_eq!(open("password-1", &sealed[..HEADER_LEN], T0).unwrap_err(), BAD_TEXT);
        assert_eq!(open("password-1", &[7u8; 200], T0).unwrap_err(), BAD_TEXT);
        let mut greedy = sealed.clone();
        greedy[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(open("password-1", &greedy, T0).unwrap_err(), BAD_TEXT);
    }

    /// 사용자 지시: 만든 지 10분이 지나면 쓸 수 없다
    #[test]
    fn it_cannot_be_used_more_than_ten_minutes_after_it_was_made() {
        let sealed = seal_fast("password-1", b"payload");
        assert!(open("password-1", &sealed, T0 + VALID_SECS).is_ok());
        let late = open("password-1", &sealed, T0 + VALID_SECS + 1).unwrap_err();
        assert!(late.contains("만든 지 10분이 지나"), "{late}");
        assert!(open("password-1", &sealed, T0 + 3 * 86_400).unwrap_err().contains("만든 지 3일이 지나"));
        // 만료는 비밀번호와 상관없이 먼저 말한다
        assert!(open("wrong-password", &sealed, T0 + VALID_SECS + 1).unwrap_err().contains("10분"));
        // 받는 PC의 시계가 조금 느린 것은 받아 주고, 많이 다르면 시계를 말한다
        assert!(open("password-1", &sealed, T0 - CLOCK_SKEW_SECS).is_ok());
        assert!(open("password-1", &sealed, T0 - CLOCK_SKEW_SECS - 1).unwrap_err().contains("시계"));
        // 만든 시각을 고쳐서 새것처럼 보이게 해도 풀리지 않는다 — 머리말은 잠금에 묶여 있다
        let much_later = T0 + 7 * 86_400;
        let mut forged = sealed.clone();
        forged[CREATED_AT..CREATED_AT + 8].copy_from_slice(&much_later.to_le_bytes());
        assert_eq!(created_at_of(&forged).unwrap(), much_later);
        assert_eq!(open("password-1", &forged, much_later).unwrap_err(), BAD_PASSWORD);
    }

    #[test]
    fn the_copied_text_survives_what_messengers_do_to_it() {
        // 줄지 않는 내용이어야 글이 길다 (같은 글자만 있으면 몇십 자로 줄어든다)
        let noise: Vec<u8> = (0..900u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        let sealed = seal_fast("password-1", &noise);
        let text = armor(&sealed);
        assert!(text.starts_with("SWX2.") && text.is_ascii() && !text.contains(['_', '-', '*', '~', '=', ' ', '\n']));
        assert_eq!(dearmor(&text).unwrap(), sealed);
        // 줄바꿈·공백·보이지 않는 글자가 끼고, 앞뒤에 다른 글이 붙어도 같은 것이 나온다
        let mut wrapped = String::from("홍길동 [오후 3:15]\n");
        for (at, c) in text.chars().enumerate() {
            wrapped.push(c);
            match at % 61 {
                17 => wrapped.push('\n'),
                40 => wrapped.push('\u{200B}'),
                52 => wrapped.push_str(" \r\n\u{00A0}"),
                _ => {}
            }
        }
        wrapped.push_str("\n보냄 3:16PM abc123");
        assert_eq!(dearmor(&wrapped).unwrap(), sealed);
        // 실수로 두 번 붙여넣어도 앞의 것을 쓴다
        assert_eq!(dearmor(&format!("{text}{text}")).unwrap(), sealed);
        // 잘린 글은 "잘렸다"고 말한다 — 비밀번호 탓으로 돌리지 않는다
        let body_len = text_codec().encode(&sealed).len();
        let cut = dearmor(&text[..text.len() - 300]).unwrap_err();
        assert!(cut.contains(&format!("{body_len}자 중 {}자만", body_len - 300)), "{cut}");
        assert!(dearmor("SWX2.1200").unwrap_err().contains("1200자 중 0자만"));
        // 글자가 바뀐 글, 우리 글이 아닌 것, 처음 판의 파일
        let swapped = text.replacen('A', "?", 1).replacen('B', "?", 1).replacen('C', "?", 1);
        assert!(dearmor(&swapped).unwrap_err().contains("바뀌었습니다"));
        assert_eq!(dearmor("안녕하세요 그냥 메모입니다").unwrap_err(), BAD_TEXT);
        assert_eq!(dearmor("").unwrap_err(), BAD_TEXT);
        assert_eq!(dearmor("SWX2.abc.def").unwrap_err(), BAD_TEXT);
        assert_eq!(dearmor("SWXFER1\n\u{1}\u{2}binary").unwrap_err(), OLD_FORMAT);
        assert_eq!(peek_created_at(&text[..200]), Some(T0));
        assert_eq!(peek_created_at("SWX2.12.AAAA"), None);
    }

    #[test]
    fn logins_arrive_on_an_empty_pc_and_nothing_else_does() {
        let from = source("transfer-from");
        // 묶음에는 로그인만 있다 — 설정·메모는 들어가지 않는다
        let plain = String::from_utf8(serde_json::to_vec(&bundle_of(&from)).unwrap()).unwrap();
        assert!(!plain.contains("memo-from-a") && !plain.contains("violet") && !plain.contains("old-generation"));
        let text = text_of(&from, "password-1");
        for secret in ["fake-access", "fake-refresh", "acct-alpha"] {
            assert!(!text.contains(secret));
        }
        let to = test_env("transfer-to");
        // 틀린 비밀번호·만료된 글은 아무것도 쓰지 않는다
        assert_eq!(import_at(&to, "password-2", Some(&text), None, T0).unwrap_err(), BAD_PASSWORD);
        assert!(import_at(&to, "password-1", Some(&text), None, T0 + VALID_SECS + 1).unwrap_err().contains("10분"));
        assert!(accounts::profile_dirs(&to, Provider::Claude).unwrap().is_empty() && !to.store.join("api-connections").exists());
        let result = import_at(&to, "password-1", Some(&format!("  {text}\n")), None, T0 + 60).unwrap();
        assert_eq!((result.accounts_added, result.accounts_replaced, result.live_replaced), (2, 0, 0));
        assert!(result.problems.is_empty() && result.rejected == 0);
        // 계정: 같은 이름·같은 ID·같은 로그인. .bak은 오지 않는다.
        let alpha = to.profiles_dir(Provider::Claude).join("alpha");
        assert_eq!(accounts::read_meta(&alpha).unwrap().id, "acct-alpha");
        assert_eq!(profile_cred(&to, Provider::Claude, "alpha"), profile_cred(&from, Provider::Claude, "alpha"));
        assert!(alpha.join("plan.json").is_file() && !alpha.join("credentials.json.bak").exists());
        assert!(profile_cred(&to, Provider::Codex, "delta").contains("fake-refresh-a-delta"));
        // 설정·메모는 생기지 않는다. 받는 PC에 로그인된 CLI가 없으면 로그인 자리도 그대로 비어 있다.
        assert!(!to.store.join("settings.json").exists() && !to.store.join("memo.json").exists());
        // API 키 자리는 늘 비어 있고, 받는 PC에 키 보관소도 생기지 않는다 (#191)
        assert!(bundle_of(&from).api_keys.is_empty() && !to.store.join("api-connections").exists());
        assert!(!to.home.join(".claude/.credentials.json").exists() && !to.codex_home.join("auth.json").exists());
    }

    /// 사용자 결정: "그냥 완전히 덮어쓰게 해줘."
    #[test]
    fn an_existing_pc_is_overwritten_but_loses_nothing_that_was_not_sent() {
        let from = source("transfer-over-from");
        let to = test_env("transfer-over-to");
        // 받는 PC: 같은 계정이 다른 이름(mine)으로 있고, alpha라는 이름은 다른 계정이 쓰고, 받지 않은 beta도 있다
        account(&to, Provider::Claude, "mine", "acct-alpha", "b-old");
        account(&to, Provider::Claude, "alpha", "someone-else", "b-other");
        account(&to, Provider::Claude, "beta", "acct-beta", "b-beta");
        fs::write(to.store.join("settings.json"), br#"{"language":"en","accent_theme":"blue"}"#).unwrap();
        fs::write(to.store.join("memo.json"), br#"{"tabs":["memo-on-b"]}"#).unwrap();
        let result = bring(&to, bundle_of(&from));
        assert_eq!((result.accounts_added, result.accounts_replaced, result.live_replaced), (1, 1, 0));
        assert!(result.problems.is_empty() && result.rejected == 0);
        // 같은 계정: 받는 PC의 이름 그대로, 로그인만 받은 것으로. 옛 것은 한 세대 남는다.
        assert!(profile_cred(&to, Provider::Claude, "mine").contains("fake-refresh-a-alpha"));
        assert!(to.profiles_dir(Provider::Claude).join("mine/credentials.json.bak").is_file());
        // 이름만 같은 다른 계정과 받지 않은 계정은 그대로다
        assert_eq!(accounts::read_meta(&to.profiles_dir(Provider::Claude).join("alpha")).unwrap().id, "someone-else");
        assert!(profile_cred(&to, Provider::Claude, "alpha").contains("fake-refresh-b-other"));
        assert!(profile_cred(&to, Provider::Claude, "beta").contains("fake-refresh-b-beta"));
        // 설정과 메모는 한 글자도 바뀌지 않는다
        assert_eq!(fs::read(to.store.join("settings.json")).unwrap(), br#"{"language":"en","accent_theme":"blue"}"#);
        assert_eq!(fs::read(to.store.join("memo.json")).unwrap(), br#"{"tabs":["memo-on-b"]}"#);
        // 같은 것을 한 번 더 넣어도 계정이 늘지 않는다
        let again = bring(&to, bundle_of(&from));
        assert_eq!((again.accounts_added, again.accounts_replaced), (0, 2));
        assert_eq!(accounts::profile_dirs(&to, Provider::Claude).unwrap().len(), 3);
    }

    /// 받는 PC에서 지금 CLI에 로그인된 계정을 받으면 그 로그인도 바꾼다 — 만료된 로그인이 살아난다
    #[test]
    fn the_account_logged_in_on_the_receiving_pc_gets_the_new_login_too() {
        let from = source("transfer-live-from");
        let to = test_env("transfer-live-to");
        account(&to, Provider::Claude, "alpha", "acct-alpha", "b-expired");
        log_in(&to, Provider::Claude, "acct-alpha", "b-expired");
        // Codex에는 받지 않은 다른 계정이 로그인돼 있다 — 건드리지 않는다
        log_in(&to, Provider::Codex, "acct-other", "b-other");
        let result = bring(&to, bundle_of(&from));
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

    /// 이전 버전에서 Codex를 API 키로 켜 둔 PC면 그 계정의 구독 로그인은 따로 보관돼 있다 — 그것도 바꾼다
    #[test]
    fn a_login_held_aside_while_an_api_key_is_in_use_is_replaced_too() {
        let from = source("transfer-held-from");
        let to = test_env("transfer-held-to");
        let to_secrets = Memory::default();
        account(&to, Provider::Codex, "delta", "acct-delta", "b-old");
        log_in(&to, Provider::Codex, "acct-delta", "b-old");
        save_fixture(&to, &to_secrets, "codex", "acct-delta", "work", "sk-fake-b-codex-work-key-01");
        use_fixture(&to, &to_secrets, Provider::Codex, "work");
        assert!(live_cred(&to, Provider::Codex).contains("sk-fake-b-codex-work-key-01"));
        let result = bring(&to, bundle_of(&from));
        assert_eq!((result.accounts_added, result.accounts_replaced, result.live_replaced), (1, 1, 1));
        assert!(result.problems.is_empty());
        // API는 계속 사용 중이고 활성 자리의 키는 그대로다. 프로필은 받은 로그인이다.
        assert!(live_cred(&to, Provider::Codex).contains("sk-fake-b-codex-work-key-01"));
        assert!(api_connections::active_connection(&to, Provider::Codex).is_some());
        assert!(profile_cred(&to, Provider::Codex, "delta").contains("fake-refresh-a-delta"));
        // 사용을 해제하면 받은 로그인이 돌아온다 — 옛 로그인이 아니다
        api_connections::deactivate_for_oauth_locked(&to, Provider::Codex).unwrap();
        let live = live_cred(&to, Provider::Codex);
        assert!(live.contains("fake-refresh-a-delta") && !live.contains("fake-refresh-b-old"), "the old login must not come back");
    }

    /// 보내는 PC에서 지금 로그인된 계정은 프로필 사본이 아니라 활성 로그인을 담는다 —
    /// 사본은 마지막 전환 때의 것이라 그 뒤 갱신됐으면 이미 못 쓰는 값이다
    #[test]
    fn the_account_logged_in_on_the_sending_pc_is_sent_with_its_current_login() {
        let from = source("transfer-fresh-from");
        log_in(&from, Provider::Claude, "acct-alpha", "a-refreshed");
        let to = test_env("transfer-fresh-to");
        bring(&to, bundle_of(&from));
        let cred = profile_cred(&to, Provider::Claude, "alpha");
        assert!(cred.contains("fake-refresh-a-refreshed") && !cred.contains("fake-refresh-a-alpha"));
        // 보내는 PC는 읽기만 한다
        assert!(profile_cred(&from, Provider::Claude, "alpha").contains("fake-refresh-a-alpha"));
    }

    #[test]
    fn a_crafted_bundle_cannot_write_outside_the_profile_folder() {
        let to = test_env("transfer-crafted");
        let meta = r#"{"id":"x","email":null,"saved_at":1}"#;
        let entry = |name: &str, files: &[(&str, &str)]| ProfileEntry {
            provider: "claude".into(),
            name: name.into(),
            files: files.iter().map(|(file, body)| (file.to_string(), body.to_string())).collect(),
        };
        let cred = String::from_utf8(claude_cred("crafted")).unwrap();
        let bundle = Bundle {
            version: BUNDLE_VERSION,
            source_os: "test".into(),
            profiles: vec![
                entry("../escape", &[("credentials.json", &cred), ("meta.json", meta)]),
                entry("nul", &[("credentials.json", &cred), ("meta.json", meta)]),
                entry("nocred", &[("meta.json", meta)]),
                entry("notlogin", &[("credentials.json", "{}"), ("meta.json", meta)]),
                entry("extra", &[("credentials.json", &cred), ("meta.json", meta), ("../../evil.sh", &cred), ("run.sh", &cred), ("auth.json", &cred)]),
            ],
            // v2.2.x가 보낸 키는 무엇이 들어 있든 읽지 않는다
            api_keys: serde_json::from_value(serde_json::json!([
                {"provider": "claude", "name": "../up", "key": "sk-fake-transfer-key-000001"},
                {"provider": "claude", "account_id": "x", "name": "work", "key": "sk-fake-transfer-key-000001"}
            ]))
            .unwrap(),
        };
        let result = bring(&to, bundle);
        assert_eq!((result.accounts_added, result.rejected), (1, 4));
        let extra = to.profiles_dir(Provider::Claude).join("extra");
        let mut names: Vec<_> = fs::read_dir(&extra).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["credentials.json", "meta.json"]);
        assert!(!to.store.join("evil.sh").exists() && !to.home.join("evil.sh").exists());
        assert_eq!(accounts::profile_dirs(&to, Provider::Claude).unwrap().len(), 1);
        assert!(!to.store.join("api-connections").exists());
        // 처음 판의 묶음은 받지 않는다 — 무엇을 하면 되는지 말해 준다
        let old = seal_fast("password-1", br#"{"version":2,"created_at":0,"source_os":"macos","profiles":[],"api_keys":[]}"#);
        assert_eq!(parse("password-1", &old, T0).err().unwrap(), OLD_FORMAT);
    }

    /// 사용자 지시: 복사와 파일, 둘 다 된다. 파일의 내용은 복사하는 글과 같다.
    #[test]
    fn the_same_text_also_travels_as_a_file() {
        let from = source("transfer-file");
        fs::create_dir_all(from.home.join("Downloads")).unwrap();
        assert!(check_password("short").is_err());
        let made = make_text("password-1", &bundle_of(&from), T0).unwrap();
        assert_eq!((made.accounts, made.expires_at), (2, T0 + VALID_SECS));
        assert_eq!(made.chars, made.text.len());
        let saved = write_file(&from, &dearmor(&made.text).unwrap(), "20261123-101500").unwrap();
        assert_eq!(saved.name, "switcher-transfer-20261123-101500.swx");
        assert_eq!(fs::read_to_string(&saved.path).unwrap().trim(), made.text);
        // 우리 글이 아니면 파일로 쓰지 않는다
        assert!(save_file(&from, "그냥 글").is_err());
        // 찾기: 지금 쓸 수 있는 우리 파일만. 확장자만 같은 파일, 10분이 지난 파일은 내놓지 않는다.
        fs::write(from.home.join("Downloads/other.swx"), vec![7u8; 200]).unwrap();
        let found = candidates_at(&from, T0 + 60);
        assert_eq!(found.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), [saved.path.as_str()]);
        assert!(candidates_at(&from, T0 + VALID_SECS + 1).is_empty());
        assert!(is_transfer_location(&from, Path::new(&saved.path)));
        // 받는 PC: 그 파일을 다운로드 폴더에 두고 경로로 가져온다
        let to = test_env("transfer-file-to");
        fs::create_dir_all(to.home.join("Downloads")).unwrap();
        let moved = to.home.join("Downloads").join(&saved.name);
        fs::copy(&saved.path, &moved).unwrap();
        let moved = moved.to_string_lossy().into_owned();
        // 기본 열쇠 만들기 값으로 잠근 것을 실제 길(파일 → 풀기 → 넣기)로 끝까지 가 본다
        let result = import_at(&to, "password-1", None, Some(&moved), T0 + 120).unwrap();
        assert_eq!(result.accounts_added, 2);
        assert!(import_at(&to, "password-1", None, Some(&moved), T0 + VALID_SECS + 1).unwrap_err().contains("10분"));
        // 우리가 다루는 자리 밖의 경로는 읽지 않는다
        let outside = to.store.join("elsewhere.swx");
        fs::write(&outside, &made.text).unwrap();
        assert_eq!(
            import_at(&to, "password-1", None, Some(&outside.to_string_lossy()), T0).unwrap_err(),
            "파일을 찾을 수 없습니다"
        );
        assert!(import_at(&to, "password-1", None, None, T0).unwrap_err().contains("붙여넣거나"));
    }
}

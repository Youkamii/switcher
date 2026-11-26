//! 다른 PC로 옮기기 (#189, 사용자 지시·결정 2026-11-23).
//!
//! A PC의 구독 계정 로그인을 잠가서 B PC에 넣는다. "옮기는 게 귀찮아서" 쓰는 기능이다.
//!
//! **받는 PC만 열 수 있다** (#192, 사용자 결정 2026-11-26: "암호를 해독하기 힘든 방법을 고민해봐" →
//! 받는 코드 + 비밀번호). 받는 PC가 먼저 **받을 준비**를 눌러 받는 코드(X25519 공개 값)를 만들고,
//! 보내는 PC는 그 코드와 비밀번호로 잠근다. 여는 열쇠는 받는 PC의 메모리에만 있고(디스크에 쓰지 않는다)
//! 10분 뒤나 한 번 가져온 뒤에 지운다. 그래서
//! - 글만 가로챈 사람은 비밀번호를 찍어 볼 수조차 없다(열쇠의 절반이 받는 PC 안에만 있다).
//! - 10분 제한은 시계가 아니라 "열쇠가 없어졌다"로 지켜진다 — 시계를 되돌려도 열리지 않는다.
//! - 한 번 가져온 글은 다시 열리지 않는다.
//! 비밀번호를 함께 두는 것은 받는 코드가 가는 길에 바뀌치기당한 경우를 위해서다 — 그때는 비밀번호가
//! 마지막 잠금이다. 비밀번호만으로 잠그던 형식(`SWX2`)은 받지 않는다. 그 방식으로 되돌리지 말 것.
//!
//! **나르는 방법은 둘 다 있다** (사용자 지시: "둘 다 있으면 되는 거 아냐?"): 복사해서 붙여넣는 한 줄
//! 글과, 그 글을 그대로 담은 파일(`.swx`). 형식은 하나다 — `SWX3.<글자 수>.<base64>`.
//! 새 방법을 넣는다고 있던 방법을 없애지 말 것.
//!
//! **10분이 지나면 쓸 수 없다** (사용자 지시). 받는 코드를 만든 지 10분이 지나면 받는 PC가 열쇠를
//! 지운다(`Receiver::alive` — 흐른 시간과 벽시계 둘 다 본다). 머리말의 만든 시각은 찾아 주는 파일
//! 목록에서 옛 파일을 빼는 데만 쓴다. 남는 한계: 받는 PC 자체가 뚫려 있으면(악성 프로그램) 막지 못한다.
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
//! - 열쇠는 둘을 섞어 만든다(HKDF-SHA256): 받는 코드와 보내는 쪽 일회용 열쇠의 X25519 공유 값,
//!   그리고 비밀번호에서 만든 값(Argon2id — 일부러 느리고 메모리를 많이 쓴다). 소금은 매번 새로 뽑는다.
//! - 묶음은 줄인(deflate) 뒤 XChaCha20-Poly1305로 잠근다. 머리말(형식·열쇠 만들기 값·만든 시각·
//!   소금·논스·받는 코드·보내는 쪽 공개 값)까지 인증에 넣어 한 바이트라도 바뀌면 풀리지 않는다.
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
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroize;

const MAGIC: &[u8; 8] = b"SWXFER3\n";
/// 처음 판의 파일(잠근 바이트 그대로)이 시작하는 글자 — 받지 않고, 다시 만들라고 말한다
const OLD_MAGIC: &str = "SWXFER1";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;
/// 머리말: 형식 표시 + 열쇠 만들기 값(메모리 KiB·반복·병렬, 각 u32 LE) + 만든 시각(u64 LE, 초) + 소금 + 논스
/// + 받는 코드(받는 PC의 공개 값) + 보내는 쪽 일회용 공개 값
const CREATED_AT: usize = MAGIC.len() + 12;
const SALT_AT: usize = CREATED_AT + 8;
const RECIPIENT_AT: usize = SALT_AT + SALT_LEN + NONCE_LEN;
const SENDER_AT: usize = RECIPIENT_AT + KEY_LEN;
const HEADER_LEN: usize = SENDER_AT + KEY_LEN;
/// 열쇠 만들기 기본값 — 64MiB·3회. 2026년의 보통 PC에서 0.2~0.5초.
const KDF_MEMORY_KIB: u32 = 64 * 1024;
const KDF_ITERATIONS: u32 = 3;
const KDF_LANES: u32 = 1;
/// 받는 쪽이 허용하는 상한 — 조작된 입력이 메모리를 끝없이 쓰게 하지 못한다
const KDF_MEMORY_MAX_KIB: u32 = 1024 * 1024;
const KDF_ITERATIONS_MAX: u32 = 16;
const KDF_LANES_MAX: u32 = 8;
pub const PASSWORD_MIN: usize = 8;
/// 받는 코드를 만든 뒤 이 시간(초) 안에만 가져올 수 있다 (사용자 지시: 10분)
pub const VALID_SECS: u64 = 10 * 60;
/// 두 PC의 시계가 달라 "아직 만들어지지 않은" 것이 올 수 있다 — 이만큼까지는 받는다
const CLOCK_SKEW_SECS: u64 = 2 * 60;
/// 글·파일의 상한(계정 수백 개 분량)과, 풀어서 펼친 크기의 상한
const TEXT_MAX: usize = 8 * 1024 * 1024;
const PLAIN_MAX: usize = 32 * 1024 * 1024;
const PROFILE_FILE_MAX: usize = 1024 * 1024;
pub const FILE_EXT: &str = "swx";
/// 복사해 옮기는 글의 머리 — 뒤에 본문 글자 수와 본문(base64)이 온다
const TEXT_MARK: &str = "SWX3.";
/// 비밀번호만으로 잠그던 형식(v2.2.0~2.2.3)의 머리 — 받지 않는다 (#192)
const OLD_TEXT_MARK: &str = "SWX2.";
/// 받는 코드의 머리 — 뒤에 공개 값과 확인용 3바이트를 담은 base64 47자가 온다
const CODE_MARK: &str = "SWR1.";
const CODE_CHECK_LEN: usize = 3;
const CODE_CHARS: usize = 47;
/// 묶음 형식. 1·2는 배포되지 않은 처음 판들이다(설정·메모까지 담았거나 파일 내용을 base64로 담았다).
const BUNDLE_VERSION: u64 = 3;
const BAD_TEXT: &str = "옮기기용 글이나 파일이 아닙니다 — 보내는 PC에서 만든 것 전체를 넣으세요";
const BAD_PASSWORD: &str = "비밀번호가 다르거나 내용이 손상됐습니다";
const OLD_FORMAT: &str = "이전 버전의 위젯에서 만든 것입니다 — 두 PC의 위젯을 업데이트하고, 받는 PC의 「받을 준비」부터 다시 하세요";
const BAD_CODE: &str = "받는 코드가 맞지 않습니다 — 받는 PC에 보이는 코드 전체를 복사해 넣으세요";
const NOT_READY: &str = "받을 준비가 되어 있지 않습니다 — 「받을 준비」를 눌러 나온 코드를 보내는 PC에 넣고 다시 만드세요";
const NOT_FOR_THIS: &str = "지금의 받는 코드로 잠근 것이 아닙니다 — 지금 보이는 코드를 보내는 PC에 넣고 다시 만드세요";

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
}

/// 받는 PC가 보내는 PC에 알려 주는 코드. 공개 값이라 남이 봐도 된다.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct ReceiveCode {
    pub code: String,
    /// 이 시각(유닉스 초, 이 PC의 시계)까지만 받을 수 있다
    pub expires_at: u64,
    pub seconds_left: u64,
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

/// 받는 PC의 열쇠. 메모리에만 있다 — 디스크에 쓰지 말 것, Debug를 달지 말 것.
/// 위젯을 끄면 사라지고, 그러면 받는 코드를 다시 만들어야 한다.
pub struct Receiver {
    secret: StaticSecret,
    public: [u8; KEY_LEN],
    started: Instant,
    started_wall: u64,
}

impl Receiver {
    fn new(now: u64) -> Result<Self, String> {
        let mut bytes = [0u8; KEY_LEN];
        getrandom::getrandom(&mut bytes).map_err(|_| "난수를 만들지 못했습니다")?;
        let secret = StaticSecret::from(bytes);
        bytes.zeroize();
        let public = PublicKey::from(&secret).to_bytes();
        Ok(Self { secret, public, started: Instant::now(), started_wall: now })
    }

    /// 흐른 시간과 벽시계 가운데 하나라도 10분을 넘겼으면 끝이다. 시계를 되돌려도 흐른 시간이 남고,
    /// PC가 잠들어 흐른 시간이 멈춰도 벽시계가 남는다.
    fn alive(&self, now: u64) -> bool {
        self.started.elapsed() <= Duration::from_secs(VALID_SECS) && now.saturating_sub(self.started_wall) <= VALID_SECS
    }

    fn seconds_left(&self, now: u64) -> u64 {
        let by_run = VALID_SECS.saturating_sub(self.started.elapsed().as_secs());
        by_run.min(VALID_SECS.saturating_sub(now.saturating_sub(self.started_wall)))
    }

    fn code(&self, now: u64) -> ReceiveCode {
        let left = self.seconds_left(now);
        ReceiveCode { code: code_text(&self.public), expires_at: now + left, seconds_left: left }
    }
}

/// 이 PC가 지금 기다리는 받기. 하나뿐이다 — 새로 만들면 앞의 코드는 죽는다.
static RECEIVER: Mutex<Option<Receiver>> = Mutex::new(None);

fn code_check(public: &[u8; KEY_LEN]) -> [u8; CODE_CHECK_LEN] {
    let digest = Sha256::new().chain_update(b"switcher-receive-code").chain_update(public).finalize();
    let mut check = [0u8; CODE_CHECK_LEN];
    check.copy_from_slice(&digest[..CODE_CHECK_LEN]);
    check
}

fn code_text(public: &[u8; KEY_LEN]) -> String {
    let mut bytes = public.to_vec();
    bytes.extend_from_slice(&code_check(public));
    format!("{CODE_MARK}{}", text_codec().encode(bytes))
}

/// 받는 코드에서 공개 값을 꺼낸다. 확인용 바이트가 맞지 않으면(잘렸거나 잘못 옮겼으면) 받지 않는다.
fn parse_code(text: &str) -> Result<[u8; KEY_LEN], String> {
    if text.len() > 4096 {
        return Err(BAD_CODE.into());
    }
    let cleaned = clean(text);
    let start = cleaned.find(CODE_MARK).ok_or(BAD_CODE)?;
    let body: String = cleaned[start + CODE_MARK.len()..].chars().take(CODE_CHARS).collect();
    let bytes = text_codec().decode(body).map_err(|_| BAD_CODE)?;
    if bytes.len() != KEY_LEN + CODE_CHECK_LEN {
        return Err(BAD_CODE.into());
    }
    let mut public = [0u8; KEY_LEN];
    public.copy_from_slice(&bytes[..KEY_LEN]);
    if bytes[KEY_LEN..] != code_check(&public) {
        return Err(BAD_CODE.into());
    }
    Ok(public)
}

/// 잠금 열쇠: X25519 공유 값과 비밀번호에서 만든 값을 섞는다. 둘 중 하나만 알아서는 만들 수 없다.
fn mix_key(shared: &[u8; KEY_LEN], password_key: &[u8; KEY_LEN], salt: &[u8], keys: &[u8]) -> [u8; KEY_LEN] {
    let mut material = [0u8; KEY_LEN * 2];
    material[..KEY_LEN].copy_from_slice(shared);
    material[KEY_LEN..].copy_from_slice(password_key);
    let mut info = b"switcher-transfer-v3".to_vec();
    info.extend_from_slice(keys);
    let mut key = [0u8; KEY_LEN];
    hkdf::Hkdf::<Sha256>::new(Some(salt), &material)
        .expand(&info, &mut key)
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    material.zeroize();
    key
}

fn seal_with(
    password: &str,
    recipient: &[u8; KEY_LEN],
    plain: &[u8],
    created_at: u64,
    memory_kib: u32,
    iterations: u32,
) -> Result<Vec<u8>, String> {
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    for value in [memory_kib, iterations, KDF_LANES] {
        header.extend_from_slice(&value.to_le_bytes());
    }
    header.extend_from_slice(&created_at.to_le_bytes());
    let mut random = [0u8; SALT_LEN + NONCE_LEN + KEY_LEN];
    getrandom::getrandom(&mut random).map_err(|_| "난수를 만들지 못했습니다")?;
    // 이 글 하나에만 쓰는 열쇠 — 잠그고 나면 버린다. 보내는 PC도 다시 열 수 없다.
    let mut seed = [0u8; KEY_LEN];
    seed.copy_from_slice(&random[SALT_LEN + NONCE_LEN..]);
    let sender = StaticSecret::from(seed);
    seed.zeroize();
    random[SALT_LEN + NONCE_LEN..].zeroize();
    header.extend_from_slice(&random[..SALT_LEN + NONCE_LEN]);
    header.extend_from_slice(recipient);
    header.extend_from_slice(PublicKey::from(&sender).as_bytes());
    let shared = sender.diffie_hellman(&PublicKey::from(*recipient));
    if !shared.was_contributory() {
        return Err(BAD_CODE.into());
    }
    let (salt, nonce) = random[..SALT_LEN + NONCE_LEN].split_at(SALT_LEN);
    let mut password_key = derive_key(password.as_bytes(), salt, memory_kib, iterations, KDF_LANES)?;
    let mut key = mix_key(shared.as_bytes(), &password_key, salt, &header[RECIPIENT_AT..]);
    password_key.zeroize();
    let cipher = XChaCha20Poly1305::new((&key).into());
    key.zeroize();
    // 글이 짧아야 복사해 옮기기 쉽다 — 잠그기 전에 줄인다 (잠근 것은 줄지 않는다)
    let mut packed = miniz_oxide::deflate::compress_to_vec(plain, 9);
    let sealed = cipher.encrypt(XNonce::from_slice(nonce), Payload { msg: &packed, aad: &header });
    packed.zeroize();
    header.extend_from_slice(&sealed.map_err(|_| "잠그지 못했습니다")?);
    Ok(header)
}

fn seal(password: &str, recipient: &[u8; KEY_LEN], plain: &[u8], created_at: u64) -> Result<Vec<u8>, String> {
    seal_with(password, recipient, plain, created_at, KDF_MEMORY_KIB, KDF_ITERATIONS)
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

/// 만든 지 10분 안인가 — 찾아 주는 파일 목록에서 옛 파일을 빼는 데 쓴다.
/// 가져오기를 막는 것은 이 검사가 아니라 받는 PC의 열쇠다 (`Receiver::alive`).
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

fn open(receiver: &Receiver, password: &str, sealed: &[u8]) -> Result<Vec<u8>, String> {
    created_at_of(sealed)?;
    let (header, body) = sealed.split_at(HEADER_LEN);
    // 이 PC의 받는 코드로 잠근 것인가 — 비밀번호를 보기 전에 가른다
    if header[RECIPIENT_AT..SENDER_AT] != receiver.public {
        return Err(NOT_FOR_THIS.into());
    }
    let number = |at: usize| u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]]);
    let (memory_kib, iterations, lanes) = (number(8), number(12), number(16));
    if memory_kib > KDF_MEMORY_MAX_KIB || iterations > KDF_ITERATIONS_MAX || lanes > KDF_LANES_MAX {
        return Err(BAD_TEXT.into());
    }
    let salt = &header[SALT_AT..SALT_AT + SALT_LEN];
    let nonce = &header[SALT_AT + SALT_LEN..RECIPIENT_AT];
    let mut sender = [0u8; KEY_LEN];
    sender.copy_from_slice(&header[SENDER_AT..]);
    let shared = receiver.secret.diffie_hellman(&PublicKey::from(sender));
    if !shared.was_contributory() {
        return Err(BAD_TEXT.into());
    }
    let mut password_key = derive_key(password.as_bytes(), salt, memory_kib, iterations, lanes)?;
    let mut key = mix_key(shared.as_bytes(), &password_key, salt, &header[RECIPIENT_AT..]);
    password_key.zeroize();
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
// `SWX3.<본문 글자 수>.<본문>` 한 줄. 본문은 표준 base64(`A-Z a-z 0-9 + /`, 패딩 없음)다 —
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

/// 글 머리(`SWX3.<글자 수>.`) 뒤의 본문과, 적혀 있는 본문 글자 수
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
    if !cleaned.contains(TEXT_MARK) && cleaned.contains(OLD_TEXT_MARK) {
        return Err(OLD_FORMAT.into());
    }
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

fn make_text(password: &str, recipient: &[u8; KEY_LEN], bundle: &Bundle, now: u64) -> Result<ExportResult, String> {
    let mut plain = serde_json::to_vec(bundle).map_err(|_| "묶음을 만들지 못했습니다")?;
    let sealed = seal(password, recipient, &plain, now);
    plain.zeroize();
    let text = armor(&sealed?);
    Ok(ExportResult { chars: text.chars().count(), text, accounts: bundle.profiles.len() })
}

/// 복사해 옮길 글을 만든다 — `code`는 받는 PC가 보여 준 받는 코드다.
/// 파일로도 남기려면 이 글을 `save_file`에 넘긴다.
pub fn export(env: &Env, password: &str, code: &str) -> Result<ExportResult, String> {
    let recipient = parse_code(code)?;
    check_password(password)?;
    let _mutation = accounts::MUTATION_LOCK.lock().map_err(|_| "내부 잠금 오류")?;
    let bundle = collect(env)?;
    make_text(password, &recipient, &bundle, accounts::now())
}

/// 받을 준비 — 새 열쇠를 만들고 받는 코드를 돌려준다. 앞서 만든 코드는 이때 죽는다.
pub fn receive_start() -> Result<ReceiveCode, String> {
    let now = accounts::now();
    let receiver = Receiver::new(now)?;
    let code = receiver.code(now);
    *RECEIVER.lock().map_err(|_| "내부 잠금 오류")? = Some(receiver);
    Ok(code)
}

/// 지금 기다리는 받는 코드. 10분이 지났으면 열쇠를 지우고 없다고 답한다.
pub fn receive_state() -> Option<ReceiveCode> {
    let now = accounts::now();
    let mut slot = RECEIVER.lock().ok()?;
    if slot.as_ref().is_some_and(|receiver| !receiver.alive(now)) {
        *slot = None;
    }
    slot.as_ref().map(|receiver| receiver.code(now))
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

fn parse(receiver: &Receiver, password: &str, sealed: &[u8]) -> Result<Bundle, String> {
    let mut plain = open(receiver, password, sealed)?;
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

/// `slot`은 이 PC가 기다리는 받기다. 풀리면 그 열쇠를 지운다 — 같은 글은 다시 열리지 않는다.
fn import_at(
    env: &Env,
    slot: &mut Option<Receiver>,
    password: &str,
    text: Option<&str>,
    path: Option<&str>,
    now: u64,
) -> Result<ImportResult, String> {
    if slot.as_ref().is_some_and(|receiver| !receiver.alive(now)) {
        *slot = None;
    }
    let sealed = read_source(env, text, path)?;
    let receiver = slot.as_ref().ok_or(NOT_READY)?;
    // 다 풀고 검사한 뒤에야 쓴다 — 다른 코드로 잠근 것·틀린 비밀번호·변조는 여기서 끝난다
    let bundle = parse(receiver, password, &sealed)?;
    // 틀린 비밀번호로는 열쇠를 지우지 않는다(다시 칠 수 있게). 풀렸으면 지운다.
    *slot = None;
    apply(env, bundle)
}

pub fn import(env: &Env, password: &str, text: Option<&str>, path: Option<&str>) -> Result<ImportResult, String> {
    let mut slot = RECEIVER.lock().map_err(|_| "내부 잠금 오류")?;
    import_at(env, &mut slot, password, text, path, accounts::now())
}

/// 흔히 받아 두는 곳에서 지금 가져올 수 있는 옮기기 파일을 찾는다 — 최근 것부터.
/// 만든 지 10분이 지난 것은 내놓지 않는다(받는 코드가 이미 죽었으니 골라도 못 쓴다).
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

    /// 받는 PC의 열쇠 — `age`초 전에 만든 것처럼
    fn receiver_aged(age: u64) -> Receiver {
        let mut receiver = Receiver::new(T0).unwrap();
        receiver.started = Instant::now().checked_sub(Duration::from_secs(age)).unwrap();
        receiver
    }
    fn receiver() -> Receiver {
        receiver_aged(0)
    }

    // 검사는 가벼운 열쇠 만들기 값으로 돈다 — 기본값(64MiB)은 한 번만 왕복해 본다
    fn seal_fast(password: &str, to: &Receiver, plain: &[u8]) -> Vec<u8> {
        seal_with(password, &to.public, plain, T0, 64, 1).unwrap()
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
    fn text_of(env: &Env, password: &str, to: &Receiver) -> String {
        armor(&seal_fast(password, to, &serde_json::to_vec(&bundle_of(env)).unwrap()))
    }

    #[test]
    fn the_default_lock_round_trips_and_a_wrong_password_opens_nothing() {
        let to = receiver();
        let sealed = seal("correct horse", &to.public, b"secret payload", T0).unwrap();
        assert_eq!(open(&to, "correct horse", &sealed).unwrap(), b"secret payload");
        assert_eq!(open(&to, "correct horsf", &sealed).unwrap_err(), BAD_PASSWORD);
    }

    /// #192: 받는 코드를 만든 PC만 연다. 글과 비밀번호를 둘 다 가진 다른 PC는 열지 못한다.
    #[test]
    fn only_the_pc_that_made_the_receive_code_can_open_it() {
        let (to, other) = (receiver(), receiver());
        assert_ne!(to.public, other.public);
        let sealed = seal_fast("password-1", &to, b"payload");
        assert_eq!(open(&to, "password-1", &sealed).unwrap(), b"payload");
        assert_eq!(open(&other, "password-1", &sealed).unwrap_err(), NOT_FOR_THIS);
        // 머리말의 받는 코드를 제 것으로 바꿔 써도 열쇠가 달라 풀리지 않는다
        let mut retargeted = sealed.clone();
        retargeted[RECIPIENT_AT..SENDER_AT].copy_from_slice(&other.public);
        assert_eq!(open(&other, "password-1", &retargeted).unwrap_err(), BAD_PASSWORD);
        // 받는 코드를 가로채 제 것으로 바꿔치기한 사람에게 간 글: 그 사람도 비밀번호 없이는 못 연다
        let hijacked = seal_fast("password-1", &other, b"payload");
        assert_eq!(open(&other, "guess-123", &hijacked).unwrap_err(), BAD_PASSWORD);
        // 비밀번호에서 만든 값만으로는(받는 PC의 열쇠 없이) 잠금 열쇠가 나오지 않는다
        let salt = &sealed[SALT_AT..SALT_AT + SALT_LEN];
        let password_key = derive_key(b"password-1", salt, 64, 1, KDF_LANES).unwrap();
        let cipher = XChaCha20Poly1305::new((&password_key).into());
        let nonce = &sealed[SALT_AT + SALT_LEN..RECIPIENT_AT];
        assert!(cipher
            .decrypt(XNonce::from_slice(nonce), Payload { msg: &sealed[HEADER_LEN..], aad: &sealed[..HEADER_LEN] })
            .is_err());
        // 보내는 쪽 열쇠는 글마다 새로 뽑는다
        assert_ne!(sealed[SENDER_AT..HEADER_LEN], seal_fast("password-1", &to, b"payload")[SENDER_AT..HEADER_LEN]);
    }

    #[test]
    fn the_receive_code_survives_copying_and_refuses_what_is_not_one() {
        let to = receiver();
        let code = to.code(T0);
        assert_eq!((code.seconds_left, code.expires_at), (VALID_SECS, T0 + VALID_SECS));
        assert!(code.code.starts_with("SWR1.") && code.code.len() == CODE_MARK.len() + CODE_CHARS && code.code.is_ascii());
        assert_eq!(parse_code(&code.code).unwrap(), to.public);
        // 열쇠 자체는 코드에 없다
        assert!(!code.code.contains(&text_codec().encode(to.secret.to_bytes())));
        // 앞뒤에 딸려 온 글·줄바꿈·보이지 않는 글자를 걷어낸다
        let (head, tail) = code.code.split_at(20);
        assert_eq!(parse_code(&format!("받는 코드: {head}\n\u{200B}{tail} 입니다")).unwrap(), to.public);
        // 한 글자가 바뀌었거나 잘렸으면 받지 않는다 — 엉뚱한 열쇠로 잠그지 않는다
        let at = CODE_MARK.len() + 7;
        let wrong = if &code.code[at..at + 1] == "A" { "B" } else { "A" };
        let typo = format!("{}{wrong}{}", &code.code[..at], &code.code[at + 1..]);
        assert_eq!(parse_code(&typo).unwrap_err(), BAD_CODE);
        assert_eq!(parse_code(&code.code[..code.code.len() - 4]).unwrap_err(), BAD_CODE);
        assert_eq!(parse_code("").unwrap_err(), BAD_CODE);
        assert_eq!(parse_code("SWX3.12.AAAA").unwrap_err(), BAD_CODE);
        // 내보내기는 코드가 틀리면 비밀번호를 보기도 전에 끝난다
        let from = source("transfer-code");
        assert_eq!(export(&from, "password-1", "아무 글").unwrap_err(), BAD_CODE);
        assert!(export(&from, "short", &code.code).unwrap_err().contains("8자"));
    }

    #[test]
    fn the_locked_bytes_show_nothing_and_any_changed_byte_refuses_to_open() {
        let to = receiver();
        let plain = br#"{"key":"sk-fake-transfer-key-000001","more":"sk-fake-transfer-key-000001 sk-fake-transfer-key-000001"}"#;
        let sealed = seal_fast("password-1", &to, plain);
        // 같은 내용을 두 번 잠가도 같은 것이 나오지 않는다 (소금·논스·보내는 쪽 열쇠가 매번 새로 뽑힌다)
        assert_ne!(sealed, seal_fast("password-1", &to, plain));
        assert!(!sealed.windows(7).any(|window| window == b"sk-fake"));
        // 잠그기 전에 줄인다 — 복사할 글이 짧아진다
        assert!(sealed.len() < plain.len() + HEADER_LEN);
        for at in [8, 12, 16, CREATED_AT, SALT_AT, SALT_AT + SALT_LEN, RECIPIENT_AT, SENDER_AT, HEADER_LEN - 1, HEADER_LEN, sealed.len() - 1] {
            let mut changed = sealed.clone();
            changed[at] ^= 1;
            assert!(open(&to, "password-1", &changed).is_err(), "byte {at} must be authenticated");
        }
        // 잘린 것, 다른 것, 터무니없는 열쇠 만들기 값
        assert_eq!(open(&to, "password-1", &sealed[..HEADER_LEN]).unwrap_err(), BAD_TEXT);
        assert_eq!(open(&to, "password-1", &[7u8; 200]).unwrap_err(), BAD_TEXT);
        let mut greedy = sealed.clone();
        greedy[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(open(&to, "password-1", &greedy).unwrap_err(), BAD_TEXT);
    }

    /// 사용자 지시: 10분이 지나면 쓸 수 없다. #192: 시계가 아니라 열쇠가 없어져서 못 쓴다.
    #[test]
    fn after_ten_minutes_the_key_is_gone_and_turning_the_clock_back_does_not_bring_it_back() {
        let from = source("transfer-expiry");
        let to = test_env("transfer-expiry-to");
        // 받을 준비를 하지 않은 PC는 아무것도 열지 못한다
        let stranger = receiver();
        let text = text_of(&from, "password-1", &stranger);
        let mut none = None;
        assert_eq!(import_at(&to, &mut none, "password-1", Some(&text), None, T0).unwrap_err(), NOT_READY);
        // 벽시계로 10분이 지났다
        let mut slot = Some(receiver());
        let text = text_of(&from, "password-1", slot.as_ref().unwrap());
        assert!(slot.as_ref().unwrap().alive(T0 + VALID_SECS));
        assert_eq!(import_at(&to, &mut slot, "password-1", Some(&text), None, T0 + VALID_SECS + 1).unwrap_err(), NOT_READY);
        assert!(slot.is_none(), "the key must be dropped once it has expired");
        // 열쇠가 지워졌으니 시계를 되돌려도 열리지 않는다
        assert_eq!(import_at(&to, &mut slot, "password-1", Some(&text), None, T0).unwrap_err(), NOT_READY);
        // 시계를 되돌려 둔 PC: 벽시계는 이르지만 흐른 시간이 10분을 넘겼다
        let mut slot = Some(receiver_aged(VALID_SECS + 1));
        let text = text_of(&from, "password-1", slot.as_ref().unwrap());
        assert_eq!(slot.as_ref().unwrap().seconds_left(T0 - 3600), 0);
        assert_eq!(import_at(&to, &mut slot, "password-1", Some(&text), None, T0 - 3600).unwrap_err(), NOT_READY);
        assert!(slot.is_none() && accounts::profile_dirs(&to, Provider::Claude).unwrap().is_empty());
        // 남은 시간은 둘 중 먼저 끝나는 쪽이다
        assert_eq!(receiver_aged(60).seconds_left(T0 + 200), VALID_SECS - 200);
        assert!(receiver_aged(200).seconds_left(T0 + 60) <= VALID_SECS - 200);
    }

    /// #192: 한 번 가져온 글은 다시 열리지 않는다. 틀린 비밀번호는 열쇠를 지우지 않는다.
    #[test]
    fn a_text_opens_once_and_a_wrong_password_can_be_retried() {
        let from = source("transfer-once");
        let to = test_env("transfer-once-to");
        let mut slot = Some(receiver());
        let text = text_of(&from, "password-1", slot.as_ref().unwrap());
        assert_eq!(import_at(&to, &mut slot, "password-2", Some(&text), None, T0).unwrap_err(), BAD_PASSWORD);
        assert!(slot.is_some() && accounts::profile_dirs(&to, Provider::Claude).unwrap().is_empty());
        // 다른 받는 코드로 잠근 글도 열쇠를 지우지 않는다
        let elsewhere = text_of(&from, "password-1", &receiver());
        assert_eq!(import_at(&to, &mut slot, "password-1", Some(&elsewhere), None, T0).unwrap_err(), NOT_FOR_THIS);
        assert!(slot.is_some());
        assert_eq!(import_at(&to, &mut slot, "password-1", Some(&text), None, T0 + 60).unwrap().accounts_added, 2);
        assert!(slot.is_none());
        assert_eq!(import_at(&to, &mut slot, "password-1", Some(&text), None, T0 + 61).unwrap_err(), NOT_READY);
    }

    #[test]
    fn the_copied_text_survives_what_messengers_do_to_it() {
        let to = receiver();
        // 줄지 않는 내용이어야 글이 길다 (같은 글자만 있으면 몇십 자로 줄어든다)
        let noise: Vec<u8> = (0..900u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        let sealed = seal_fast("password-1", &to, &noise);
        let text = armor(&sealed);
        assert!(text.starts_with("SWX3.") && text.is_ascii() && !text.contains(['_', '-', '*', '~', '=', ' ', '\n']));
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
        assert!(dearmor("SWX3.1200").unwrap_err().contains("1200자 중 0자만"));
        // 글자가 바뀐 글, 우리 글이 아닌 것, 이전 형식
        let swapped = text.replacen('A', "?", 1).replacen('B', "?", 1).replacen('C', "?", 1);
        assert!(dearmor(&swapped).unwrap_err().contains("바뀌었습니다"));
        assert_eq!(dearmor("안녕하세요 그냥 메모입니다").unwrap_err(), BAD_TEXT);
        assert_eq!(dearmor("").unwrap_err(), BAD_TEXT);
        assert_eq!(dearmor("SWX3.abc.def").unwrap_err(), BAD_TEXT);
        assert_eq!(dearmor("SWXFER1\n\u{1}\u{2}binary").unwrap_err(), OLD_FORMAT);
        // 비밀번호만으로 잠그던 형식(v2.2.0~2.2.3)은 받지 않는다 — 무엇을 하면 되는지 말한다
        assert_eq!(dearmor("SWX2.1200.AAAABBBBCCCC").unwrap_err(), OLD_FORMAT);
        // 받는 코드를 글 자리에 넣었다
        assert_eq!(dearmor(&to.code(T0).code).unwrap_err(), BAD_TEXT);
        assert_eq!(peek_created_at(&text[..240]), Some(T0));
        assert_eq!(peek_created_at("SWX3.12.AAAA"), None);
    }

    #[test]
    fn logins_arrive_on_an_empty_pc_and_nothing_else_does() {
        let from = source("transfer-from");
        // 묶음에는 로그인만 있다 — 설정·메모는 들어가지 않는다
        let plain = String::from_utf8(serde_json::to_vec(&bundle_of(&from)).unwrap()).unwrap();
        assert!(!plain.contains("memo-from-a") && !plain.contains("violet") && !plain.contains("old-generation"));
        let mut slot = Some(receiver());
        let text = text_of(&from, "password-1", slot.as_ref().unwrap());
        for secret in ["fake-access", "fake-refresh", "acct-alpha"] {
            assert!(!text.contains(secret));
        }
        let to = test_env("transfer-to");
        // 틀린 비밀번호는 아무것도 쓰지 않는다
        assert_eq!(import_at(&to, &mut slot, "password-2", Some(&text), None, T0).unwrap_err(), BAD_PASSWORD);
        assert!(accounts::profile_dirs(&to, Provider::Claude).unwrap().is_empty() && !to.store.join("api-connections").exists());
        let result = import_at(&to, &mut slot, "password-1", Some(&format!("  {text}\n")), None, T0 + 60).unwrap();
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
        let receiver = receiver();
        let old = seal_fast("password-1", &receiver, br#"{"version":2,"created_at":0,"source_os":"macos","profiles":[],"api_keys":[]}"#);
        assert_eq!(parse(&receiver, "password-1", &old).err().unwrap(), OLD_FORMAT);
    }

    /// 사용자 지시: 복사와 파일, 둘 다 된다. 파일의 내용은 복사하는 글과 같다.
    #[test]
    fn the_same_text_also_travels_as_a_file() {
        let from = source("transfer-file");
        fs::create_dir_all(from.home.join("Downloads")).unwrap();
        assert!(check_password("short").is_err());
        let mut slot = Some(receiver());
        let recipient = slot.as_ref().unwrap().public;
        let made = make_text("password-1", &recipient, &bundle_of(&from), T0).unwrap();
        assert_eq!(made.accounts, 2);
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
        let result = import_at(&to, &mut slot, "password-1", None, Some(&moved), T0 + 120).unwrap();
        assert_eq!(result.accounts_added, 2);
        // 한 번 가져온 파일은 남아 있어도 다시 열리지 않는다
        assert_eq!(import_at(&to, &mut slot, "password-1", None, Some(&moved), T0 + 130).unwrap_err(), NOT_READY);
        let mut slot = Some(receiver());
        // 우리가 다루는 자리 밖의 경로는 읽지 않는다
        let outside = to.store.join("elsewhere.swx");
        fs::write(&outside, &made.text).unwrap();
        assert_eq!(
            import_at(&to, &mut slot, "password-1", None, Some(&outside.to_string_lossy()), T0).unwrap_err(),
            "파일을 찾을 수 없습니다"
        );
        assert!(import_at(&to, &mut slot, "password-1", None, None, T0).unwrap_err().contains("붙여넣거나"));
    }
}

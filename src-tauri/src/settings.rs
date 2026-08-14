//! 위젯 자체 설정 (`~/.switcher/settings.json`) — 현재는 UI 언어(lang)만 담는다.
//! 토큰과 무관한 파일이라 프로필 보관소와 분리해 보관소 루트에 둔다.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// 지원 언어 (코드, 트레이 메뉴에 보일 이름). 배열 순서가 곧 메뉴 순서다.
/// 언어 이름은 번역하지 않는다 — 어떤 언어로 잘못 바뀌어도 자기 언어를 찾을 수 있게.
/// 주의: 프론트 src/i18n.ts의 SUPPORTED_LANGS와 짝이다 — 한쪽만 추가하면
/// 트레이 체크는 옮겨가는데 UI는 안 바뀌는 반쪽 상태가 된다.
pub const LANGS: [(&str, &str); 6] = [
    ("ko", "한국어"),
    ("en", "English"),
    ("ja", "日本語"),
    ("zh-CN", "简体中文"),
    ("zh-TW", "繁體中文"),
    ("hi", "हिन्दी"),
];

pub fn is_supported(lang: &str) -> bool {
    LANGS.iter().any(|(code, _)| *code == lang)
}

/// settings.json 불리언 키 (트레이 설정 토글)
pub const KEY_AUTO_UPDATE: &str = "auto_update";
pub const KEY_AUTO_START: &str = "auto_start";
/// 바탕화면 바로가기는 Windows 전용 기능 — 맥 빌드의 dead_code 경고 방지
#[cfg(windows)]
pub const KEY_SHORTCUT_DONE: &str = "desktop_shortcut_done";
/// 표시 기능 — 끄면 위젯에서 해당 섹션·기능이 사라진다 (기본 전부 켜짐)
pub const KEY_SHOW_CLAUDE: &str = "show_claude";
pub const KEY_SHOW_CODEX: &str = "show_codex";
pub const KEY_SHOW_GITHUB: &str = "show_github";
pub const KEY_SHOW_BLACK: &str = "show_black";
pub const KEY_SHOW_DISPLAY: &str = "show_display";
/// TFSD (Token Full Self-Driving) — 사용량 기반 자동 계정 전환 (기본 꺼짐, 옵트인)
pub const KEY_TFSD: &str = "tfsd";
/// 첫 실행 GitHub Star 안내 선택. 버전과 무관하게 한 번만 묻는다.
const KEY_GITHUB_STAR_PROMPT_CHOICE: &str = "github_star_prompt_choice";
pub const STAR_PROMPT_CHOICE_STAR: &str = "star";
pub const STAR_PROMPT_CHOICE_DISMISSED: &str = "dismissed";

/// 불리언 설정 읽기 — 파일이 없거나 키가 없거나 타입이 다르면 default
pub fn load_flag(store: &Path, key: &str, default: bool) -> bool {
    read_settings(store)
        .and_then(|v| v.get(key).and_then(Value::as_bool))
        .unwrap_or(default)
}

/// 불리언 설정 저장 — 다른 키는 보존한다
pub fn save_flag(store: &Path, key: &str, value: bool) -> Result<(), String> {
    save_value(store, key, Value::Bool(value))
}

/// 선택값이 없거나 알 수 없는 값이면 다시 묻는다.
pub fn load_github_star_prompt_choice(store: &Path) -> Result<Option<String>, String> {
    let Some(value) = read_settings_checked(store)? else {
        return Ok(None);
    };
    let root = value
        .as_object()
        .ok_or_else(|| "설정 파일 형식이 올바르지 않습니다".to_string())?;
    Ok(root
        .get(KEY_GITHUB_STAR_PROMPT_CHOICE)
        .and_then(Value::as_str)
        .map(String::from)
        .filter(|choice| {
            matches!(
                choice.as_str(),
                STAR_PROMPT_CHOICE_STAR | STAR_PROMPT_CHOICE_DISMISSED
            )
        }))
}

pub fn save_github_star_prompt_choice(store: &Path, choice: &str) -> Result<(), String> {
    if !matches!(choice, STAR_PROMPT_CHOICE_STAR | STAR_PROMPT_CHOICE_DISMISSED) {
        return Err("잘못된 GitHub Star 안내 선택입니다".to_string());
    }
    save_value_checked(
        store,
        KEY_GITHUB_STAR_PROMPT_CHOICE,
        Value::String(choice.to_string()),
    )
}

/// 첫 실행 안내는 선택형 기능이므로 읽을 수 없는 기존 설정을 빈 파일로 덮어쓰지 않는다.
fn save_value_checked(store: &Path, key: &str, value: Value) -> Result<(), String> {
    fs::create_dir_all(store).map_err(|e| format!("설정 폴더 생성 실패: {e}"))?;
    let mut root = match read_settings_checked(store)? {
        Some(v @ Value::Object(_)) => v,
        Some(_) => return Err("설정 파일 형식이 올바르지 않습니다".to_string()),
        None => Value::Object(Default::default()),
    };
    root[key] = value;
    write_settings(store, &root)
}

/// 키 하나를 갱신해 저장. 다른 키는 보존하고, 임시 파일 + rename으로 원자적으로 쓴다 —
/// 쓰다 만 파일이 남으면 다음 시작에서 모든 설정이 기본값으로 뒤집힌다 (자동 실행 재등록 등).
fn save_value(store: &Path, key: &str, value: Value) -> Result<(), String> {
    fs::create_dir_all(store).map_err(|e| format!("설정 폴더 생성 실패: {e}"))?;
    let mut root = match read_settings(store) {
        Some(v @ Value::Object(_)) => v,
        _ => Value::Object(Default::default()),
    };
    root[key] = value;
    write_settings(store, &root)
}

fn write_settings(store: &Path, root: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(root).map_err(|e| format!("설정 직렬화 실패: {e}"))?;
    let path = settings_path(store);
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|e| format!("설정 저장 실패: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("설정 저장 실패: {e}"))
}

/// 트레이 라벨 — [열기, 숨기기, 설정, 언어, 자동 업데이트, 부팅 시 자동 실행,
/// 블랙 모니터, 표시 기능, 디스플레이 밝기, TFSD, 종료] 순서
pub fn tray_labels(lang: &str) -> [&'static str; 12] {
    match lang {
        "en" => [
            "Open",
            "Hide",
            "Settings",
            "Language",
            "Auto-update",
            "Run at startup",
            "Black monitor",
            "Visible features",
            "Display brightness",
            "TFSD auto-switch",
            "Check for updates",
            "Quit",
        ],
        "ja" => [
            "開く",
            "隠す",
            "設定",
            "言語",
            "自動アップデート",
            "起動時に自動実行",
            "ブラックモニター",
            "表示する機能",
            "ディスプレイの明るさ",
            "TFSD 自動切り替え",
            "アップデートを確認",
            "終了",
        ],
        "zh-CN" => [
            "打开", "隐藏", "设置", "语言", "自动更新", "开机自启动", "黑屏模式", "显示的功能",
            "显示器亮度", "TFSD 自动切换", "检查更新", "退出",
        ],
        "zh-TW" => [
            "開啟", "隱藏", "設定", "語言", "自動更新", "開機自動啟動", "黑屏模式", "顯示的功能",
            "螢幕亮度", "TFSD 自動切換", "檢查更新", "結束",
        ],
        "hi" => [
            "खोलें",
            "छिपाएँ",
            "सेटिंग्स",
            "भाषा",
            "ऑटो-अपडेट",
            "बूट पर स्वतः चलाएँ",
            "ब्लैक मॉनिटर",
            "दिखाए जाने वाले फ़ीचर",
            "डिस्प्ले चमक",
            "TFSD ऑटो-स्विच",
            "अपडेट जाँचें",
            "बंद करें",
        ],
        _ => [
            "열기",
            "숨기기",
            "설정",
            "언어",
            "자동 업데이트",
            "부팅 시 자동 실행",
            "블랙 모니터",
            "표시 기능",
            "디스플레이 밝기",
            "TFSD 자동 전환",
            "업데이트 확인",
            "종료",
        ],
    }
}

fn settings_path(store: &Path) -> PathBuf {
    store.join("settings.json")
}

fn read_settings(store: &Path) -> Option<Value> {
    let text = fs::read_to_string(settings_path(store)).ok()?;
    serde_json::from_str(&text).ok()
}

fn read_settings_checked(store: &Path) -> Result<Option<Value>, String> {
    let path = settings_path(store);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("설정 읽기 실패: {error}")),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| format!("설정 파일 분석 실패: {error}"))
}

/// 저장된 UI 언어. 파일이 없거나 값이 지원 목록 밖이면 "ko".
pub fn load_language(store: &Path) -> String {
    match read_settings(store)
        .and_then(|v| v.get("lang").and_then(Value::as_str).map(String::from))
    {
        Some(lang) if is_supported(&lang) => lang,
        _ => "ko".to_string(),
    }
}

/// UI 언어 저장 — settings.json에 이미 있는 다른 키는 보존한다.
pub fn save_language(store: &Path, lang: &str) -> Result<(), String> {
    if !is_supported(lang) {
        return Err(format!("지원하지 않는 언어: {lang}"));
    }
    save_value(store, "lang", Value::String(lang.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "switcher-settings-test-{}-{tag}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        base
    }

    #[test]
    fn default_is_korean() {
        let store = test_store("default");
        assert_eq!(load_language(&store), "ko");
    }

    #[test]
    fn save_then_load_roundtrip() {
        let store = test_store("roundtrip");
        save_language(&store, "en").unwrap();
        assert_eq!(load_language(&store), "en");
        save_language(&store, "zh-TW").unwrap();
        assert_eq!(load_language(&store), "zh-TW");
    }

    #[test]
    fn rejects_unsupported_language() {
        let store = test_store("unsupported");
        assert!(save_language(&store, "fr").is_err());
        assert_eq!(load_language(&store), "ko");
    }

    #[test]
    fn corrupt_or_alien_value_falls_back_to_korean() {
        let store = test_store("corrupt");
        fs::create_dir_all(&store).unwrap();
        fs::write(settings_path(&store), "{not json").unwrap();
        assert_eq!(load_language(&store), "ko");
        fs::write(settings_path(&store), r#"{"lang":"xx"}"#).unwrap();
        assert_eq!(load_language(&store), "ko");
    }

    #[test]
    fn flag_defaults_and_roundtrip() {
        let store = test_store("flags");
        assert!(load_flag(&store, KEY_AUTO_UPDATE, true));
        assert!(!load_flag(&store, KEY_AUTO_START, false));
        save_flag(&store, KEY_AUTO_UPDATE, false).unwrap();
        assert!(!load_flag(&store, KEY_AUTO_UPDATE, true));
        // 플래그 저장이 언어 키를 보존하고, 언어 저장이 플래그를 보존한다
        save_language(&store, "en").unwrap();
        assert!(!load_flag(&store, KEY_AUTO_UPDATE, true));
        save_flag(&store, KEY_AUTO_START, true).unwrap();
        assert_eq!(load_language(&store), "en");
        assert!(load_flag(&store, KEY_AUTO_START, false));
    }

    #[test]
    fn preserves_other_keys() {
        let store = test_store("preserve");
        fs::create_dir_all(&store).unwrap();
        fs::write(settings_path(&store), r#"{"future_key":42}"#).unwrap();
        save_language(&store, "ja").unwrap();
        let saved: Value =
            serde_json::from_str(&fs::read_to_string(settings_path(&store)).unwrap()).unwrap();
        assert_eq!(saved["future_key"], 42);
        assert_eq!(saved["lang"], "ja");
    }

    #[test]
    fn github_star_prompt_choice_is_stable_and_preserves_settings() {
        let store = test_store("github-star-prompt");
        assert_eq!(load_github_star_prompt_choice(&store), Ok(None));

        save_language(&store, "en").unwrap();
        save_github_star_prompt_choice(&store, STAR_PROMPT_CHOICE_STAR).unwrap();
        assert_eq!(
            load_github_star_prompt_choice(&store)
                .unwrap()
                .as_deref(),
            Some(STAR_PROMPT_CHOICE_STAR)
        );
        assert_eq!(load_language(&store), "en");

        save_github_star_prompt_choice(&store, STAR_PROMPT_CHOICE_DISMISSED).unwrap();
        assert_eq!(
            load_github_star_prompt_choice(&store)
                .unwrap()
                .as_deref(),
            Some(STAR_PROMPT_CHOICE_DISMISSED)
        );
    }

    #[test]
    fn github_star_prompt_rejects_unknown_choices() {
        let store = test_store("github-star-prompt-invalid");
        assert!(save_github_star_prompt_choice(&store, "later").is_err());
        assert_eq!(load_github_star_prompt_choice(&store), Ok(None));

        fs::create_dir_all(&store).unwrap();
        fs::write(
            settings_path(&store),
            r#"{"github_star_prompt_choice":"unexpected"}"#,
        )
        .unwrap();
        assert_eq!(load_github_star_prompt_choice(&store), Ok(None));
    }

    #[test]
    fn github_star_prompt_never_overwrites_unreadable_settings() {
        let store = test_store("github-star-prompt-corrupt");
        fs::create_dir_all(&store).unwrap();
        let path = settings_path(&store);
        fs::write(&path, r#"{"lang":"en""#).unwrap();

        assert!(load_github_star_prompt_choice(&store).is_err());
        assert!(save_github_star_prompt_choice(&store, STAR_PROMPT_CHOICE_STAR).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), r#"{"lang":"en""#);

        fs::write(settings_path(&store), "[]").unwrap();
        assert!(load_github_star_prompt_choice(&store).is_err());
        assert!(save_github_star_prompt_choice(&store, STAR_PROMPT_CHOICE_STAR).is_err());
        assert_eq!(fs::read_to_string(settings_path(&store)).unwrap(), "[]");
    }
}

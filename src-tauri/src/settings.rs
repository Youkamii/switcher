//! 위젯 자체 설정 (`~/.switcher/settings.json`) — 현재는 UI 언어(lang)만 담는다.
//! 토큰과 무관한 파일이라 프로필 보관소와 분리해 보관소 루트에 둔다.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// 지원 언어 (코드, 트레이 메뉴에 보일 이름). 배열 순서가 곧 메뉴 순서다.
/// 언어 이름은 번역하지 않는다 — 어떤 언어로 잘못 바뀌어도 자기 언어를 찾을 수 있게.
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

/// 트레이 라벨 — [열기, 숨기기, 설정, 언어, 종료] 순서
pub fn tray_labels(lang: &str) -> [&'static str; 5] {
    match lang {
        "en" => ["Open", "Hide", "Settings", "Language", "Quit"],
        "ja" => ["開く", "隠す", "設定", "言語", "終了"],
        "zh-CN" => ["打开", "隐藏", "设置", "语言", "退出"],
        "zh-TW" => ["開啟", "隱藏", "設定", "語言", "結束"],
        "hi" => ["खोलें", "छिपाएँ", "सेटिंग्स", "भाषा", "बंद करें"],
        _ => ["열기", "숨기기", "설정", "언어", "종료"],
    }
}

fn settings_path(store: &Path) -> PathBuf {
    store.join("settings.json")
}

fn read_settings(store: &Path) -> Option<Value> {
    let text = fs::read_to_string(settings_path(store)).ok()?;
    serde_json::from_str(&text).ok()
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
    fs::create_dir_all(store).map_err(|e| format!("설정 폴더 생성 실패: {e}"))?;
    let mut value = match read_settings(store) {
        Some(v @ Value::Object(_)) => v,
        _ => Value::Object(Default::default()),
    };
    value["lang"] = Value::String(lang.to_string());
    let text = serde_json::to_string_pretty(&value).map_err(|e| format!("설정 직렬화 실패: {e}"))?;
    fs::write(settings_path(store), text).map_err(|e| format!("설정 저장 실패: {e}"))
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
}

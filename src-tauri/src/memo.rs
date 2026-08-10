//! 간단 메모장 저장 (`~/.switcher/memo.json`) — Type2 위젯의 부속 메모창 내용.
//! 토큰과 무관한 사용자 콘텐츠라 settings.json과 같은 층(보관소 루트)에 별도 파일로 둔다.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// 메모 탭 개수 — 프론트 memo.html의 탭 버튼 1~5와 짝이다
pub const TAB_COUNT: usize = 5;

/// 탭 하나의 본문 상한 (바이트) — 대용량 붙여넣기가 디바운스마다 전체 직렬화·
/// 재기록을 돌려 자가 DoS가 되는 것을 막는다 (red-review)
pub const TAB_MAX_BYTES: usize = 1_000_000;

/// 누락 필드는 `Default`에서 가져온다 (컨테이너 `serde(default)`) —
/// 기본값을 한 곳(Default impl)에만 명세하기 위함
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct MemoData {
    /// 탭 1~5의 본문 (load/save에서 항상 TAB_COUNT개로 정규화)
    pub tabs: Vec<String>,
    /// 마지막 활성 탭 (0-기준)
    pub active: usize,
    /// 메모창 자체 투명도 0~100 — 위젯 투명도와 독립
    pub alpha: u8,
}

impl Default for MemoData {
    fn default() -> Self {
        MemoData {
            tabs: vec![String::new(); TAB_COUNT],
            active: 0,
            alpha: 100,
        }
    }
}

impl MemoData {
    /// 손으로 고쳐졌거나 구버전 파일이어도 항상 유효한 형태로 맞춘다
    fn normalize(mut self) -> Self {
        self.tabs.resize(TAB_COUNT, String::new());
        for tab in &mut self.tabs {
            if tab.len() > TAB_MAX_BYTES {
                let mut end = TAB_MAX_BYTES;
                while !tab.is_char_boundary(end) {
                    end -= 1;
                }
                tab.truncate(end);
            }
        }
        if self.active >= TAB_COUNT {
            self.active = 0;
        }
        if self.alpha > 100 {
            self.alpha = 100;
        }
        self
    }
}

fn memo_path(store: &Path) -> PathBuf {
    store.join("memo.json")
}

/// 파일이 없거나 깨져 있으면 빈 탭 5개 기본값 — 메모창은 언제나 뜬다
pub fn load(store: &Path) -> MemoData {
    fs::read_to_string(memo_path(store))
        .ok()
        .and_then(|text| serde_json::from_str::<MemoData>(&text).ok())
        .unwrap_or_default()
        .normalize()
}

/// 임시 파일 + rename 원자적 쓰기 — 쓰다 만 파일이 남으면 메모 전체가 유실된다
/// (settings.rs와 같은 이유).
/// 실제로 저장된(정규화된) 데이터를 돌려준다 — 1MB 상한 절단이 일어났으면
/// 프론트가 그걸 화면에 반영해 "보이는 것 = 저장된 것"을 지킨다 (리뷰 #53:
/// 조용한 절단이 재시작 후에야 드러나던 문제)
pub fn save(store: &Path, data: MemoData) -> Result<MemoData, String> {
    fs::create_dir_all(store).map_err(|e| format!("메모 폴더 생성 실패: {e}"))?;
    let data = data.normalize();
    let text =
        serde_json::to_string_pretty(&data).map_err(|e| format!("메모 직렬화 실패: {e}"))?;
    let path = memo_path(store);
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|e| format!("메모 저장 실패: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("메모 저장 실패: {e}"))?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "switcher-memo-test-{}-{tag}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        base
    }

    #[test]
    fn missing_file_gives_empty_tabs() {
        let store = test_store("missing");
        let data = load(&store);
        assert_eq!(data.tabs.len(), TAB_COUNT);
        assert!(data.tabs.iter().all(String::is_empty));
        assert_eq!(data.active, 0);
        assert_eq!(data.alpha, 100);
    }

    #[test]
    fn save_then_load_roundtrip() {
        let store = test_store("roundtrip");
        let mut data = MemoData::default();
        data.tabs[0] = "첫 메모".to_string();
        data.tabs[4] = "다섯째 탭\n둘째 줄".to_string();
        data.active = 4;
        data.alpha = 40;
        save(&store, data.clone()).unwrap();
        assert_eq!(load(&store), data);
    }

    #[test]
    fn corrupt_file_falls_back_to_default() {
        let store = test_store("corrupt");
        fs::create_dir_all(&store).unwrap();
        fs::write(memo_path(&store), "{not json").unwrap();
        assert_eq!(load(&store), MemoData::default());
    }

    #[test]
    fn alien_values_are_normalized() {
        let store = test_store("alien");
        fs::create_dir_all(&store).unwrap();
        // 탭 7개·범위 밖 active — 손으로 고친 파일도 5개·0으로 정규화된다
        fs::write(
            memo_path(&store),
            r#"{"tabs":["a","b","c","d","e","f","g"],"active":9,"alpha":100}"#,
        )
        .unwrap();
        let data = load(&store);
        assert_eq!(data.tabs, vec!["a", "b", "c", "d", "e"]);
        assert_eq!(data.active, 0);
    }

    #[test]
    fn short_tabs_are_padded() {
        let store = test_store("short");
        fs::create_dir_all(&store).unwrap();
        fs::write(memo_path(&store), r#"{"tabs":["only"]}"#).unwrap();
        let data = load(&store);
        assert_eq!(data.tabs.len(), TAB_COUNT);
        assert_eq!(data.tabs[0], "only");
        assert!(data.tabs[1..].iter().all(String::is_empty));
        assert_eq!(data.alpha, 100);
    }

    #[test]
    fn oversized_tab_is_truncated_at_char_boundary() {
        let store = test_store("oversize");
        let mut data = MemoData::default();
        // 멀티바이트 문자로 채워 경계 절단이 문자 경계를 지키는지 확인
        data.tabs[0] = "가".repeat(TAB_MAX_BYTES); // 3바이트 × 상한 = 3배 초과
        save(&store, data).unwrap();
        let loaded = load(&store);
        assert!(loaded.tabs[0].len() <= TAB_MAX_BYTES);
        assert!(loaded.tabs[0].chars().all(|c| c == '가'));
    }

    #[test]
    fn save_normalizes_alpha_over_100() {
        let store = test_store("alpha");
        let data = MemoData {
            alpha: 250,
            ..MemoData::default()
        };
        save(&store, data).unwrap();
        assert_eq!(load(&store).alpha, 100);
    }
}

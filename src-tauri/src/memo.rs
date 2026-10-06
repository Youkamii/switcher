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

/// 저장 직렬화 잠금 (save 참고). 깨진 파일 보관(load)도 이 잠금 아래서 해
/// 저장이 막 써 넣은 정상 파일을 "깨진 파일"로 옮기는 엇갈림을 막는다.
static SAVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 파일이 없거나 깨져 있으면 빈 탭 5개 기본값 — 메모창은 언제나 뜬다.
/// 필드별로 너그럽게 읽는다 (#177): 예전엔 필드 하나만 범위를 벗어나도(alpha 300,
/// active -1) serde가 통째로 실패해 5탭 전체가 기본값이 됐고, 메모창의 다음 자동
/// 저장(블러 때마다 돈다)이 그 빈 탭을 파일에 영구화했다.
/// 탭을 알아볼 수 없을 만큼 깨진 파일은 덮어쓰이기 전에 `memo.json.corrupt-<밀리초>`로
/// 옮겨 보관한다 — 손으로라도 살릴 기회를 남긴다.
pub fn load(store: &Path) -> MemoData {
    let _guard = SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = memo_path(store);
    let Ok(bytes) = fs::read(&path) else {
        return MemoData::default();
    };
    match parse_lenient(&bytes) {
        Some(data) => data.normalize(),
        None => {
            quarantine(&path);
            MemoData::default()
        }
    }
}

/// 필드마다 따로 읽어 타입만 맞춘다 — 범위(탭 개수·활성 탭·투명도)는 normalize 한 곳이
/// 정한다 (load가 이어서 부른다). JSON이 아니거나(잘못된 UTF-8 포함), 최상위가 객체가
/// 아니거나, `tabs`가 배열이 아니면 None — 탭을 살릴 수 없으니 보관 대상이다.
fn parse_lenient(bytes: &[u8]) -> Option<MemoData> {
    use serde_json::Value;
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let obj = value.as_object()?;
    let mut data = MemoData::default();
    match obj.get("tabs") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            // 문자열이 아닌 칸도 버리지 않고 글자로 남긴다 (null만 빈 탭)
            data.tabs = items
                .iter()
                .map(|item| match item {
                    Value::String(text) => text.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                })
                .collect();
        }
        Some(_) => return None,
    }
    // 숫자가 아니면 기본값을 둔다. f64 → 정수 `as`는 필드 타입 끝값으로 포화하므로(음수는
    // 0, 너무 큰 수는 최댓값) 어떤 숫자든 담긴다 — 활성 탭 범위 밖·투명도 100 초과는
    // normalize가 첫 탭·100으로 맞춘다.
    if let Some(n) = obj.get("active").and_then(Value::as_f64) {
        data.active = n as usize;
    }
    if let Some(n) = obj.get("alpha").and_then(Value::as_f64) {
        data.alpha = n.round() as u8;
    }
    Some(data)
}

/// 깨진 메모 파일을 공용 accounts::quarantine_corrupt로 `memo.json.corrupt-<밀리초>`에
/// 옮긴다 (이전 격리본과 이름이 겹치면 `-n` 접미사). 옮겨 두면 다음 load가 같은 파일을 또
/// 보관하지 않는다. 옮기기가 실패하면(다른 프로그램이 잡고 있는 등) 복사라도 남긴다 — 다음
/// 저장이 원본을 덮기 전에. 복사본 이름은 예전 규칙(초 단위)이라 밀리초 격리본과 겹치지 않고,
/// 원본이 그대로 남은 경로라 같은 초에 다시 복사돼도 같은 내용이다.
fn quarantine(path: &Path) {
    if crate::accounts::quarantine_corrupt(path).is_err() {
        let backup = path.with_extension(format!("json.corrupt-{}", crate::accounts::now()));
        let _ = fs::copy(path, &backup);
    }
}

/// 임시 파일 + rename 원자적 쓰기 — 쓰다 만 파일이 남으면 메모 전체가 유실된다
/// (settings.rs와 같은 이유).
/// 실제로 저장된(정규화된) 데이터를 돌려준다 — 1MB 상한 절단이 일어났으면
/// 프론트가 그걸 화면에 반영해 "보이는 것 = 저장된 것"을 지킨다 (리뷰 #53:
/// 조용한 절단이 재시작 후에야 드러나던 문제)
pub fn save(store: &Path, data: MemoData) -> Result<MemoData, String> {
    // async 커맨드는 tokio 풀에서 병렬이라 flush가 겹치면(디바운스+블러 등) 같은
    // tmp 파일에 두 태스크가 쓰다 rename이 꼬일 수 있다 — 저장을 직렬화한다
    // (red-review). 나중 스냅샷이 먼저 완료되는 순서 역전까지 막지는 못하지만
    // (발생 창 µs, 다음 플러시가 치유) 반쪽 상태·무음 rename 실패는 사라진다.
    let _guard = SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

    fn corrupt_backups(store: &Path) -> Vec<PathBuf> {
        crate::accounts::corrupt_copies(&memo_path(store))
    }

    /// 필드 하나가 범위를 벗어나도 탭은 살고 그 필드만 맞춰진다 (#177)
    #[test]
    fn out_of_range_field_keeps_tabs() {
        let store = test_store("lenient");
        fs::create_dir_all(&store).unwrap();
        fs::write(
            memo_path(&store),
            r#"{"tabs":["a","b","c","d","e"],"active":-1,"alpha":300}"#,
        )
        .unwrap();
        let data = load(&store);
        assert_eq!(data.tabs, vec!["a", "b", "c", "d", "e"]);
        assert_eq!(data.active, 0);
        assert_eq!(data.alpha, 100);

        // 범위 안 값은 그대로, 음수 투명도는 0, 문자열이 아닌 칸도 글자로 남는다
        fs::write(
            memo_path(&store),
            r#"{"tabs":["a",null,3],"active":2,"alpha":-5}"#,
        )
        .unwrap();
        let data = load(&store);
        assert_eq!(data.tabs, vec!["a", "", "3", "", ""]);
        assert_eq!(data.active, 2);
        assert_eq!(data.alpha, 0);

        // 거대한 수·숫자 아닌 값도 그 필드만 기본값
        fs::write(
            memo_path(&store),
            r#"{"tabs":["x"],"active":1e30,"alpha":"half"}"#,
        )
        .unwrap();
        let data = load(&store);
        assert_eq!(data.tabs[0], "x");
        assert_eq!(data.active, 0);
        assert_eq!(data.alpha, 100);
        // 너그럽게 읽힌 파일은 보관 대상이 아니다
        assert!(corrupt_backups(&store).is_empty());
    }

    /// 범위는 normalize 한 곳이 맞춘다 — parse_lenient는 타입만 바꿔 담는다
    #[test]
    fn parse_lenient_converts_types_and_normalize_owns_ranges() {
        let raw = parse_lenient(br#"{"tabs":["a"],"active":9,"alpha":300}"#).unwrap();
        assert_eq!(raw.tabs, vec!["a"]);
        assert_eq!(raw.active, 9);
        assert_eq!(raw.alpha, u8::MAX);
        let data = raw.normalize();
        assert_eq!(data.tabs.len(), TAB_COUNT);
        assert_eq!(data.active, 0);
        assert_eq!(data.alpha, 100);
    }

    /// JSON으로 읽히지 않는 파일은 덮어쓰이기 전에 원본 그대로 보관된다 (#177)
    #[test]
    fn unparsable_file_is_kept_before_overwrite() {
        let store = test_store("quarantine");
        fs::create_dir_all(&store).unwrap();
        // 한글 탭 본문이 든 채 중간에 잘린 JSON
        let original: &[u8] = b"{\"tabs\":[\"\xEC\xA4\x91\xEC\x9A\x94\", oops";
        fs::write(memo_path(&store), original).unwrap();
        assert_eq!(load(&store), MemoData::default());
        let backups = corrupt_backups(&store);
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(&backups[0]).unwrap(), original);

        // 다음 저장이 새 파일을 써도 보관본은 그대로 남고, 다시 읽어도 또 보관하지 않는다
        let mut data = MemoData::default();
        data.tabs[0] = "새 메모".to_string();
        save(&store, data.clone()).unwrap();
        assert_eq!(load(&store), data);
        assert_eq!(corrupt_backups(&store), backups);
        assert_eq!(fs::read(&backups[0]).unwrap(), original);
    }

    /// 탭 자리가 배열이 아니거나, 최상위가 객체가 아니거나, UTF-8이 깨졌으면
    /// 탭을 살릴 수 없으므로 역시 보관한다
    #[test]
    fn unrecoverable_shapes_are_kept() {
        let cases: [(&str, &[u8]); 3] = [
            ("tabs-type", br#"{"tabs":"oops","alpha":50}"#),
            ("top-array", br#"["a","b"]"#),
            ("bad-utf8", b"{\"tabs\":[\"\xFF\xFE\"]}"),
        ];
        for (tag, bytes) in cases {
            let store = test_store(tag);
            fs::create_dir_all(&store).unwrap();
            fs::write(memo_path(&store), bytes).unwrap();
            assert_eq!(load(&store), MemoData::default(), "{tag}");
            let backups = corrupt_backups(&store);
            assert_eq!(backups.len(), 1, "{tag}");
            assert_eq!(fs::read(&backups[0]).unwrap(), bytes, "{tag}");
        }
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

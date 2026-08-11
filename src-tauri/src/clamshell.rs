//! 클램셸 슬립 방지 (macOS 전용) — 덮개를 닫아도 잠들지 않게 해 내부 터미널의
//! AI 작업이 계속 돌게 한다 (사용자 요청 2026-08-11).
//!
//! 뼈대: `pmset disablesleep 1/0` (root 필요 — 덮개 닫힘 슬립은 caffeinate류
//! 어서션으로는 못 막는다). 관리자 암호 프롬프트는 **켤 때 한 번만** — 그 승인으로
//! root 감시 스크립트를 함께 띄워, 이후의 복원(끄기 클릭·일회성 덮개 열림·위젯
//! 종료·크래시)은 전부 그 감시자가 암호 없이 수행한다.
//!
//! 상태 순환: off(0) → 일회성(1) → 지속(2) → off. 일회성은 덮개가 닫혔다 열리는
//! 사이클 하나가 끝나면 자동 복원, 지속은 끄거나 위젯이 종료될 때까지 유지된다.
//! 켜기 전 SleepDisabled 값을 저장해 복원 시 그대로 되돌린다 (블랙 모니터 밝기와
//! 같은 "저장 → 변경 → 복원 + 잔존 복원" 패턴).
//!
//! 파일 (모두 ~/.switcher):
//! - clamshell.json  : {"mode":1|2,"saved":0|1} — 위젯이 쓰고 감시자가 정리
//! - clamshell.mode  : "1"/"2" — 감시자가 매 루프 읽는 현재 모드
//! - clamshell.off   : 존재 = 위젯의 해제 요청 (끄기·종료)
//! - clamshell-watch.sh : root 감시 스크립트 (감시자가 스스로 지운다)

#[cfg(target_os = "macos")]
pub use imp::{cycle, mode, on_quit, on_start};

#[cfg(not(target_os = "macos"))]
pub fn mode(_store: &std::path::Path) -> i8 {
    -1 // 미지원 플랫폼 — 프론트는 버튼을 그리지 않는다
}

#[cfg(not(target_os = "macos"))]
pub fn cycle(_app: &tauri::AppHandle, _store: &std::path::Path) -> Result<i8, String> {
    Err("클램셸 슬립 방지는 macOS 전용 기능입니다".into())
}

#[cfg(not(target_os = "macos"))]
pub fn on_quit(_store: &std::path::Path) {}

#[cfg(not(target_os = "macos"))]
pub fn on_start(_app: &tauri::AppHandle, _store: &std::path::Path) {}

#[cfg(target_os = "macos")]
mod imp {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use tauri::Emitter;

    struct Files {
        json: PathBuf,
        mode: PathBuf,
        off: PathBuf,
        script: PathBuf,
    }

    fn files(store: &Path) -> Files {
        Files {
            json: store.join("clamshell.json"),
            mode: store.join("clamshell.mode"),
            off: store.join("clamshell.off"),
            script: store.join("clamshell-watch.sh"),
        }
    }

    /// 현재 모드 — 진실의 원천은 상태 파일이다 (감시자가 복원하며 지우면 0으로 돌아온다)
    pub fn mode(store: &Path) -> i8 {
        let f = files(store);
        let Ok(text) = std::fs::read_to_string(&f.json) else {
            return 0;
        };
        serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("mode").and_then(|m| m.as_i64()))
            .map(|m| m.clamp(0, 2) as i8)
            .unwrap_or(0)
    }

    /// pmset -g의 SleepDisabled 현재 값 (root 불필요). 못 읽으면 0으로 간주 —
    /// 복원 시 "잠들 수 있는 보통 상태"로 돌리는 것이 안전한 기본값이다.
    fn sleep_disabled_now() -> u8 {
        let Ok(out) = Command::new("/usr/bin/pmset").arg("-g").output() else {
            return 0;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("SleepDisabled") {
                if rest.trim() == "1" {
                    return 1;
                }
            }
        }
        0
    }

    /// 셸 단일 인용 — 경로에 어떤 문자가 있어도 한 토큰으로 안전하게 전달된다
    fn shell_quote(path: &Path) -> String {
        format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
    }

    /// root 감시 스크립트 본문. 값(복원치·위젯 pid·파일 경로)은 생성 시점에 굽는다 —
    /// 감시자는 어떤 종료 경로로든 pmset을 원래 값으로 되돌리고 상태 파일을 정리한다.
    fn watch_script(store: &Path, saved: u8, widget_pid: u32) -> String {
        let f = files(store);
        let q = |p: &PathBuf| p.to_string_lossy().replace('\'', r"'\''");
        format!(
            "#!/bin/sh\n\
             # switcher 클램셸 감시자 (root) — 스스로 종료하며 SleepDisabled를 원복한다\n\
             MODE_FILE='{mode}'\n\
             OFF_FLAG='{off}'\n\
             STATE_JSON='{json}'\n\
             SELF='{script}'\n\
             SEEN_CLOSED=0\n\
             while :; do\n\
               [ -f \"$OFF_FLAG\" ] && break\n\
               [ -f \"$MODE_FILE\" ] || break\n\
               kill -0 {pid} 2>/dev/null || break\n\
               MODE=`cat \"$MODE_FILE\" 2>/dev/null`\n\
               if /usr/sbin/ioreg -r -k AppleClamshellState -d 1 | /usr/bin/grep -q '\"AppleClamshellState\" = Yes'; then\n\
                 SEEN_CLOSED=1\n\
               else\n\
                 if [ \"$SEEN_CLOSED\" = \"1\" ] && [ \"$MODE\" = \"1\" ]; then break; fi\n\
               fi\n\
               /bin/sleep 3\n\
             done\n\
             /usr/bin/pmset disablesleep {saved}\n\
             /bin/rm -f \"$MODE_FILE\" \"$OFF_FLAG\" \"$STATE_JSON\" \"$SELF\"\n",
            mode = q(&f.mode),
            off = q(&f.off),
            json = q(&f.json),
            script = q(&f.script),
            pid = widget_pid,
            saved = saved,
        )
    }

    /// 관리자 승인 한 번으로 pmset 켜기 + 감시자 기동. 사용자가 암호를 취소하면 Err.
    fn arm_with_admin(script: &Path) -> Result<(), String> {
        let shell_cmd = format!(
            "/usr/bin/pmset disablesleep 1 && ((/usr/bin/nohup /bin/sh {} >/dev/null 2>&1) &)",
            shell_quote(script)
        );
        run_admin(&shell_cmd)
    }

    fn run_admin(shell_cmd: &str) -> Result<(), String> {
        // AppleScript 문자열 이스케이프 (역슬래시 → 따옴표 순서)
        let escaped = shell_cmd.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "do shell script \"{escaped}\" with prompt \"switcher 클램셸 슬립 방지\" with administrator privileges"
        );
        let out = Command::new("/usr/bin/osascript")
            .args(["-e", &script])
            .output()
            .map_err(|e| format!("osascript 실행 실패: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            if err.contains("-128") {
                // User canceled
                Err("관리자 승인이 취소됐습니다 — 클램셸 모드를 켜지 않았습니다".into())
            } else {
                Err(format!("관리자 명령 실패: {}", err.trim()))
            }
        }
    }

    /// 상태 파일이 사라질 때까지(감시자의 복원 완료) 위젯 쪽에서 지켜보다가
    /// 프론트에 알린다 — 일회성 모드의 "덮개 열림 → 자동 꺼짐"이 버튼 표시에 반영된다.
    fn spawn_state_monitor(app: &tauri::AppHandle, store: PathBuf) {
        static RUNNING: AtomicBool = AtomicBool::new(false);
        if RUNNING.swap(true, Ordering::SeqCst) {
            return; // 이미 감시 중
        }
        let app = app.clone();
        std::thread::spawn(move || {
            let f = files(&store);
            while f.json.exists() {
                std::thread::sleep(Duration::from_secs(2));
            }
            RUNNING.store(false, Ordering::SeqCst);
            let _ = app.emit("clamshell-changed", ());
        });
    }

    /// 버튼 클릭: off → 일회성 → 지속 → off 순환. 새 모드를 돌려준다.
    pub fn cycle(app: &tauri::AppHandle, store: &Path) -> Result<i8, String> {
        let f = files(store);
        match mode(store) {
            0 => {
                std::fs::create_dir_all(store).map_err(|e| format!("폴더 생성 실패: {e}"))?;
                let saved = sleep_disabled_now();
                let script = watch_script(store, saved, std::process::id());
                std::fs::write(&f.script, script).map_err(|e| format!("스크립트 쓰기 실패: {e}"))?;
                std::fs::write(&f.mode, "1").map_err(|e| e.to_string())?;
                std::fs::write(
                    &f.json,
                    serde_json::json!({"mode": 1, "saved": saved}).to_string(),
                )
                .map_err(|e| e.to_string())?;
                let _ = std::fs::remove_file(&f.off); // 이전 세션 잔재가 감시자를 즉사시키지 않게
                if let Err(e) = arm_with_admin(&f.script) {
                    // 승인 실패 — 상태 파일을 남기면 켜진 것처럼 보인다. 전부 원상 복구.
                    let _ = std::fs::remove_file(&f.json);
                    let _ = std::fs::remove_file(&f.mode);
                    let _ = std::fs::remove_file(&f.script);
                    return Err(e);
                }
                spawn_state_monitor(app, store.to_path_buf());
                Ok(1)
            }
            1 => {
                // 일회성 → 지속: pmset은 이미 켜져 있다 — 모드 표시만 바꾼다 (암호 없음)
                std::fs::write(&f.mode, "2").map_err(|e| e.to_string())?;
                let saved = saved_value(&f.json);
                std::fs::write(
                    &f.json,
                    serde_json::json!({"mode": 2, "saved": saved}).to_string(),
                )
                .map_err(|e| e.to_string())?;
                Ok(2)
            }
            _ => {
                // 끄기: 해제 깃발을 세우면 root 감시자가 3초 안에 복원·정리한다 (암호 없음)
                std::fs::write(&f.off, b"off").map_err(|e| e.to_string())?;
                for _ in 0..16 {
                    std::thread::sleep(Duration::from_millis(500));
                    if !f.json.exists() {
                        return Ok(0);
                    }
                }
                // 감시자가 없다 (승인 후 죽었거나 재부팅으로 소멸) — 마지막 수단으로
                // 관리자 승인을 다시 받아 직접 복원한다
                let saved = saved_value(&f.json);
                run_admin(&format!("/usr/bin/pmset disablesleep {saved}"))?;
                let _ = std::fs::remove_file(&f.json);
                let _ = std::fs::remove_file(&f.mode);
                let _ = std::fs::remove_file(&f.off);
                let _ = std::fs::remove_file(&f.script);
                Ok(0)
            }
        }
    }

    fn saved_value(json: &Path) -> u8 {
        std::fs::read_to_string(json)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.get("saved").and_then(|s| s.as_u64()))
            .map(|s| (s.min(1)) as u8)
            .unwrap_or(0)
    }

    /// 위젯 종료 준비 — 해제 깃발만 세운다 (감시자가 뒤처리, 우리는 기다리지 않는다).
    /// 지속 모드도 여기서 끝난다: 감시자 없는 no-sleep 상태를 남기지 않는 것이
    /// 배터리 안전상 우선이다 (재시작 후 계속 원하면 버튼을 다시 누른다).
    pub fn on_quit(store: &Path) {
        let f = files(store);
        if f.json.exists() {
            let _ = std::fs::write(&f.off, b"off");
        }
    }

    /// 앱 시작 시 잔존 상태 복원 — 지난 세션이 클램셸 모드를 켠 채 죽었어도
    /// 감시자(pid 감시)가 이미 복원했거나, 이 깃발로 곧 복원된다. 15초가 지나도
    /// 정리가 안 되면 감시자가 소멸한 것 — 파일만 걷어내고 경고를 남긴다
    /// (재부팅이면 SleepDisabled도 이미 초기화돼 있어 실해는 없다).
    pub fn on_start(app: &tauri::AppHandle, store: &Path) {
        let f = files(store);
        if !f.json.exists() {
            return;
        }
        let _ = std::fs::write(&f.off, b"off");
        let app = app.clone();
        let store = store.to_path_buf();
        std::thread::spawn(move || {
            let f = files(&store);
            for _ in 0..30 {
                std::thread::sleep(Duration::from_millis(500));
                if !f.json.exists() {
                    let _ = app.emit("clamshell-changed", ());
                    return;
                }
            }
            eprintln!(
                "클램셸 잔존 상태를 감시자가 정리하지 않음 — 파일만 정리합니다 \
                 (SleepDisabled가 남았다면 재부팅 또는 `sudo pmset disablesleep 0`)"
            );
            let _ = std::fs::remove_file(&f.json);
            let _ = std::fs::remove_file(&f.mode);
            let _ = std::fs::remove_file(&f.off);
            let _ = std::fs::remove_file(&f.script);
            let _ = app.emit("clamshell-changed", ());
        });
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn test_store(tag: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!(
                "switcher-clamshell-{}-{tag}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        #[test]
        fn script_bakes_paths_pid_and_restore_value() {
            let store = test_store("script");
            let script = watch_script(&store, 1, 4242);
            assert!(script.contains("pmset disablesleep 1"));
            assert!(script.contains("kill -0 4242"));
            assert!(script.contains(&*store.join("clamshell.mode").to_string_lossy()));
            // 복원값 0도 그대로 박힌다
            assert!(watch_script(&store, 0, 1).contains("pmset disablesleep 0"));
        }

        #[test]
        fn mode_reads_state_file_and_defaults_to_off() {
            let store = test_store("mode");
            assert_eq!(mode(&store), 0);
            std::fs::write(store.join("clamshell.json"), r#"{"mode":2,"saved":0}"#).unwrap();
            assert_eq!(mode(&store), 2);
            std::fs::write(store.join("clamshell.json"), "broken").unwrap();
            assert_eq!(mode(&store), 0, "깨진 상태 파일은 off로 취급");
        }

        #[test]
        fn sleep_disabled_parse_runs_on_real_pmset() {
            // 실제 pmset -g 출력 파싱 — root 불필요. 보통 환경에선 0이다.
            let v = sleep_disabled_now();
            assert!(v == 0 || v == 1);
        }

        #[test]
        fn shell_quote_wraps_and_escapes() {
            assert_eq!(shell_quote(Path::new("/a b/c")), "'/a b/c'");
            assert_eq!(
                shell_quote(Path::new("/a'b")),
                r"'/a'\''b'"
            );
        }

        /// 감시자 스크립트 실체 검증 (pmset 효과 제외 — root 없이 실행하므로
        /// pmset 줄만 실패하고 루프·깃발·정리 로직은 그대로 돈다).
        /// 몇 초짜리 폴링이라 평시 스위트에서는 제외: `cargo test -- --ignored slow_watcher`
        #[test]
        #[ignore]
        fn slow_watcher_cleans_up_on_off_flag() {
            let store = test_store("watch");
            let f = files(&store);
            std::fs::write(&f.mode, "2").unwrap();
            std::fs::write(&f.json, r#"{"mode":2,"saved":0}"#).unwrap();
            std::fs::write(&f.script, watch_script(&store, 0, std::process::id())).unwrap();
            let mut child = Command::new("/bin/sh").arg(&f.script).spawn().unwrap();
            std::thread::sleep(Duration::from_millis(500));
            std::fs::write(&f.off, b"off").unwrap();
            let mut cleaned = false;
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(500));
                if !f.json.exists() && !f.mode.exists() && !f.script.exists() {
                    cleaned = true;
                    break;
                }
            }
            let _ = child.kill();
            assert!(cleaned, "해제 깃발 후 감시자가 상태 파일을 정리해야 한다");
        }

        /// 위젯 죽음(pid 소멸) 감지 — 존재하지 않는 pid로 스크립트를 돌리면
        /// 첫 루프에서 빠져나와 정리해야 한다 (크래시 안전망)
        #[test]
        #[ignore]
        fn slow_watcher_exits_when_widget_pid_dead() {
            let store = test_store("watch-pid");
            let f = files(&store);
            std::fs::write(&f.mode, "2").unwrap();
            std::fs::write(&f.json, r#"{"mode":2,"saved":0}"#).unwrap();
            // pid 4194304는 macOS pid_max(99999) 위 — 존재할 수 없다
            std::fs::write(&f.script, watch_script(&store, 0, 4_194_304)).unwrap();
            let mut child = Command::new("/bin/sh").arg(&f.script).spawn().unwrap();
            let mut cleaned = false;
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(500));
                if !f.json.exists() {
                    cleaned = true;
                    break;
                }
            }
            let _ = child.kill();
            assert!(cleaned, "위젯 pid가 죽어 있으면 감시자는 즉시 복원·정리해야 한다");
        }
    }
}

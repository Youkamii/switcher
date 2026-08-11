//! 클램셸 슬립 방지 (macOS 전용) — 덮개를 닫아도 잠들지 않게 해 내부 터미널의
//! AI 작업이 계속 돌게 한다 (사용자 요청 2026-08-11, #55).
//!
//! 뼈대: `pmset disablesleep 1/0` (root 필요 — 덮개 닫힘 슬립은 caffeinate류
//! 어서션으로 못 막는다는 것이 통설이며, 실기기 덮개 사이클은 사용자 실측 항목).
//! 관리자 암호 프롬프트는 **켤 때 한 번만** — 그 승인으로 root 감시자를 함께 띄워,
//! 이후의 복원(끄기 클릭·일회성 덮개 열림·위젯 종료·크래시)은 전부 감시자가
//! 암호 없이 수행한다.
//!
//! 보안 (적대 리뷰 2026-08-11): 감시자 본문은 디스크 스크립트가 아니라 승인 명령
//! 문자열에 **인라인**한다 — root가 사용자 소유 파일을 실행하면 같은 사용자 권한의
//! 다른 프로세스가 승인 전후·실행 중에 내용을 바꿔 root로 코드를 심을 수 있다
//! (sh는 스크립트 파일을 실행 중에도 이어 읽는다). 인라인이면 승인 순간의 문자열이
//! 전부이고, 이후 바꿀 파일 자체가 없다.
//!
//! 상태 순환: off(0) → 일회성(1) → 지속(2) → off. 일회성은 덮개가 닫혔다 열리는
//! 사이클 하나가 끝나면 자동 복원, 지속은 끄거나 위젯이 종료될 때까지.
//! 켜기 전 SleepDisabled 값을 저장해 복원 시 그대로 되돌린다 (블랙 모니터 밝기와
//! 같은 "저장 → 변경 → 복원 + 잔존 복원" 패턴).
//!
//! 파일 (모두 ~/.switcher — 스크립트 파일은 없다):
//! - clamshell.json : {"mode":1|2,"saved":0|1} — 위젯이 쓰고 감시자가 정리
//! - clamshell.mode : "1"/"2" — 감시자가 매 루프 읽는 현재 모드
//! - clamshell.off  : 존재 = 위젯의 해제 요청 (끄기·종료·시작 시 잔존 정리)
//! - clamshell.pid  : 감시자(root sh)의 pid — 위젯이 감시자 생존을 확인하는 용도

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
    use std::time::Duration;
    use tauri::Emitter;

    struct Files {
        json: PathBuf,
        mode: PathBuf,
        off: PathBuf,
        wpid: PathBuf,
        /// v1.7.34가 잠깐 쓰던 스크립트 파일 — 잔재가 있으면 정리만 한다
        legacy_script: PathBuf,
    }

    fn files(store: &Path) -> Files {
        Files {
            json: store.join("clamshell.json"),
            mode: store.join("clamshell.mode"),
            off: store.join("clamshell.off"),
            wpid: store.join("clamshell.pid"),
            legacy_script: store.join("clamshell-watch.sh"),
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

    /// 셸 단일 인용 — 어떤 문자가 있어도 한 토큰으로 안전하게 전달된다
    fn sq(s: &str) -> String {
        format!("'{}'", s.replace('\'', r"'\''"))
    }

    /// root 감시자 본문 (한 줄 POSIX sh — osascript 승인 명령에 인라인된다).
    /// 값(복원치·위젯 pid·파일 경로)은 생성 시점에 굽는다. 감시자는 어떤 종료
    /// 경로로든 pmset을 원래 값으로 되돌리고 상태 파일을 정리한다.
    /// - 위젯 생존은 pid의 "정체"(ps comm=에 switcher 포함)로 본다 — 맨 kill -0은
    ///   pid 재사용에 속아 no-sleep이 고착될 수 있다 (적대 리뷰)
    /// - 자기 pid를 PIDF에 남긴다 — 위젯이 감시자 생존을 확인하는 근거
    fn watch_body(store: &Path, saved: u8, widget_pid: u32) -> String {
        let f = files(store);
        let q = |p: &PathBuf| sq(&p.to_string_lossy());
        format!(
            "PIDF={pidf}; MODE_FILE={mode}; OFF_FLAG={off}; STATE_JSON={json}; \
             echo $$ > \"$PIDF\"; SEEN=0; \
             while :; do \
             [ -f \"$OFF_FLAG\" ] && break; \
             [ -f \"$MODE_FILE\" ] || break; \
             case \"$(/bin/ps -p {pid} -o comm= 2>/dev/null)\" in *switcher*) : ;; *) break ;; esac; \
             M=$(cat \"$MODE_FILE\" 2>/dev/null); \
             if /usr/sbin/ioreg -r -k AppleClamshellState -d 1 | /usr/bin/grep -q '\"AppleClamshellState\" = Yes'; then SEEN=1; \
             elif [ \"$SEEN\" = 1 ] && [ \"$M\" = 1 ]; then break; fi; \
             /bin/sleep 3; \
             done; \
             /usr/bin/pmset disablesleep {saved}; \
             /bin/rm -f \"$MODE_FILE\" \"$OFF_FLAG\" \"$STATE_JSON\" \"$PIDF\"",
            pidf = q(&f.wpid),
            mode = q(&f.mode),
            off = q(&f.off),
            json = q(&f.json),
            pid = widget_pid,
            saved = saved,
        )
    }

    /// 관리자 승인 한 번에 실행되는 전체 명령: pmset 켜기 + 감시자(백그라운드) 기동.
    /// 감시자 본문은 sh -c 인자로 인라인 — 디스크에 root가 실행할 파일을 남기지 않는다.
    fn arm_command(body: &str) -> String {
        format!(
            "/usr/bin/pmset disablesleep 1 && ( /usr/bin/nohup /bin/sh -c {} >/dev/null 2>&1 & )",
            sq(body)
        )
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

    /// 감시자(root sh) 생존 확인 — clamshell.pid의 pid에 신호 0을 보내 본다.
    /// root 프로세스라 EPERM이 돌아오면 그것이 곧 "살아 있다"는 뜻이다.
    fn watcher_alive(store: &Path) -> bool {
        let f = files(store);
        let Some(pid) = std::fs::read_to_string(&f.wpid)
            .ok()
            .and_then(|t| t.trim().parse::<i32>().ok())
            .filter(|p| *p > 0)
        else {
            return false;
        };
        let ret = unsafe { libc::kill(pid, 0) };
        ret == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    /// 상태 파일이 사라질 때까지(감시자의 복원 완료) 지켜보다가 프론트에 알린다 —
    /// 일회성의 "덮개 열림 → 자동 꺼짐"이 버튼 표시에 반영된다. 켤 때마다 하나씩
    /// 띄운다 — 중복 스레드는 무해(같은 파일을 보다 같이 끝난다)하고, 재사용
    /// 가드를 두면 빠른 끄기→켜기에서 새 감시가 안 붙는 경합이 생긴다 (적대 리뷰).
    fn spawn_state_monitor(app: &tauri::AppHandle, store: PathBuf) {
        let app = app.clone();
        std::thread::spawn(move || {
            let f = files(&store);
            while f.json.exists() {
                std::thread::sleep(Duration::from_secs(2));
            }
            let _ = app.emit("clamshell-changed", ());
        });
    }

    fn cleanup_files(f: &Files) {
        let _ = std::fs::remove_file(&f.json);
        let _ = std::fs::remove_file(&f.mode);
        let _ = std::fs::remove_file(&f.off);
        let _ = std::fs::remove_file(&f.wpid);
        let _ = std::fs::remove_file(&f.legacy_script);
    }

    /// 버튼 클릭: off → 일회성 → 지속 → off 순환. 새 모드를 돌려준다.
    pub fn cycle(app: &tauri::AppHandle, store: &Path) -> Result<i8, String> {
        let f = files(store);
        match mode(store) {
            0 => {
                std::fs::create_dir_all(store).map_err(|e| format!("폴더 생성 실패: {e}"))?;
                let saved = sleep_disabled_now();
                std::fs::write(&f.mode, "1").map_err(|e| e.to_string())?;
                std::fs::write(
                    &f.json,
                    serde_json::json!({"mode": 1, "saved": saved}).to_string(),
                )
                .map_err(|e| e.to_string())?;
                // 이전 세션 잔재가 새 감시자를 즉사시키거나 생존 판정을 속이지 않게
                let _ = std::fs::remove_file(&f.off);
                let _ = std::fs::remove_file(&f.wpid);
                let _ = std::fs::remove_file(&f.legacy_script);
                let body = watch_body(store, saved, std::process::id());
                if let Err(e) = run_admin(&arm_command(&body)) {
                    // 승인 실패 — 상태 파일을 남기면 켜진 것처럼 보인다. 전부 원상 복구.
                    let _ = std::fs::remove_file(&f.json);
                    let _ = std::fs::remove_file(&f.mode);
                    return Err(e);
                }
                spawn_state_monitor(app, store.to_path_buf());
                Ok(1)
            }
            1 => {
                // 일회성 → 지속. 그 사이 감시자가 사라졌다면(덮개 사이클 완료 직후의
                // 겹침 등) "켜짐 표시인데 실제는 꺼짐"이 되므로 생존부터 확인한다 (적대 리뷰)
                if !watcher_alive(store) {
                    let saved = saved_value(&f.json);
                    // 감시자 없이 남은 no-sleep 가능성을 관리자 승인으로 직접 복원
                    run_admin(&format!("/usr/bin/pmset disablesleep {saved}"))?;
                    cleanup_files(&f);
                    let _ = app.emit("clamshell-changed", ());
                    return Err(
                        "클램셸 감시자가 이미 종료돼 있어 안전하게 껐습니다 — 필요하면 다시 켜세요".into(),
                    );
                }
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
                cleanup_files(&f);
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
    /// 감시자(정체 감시)가 이미 복원했거나, 이 깃발로 곧 복원된다. 15초가 지나도
    /// 정리가 안 되면 감시자가 소멸한 것 — 파일만 걷어내고 경고를 남긴다
    /// (재부팅이면 SleepDisabled도 이미 초기화돼 있어 실해는 없다).
    pub fn on_start(app: &tauri::AppHandle, store: &Path) {
        let f = files(store);
        // 스크립트 파일을 쓰던 구버전(v1.7.34) 잔재는 상태와 무관하게 정리
        let _ = std::fs::remove_file(&f.legacy_script);
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
            cleanup_files(&f);
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
        fn body_bakes_paths_pid_and_restore_value() {
            let store = test_store("body");
            let body = watch_body(&store, 1, 4242);
            assert!(body.contains("pmset disablesleep 1"));
            assert!(body.contains("/bin/ps -p 4242 -o comm="));
            assert!(body.contains("*switcher*"), "pid 재사용 방어(정체 확인)가 있어야 한다");
            assert!(body.contains(&*store.join("clamshell.mode").to_string_lossy()));
            assert!(body.contains("clamshell.pid"), "감시자 pid 기록이 있어야 한다");
            assert!(!body.contains('\n'), "인라인 본문은 한 줄이어야 한다 (AppleScript 문자열)");
            // 복원값 0도 그대로 박힌다
            assert!(watch_body(&store, 0, 1).contains("pmset disablesleep 0"));
        }

        /// 본문·승인 명령이 실제 /bin/sh 문법으로 유효한지 — 실행 없이 파싱만 (sh -n)
        #[test]
        fn body_and_arm_command_are_valid_sh() {
            let store = test_store("syntax");
            let body = watch_body(&store, 0, std::process::id());
            for cmd in [body.clone(), arm_command(&body)] {
                let out = Command::new("/bin/sh").args(["-n", "-c", &cmd]).output().unwrap();
                assert!(
                    out.status.success(),
                    "sh 문법 오류: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
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
        fn shell_single_quote_wraps_and_escapes() {
            assert_eq!(sq("/a b/c"), "'/a b/c'");
            assert_eq!(sq("/a'b"), r"'/a'\''b'");
            // 본문 인라인 이중 인용도 왕복 가능해야 한다: sh가 그대로 되돌려주는지
            let tricky = r#"x 'y' "z" $HOME"#;
            let out = Command::new("/bin/sh")
                .args(["-c", &format!("printf %s {}", sq(tricky))])
                .output()
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), tricky);
        }

        #[test]
        fn watcher_alive_reflects_pid_file() {
            let store = test_store("alive");
            let f = files(&store);
            assert!(!watcher_alive(&store), "pid 파일이 없으면 죽은 것");
            // 자기 자신 = 살아 있는 사용자 프로세스 (kill 0 == 0)
            std::fs::write(&f.wpid, std::process::id().to_string()).unwrap();
            assert!(watcher_alive(&store));
            // pid 1(launchd, root) = EPERM이 곧 생존 신호 — 감시자(root)의 실제 경로
            std::fs::write(&f.wpid, "1").unwrap();
            assert!(watcher_alive(&store), "root 프로세스는 EPERM으로 생존 판정");
            // macOS pid 상한(99999) 위 = 존재 불가
            std::fs::write(&f.wpid, "4194304").unwrap();
            assert!(!watcher_alive(&store));
            std::fs::write(&f.wpid, "garbage").unwrap();
            assert!(!watcher_alive(&store));
        }

        /// 감시자 본문 실동작 검증 (pmset 효과 제외 — root 없이 실행하므로
        /// pmset 줄만 실패하고 루프·깃발·정리 로직은 그대로 돈다).
        /// 몇 초짜리 폴링이라 평시 스위트에서는 제외: `cargo test -- --ignored slow_watcher`
        #[test]
        #[ignore]
        fn slow_watcher_cleans_up_on_off_flag() {
            let store = test_store("watch");
            let f = files(&store);
            std::fs::write(&f.mode, "2").unwrap();
            std::fs::write(&f.json, r#"{"mode":2,"saved":0}"#).unwrap();
            // 테스트 바이너리 이름에 "switcher"가 들어가 정체 확인을 통과한다
            let body = watch_body(&store, 0, std::process::id());
            let mut child = Command::new("/bin/sh").args(["-c", &body]).spawn().unwrap();
            std::thread::sleep(Duration::from_millis(700));
            assert!(f.wpid.exists(), "감시자가 자기 pid를 남겨야 한다");
            assert!(watcher_alive(&store));
            std::fs::write(&f.off, b"off").unwrap();
            let mut cleaned = false;
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(500));
                if !f.json.exists() && !f.mode.exists() && !f.wpid.exists() {
                    cleaned = true;
                    break;
                }
            }
            let _ = child.kill();
            assert!(cleaned, "해제 깃발 후 감시자가 상태 파일을 정리해야 한다");
        }

        /// 위젯 죽음(pid 소멸·정체 불일치) 감지 — 존재하지 않는 pid로 돌리면
        /// 첫 루프에서 빠져나와 정리해야 한다 (크래시 안전망 + pid 재사용 방어)
        #[test]
        #[ignore]
        fn slow_watcher_exits_when_widget_pid_dead() {
            let store = test_store("watch-pid");
            let f = files(&store);
            std::fs::write(&f.mode, "2").unwrap();
            std::fs::write(&f.json, r#"{"mode":2,"saved":0}"#).unwrap();
            // pid 4194304는 macOS pid_max(99999) 위 — 존재할 수 없다
            let body = watch_body(&store, 0, 4_194_304);
            let mut child = Command::new("/bin/sh").args(["-c", &body]).spawn().unwrap();
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

        /// pid 재사용 시나리오: 살아 있지만 switcher가 아닌 프로세스의 pid를 주면
        /// 정체 불일치로 즉시 끝나야 한다 (맨 kill -0이면 영구 고착되는 케이스)
        #[test]
        #[ignore]
        fn slow_watcher_exits_on_pid_identity_mismatch() {
            let store = test_store("watch-ident");
            let f = files(&store);
            std::fs::write(&f.mode, "2").unwrap();
            std::fs::write(&f.json, r#"{"mode":2,"saved":0}"#).unwrap();
            // pid 1 = launchd: 살아 있(고 kill -0은 통과했)지만 switcher가 아니다
            let body = watch_body(&store, 0, 1);
            let mut child = Command::new("/bin/sh").args(["-c", &body]).spawn().unwrap();
            let mut cleaned = false;
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(500));
                if !f.json.exists() {
                    cleaned = true;
                    break;
                }
            }
            let _ = child.kill();
            assert!(cleaned, "pid가 딴 프로세스로 재사용됐으면 감시자는 끝나야 한다");
        }
    }
}

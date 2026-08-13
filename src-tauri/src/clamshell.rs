//! 클램셸 슬립 방지 (macOS 전용) — 덮개를 닫아도 내부 작업이 계속 돌게 한다.
//!
//! 상태 순환: off(0) → 일회성(1) → 지속(2) → off.
//! - 일회성: 덮개를 한 번 닫았다 열면 켜기 전 SleepDisabled 값으로 자동 복원한다.
//! - 지속: 사용자가 버튼으로 명시적으로 끌 때까지 유지한다. 위젯 종료·재시작은
//!   살아 있는 감시자를 입양해 유지하지만, **재부팅(감시자 소멸)은 원상 복원 후
//!   off로 시작한다** — 재무장 암호창을 띄우지 않는다 (사용자 결정 2026-08-12).
//!
//! mode·saved·revision·watcher를 상태 파일 하나에 원자적으로 쓴다. 관리자 권한의
//! 감시자는 그 파일을 최대 512바이트·1초 제한으로 읽을 뿐 사용자 경로에 쓰거나
//! 지우지 않는다. pid 기록과 상태 정리는 위젯이 자기 권한으로 수행한다.

#[cfg(target_os = "macos")]
pub use imp::{cycle, mode, on_quit, on_start};

#[cfg(not(target_os = "macos"))]
pub fn mode(_store: &std::path::Path) -> i8 {
    -1
}

#[cfg(not(target_os = "macos"))]
pub fn cycle(_app: &tauri::AppHandle, _store: &std::path::Path) -> Result<i8, String> {
    Err("클램셸 슬립 방지는 macOS 전용 기능입니다".into())
}

#[cfg(not(target_os = "macos"))]
pub fn on_quit(_store: &std::path::Path) {}

#[cfg(not(target_os = "macos"))]
pub fn on_start(_app: &tauri::AppHandle, _store: &std::path::Path) {}

#[cfg(any(target_os = "macos", test))]
#[cfg_attr(all(test, not(target_os = "macos")), allow(dead_code))]
mod imp {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use tauri::Emitter;

    static OPERATION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static STARTUP_RECOVERY: AtomicBool = AtomicBool::new(false);
    static TOKEN_SEQ: AtomicU64 = AtomicU64::new(0);
    static MONITOR_SEQ: AtomicU64 = AtomicU64::new(0);

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct State {
        mode: u8,
        saved: u8,
        revision: String,
        watcher: String,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct WatcherRecord {
        pid: i32,
        watcher: String,
    }

    struct Files {
        state: PathBuf,
        pid: PathBuf,
        legacy_json: PathBuf,
        legacy_mode: PathBuf,
        legacy_off: PathBuf,
        legacy_script: PathBuf,
    }

    fn files(store: &Path) -> Files {
        Files {
            state: store.join("clamshell.state"),
            pid: store.join("clamshell.pid"),
            legacy_json: store.join("clamshell.json"),
            legacy_mode: store.join("clamshell.mode"),
            legacy_off: store.join("clamshell.off"),
            legacy_script: store.join("clamshell-watch.sh"),
        }
    }

    fn token(prefix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let seq = TOKEN_SEQ.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}-{}-{nanos}-{seq}", std::process::id())
    }

    fn valid_token(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    }

    fn state_text(state: &State) -> String {
        format!(
            "mode={}\nsaved={}\nrevision={}\nwatcher={}\n",
            state.mode, state.saved, state.revision, state.watcher
        )
    }

    fn parse_state(text: &str) -> Result<State, String> {
        let mut mode = None;
        let mut saved = None;
        let mut revision = None;
        let mut watcher = None;
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                return Err("클램셸 상태 파일 형식이 올바르지 않습니다".into());
            };
            match key {
                "mode" if mode.is_none() => mode = value.parse::<u8>().ok(),
                "saved" if saved.is_none() => saved = value.parse::<u8>().ok(),
                "revision" if revision.is_none() => revision = Some(value.to_string()),
                "watcher" if watcher.is_none() => watcher = Some(value.to_string()),
                _ => return Err("클램셸 상태 파일 형식이 올바르지 않습니다".into()),
            }
        }
        let state = State {
            mode: mode.ok_or("클램셸 상태에 mode가 없습니다")?,
            saved: saved.ok_or("클램셸 상태에 원래 설정이 없습니다")?,
            revision: revision.ok_or("클램셸 상태에 revision이 없습니다")?,
            watcher: watcher.ok_or("클램셸 상태에 watcher가 없습니다")?,
        };
        if state.mode > 2
            || state.saved > 1
            || !valid_token(&state.revision)
            || !valid_token(&state.watcher)
        {
            return Err("클램셸 상태 값이 올바르지 않습니다".into());
        }
        Ok(state)
    }

    fn read_state(path: &Path) -> Result<Option<State>, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!("클램셸 상태 읽기 실패 {}: {error}", path.display()));
            }
        };
        parse_state(&text).map(Some)
    }

    fn write_state(path: &Path, state: &State) -> Result<(), String> {
        crate::accounts::atomic_write_existing_parent(path, state_text(state).as_bytes())
            .map_err(|error| format!("클램셸 상태 저장 실패: {error}"))
    }

    fn transitioned(state: &State, mode: u8) -> State {
        State {
            mode,
            saved: state.saved,
            revision: token("rev"),
            watcher: state.watcher.clone(),
        }
    }

    fn fresh_state(mode: u8, saved: u8) -> State {
        State {
            mode,
            saved,
            revision: token("rev"),
            watcher: token("watch"),
        }
    }

    fn legacy_mode(store: &Path) -> i8 {
        let Ok(text) = std::fs::read_to_string(files(store).legacy_json) else {
            return 0;
        };
        serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|value| value.get("mode").and_then(|mode| mode.as_i64()))
            .filter(|mode| (1..=2).contains(mode))
            .map(|mode| mode as i8)
            .unwrap_or(0)
    }

    pub fn mode(store: &Path) -> i8 {
        match read_state(&files(store).state) {
            Ok(Some(state)) if (1..=2).contains(&state.mode) => state.mode as i8,
            Ok(Some(_)) => 0,
            Ok(None) => legacy_mode(store),
            Err(_) => 0,
        }
    }

    /// SleepDisabled가 한 번도 설정되지 않은 정상 Mac은 이 줄 자체가 없다. 그 경우는
    /// 기능상 0이다. 줄이 있는데 값이 깨진 경우만 오류로 구분한다.
    fn parse_sleep_disabled(text: &str) -> Result<u8, String> {
        for line in text.lines() {
            let mut fields = line.split_whitespace();
            if fields.next() != Some("SleepDisabled") {
                continue;
            }
            return match (fields.next(), fields.next()) {
                (Some("0"), None) => Ok(0),
                (Some("1"), None) => Ok(1),
                _ => Err("pmset의 SleepDisabled 값이 올바르지 않습니다".into()),
            };
        }
        Ok(0)
    }

    fn sleep_disabled_now() -> Result<u8, String> {
        let out = Command::new("/usr/bin/pmset")
            .arg("-g")
            .output()
            .map_err(|error| format!("SleepDisabled 확인 실패: {error}"))?;
        if !out.status.success() {
            return Err(format!(
                "SleepDisabled 확인 실패: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        parse_sleep_disabled(&String::from_utf8_lossy(&out.stdout))
    }

    fn sq(text: &str) -> String {
        format!("'{}'", text.replace('\'', r"'\''"))
    }

    /// root 감시자는 사용자 상태를 최대 512바이트만 읽고, FIFO·장치 파일로 바뀌어도
    /// 1초 뒤 reader를 죽인다. watcher ID가 다르면 새 세대가 주인이므로 복원도 하지 않고
    /// 종료한다. 사용자 경로에는 어떤 쓰기·삭제도 하지 않는다.
    fn watch_body(state_path: &Path, saved: u8, watcher: &str) -> String {
        let template = r#"SWITCHER_CLAMSHELL_WATCH=1
trap '' HUP
STATE_FILE=__STATE__
WATCH_ID=__WATCHER__
SEEN=0
set -f
read_state() {
  DATA=$(
    /usr/bin/head -c 512 "$STATE_FILE" 2>/dev/null & R=$!
    ( /bin/sleep 1; /bin/kill "$R" 2>/dev/null ) >/dev/null 2>&1 & T=$!
    wait "$R" 2>/dev/null
    /bin/kill "$T" 2>/dev/null
    wait "$T" 2>/dev/null
  )
  M=
  W=
  for F in $DATA; do
    case "$F" in
      mode=0|mode=1|mode=2) M=${F#mode=} ;;
      watcher=*) W=${F#watcher=} ;;
    esac
  done
}
while :; do
  read_state
  if [ -n "$W" ] && [ "$W" != "$WATCH_ID" ]; then exit 0; fi
  WANT_RESTORE=0
  if [ "$W" != "$WATCH_ID" ] || [ "$M" = 0 ]; then
    WANT_RESTORE=1
  elif /usr/sbin/ioreg -r -k AppleClamshellState -d 1 | /usr/bin/grep -q '"AppleClamshellState" = Yes'; then
    SEEN=1
  elif [ "$SEEN" = 1 ] && [ "$M" = 1 ]; then
    /bin/sleep 2
    read_state
    if [ -n "$W" ] && [ "$W" != "$WATCH_ID" ]; then exit 0; fi
    if [ "$W" = "$WATCH_ID" ] && [ "$M" = 2 ]; then
      SEEN=0
    else
      WANT_RESTORE=1
    fi
  fi
  if [ "$WANT_RESTORE" = 1 ]; then
    read_state
    if [ -n "$W" ] && [ "$W" != "$WATCH_ID" ]; then exit 0; fi
    if [ "$W" = "$WATCH_ID" ] && [ "$M" = 2 ]; then
      SEEN=0
    elif /usr/bin/pmset -a disablesleep __SAVED__; then
      read_state
      if [ "$W" = "$WATCH_ID" ] && [ "$M" = 2 ]; then
        /usr/bin/pmset -a disablesleep 1
        SEEN=0
      else
        exit 0
      fi
    fi
  fi
  /bin/sleep 2
done"#;
        template
            .replace("__STATE__", &sq(&state_path.to_string_lossy()))
            .replace("__WATCHER__", watcher)
            .replace("__SAVED__", &saved.to_string())
    }

    /// nohup 금지 (실측 macOS 26.5): 관리자 승인(do shell script) 컨텍스트에는 제어
    /// 터미널이 없어 /usr/bin/nohup이 "can't detach from console"로 명령 실행 전에
    /// 즉사한다 — 순수 백그라운드 + 본문의 `trap '' HUP`으로 대체한다 (승인 셸이
    /// 끝나면 launchd로 재입양되어 계속 돈다, 실측 확인).
    fn arm_command(body: &str) -> String {
        format!(
            "/usr/bin/pmset -a disablesleep 1 && {{ \
             /bin/sh -c {} </dev/null >/dev/null 2>&1 & P=$!; \
             /bin/sleep 1; /bin/kill -0 \"$P\" && /bin/echo \"$P\"; \
             }}",
            sq(body)
        )
    }

    /// 감시자 강제 종료 + 복원 (관리자 경로). pid 정체(argv 표식·토큰)를 kill 직전에
    /// 같은 승인 명령 안에서 재확인한다 — 승인 창이 떠 있는 동안 감시자가 스스로 죽고
    /// pid가 재사용되면 root가 무관한 프로세스를 죽이게 된다. 토큰은 내부 생성이거나
    /// valid_token 관문을 지난 안전 문자셋이라 패턴에 그대로 넣어도 된다.
    fn kill_watcher_command(record: &WatcherRecord, saved: u8) -> String {
        format!(
            "case \"$(/bin/ps -ww -p {pid} -o command= 2>/dev/null)\" in \
             *'SWITCHER_CLAMSHELL_WATCH=1'*'{watcher}'*) /bin/kill {pid} 2>/dev/null ;; \
             esac; /usr/bin/pmset -a disablesleep {saved}",
            pid = record.pid,
            watcher = record.watcher,
            saved = saved,
        )
    }

    fn run_admin(shell_cmd: &str) -> Result<String, String> {
        // 명령을 AppleScript 문자열에 보간하지 않고 argv로 넘긴다. HOME 경로에 따옴표나
        // 줄바꿈이 있어도 AppleScript 소스가 바뀌지 않는다.
        let script = "on run argv\n  do shell script (item 1 of argv) with prompt \"switcher 클램셸 슬립 방지\" with administrator privileges\nend run";
        let out = Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(script)
            .arg(shell_cmd)
            .output()
            .map_err(|error| format!("osascript 실행 실패: {error}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            let error = String::from_utf8_lossy(&out.stderr);
            if error.contains("-128") {
                Err("관리자 승인이 취소됐습니다 — 클램셸 상태를 변경하지 않았습니다".into())
            } else {
                Err(format!("관리자 명령 실패: {}", error.trim()))
            }
        }
    }

    fn parse_watcher_pid(text: &str) -> Option<i32> {
        text.lines()
            .rev()
            .find_map(|line| line.trim().parse::<i32>().ok())
            .filter(|pid| *pid > 1)
    }

    fn watcher_record_text(record: &WatcherRecord) -> String {
        format!("pid={}\nwatcher={}\n", record.pid, record.watcher)
    }

    fn parse_watcher_record(text: &str) -> Option<WatcherRecord> {
        let mut pid = None;
        let mut watcher = None;
        for line in text.lines() {
            let (key, value) = line.split_once('=')?;
            match key {
                "pid" if pid.is_none() => pid = value.parse::<i32>().ok(),
                "watcher" if watcher.is_none() => watcher = Some(value.to_string()),
                _ => return None,
            }
        }
        let record = WatcherRecord {
            pid: pid.filter(|pid| *pid > 1)?,
            watcher: watcher?,
        };
        valid_token(&record.watcher).then_some(record)
    }

    fn read_watcher_record(path: &Path) -> Option<WatcherRecord> {
        parse_watcher_record(&std::fs::read_to_string(path).ok()?)
    }

    fn watcher_alive_record(record: &WatcherRecord) -> bool {
        let out = Command::new("/bin/ps")
            .args(["-ww", "-p", &record.pid.to_string(), "-o", "uid=", "-o", "command="])
            .output();
        let Ok(out) = out else { return false };
        if !out.status.success() {
            return false;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let trimmed = text.trim();
        let Some(split_at) = trimmed.find(char::is_whitespace) else {
            return false;
        };
        let uid = &trimmed[..split_at];
        let command = trimmed[split_at..].trim();
        uid == "0"
            && command.contains("SWITCHER_CLAMSHELL_WATCH=1")
            && command.contains(&record.watcher)
    }

    fn active_watcher(files: &Files) -> Option<WatcherRecord> {
        let record = read_watcher_record(&files.pid)?;
        let state = read_state(&files.state).ok().flatten()?;
        (record.watcher == state.watcher && watcher_alive_record(&record)).then_some(record)
    }

    fn start_watcher(files: &Files, state: &State) -> Result<WatcherRecord, String> {
        let _ = remove_file(&files.pid);
        let body = watch_body(&files.state, state.saved, &state.watcher);
        let output = run_admin(&arm_command(&body))?;
        let pid = parse_watcher_pid(&output)
            .ok_or_else(|| "관리자 감시자의 pid를 받지 못했습니다".to_string())?;
        let record = WatcherRecord {
            pid,
            watcher: state.watcher.clone(),
        };
        if let Err(error) = crate::accounts::atomic_write_existing_parent(
            &files.pid,
            watcher_record_text(&record).as_bytes(),
        ) {
            let cleanup = run_admin(&kill_watcher_command(&record, state.saved));
            return Err(match cleanup {
                Ok(_) => format!("감시자 pid 저장 실패: {error} — 감시자를 종료하고 복원했습니다"),
                Err(cleanup_error) => format!(
                    "감시자 pid 저장 실패: {error}; 감시자 정리도 실패했습니다: {cleanup_error}"
                ),
            });
        }
        Ok(record)
    }

    fn wait_for_watcher_end(record: &WatcherRecord, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if !watcher_alive_record(record) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        !watcher_alive_record(record)
    }

    fn restore_direct(saved: u8) -> Result<(), String> {
        run_admin(&format!("/usr/bin/pmset -a disablesleep {saved}"))?;
        let current = sleep_disabled_now()?;
        if current == saved {
            Ok(())
        } else {
            Err(format!(
                "SleepDisabled 복원 확인 실패 (현재 {current}, 원래 {saved})"
            ))
        }
    }

    fn remove_file(path: &Path) -> Result<(), String> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("파일 정리 실패 {}: {error}", path.display())),
        }
    }

    fn cleanup_legacy(files: &Files) -> Result<(), String> {
        remove_file(&files.legacy_json)?;
        remove_file(&files.legacy_mode)?;
        remove_file(&files.legacy_off)?;
        remove_file(&files.legacy_script)
    }

    fn cleanup_if_restored(files: &Files, expected: &State) -> Result<bool, String> {
        let current = sleep_disabled_now()?;
        if current != expected.saved {
            return Err(format!(
                "SleepDisabled 복원이 끝나지 않았습니다 (현재 {current}, 원래 {})",
                expected.saved
            ));
        }
        let Some(latest) = read_state(&files.state)? else {
            return Ok(false);
        };
        if latest.revision != expected.revision || latest.watcher != expected.watcher {
            return Ok(false);
        }
        // OPERATION_LOCK 아래에서 한 번 더 비교한 뒤 정리한다. 이전 monitor가 새 세대
        // 상태를 지우는 check-then-delete 경합을 막는다.
        let Some(latest) = read_state(&files.state)? else {
            return Ok(false);
        };
        if latest.revision != expected.revision || latest.watcher != expected.watcher {
            return Ok(false);
        }
        remove_file(&files.pid)?;
        remove_file(&files.state)?;
        cleanup_legacy(files)?;
        Ok(true)
    }

    fn spawn_state_monitor(
        app: &tauri::AppHandle,
        store: PathBuf,
        expected: State,
        record: WatcherRecord,
    ) {
        let generation = MONITOR_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
        let app = app.clone();
        std::thread::spawn(move || {
            loop {
                if MONITOR_SEQ.load(Ordering::SeqCst) != generation {
                    return;
                }
                if !watcher_alive_record(&record) {
                    break;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
            // 같은 감시자를 승격하며 새 monitor가 인계받았으면 옛 monitor는 정리하지 않는다.
            if MONITOR_SEQ.load(Ordering::SeqCst) != generation {
                return;
            }
            let Ok(_guard) = OPERATION_LOCK.lock() else {
                return;
            };
            let files = files(&store);
            match cleanup_if_restored(&files, &expected) {
                Ok(true) => {
                    let _ = app.emit("clamshell-changed", ());
                }
                Ok(false) => {}
                Err(error) => {
                    // root 감시자가 복원 전에 비정상 종료됐으면 no-sleep을 그대로
                    // 남기지 않는다. 관리자 복원을 한 번 시도하고 성공하면 off로 정리한다.
                    eprintln!("클램셸 감시자 비정상 종료 — 원상 복원을 시도합니다: {error}");
                    if restore_direct(expected.saved).is_ok()
                        && cleanup_if_restored(&files, &expected).unwrap_or(false)
                    {
                        let _ = app.emit("clamshell-changed", ());
                    }
                }
            }
        });
    }

    fn stop_state(files: &Files, state: &State) -> Result<(), String> {
        let stopping = transitioned(state, 0);
        let record = active_watcher(files);
        write_state(&files.state, &stopping)?;

        if let Some(record) = record.as_ref() {
            if !wait_for_watcher_end(record, Duration::from_secs(12)) {
                // bounded reader까지 끝나지 않는 비정상 감시자는 관리자 경로에서 강제
                // 종료·복원한다 (정체 재확인은 kill_watcher_command 안에서).
                run_admin(&kill_watcher_command(record, stopping.saved))?;
            }
        } else {
            std::thread::sleep(Duration::from_secs(2));
        }
        if sleep_disabled_now()? != stopping.saved {
            restore_direct(stopping.saved)?;
        }
        if !cleanup_if_restored(files, &stopping)? {
            return Err("클램셸 상태가 바뀌어 이전 해제 작업의 정리를 중단했습니다".into());
        }
        Ok(())
    }

    fn arm_new(
        app: &tauri::AppHandle,
        store: &Path,
        mode: u8,
        saved: u8,
    ) -> Result<i8, String> {
        let files = files(store);
        let state = fresh_state(mode, saved);
        write_state(&files.state, &state)?;
        let record = match start_watcher(&files, &state) {
            Ok(record) => record,
            Err(error) => {
                if sleep_disabled_now().ok() == Some(saved) {
                    // 승인 취소 등 pmset 실행 전 실패 — 상태 파일만 걷어내면 원상이다
                    let _ = cleanup_if_restored(&files, &state);
                    return Err(error);
                }
                // 승인은 됐는데 감시자만 실패한 경우 — 감시자 없는 no-sleep을 남기지
                // 않는다. 복원까지 실패하면 상태 파일을 남겨 재클릭·재시작 복구가
                // 이어받게 하고, 에러에 실상을 적는다.
                return Err(match restore_direct(saved) {
                    Ok(()) => {
                        let _ = cleanup_if_restored(&files, &state);
                        error
                    }
                    Err(restore_error) => format!(
                        "{error}; SleepDisabled 복원도 실패해 켜짐 상태가 남았습니다 — \
                         버튼을 다시 눌러 복원하세요 ({restore_error})"
                    ),
                });
            }
        };
        spawn_state_monitor(app, store.to_path_buf(), state, record);
        Ok(mode as i8)
    }

    pub fn cycle(app: &tauri::AppHandle, store: &Path) -> Result<i8, String> {
        // 시작 복구가 관리자 승인 창을 기다리는 동안 잠금 뒤에 줄 서지 않는다.
        // 먼저 빠르게 거절하고, 잠금 취득 사이에 복구가 시작되는 경우를 아래에서 재확인한다.
        if STARTUP_RECOVERY.load(Ordering::SeqCst) {
            return Err("이전 클램셸 상태를 복구하는 중입니다 — 잠시 후 다시 누르세요".into());
        }
        let _guard = OPERATION_LOCK.lock().map_err(|_| "클램셸 내부 잠금 오류")?;
        if STARTUP_RECOVERY.load(Ordering::SeqCst) {
            return Err("이전 클램셸 상태를 복구하는 중입니다 — 잠시 후 다시 누르세요".into());
        }
        std::fs::create_dir_all(store)
            .map_err(|error| format!("클램셸 상태 폴더 생성 실패: {error}"))?;
        let files = files(store);
        if files.legacy_json.exists() && !files.state.exists() {
            return Err("이전 클램셸 상태 복구가 필요합니다 — 위젯을 다시 실행하세요".into());
        }
        let state = read_state(&files.state)?;
        match state {
            None => {
                cleanup_legacy(&files)?;
                let saved = sleep_disabled_now()?;
                arm_new(app, store, 1, saved)
            }
            Some(state) if state.mode == 0 => {
                stop_state(&files, &state)?;
                let saved = sleep_disabled_now()?;
                arm_new(app, store, 1, saved)
            }
            Some(state) if state.mode == 1 => {
                let Some(record) = active_watcher(&files) else {
                    // 감시자가 없는 상태는 완료 직후 정리 전이거나, 켜기 실패 뒤 복구가
                    // 남은 경우다. 어느 쪽이든 이 클릭은 복원·해제만 하고 다시 켜지 않는다.
                    if sleep_disabled_now()? != state.saved {
                        restore_direct(state.saved)?;
                    }
                    cleanup_if_restored(&files, &state)?;
                    return Ok(0);
                };
                if sleep_disabled_now()? != 1 {
                    // 외부에서 SleepDisabled를 끈 상태 (예: 수동 sudo pmset). 승격 대신
                    // 안전하게 끈다 — 에러로 두면 덮개를 실제로 닫았다 열기 전까지
                    // 어떤 클릭도 통하지 않는 막다른 길이 된다.
                    stop_state(&files, &state)?;
                    return Ok(0);
                }
                let promoted = transitioned(&state, 2);
                write_state(&files.state, &promoted)?;
                std::thread::sleep(Duration::from_secs(2));
                if watcher_alive_record(&record) && sleep_disabled_now()? == 1 {
                    spawn_state_monitor(app, store.to_path_buf(), promoted, record);
                    return Ok(2);
                }

                // 덮개가 열리는 경계와 승격이 정확히 겹쳐 옛 감시자가 끝났다면 원래
                // 설정을 확인한 뒤 지속 감시자를 새 ID로 다시 건다.
                stop_state(&files, &promoted)?;
                arm_new(app, store, 2, state.saved)
            }
            Some(state) => {
                stop_state(&files, &state)?;
                Ok(0)
            }
        }
    }

    pub fn on_quit(_store: &Path) {}

    struct RecoveryFlag;

    impl Drop for RecoveryFlag {
        fn drop(&mut self) {
            STARTUP_RECOVERY.store(false, Ordering::SeqCst);
        }
    }

    fn legacy_saved(files: &Files) -> Result<(u8, u8), String> {
        let text = std::fs::read_to_string(&files.legacy_json)
            .map_err(|error| format!("이전 클램셸 상태 읽기 실패: {error}"))?;
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|error| format!("이전 클램셸 상태가 손상됐습니다: {error}"))?;
        let mode = value
            .get("mode")
            .and_then(|value| value.as_u64())
            .filter(|mode| (1..=2).contains(mode))
            .ok_or("이전 클램셸 mode가 없습니다")? as u8;
        let saved = value
            .get("saved")
            .and_then(|value| value.as_u64())
            .filter(|saved| *saved <= 1)
            .ok_or("이전 클램셸 원래 설정이 없습니다")? as u8;
        Ok((mode, saved))
    }

    fn legacy_watcher_alive(files: &Files) -> bool {
        let Some(pid) = std::fs::read_to_string(&files.pid)
            .ok()
            .and_then(|text| text.trim().parse::<i32>().ok())
            .filter(|pid| *pid > 1)
        else {
            return false;
        };
        let out = Command::new("/bin/ps")
            .args(["-p", &pid.to_string(), "-o", "uid=", "-o", "comm="])
            .output();
        let Ok(out) = out else { return false };
        let text = String::from_utf8_lossy(&out.stdout);
        let mut fields = text.split_whitespace();
        fields.next() == Some("0")
            && fields
                .next()
                .is_some_and(|command| command == "sh" || command.ends_with("/sh"))
    }

    fn recover_legacy(_app: &tauri::AppHandle, store: &Path) -> Result<(), String> {
        let files = files(store);
        // 구버전 잔재는 모드와 무관하게 복원·정리만 한다 — 재무장하지 않는다
        // (재부팅·재설치 후 지속 모드는 off로 시작, 사용자 결정 2026-08-12)
        let (_old_mode, saved) = legacy_saved(&files)?;
        std::fs::write(&files.legacy_off, b"off")
            .map_err(|error| format!("이전 감시자 해제 요청 실패: {error}"))?;
        let deadline = std::time::Instant::now() + Duration::from_secs(12);
        while legacy_watcher_alive(&files) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(250));
        }
        if sleep_disabled_now()? != saved {
            restore_direct(saved)?;
        }
        cleanup_legacy(&files)?;
        remove_file(&files.pid)?;
        Ok(())
    }

    fn recover_startup(
        app: tauri::AppHandle,
        store: PathBuf,
        expected_revision: Option<String>,
        has_legacy: bool,
    ) {
        let _recovery = RecoveryFlag;
        let Ok(_guard) = OPERATION_LOCK.lock() else {
            return;
        };
        let result = if has_legacy {
            recover_legacy(&app, &store)
        } else {
            let files = files(&store);
            match read_state(&files.state) {
                Ok(Some(state))
                    if expected_revision.as_deref() == Some(state.revision.as_str()) =>
                {
                    // 감시자가 죽어 있으면 모드와 무관하게 원상 복원 후 정리한다 —
                    // 재부팅 후에는 지속 모드도 재무장하지 않고 off로 시작한다
                    // (사용자 결정 2026-08-12). 앱 재시작 생존은 on_start의 살아 있는
                    // 감시자 입양이 담당하고, 여기는 감시자 소멸 = 해제 의미다.
                    stop_state(&files, &state)
                }
                Ok(_) => Ok(()),
                Err(error) => Err(error),
            }
        };
        match result {
            Ok(()) => {
                let _ = app.emit("clamshell-changed", ());
            }
            Err(error) => eprintln!(
                "클램셸 시작 상태 복구 실패 — 상태를 보존합니다: {error}"
            ),
        }
    }

    pub fn on_start(app: &tauri::AppHandle, store: &Path) {
        let files = files(store);
        let _ = remove_file(&files.legacy_script);
        let has_legacy = files.legacy_json.exists() && !files.state.exists();
        if has_legacy {
            if STARTUP_RECOVERY.swap(true, Ordering::SeqCst) {
                return;
            }
            let app = app.clone();
            let store = store.to_path_buf();
            std::thread::spawn(move || recover_startup(app, store, None, true));
            return;
        }

        let state = match read_state(&files.state) {
            Ok(Some(state)) => state,
            Ok(None) => {
                let _ = remove_file(&files.pid);
                let _ = cleanup_legacy(&files);
                return;
            }
            Err(error) => {
                eprintln!("클램셸 잔존 상태를 읽지 못했습니다: {error}");
                return;
            }
        };
        if let Some(record) = active_watcher(&files) {
            spawn_state_monitor(app, store.to_path_buf(), state, record);
            return;
        }
        if STARTUP_RECOVERY.swap(true, Ordering::SeqCst) {
            return;
        }
        let revision = state.revision.clone();
        let app = app.clone();
        let store = store.to_path_buf();
        std::thread::spawn(move || recover_startup(app, store, Some(revision), false));
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

        fn test_shell() -> Option<Command> {
            #[cfg(windows)]
            {
                let base = std::env::var_os("ProgramFiles")?;
                let path = PathBuf::from(base).join("Git").join("bin").join("sh.exe");
                return path.is_file().then(|| Command::new(path));
            }
            #[cfg(not(windows))]
            {
                Some(Command::new("/bin/sh"))
            }
        }

        #[test]
        fn state_is_one_strict_roundtrippable_record() {
            let state = State {
                mode: 2,
                saved: 0,
                revision: "rev-1".into(),
                watcher: "watch-1".into(),
            };
            assert_eq!(parse_state(&state_text(&state)).unwrap(), state);
            assert!(parse_state("mode=2\nsaved=0\n").is_err());
            assert!(parse_state("mode=9\nsaved=0\nrevision=r\nwatcher=w\n").is_err());
        }

        #[test]
        fn absent_sleep_disabled_is_normal_zero_but_broken_line_is_error() {
            assert_eq!(parse_sleep_disabled("System-wide power settings:\n").unwrap(), 0);
            assert_eq!(parse_sleep_disabled(" SleepDisabled 0\n").unwrap(), 0);
            assert_eq!(parse_sleep_disabled("SleepDisabled 1\n").unwrap(), 1);
            assert!(parse_sleep_disabled("SleepDisabled x\n").is_err());
        }

        #[test]
        fn root_watcher_has_bounded_reads_and_no_user_path_writes() {
            let store = test_store("body");
            let body = watch_body(&files(&store).state, 0, "watch-test");
            assert!(body.contains("head -c 512"));
            assert!(body.contains("SWITCHER_CLAMSHELL_WATCH=1"));
            assert!(body.contains("pmset -a disablesleep 0"));
            assert!(!body.contains("/bin/cat"));
            assert!(!body.contains("/bin/rm"));
            assert!(!body.contains("clamshell.pid"));
            // 관리자 컨텍스트에는 제어 터미널이 없어 nohup이 즉사한다 (실측 macOS 26.5)
            // — HUP 무시는 본문 trap이 맡는다
            assert!(body.contains("trap '' HUP"));
            assert!(
                !arm_command(&body).contains("nohup"),
                "nohup은 관리자 승인 컨텍스트에서 can't detach from console로 즉사한다"
            );
        }

        /// root kill은 반드시 argv 표식·토큰 재확인을 거쳐야 한다 — 승인 대기 동안
        /// 감시자가 죽고 pid가 재사용되면 무관한 프로세스를 죽이게 된다
        #[test]
        fn kill_command_verifies_watcher_identity_before_root_kill() {
            let record = WatcherRecord {
                pid: 4242,
                watcher: "watch-1".into(),
            };
            let cmd = kill_watcher_command(&record, 0);
            assert!(cmd.contains("/bin/ps"), "kill 전에 ps 정체 확인이 있어야 한다");
            assert!(cmd.contains("SWITCHER_CLAMSHELL_WATCH=1"));
            assert!(cmd.contains("watch-1"));
            assert!(cmd.contains("pmset -a disablesleep 0"));
            let ps_at = cmd.find("/bin/ps").unwrap();
            let kill_at = cmd.find("/bin/kill").unwrap();
            assert!(ps_at < kill_at, "정체 확인이 kill보다 먼저여야 한다");
        }

        #[test]
        fn body_and_arm_command_are_valid_sh() {
            let store = test_store("syntax");
            let body = watch_body(&files(&store).state, 0, "watch-test");
            let kill_cmd = kill_watcher_command(
                &WatcherRecord {
                    pid: 4242,
                    watcher: "watch-test".into(),
                },
                1,
            );
            for cmd in [body.clone(), arm_command(&body), kill_cmd] {
                let Some(mut shell) = test_shell() else { return };
                let out = shell.args(["-n", "-c", &cmd]).output().unwrap();
                assert!(
                    out.status.success(),
                    "sh 문법 오류: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
        }

        #[test]
        fn watcher_record_rejects_partial_or_invalid_values() {
            let record = WatcherRecord {
                pid: 1234,
                watcher: "watch-1".into(),
            };
            assert_eq!(parse_watcher_record(&watcher_record_text(&record)), Some(record));
            assert!(parse_watcher_record("pid=1\nwatcher=watch-1\n").is_none());
            assert!(parse_watcher_record("pid=1234\nwatcher=bad value\n").is_none());
        }

        #[test]
        fn shell_single_quote_wraps_and_escapes() {
            assert_eq!(sq("/a b/c"), "'/a b/c'");
            assert_eq!(sq("/a'b"), r"'/a'\''b'");
        }
    }
}

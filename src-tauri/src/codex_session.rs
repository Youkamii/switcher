//! Reload a managed Codex daemon after a subscription switch. The helper survives
//! the widget closing. It never hosts OAuth, forces a process down, opens a terminal,
//! or starts a daemon when none was running. Embedded/older clients remain separate.
use crate::accounts::{self, Env};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tungstenite::{Message, WebSocket};

#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(windows)]
use uds_windows::UnixStream;

const REQUEST: &str = "codex-session-request.json";
const STATUS: &str = "codex-session-status.json";
const LOCK: &str = "codex-session.lock";
const SOCKET_DIR: &str = "app-server-control";
const SOCKET: &str = "app-server-control.sock";
const IO_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Serialize, Deserialize)]
struct Request {
    generation: String,
    account_id: String,
    codex_home: PathBuf,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Status {
    generation: String,
    pub phase: String,
    pub reason: Option<String>,
}

fn read_json(path: &Path) -> Result<Value, &'static str> {
    let file = File::open(path).map_err(|_| "read_failed")?;
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "read_failed")?;
    if bytes.len() > 1024 * 1024 {
        return Err("invalid_data");
    }
    serde_json::from_slice(&bytes).map_err(|_| "invalid_data")
}

fn active_account(home: &Path) -> Option<String> {
    // Never refresh a file that has stopped being the CLI's credential store.
    let config = match std::fs::read_to_string(home.join("config.toml")) {
        Ok(text) => toml::from_str::<toml::Value>(&text).ok()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            toml::Value::Table(Default::default())
        }
        Err(_) => return None,
    };
    if config
        .get("cli_auth_credentials_store")
        .is_some_and(|v| v.as_str() != Some("file"))
    {
        return None;
    }
    let value = read_json(&home.join("auth.json")).ok()?;
    if value["OPENAI_API_KEY"]
        .as_str()
        .is_some_and(|s| !s.is_empty())
    {
        return None;
    }
    if value
        .get("auth_mode")
        .is_some_and(|v| v.as_str() != Some("chatgpt"))
    {
        return None;
    }
    value["tokens"]["account_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn request(store: &Path) -> Option<Request> {
    serde_json::from_value(read_json(&store.join(REQUEST)).ok()?).ok()
}

fn write_status(store: &Path, req: &Request, phase: &str, reason: Option<&str>) {
    // A slow helper must not relabel a later selection as complete.
    if request(store).is_none_or(|r| r.generation != req.generation) {
        return;
    }
    let state = Status {
        generation: req.generation.clone(),
        phase: phase.into(),
        reason: reason.map(str::to_owned),
    };
    if let Ok(bytes) = serde_json::to_vec(&state) {
        let _ = accounts::atomic_write(&store.join(STATUS), &bytes);
    }
}

pub(crate) fn status(env: &Env) -> Option<Status> {
    let req = request(&env.store)?;
    if req.codex_home != env.codex_home {
        return None;
    }
    if active_account(&env.codex_home).as_deref() != Some(&req.account_id) {
        return None;
    }
    let state: Status = serde_json::from_value(read_json(&env.store.join(STATUS)).ok()?).ok()?;
    (state.generation == req.generation).then_some(state)
}

/// Called only after the existing backup -> target-copy transaction succeeds.
pub(crate) fn schedule(env: &Env) {
    let Some(account_id) = active_account(&env.codex_home) else {
        return;
    };
    let req = Request {
        generation: format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ),
        account_id,
        codex_home: env.codex_home.clone(),
    };
    let Ok(bytes) = serde_json::to_vec(&req) else {
        return;
    };
    if accounts::atomic_write(&env.store.join(REQUEST), &bytes).is_err() {
        return;
    }
    if !env.codex_home.join(SOCKET_DIR).join(SOCKET).exists() {
        write_status(&env.store, &req, "next_launch", None);
        return;
    }
    write_status(&env.store, &req, "checking", None);
    #[cfg(not(test))]
    {
        let spawned = std::env::current_exe().ok().and_then(|exe| {
            #[cfg(windows)]
            let (exe, _copy_guard) = helper_copy(&exe, &env.store, &req.generation).ok()?;
            let mut cmd = Command::new(exe);
            cmd.arg("--codex-session-reload")
                .arg(&env.store)
                .arg(&env.codex_home)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            hide_window(&mut cmd);
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                cmd.process_group(0);
            }
            cmd.spawn().ok()
        });
        if let Some(mut child) = spawned {
            // Reap helpers while the widget remains open; dropping Child alone
            // leaves zombies on Unix. The helper still survives widget exit.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        } else {
            write_status(&env.store, &req, "failed", Some("helper_start"));
        }
    }
}

#[cfg(windows)]
fn helper_copy(source: &Path, store: &Path, generation: &str) -> Result<(PathBuf, File), String> {
    // Waiting for a long Codex turn must not lock the installed Switcher EXE and
    // prevent its updater from replacing it. Only retire our own unlocked copies.
    if let Ok(entries) = std::fs::read_dir(store) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("codex-session-helper-")
                && name.ends_with(".exe")
                && entry.file_type().is_ok_and(|t| t.is_file())
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let copy = store.join(format!("codex-session-helper-{generation}.exe"));
    crate::clamshell::create_helper_copy(source, &copy)?;
    let guard = crate::clamshell::open_verified_helper_copy(source, &copy)?;
    Ok((copy, guard))
}

fn hide_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = command;
}

fn codex(home: &Path, action: &str) -> Result<Value, &'static str> {
    // action is a private constant, never a profile name or user-supplied shell text.
    let mut args = vec![
        "app-server".to_owned(),
        "daemon".to_owned(),
        action.to_owned(),
    ];
    if action == "start" {
        // daemon start otherwise clears persisted feature overrides. Reuse them
        // exactly; changing another client's server features is outside this action.
        let settings = home.join("app-server-daemon/settings.json");
        if settings.exists() {
            let value = read_json(&settings)?;
            if let Some(features) = value.get("featureOverrides") {
                for (name, value) in features.as_object().ok_or("unsupported_config")? {
                    if name.is_empty()
                        || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                    {
                        return Err("unsupported_config");
                    }
                    args.push(
                        if value.as_bool().ok_or("unsupported_config")? {
                            "--enable"
                        } else {
                            "--disable"
                        }
                        .into(),
                    );
                    args.push(name.clone());
                }
            }
        }
    }
    let binary = managed_binary(home);
    #[cfg(windows)]
    let mut command = {
        if let Some(binary) = binary {
            let mut cmd = Command::new(binary);
            cmd.args(&args);
            cmd
        } else {
            let mut cmd = Command::new("cmd.exe");
            // All words are fixed commands or validated ASCII feature identifiers.
            cmd.args(["/d", "/s", "/c", &format!("codex {}", args.join(" "))]);
            cmd
        }
    };
    #[cfg(not(windows))]
    let mut command = {
        // The login shell can resolve an unrelated older wrapper. Use the exact
        // daemon package first, then the standalone installation and PATH.
        let binary = binary
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|p| p.join(".local/bin/codex"))
                    .filter(|p| p.is_file())
            })
            .unwrap_or_else(|| PathBuf::from("codex"));
        let mut cmd = Command::new(binary);
        cmd.args(&args);
        cmd
    };
    command
        .env("CODEX_HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(test)]
    for key in [
        "OPENAI_API_KEY",
        "CODEX_API_KEY",
        "OPENAI_BASE_URL",
        "CODEX_API_BASE_URL",
    ] {
        command.env_remove(key);
    }
    hide_window(&mut command);
    let mut child = command.spawn().map_err(|_| "cli_unavailable")?;
    let stdout = child.stdout.take().ok_or("cli_unavailable")?;
    let reader = std::thread::spawn(move || {
        let mut data = Vec::new();
        stdout.take(1024 * 1024).read_to_end(&mut data).ok();
        data
    });
    let until = Instant::now() + Duration::from_secs(45);
    loop {
        match child.try_wait() {
            Ok(Some(exit)) => {
                let bytes = reader.join().map_err(|_| "cli_failed")?;
                if !exit.success() {
                    return Err("cli_failed");
                }
                return serde_json::from_slice(&bytes).map_err(|_| "unsupported_cli");
            }
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                // Only our short-lived CLI command; never kill the daemon or its tree.
                let _ = child.kill();
                let _ = child.wait();
                return Err("cli_timeout");
            }
        }
    }
}

fn managed_binary(home: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    let name = "codex.exe";
    #[cfg(not(windows))]
    let name = "codex";
    ["app-server-daemon", "standalone"]
        .into_iter()
        .map(|package| {
            home.join("packages")
                .join(package)
                .join("current/bin")
                .join(name)
        })
        .find(|path| path.is_file())
}

struct Rpc {
    socket: WebSocket<UnixStream>,
    next_id: u64,
}

fn connect(path: &Path, route: &str) -> Result<WebSocket<UnixStream>, &'static str> {
    // Codex publishes a long-path symlink to its short, private socket on Unix.
    // std UnixStream rejects the long alias before the OS can follow it.
    #[cfg(unix)]
    let resolved = std::fs::canonicalize(path).map_err(|_| "socket_unavailable")?;
    #[cfg(unix)]
    let path = resolved.as_path();
    let stream = UnixStream::connect(path).map_err(|_| "socket_unavailable")?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|_| "socket_unavailable")?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|_| "socket_unavailable")?;
    let config = tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(1024 * 1024))
        .max_frame_size(Some(1024 * 1024));
    tungstenite::client::client_with_config(format!("ws://localhost/{route}"), stream, Some(config))
        .map(|(socket, _)| socket)
        .map_err(|_| "socket_unavailable")
}

impl Rpc {
    fn open(path: &Path) -> Result<Self, &'static str> {
        let mut rpc = Self {
            socket: connect(path, "")?,
            next_id: 0,
        };
        rpc.call("initialize", json!({"clientInfo":{"name":"switcher_session_reload","version":env!("CARGO_PKG_VERSION")}}))?;
        rpc.send(json!({"method":"initialized"}))?;
        Ok(rpc)
    }

    fn send(&mut self, value: Value) -> Result<(), &'static str> {
        self.socket
            .send(Message::Text(value.to_string().into()))
            .map_err(|_| "rpc_failed")
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, &'static str> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"id":id,"method":method,"params":params}))?;
        let until = Instant::now() + Duration::from_secs(10);
        while Instant::now() < until {
            let message = self.socket.read().map_err(|error| match error {
                tungstenite::Error::Io(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    "rpc_timeout"
                }
                _ => "rpc_failed",
            })?;
            if let Message::Text(text) = message {
                let value: Value = serde_json::from_str(&text).map_err(|_| "rpc_failed")?;
                // Ignore notifications and requests belonging to other clients. Never answer
                // approval/tool requests or print raw RPC errors, which can contain secrets.
                if value.get("method").is_none() && value["id"] == id {
                    return value.get("result").cloned().ok_or("rpc_rejected");
                }
            }
        }
        Err("rpc_timeout")
    }

    fn selected_auth(&mut self, home: &Path, expected: &str) -> Result<bool, &'static str> {
        let value = self.call(
            "getAuthStatus",
            json!({"includeToken":true,"refreshToken":false}),
        )?;
        // auth() may proactively rotate an expired credential. Compare against
        // the file after the RPC so a valid refresh is not reported as failure.
        let saved = read_json(&home.join("auth.json"))?;
        if saved["tokens"]["account_id"] != expected {
            return Err("selection_changed");
        }
        let stored = saved["tokens"]["access_token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("auth_unavailable")?;
        // Compare the credential itself: a selected workspace can differ from the
        // default workspace claim in the JWT. Never export either token to the UI.
        match value["authMethod"].as_str() {
            Some("chatgpt") => {}
            Some("apikey") => return Ok(false), // Explicit API -> subscription switch.
            _ => return Err("unsupported_auth"),
        }
        Ok(value["authToken"].as_str() == Some(stored))
    }

    fn idle(&mut self) -> Result<bool, &'static str> {
        let mut cursor = Value::Null;
        for _ in 0..100 {
            let value = self.call("thread/loaded/list", json!({"cursor":cursor,"limit":100}))?;
            for id in value["data"].as_array().ok_or("unsupported_cli")? {
                let thread =
                    self.call("thread/read", json!({"threadId":id,"includeTurns":false}))?;
                if !safe_to_reload(&thread["thread"]) {
                    return Ok(false);
                }
            }
            cursor = value["nextCursor"].clone();
            if cursor.is_null() {
                return Ok(true);
            }
        }
        Err("too_many_threads")
    }
}

fn safe_to_reload(thread: &Value) -> bool {
    // Ephemeral conversations have no saved history to reconnect to.
    thread["ephemeral"] == false && thread["status"]["type"] == "idle"
}

fn daemon_pid(home: &Path, info: &Value) -> Result<u32, &'static str> {
    let binary = Path::new(
        info["managedCodexPath"]
            .as_str()
            .ok_or("unmanaged_server")?,
    )
    .canonicalize()
    .map_err(|_| "unmanaged_server")?;
    let root = std::fs::canonicalize(home).map_err(|_| "unmanaged_server")?;
    let name = if binary.starts_with(root.join("packages/standalone")) {
        "app-server.pid"
    } else if binary.starts_with(root.join("packages/app-server-daemon")) {
        "daemon.pid"
    } else {
        return Err("unmanaged_server");
    };
    let record = read_json(&home.join("app-server-daemon").join(name))?;
    record["pid"]
        .as_u64()
        .and_then(|p| u32::try_from(p).ok())
        .filter(|p| *p > 1)
        .ok_or("unmanaged_server")
}

fn process_start(pid: u32) -> Option<u64> {
    let pid = sysinfo::Pid::from_u32(pid);
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        true,
        sysinfo::ProcessRefreshKind::nothing(),
    );
    sys.process(pid)
        .filter(|p| p.status() != sysinfo::ProcessStatus::Zombie)
        .map(|p| p.start_time())
}

fn graceful_stop(path: &Path, pid: u32) -> Result<(), &'static str> {
    #[cfg(unix)]
    {
        let _ = path;
        // SIGHUP is explicitly GracefulOnly in Codex, including repeated signals.
        if unsafe { libc::kill(pid as libc::pid_t, libc::SIGHUP) } != 0 {
            return Err("shutdown_failed");
        }
    }
    #[cfg(windows)]
    {
        let mut socket = connect(path, "daemon/shutdown")?;
        let pid = pid.to_string();
        socket
            .send(Message::Text(pid.clone().into()))
            .map_err(|_| "shutdown_failed")?;
        if !matches!(socket.read(), Ok(Message::Text(ack)) if ack == pid) {
            return Err("shutdown_failed");
        }
        let _ = socket.close(None);
    }
    Ok(())
}

fn supports_graceful_reload(version: &str) -> bool {
    let parts: Vec<_> = version.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|c| c.is_ascii_digit()))
    {
        return false;
    }
    match (
        parts[0].parse::<u64>(),
        parts[1].parse::<u64>(),
        parts[2].parse::<u64>(),
    ) {
        (Ok(major), Ok(minor), Ok(_)) => major > 0 || minor >= 167,
        _ => false,
    }
}

fn worker(store: &Path, home: &Path) -> Result<(), &'static str> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(store.join(LOCK))
        .map_err(|_| "lock_failed")?;
    // A second switch hands off to this helper; waiting helpers re-read the latest
    // request after acquiring the OS lock. No stale-PID lock files to repair.
    lock.lock_exclusive().map_err(|_| "lock_failed")?;
    let Some(mut req) = request(store) else {
        return Ok(());
    };
    if req.codex_home != home {
        return Ok(());
    }
    if read_json(&store.join(STATUS))
        .is_ok_and(|s| s["generation"] == req.generation && s["phase"] == "reloaded")
    {
        return Ok(());
    }
    let result = (|| {
        let info = codex(home, "version")?;
        if info["backend"] != "pid" {
            return Err("unmanaged_server");
        }
        if !info["appServerVersion"]
            .as_str()
            .is_some_and(supports_graceful_reload)
        {
            return Err("unsupported_cli");
        }
        let socket = home.join(SOCKET_DIR).join(SOCKET);
        let reported_socket = info["socketPath"].as_str().ok_or("unmanaged_server")?;
        if std::fs::canonicalize(reported_socket).map_err(|_| "unmanaged_server")?
            != std::fs::canonicalize(&socket).map_err(|_| "unmanaged_server")?
        {
            return Err("unmanaged_server");
        }
        let pid = daemon_pid(home, &info)?;
        let identity = process_start(pid).ok_or("daemon_exited")?;
        let until = Instant::now() + Duration::from_secs(30 * 60);
        loop {
            req = request(store).ok_or("selection_changed")?;
            if req.codex_home != home {
                return Err("selection_changed");
            }
            if active_account(home).as_deref() != Some(&req.account_id) {
                return Err("selection_changed");
            }
            let mut rpc = Rpc::open(&socket)?;
            if rpc.selected_auth(home, &req.account_id)? {
                write_status(store, &req, "reloaded", None);
                return Ok(());
            }
            if rpc.idle()? {
                break;
            }
            write_status(store, &req, "waiting", None);
            if Instant::now() >= until {
                return Err("busy_timeout");
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        if process_start(pid) != Some(identity) || daemon_pid(home, &info)? != pid {
            return Err("daemon_changed");
        }
        if request(store).is_none_or(|current| current.generation != req.generation)
            || active_account(home).as_deref() != Some(&req.account_id)
        {
            return Err("selection_changed");
        }
        write_status(store, &req, "reloading", None);
        graceful_stop(&socket, pid)?;
        // A turn admitted between idle-check and shutdown is drained by Codex.
        // Do not impose the CLI restart command's 60-second force-kill deadline.
        while process_start(pid) == Some(identity) {
            std::thread::sleep(Duration::from_millis(200));
        }
        // start is idempotent: another client may already have started a replacement.
        // A restart command here could race and stop that new daemon's work.
        // Persisted conversations are rejoined by the TUI's normal resume path.
        codex(home, "start")?;
        req = request(store).ok_or("selection_changed")?;
        let mut rpc = Rpc::open(&socket)?;
        if req.codex_home != home
            || active_account(home).as_deref() != Some(&req.account_id)
            || !rpc.selected_auth(home, &req.account_id)?
        {
            return Err("auth_mismatch");
        }
        write_status(store, &req, "reloaded", None);
        Ok(())
    })();
    if let Err(reason) = result {
        write_status(store, &req, "failed", Some(reason));
    }
    result
}

pub(crate) fn run_helper_if_requested() -> bool {
    let args: Vec<_> = std::env::args_os().collect();
    if args
        .get(1)
        .is_none_or(|arg| arg != "--codex-session-reload")
    {
        return false;
    }
    if args.len() == 4 {
        let store = PathBuf::from(&args[2]);
        let home = PathBuf::from(&args[3]);
        if store.is_absolute() && home.is_absolute() && store.is_dir() && home.is_dir() {
            let _ = worker(&store, &home);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use std::fs;
    use std::io::Write;

    #[test]
    fn only_persisted_idle_threads_are_safe_to_reload() {
        assert!(safe_to_reload(
            &json!({"ephemeral":false,"status":{"type":"idle"}})
        ));
        for value in [
            json!({"ephemeral":true,"status":{"type":"idle"}}),
            json!({"ephemeral":false,"status":{"type":"active","activeFlags":["waitingOnApproval"]}}),
            json!({"ephemeral":false,"status":{"type":"systemError"}}),
            json!({"status":{"type":"idle"}}),
        ] {
            assert!(!safe_to_reload(&value));
        }
    }

    #[test]
    fn never_signal_servers_without_the_supported_graceful_reload_contract() {
        for version in [
            "0.166.9",
            "0.144.1",
            "0.167.0-alpha.1",
            "",
            "unknown",
            "0.167",
        ] {
            assert!(!supports_graceful_reload(version));
        }
        for version in ["0.167.0", "0.169.0", "1.0.0"] {
            assert!(supports_graceful_reload(version));
        }
    }

    fn fixture_auth(name: &str) -> Vec<u8> {
        let claims = json!({"sub":format!("fixture-{name}"),"email":format!("{name}@example.test"),"exp":4102444800_u64,
            "https://api.openai.com/auth":{"chatgpt_account_id":format!("fixture-{name}"),"chatgpt_user_id":format!("user-{name}"),"chatgpt_plan_type":"plus"}});
        let token = format!(
            "e30.{}.synthetic-signature",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        serde_json::to_vec(&json!({"auth_mode":"chatgpt","OPENAI_API_KEY":null,
            "tokens":{"id_token":token,"access_token":token,"refresh_token":format!("synthetic-{name}"),"account_id":format!("fixture-{name}")},
            "last_refresh":chrono::Utc::now().to_rfc3339()})).unwrap()
    }

    struct Fixture {
        env: Env,
        daemon_started: bool,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "sr-{:x}-{:08x}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as u32
            ));
            fs::create_dir_all(&root).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            }
            let env = Env {
                home: root.clone(),
                store: root.join("store"),
                codex_home: root.join("c"),
                claude_live: accounts::ClaudeLiveStore::File(root.join("unused")),
            };
            fs::create_dir_all(&env.store).unwrap();
            fs::create_dir_all(env.codex_home.join("app-server-daemon")).unwrap();
            accounts::atomic_write(
                &env.codex_home.join("app-server-daemon/settings.json"),
                br#"{"updater":{"autoUpdateEnabled":false},"featureOverrides":{"shell_snapshot":false}}"#,
            )
            .unwrap();
            accounts::atomic_write(&env.codex_home.join("config.toml"), b"cli_auth_credentials_store = \"file\"\ncheck_for_update_on_startup = false\nchatgpt_base_url = \"http://127.0.0.1:9\"\n[model_providers.fixture]\nname = \"fixture\"\nbase_url = \"http://127.0.0.1:9/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = false\n").unwrap();
            Self {
                env,
                daemon_started: false,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if self.daemon_started {
                let _ = codex(&self.env.codex_home, "stop");
            }
            let _ = fs::remove_dir_all(&self.env.home);
        }
    }

    #[test]
    fn latest_selection_wins_and_no_daemon_is_started_for_file_only_switches() {
        let f = Fixture::new();
        accounts::atomic_write(&f.env.codex_home.join("auth.json"), &fixture_auth("alpha"))
            .unwrap();
        schedule(&f.env);
        let old = request(&f.env.store).unwrap();
        assert_eq!(status(&f.env).unwrap().phase, "next_launch");
        accounts::atomic_write(&f.env.codex_home.join("auth.json"), &fixture_auth("beta")).unwrap();
        schedule(&f.env);
        write_status(&f.env.store, &old, "reloaded", None);
        assert_eq!(status(&f.env).unwrap().phase, "next_launch");
        assert_eq!(request(&f.env.store).unwrap().account_id, "fixture-beta");
        accounts::atomic_write(
            &f.env.codex_home.join("auth.json"),
            br#"{"OPENAI_API_KEY":"synthetic-key"}"#,
        )
        .unwrap();
        assert!(status(&f.env).is_none());
        assert!(active_account(&f.env.codex_home).is_none());
    }

    #[test]
    #[ignore = "Installed Codex 0.167+, isolated home and synthetic credentials only"]
    fn installed_codex_daemon_reloads_selected_account_without_external_auth() {
        let mut f = Fixture::new();
        // One local, synthetic model response makes the conversation durable and
        // holds the turn open long enough to verify that refresh waits for it.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let (release, gate) = std::sync::mpsc::channel();
        let http = std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(40);
            loop {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        socket
                            .set_read_timeout(Some(Duration::from_secs(10)))
                            .unwrap();
                        let mut request = Vec::new();
                        let mut byte = [0];
                        while !request.ends_with(b"\r\n\r\n") && request.len() < 65536 {
                            if socket.read_exact(&mut byte).is_err() {
                                return;
                            }
                            request.push(byte[0]);
                        }
                        let headers = String::from_utf8_lossy(&request);
                        if !headers.starts_with("POST /v1/responses ") {
                            let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
                            continue;
                        }
                        let length = headers
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if length > 1024 * 1024 {
                            return;
                        }
                        let mut body = vec![0; length];
                        socket.read_exact(&mut body).unwrap();
                        if gate.recv_timeout(Duration::from_secs(30)).is_err() {
                            return;
                        }
                        let item = json!({"id":"fixture-message","type":"message","role":"assistant","content":[{"type":"output_text","text":"Fixture reply retained after refresh."}]});
                        let events = [
                            json!({"type":"response.created","response":{"id":"fixture-response"}}),
                            json!({"type":"response.output_item.done","output_index":0,"item":item}),
                            json!({"type":"response.completed","response":{"id":"fixture-response","status":"completed","output":[item],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}),
                        ];
                        let text = events
                            .into_iter()
                            .map(|v| {
                                format!("event: {}\ndata: {v}\n\n", v["type"].as_str().unwrap())
                            })
                            .collect::<String>();
                        let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",text.len());
                        let _ = socket.write_all(reply.as_bytes());
                        return;
                    }
                    Err(_) if Instant::now() < until => {
                        std::thread::sleep(Duration::from_millis(20))
                    }
                    Err(_) => return,
                }
            }
        });
        let config = fs::read_to_string(f.env.codex_home.join("config.toml"))
            .unwrap()
            .replace("http://127.0.0.1:9/v1", &format!("http://{address}/v1"));
        accounts::atomic_write(&f.env.codex_home.join("config.toml"), config.as_bytes()).unwrap();
        accounts::atomic_write(&f.env.codex_home.join("auth.json"), &fixture_auth("alpha"))
            .unwrap();
        let started = codex(&f.env.codex_home, "start").expect("isolated daemon must start");
        f.daemon_started = true;
        assert_eq!(started["backend"], "pid");
        let socket = f.env.codex_home.join(SOCKET_DIR).join(SOCKET);
        let mut running = Rpc::open(&socket).unwrap();
        assert!(running
            .selected_auth(&f.env.codex_home, "fixture-alpha")
            .unwrap());
        let thread = running.call("thread/start", json!({"model":"fixture-model","modelProvider":"fixture","cwd":f.env.home,"sandbox":"read-only","approvalPolicy":"never","experimentalRawEvents":false,"persistExtendedHistory":true})).expect("fixture thread must start");
        let thread_id = thread["thread"]["id"].as_str().unwrap().to_owned();
        running.call("turn/start", json!({"threadId":thread_id,"input":[{"type":"text","text":"Synthetic continuity check"}]})).expect("fixture turn must start");
        accounts::atomic_write(&f.env.codex_home.join("auth.json"), &fixture_auth("beta")).unwrap();
        assert!(!running
            .selected_auth(&f.env.codex_home, "fixture-beta")
            .unwrap());
        schedule(&f.env);
        let env = f.env.clone();
        let refreshing = std::thread::spawn(move || {
            // Optional built-artifact check runs the real early main() helper path,
            // without initializing a Tauri window or sharing the test's lifetime.
            if let Some(binary) = std::env::var_os("SWITCHER_SESSION_TEST_BINARY") {
                let mut command = Command::new(binary);
                for key in [
                    "OPENAI_API_KEY",
                    "CODEX_API_KEY",
                    "OPENAI_BASE_URL",
                    "CODEX_API_BASE_URL",
                ] {
                    command.env_remove(key);
                }
                command
                    .arg("--codex-session-reload")
                    .arg(&env.store)
                    .arg(&env.codex_home)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                hide_window(&mut command);
                let result = command.status().map_err(|_| "test_helper_failed")?;
                if !result.success() || status(&env).is_none_or(|s| s.phase != "reloaded") {
                    return Err("test_helper_failed");
                }
                Ok(())
            } else {
                worker(&env.store, &env.codex_home)
            }
        });
        let waiting_until = Instant::now() + Duration::from_secs(10);
        while status(&f.env).is_none_or(|s| s.phase != "waiting") && Instant::now() < waiting_until
        {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(
            status(&f.env).unwrap().phase,
            "waiting",
            "an active turn must never be interrupted"
        );
        assert!(!running
            .selected_auth(&f.env.codex_home, "fixture-beta")
            .unwrap());
        release.send(()).unwrap();
        http.join().unwrap();
        refreshing
            .join()
            .unwrap()
            .expect("isolated daemon reload must succeed");
        assert_eq!(status(&f.env).unwrap().phase, "reloaded");
        assert!(
            running
                .selected_auth(&f.env.codex_home, "fixture-beta")
                .is_err(),
            "previous transport must disconnect for TUI reconnection"
        );
        let mut reconnected = Rpc::open(&socket).unwrap();
        assert!(reconnected
            .selected_auth(&f.env.codex_home, "fixture-beta")
            .unwrap());
        let resumed = reconnected
            .call("thread/resume", json!({"threadId":thread_id}))
            .expect("existing thread must resume after reload");
        assert_eq!(resumed["thread"]["id"], thread_id);
        assert!(
            resumed["thread"]["turns"]
                .to_string()
                .contains("Fixture reply retained after refresh."),
            "saved conversation must survive reconnection"
        );
        let mode = reconnected
            .call(
                "getAuthStatus",
                json!({"includeToken":false,"refreshToken":false}),
            )
            .unwrap();
        assert_eq!(
            mode["authMethod"], "chatgpt",
            "Codex must retain managed OAuth refresh"
        );
        let auth = read_json(&f.env.codex_home.join("auth.json")).unwrap();
        assert_eq!(auth["tokens"]["refresh_token"], "synthetic-beta");
        assert_eq!(auth["auth_mode"], "chatgpt");
        let settings =
            read_json(&f.env.codex_home.join("app-server-daemon/settings.json")).unwrap();
        assert_eq!(
            settings["featureOverrides"]["shell_snapshot"], false,
            "server feature choices must survive reload"
        );
    }
}

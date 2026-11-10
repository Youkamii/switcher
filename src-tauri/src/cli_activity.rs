//! OAuth rotation has one owner while a provider CLI is running. A CLI may keep
//! the previous account in memory after Switcher changes the active files.
//! Process arguments are inspected only to recognize CLI launchers; never logged.
use crate::accounts::Provider;
use std::ffi::OsStr;
use std::sync::atomic::{AtomicU8, Ordering};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Activity {
    Idle,
    Running,
    Unknown,
}

fn filename(value: &str) -> &str {
    value.rsplit(['/', '\\']).next().unwrap_or(value)
}
fn cli_path(value: &str, provider: Provider) -> bool {
    let value = value.replace('\\', "/").to_ascii_lowercase();
    // Desktop shell processes have different credentials; their bundled CLI
    // children still match the regular executable/package checks below.
    if value.contains(".app/contents/macos/") {
        return false;
    }
    let base = filename(&value);
    match provider {
        Provider::Claude => {
            matches!(base, "claude" | "claude.exe")
                || value.contains("/claude/versions/")
                || value.ends_with("/@anthropic-ai/claude-code/cli.js")
        }
        Provider::Codex => {
            matches!(base, "codex" | "codex.exe") || value.ends_with("/@openai/codex/bin/codex.js")
        }
    }
}
fn inspect(
    name: &OsStr,
    exe: Option<&std::path::Path>,
    args: &[std::ffi::OsString],
    provider: Provider,
) -> Activity {
    if exe.is_some_and(|path| cli_path(&path.to_string_lossy(), provider))
        || args
            .first()
            .is_some_and(|arg| cli_path(&arg.to_string_lossy(), provider))
    {
        return Activity::Running;
    }
    let name = name.to_string_lossy().to_ascii_lowercase();
    if exe.is_none() && cli_path(&name, provider) {
        return Activity::Running;
    }
    if matches!(
        name.as_str(),
        "node" | "node.exe" | "bun" | "bun.exe" | "deno" | "deno.exe"
    ) {
        // Do not search arbitrary prompt strings for a provider name. Match the
        // actual package launcher path, stopping before script user arguments.
        let script = args
            .iter()
            .skip(1)
            .find(|arg| !arg.to_string_lossy().starts_with('-'));
        return match script {
            Some(path) if cli_path(&path.to_string_lossy(), provider) => Activity::Running,
            Some(_) => Activity::Idle,
            None => Activity::Unknown,
        };
    }
    Activity::Idle
}

pub(crate) fn detect(provider: Provider) -> Activity {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always),
    );
    if system
        .process(sysinfo::Pid::from_u32(std::process::id()))
        .is_none()
    {
        return Activity::Unknown;
    }
    let mut uncertain = false;
    for process in system.processes().values() {
        match inspect(process.name(), process.exe(), process.cmd(), provider) {
            Activity::Running => return Activity::Running,
            Activity::Unknown => uncertain = true,
            Activity::Idle => (),
        }
    }
    if uncertain {
        Activity::Unknown
    } else {
        Activity::Idle
    }
}

pub(crate) fn reason(activity: Activity) -> Option<&'static str> {
    match activity {
        Activity::Idle => None,
        Activity::Running => {
            Some("CLI 사용 중 · 토큰 충돌 방지를 위해 갱신 대기 — CLI 종료 후 자동 재시도")
        }
        Activity::Unknown => {
            Some("CLI 실행 상태를 확인할 수 없어 토큰 갱신을 보류했습니다 — 잠시 후 자동 재시도")
        }
    }
}
static DEFERRED: AtomicU8 = AtomicU8::new(0);
fn bit(provider: Provider) -> u8 {
    match provider {
        Provider::Claude => 1,
        Provider::Codex => 2,
    }
}
pub(crate) fn defer(provider: Provider) {
    DEFERRED.fetch_or(bit(provider), Ordering::Relaxed);
}
/// A native timer calls this only when a rotation was deferred. Page timers do
/// not participate, so inactive WKWebView cannot prevent automatic resumption.
pub(crate) fn take_ready() -> bool {
    take_ready_with(detect)
}
fn take_ready_with(probe: impl Fn(Provider) -> Activity) -> bool {
    let pending = DEFERRED.load(Ordering::Relaxed);
    if pending == 0 {
        return false;
    }
    let mut ready = 0;
    for provider in [Provider::Claude, Provider::Codex] {
        if pending & bit(provider) != 0 && probe(provider) == Activity::Idle {
            ready |= bit(provider);
        }
    }
    DEFERRED.fetch_and(!ready, Ordering::Relaxed);
    ready != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(name: &str, exe: Option<&str>, args: &[&str], provider: Provider) -> Activity {
        inspect(
            OsStr::new(name),
            exe.map(std::path::Path::new),
            &args
                .iter()
                .map(std::ffi::OsString::from)
                .collect::<Vec<_>>(),
            provider,
        )
    }
    #[test]
    fn recognizes_native_and_npm_clis_on_both_platforms() {
        for (provider, name, exe, args) in [
            (
                Provider::Claude,
                "claude",
                "/Users/example/.local/share/claude/versions/2.1.300",
                vec![],
            ),
            (
                Provider::Claude,
                "claude.exe",
                "C:\\Tools\\claude.exe",
                vec![],
            ),
            (
                Provider::Codex,
                "codex",
                "/Applications/Codex.app/Contents/Resources/codex",
                vec![],
            ),
            (Provider::Codex, "codex.exe", "C:\\Tools\\codex.exe", vec![]),
            (
                Provider::Claude,
                "node",
                "/usr/bin/node",
                vec![
                    "node",
                    "--trace-warnings",
                    "/tmp/node_modules/@anthropic-ai/claude-code/cli.js",
                ],
            ),
            (
                Provider::Codex,
                "node",
                "/usr/bin/node",
                vec!["node", "/tmp/node_modules/@openai/codex/bin/codex.js"],
            ),
        ] {
            assert_eq!(check(name, Some(exe), &args, provider), Activity::Running);
        }
        assert_eq!(
            check("codex.exe", None, &[], Provider::Codex),
            Activity::Running
        );
    }
    #[test]
    fn ignores_desktop_shells_unrelated_providers_and_prompt_contents() {
        assert_eq!(
            check(
                "Claude",
                Some("/Applications/Claude.app/Contents/MacOS/Claude"),
                &[],
                Provider::Claude
            ),
            Activity::Idle
        );
        assert_eq!(
            check("codex", Some("/bin/codex"), &[], Provider::Claude),
            Activity::Idle
        );
        assert_eq!(
            check(
                "node",
                Some("/bin/node"),
                &["node", "/app/server.js", "/tmp/@openai/codex/bin/codex.js"],
                Provider::Codex
            ),
            Activity::Idle
        );
        assert_eq!(
            check(
                "some-claude-monitor",
                Some("/bin/some-claude-monitor"),
                &[],
                Provider::Claude
            ),
            Activity::Idle
        );
    }
    #[test]
    fn unreadable_runtime_arguments_defer_instead_of_assuming_idle() {
        assert_eq!(
            check("node", Some("/bin/node"), &[], Provider::Codex),
            Activity::Unknown
        );
        assert!(reason(Activity::Unknown).is_some());
        assert!(reason(Activity::Running).unwrap().contains("자동 재시도"));
        assert!(reason(Activity::Idle).is_none());
    }
    #[test]
    fn resumption_waits_for_each_provider_and_unknown_does_not_clear_pending() {
        DEFERRED.store(0, Ordering::Relaxed);
        defer(Provider::Claude);
        defer(Provider::Codex);
        assert!(take_ready_with(|p| if p == Provider::Claude {
            Activity::Idle
        } else {
            Activity::Running
        }));
        assert_eq!(DEFERRED.load(Ordering::Relaxed), 2);
        assert!(!take_ready_with(|_| Activity::Unknown));
        assert_eq!(DEFERRED.load(Ordering::Relaxed), 2);
        assert!(take_ready_with(|_| Activity::Idle));
        assert!(!take_ready_with(|_| panic!(
            "no pending work should skip probing"
        )));
    }
    #[cfg(unix)]
    #[test]
    fn os_probe_recognizes_a_real_process_without_reading_any_credentials() {
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!("switcher-cli-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let executable = dir.join("claude");
        let _ = std::fs::remove_file(&executable);
        symlink("/bin/sleep", &executable).unwrap();
        let mut child = std::process::Command::new(&executable)
            .arg("10")
            .spawn()
            .unwrap();
        let pid = sysinfo::Pid::from_u32(child.id());
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing()
                .with_exe(UpdateKind::Always)
                .with_cmd(UpdateKind::Always),
        );
        let actual = system
            .process(pid)
            .map(|p| inspect(p.name(), p.exe(), p.cmd(), Provider::Claude));
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(actual, Some(Activity::Running));
    }
}

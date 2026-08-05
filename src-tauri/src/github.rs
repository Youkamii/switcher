//! GitHub 계정 전환 — gh CLI와 같은 통로(`gh auth switch`)를 쓴다.
//!
//! 클로드·코덱스와 달리 토큰 파일을 직접 만지지 않는다: gh가 계정별 토큰을
//! keyring(Windows 자격 증명 관리자·맥 키체인)에 이미 관리하므로, 위젯은
//! 목록 조회(`gh auth status`)와 활성 전환(`gh auth switch`)만 대행한다.
//! 전환 직후 `gh auth setup-git`을 함께 실행해 git push/pull(HTTPS)이
//! 활성 계정을 따라가게 한다 — GCM이 github.com 자격 증명을 따로 들고 있으면
//! 전환이 push에 안 먹는 문제의 방어이며, 반복 실행해도 안전(멱등)하다.
//!
//! 한계(README에 명시): SSH 리모트·커밋 author(user.name/email)·타 앱 세션은
//! 이 전환의 영향 밖이다.

use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct GithubAccount {
    pub login: String,
    pub active: bool,
}

#[derive(Serialize)]
pub struct GithubSnapshot {
    /// gh CLI 실행에 성공했는가 — false면 프론트가 설치 안내를 띄운다
    pub gh_found: bool,
    pub accounts: Vec<GithubAccount>,
}

/// gh 명령 골격. Windows는 콘솔 창을 띄우지 않고(CREATE_NO_WINDOW),
/// 맥 GUI 앱은 셸 PATH를 모르므로 로그인 셸로 경로를 해석한다.
fn gh_command() -> std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut cmd = std::process::Command::new("gh");
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new(crate::login::resolve_program("gh"))
    }
}

fn run_gh(args: &[&str]) -> Result<std::process::Output, String> {
    gh_command()
        .args(args)
        .output()
        .map_err(|e| format!("gh 실행 실패: {e}"))
}

/// gh에 로그인된 github.com 계정 목록. gh가 없으면 gh_found=false.
/// 미로그인 상태의 gh는 비0 종료라도 안내 텍스트를 주므로 종료 코드는 보지 않는다.
pub fn list() -> GithubSnapshot {
    let output = match run_gh(&["auth", "status"]) {
        Ok(out) => out,
        Err(_) => {
            return GithubSnapshot {
                gh_found: false,
                accounts: Vec::new(),
            }
        }
    };
    // gh 버전에 따라 stdout/stderr 어느 쪽에 쓰는지가 다르다 — 둘 다 본다
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    GithubSnapshot {
        gh_found: true,
        accounts: parse_auth_status(&text),
    }
}

/// `gh auth status` 출력 파싱 (실측 gh 2.83 형식):
/// ```text
/// github.com
///   ✓ Logged in to github.com account Youkamii (keyring)
///   - Active account: true
/// ```
/// github.com 계정만 취한다 (엔터프라이즈 호스트는 v1 범위 밖).
fn parse_auth_status(text: &str) -> Vec<GithubAccount> {
    let mut accounts: Vec<GithubAccount> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.split("Logged in to ").nth(1) {
            if !rest.starts_with("github.com ") {
                continue;
            }
            if let Some(after) = rest.split(" account ").nth(1) {
                let login = after.split_whitespace().next().unwrap_or("");
                if !login.is_empty() {
                    accounts.push(GithubAccount {
                        login: login.to_string(),
                        active: false,
                    });
                }
            }
        } else if line.contains("Active account: true") {
            // 활성 플래그는 직전에 나온 계정 블록에 속한다
            if let Some(last) = accounts.last_mut() {
                last.active = true;
            }
        }
    }
    accounts
}

/// 활성 계정 전환 + git 자격 증명 연동.
pub fn switch(login: &str) -> Result<(), String> {
    // 셸을 거치지 않아 인젝션 경로는 없지만, 파싱이 이상한 값을 물어오는 사고 방어
    // (GitHub 로그인 규칙: 영숫자·하이픈)
    if login.is_empty()
        || login.starts_with('-')
        || !login.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(format!("잘못된 GitHub 계정 이름: {login}"));
    }
    let out = run_gh(&["auth", "switch", "--hostname", "github.com", "--user", login])?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("GitHub 계정 전환 실패: {}", err.trim()));
    }
    // git push/pull(HTTPS)이 gh의 활성 계정을 따라가게 연결한다
    let setup = run_gh(&["auth", "setup-git", "--hostname", "github.com"])?;
    if !setup.status.success() {
        let err = String::from_utf8_lossy(&setup.stderr);
        return Err(format!("git 연동(setup-git) 실패: {}", err.trim()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_active_account() {
        let text = "github.com\n  ✓ Logged in to github.com account Youkamii (keyring)\n  - Active account: true\n  - Git operations protocol: https\n  - Token: gho_************\n";
        let accounts = parse_auth_status(text);
        assert_eq!(
            accounts,
            vec![GithubAccount {
                login: "Youkamii".into(),
                active: true
            }]
        );
    }

    #[test]
    fn parses_two_accounts_one_active() {
        let text = "github.com\n  ✓ Logged in to github.com account alice (keyring)\n  - Active account: true\n  - Token: gho_****\n\n  ✓ Logged in to github.com account bob-2 (keyring)\n  - Active account: false\n";
        let accounts = parse_auth_status(text);
        assert_eq!(accounts.len(), 2);
        assert!(accounts[0].active && accounts[0].login == "alice");
        assert!(!accounts[1].active && accounts[1].login == "bob-2");
    }

    #[test]
    fn ignores_enterprise_hosts_and_not_logged_in() {
        let enterprise = "ghe.example.com\n  ✓ Logged in to ghe.example.com account carol (keyring)\n  - Active account: true\n";
        assert!(parse_auth_status(enterprise).is_empty());
        let none = "You are not logged into any GitHub hosts. To log in, run: gh auth login\n";
        assert!(parse_auth_status(none).is_empty());
    }

    /// 실기기 전용: gh가 설치·로그인된 환경에서 목록이 나오는지
    /// (`cargo test -- --ignored real_`)
    #[test]
    #[ignore]
    fn real_github_list_shows_accounts() {
        let snap = list();
        assert!(snap.gh_found, "gh CLI를 찾지 못했다");
        assert!(!snap.accounts.is_empty(), "로그인된 계정이 없다");
        assert_eq!(
            snap.accounts.iter().filter(|a| a.active).count(),
            1,
            "활성 계정은 정확히 하나여야 한다"
        );
    }

    #[test]
    fn switch_rejects_suspicious_names() {
        assert!(switch("").is_err());
        assert!(switch("a b").is_err());
        assert!(switch("x;rm").is_err());
        assert!(switch("--flag").is_err());
    }
}

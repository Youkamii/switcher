//! The currently selected Anthropic SDK profile, shared by "ant" and Claude
//! Code Console sign-in. Leave rotation, other profiles and federation to them.
use super::*;
use std::{fs, io::Read, time::SystemTime};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Profile {
    credentials: PathBuf,
    modified: SystemTime,
}

fn json_file<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 64 * 1024 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

pub(super) fn active(dir: &Path, org: Uuid) -> Option<Profile> {
    let name = std::env::var("ANTHROPIC_PROFILE")
        .ok()
        .or_else(|| fs::read_to_string(dir.join("active_config")).ok());
    let name = name.as_deref().unwrap_or("default").trim();
    if name.is_empty()
        || name.len() > 128
        || matches!(name, "." | "..")
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    {
        return None;
    }
    let config: Value = json_file(&dir.join("configs").join(format!("{name}.json")))?;
    if config
        .get("organization_id")?
        .as_str()?
        .parse::<Uuid>()
        .ok()?
        != org
        || config.pointer("/authentication/type")?.as_str()? != "user_oauth"
        || config
            .get("base_url")
            .and_then(Value::as_str)
            .is_some_and(|url| {
                !matches!(url.trim_end_matches('/'), "" | "https://api.anthropic.com")
            })
    {
        return None;
    }
    let credentials = dir.join("credentials").join(format!("{name}.json"));
    if config
        .pointer("/authentication/credentials_path")
        .and_then(Value::as_str)
        .is_some_and(|path| !path.is_empty() && Path::new(path) != credentials)
    {
        return None;
    }
    Some(Profile {
        modified: fs::metadata(&credentials).ok()?.modified().ok()?,
        credentials,
    })
}

impl Profile {
    pub(super) async fn credit(&self, api: &Api<'_>, expected: Uuid) -> Result<Credit, Issue> {
        #[derive(Deserialize)]
        struct Token {
            #[serde(rename = "type")]
            kind: String,
            access_token: String,
            expires_at: Option<i64>,
            organization_uuid: String,
        }
        let token: Token = json_file(&self.credentials).ok_or(Issue::LoginExpired)?;
        if token.kind != "oauth_token"
            || token.organization_uuid.parse::<Uuid>().ok() != Some(expected)
        {
            return Err(Issue::LoginRequired);
        }
        if token.access_token.is_empty() || token.expires_at.is_some_and(|at| at <= now() as i64) {
            return Err(Issue::LoginExpired);
        }
        // Use the organization issued to this bearer by the authenticated self
        // endpoint. The locally configured target alone never grants access.
        let issuer = api.organization(Auth::Console(&token.access_token)).await?;
        if issuer.id != expected {
            return Err(Issue::LoginRequired);
        }
        api.prepaid(&token.access_token, issuer.id).await
    }
}

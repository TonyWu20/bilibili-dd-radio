//! Login state: credentials on disk and the cookie header they build.
//!
//! Credentials live in `~/.config/bilibili-dd-radio/credentials.toml`
//! with `0600` permissions. The file is written only after a login
//! succeeds.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use bili_api::UID;

/// A bilibili web login session.
///
/// The field names match the cookie names.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Credentials {
    /// The `SESSDATA` cookie.
    pub sessdata: String,
    /// The `bili_jct` cookie.
    pub bili_jct: String,
    /// The user id, from the `DedeUserID` cookie.
    pub dede_user_id: String,
    /// The `DedeUserID__ckMd5` cookie, when set.
    #[serde(default)]
    pub dede_user_id_ckmd5: Option<String>,
    /// The `buvid3` fingerprint cookie, when set.
    #[serde(default)]
    pub buvid3: Option<String>,
    /// The `buvid4` fingerprint cookie, when set.
    #[serde(default)]
    pub buvid4: Option<String>,
    /// The passport refresh token, when the login gave one.
    #[serde(default)]
    pub refresh_token: Option<String>,
}

impl Credentials {
    /// The `Cookie` header value for the bilibili API.
    pub fn cookie_string(&self) -> String {
        let mut s = format!(
            "SESSDATA={}; bili_jct={}; DedeUserID={}",
            self.sessdata, self.bili_jct, self.dede_user_id
        );
        if let Some(v) = &self.dede_user_id_ckmd5 {
            s.push_str(&format!("; DedeUserID__ckMd5={v}"));
        }
        if let Some(v) = &self.buvid3 {
            s.push_str(&format!("; buvid3={v}"));
        }
        if let Some(v) = &self.buvid4 {
            s.push_str(&format!("; buvid4={v}"));
        }
        s
    }

    /// The user id, when the `DedeUserID` value parses as a number.
    pub fn uid(&self) -> Option<UID> {
        self.dede_user_id.parse::<u64>().ok().map(UID)
    }

    /// Build credentials from a set of `name=value` cookies.
    ///
    /// Requires `SESSDATA`, `bili_jct`, and `DedeUserID`.
    pub fn from_cookies(cookies: &[(String, String)]) -> Option<Self> {
        let get = |name: &str| {
            cookies
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
        };
        Some(Self {
            sessdata: get("SESSDATA")?,
            bili_jct: get("bili_jct")?,
            dede_user_id: get("DedeUserID")?,
            dede_user_id_ckmd5: get("DedeUserID__ckMd5"),
            buvid3: get("buvid3"),
            buvid4: get("buvid4"),
            refresh_token: None,
        })
    }
}

/// The credentials file path.
///
/// `~/.config/bilibili-dd-radio/credentials.toml`, or the platform
/// config directory when the platform differs.
pub fn credentials_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".config"));
    base.join("bilibili-dd-radio").join("credentials.toml")
}

/// Load the saved credentials. `Ok(None)` when no file exists.
pub fn load() -> anyhow::Result<Option<Credentials>> {
    match fs::read_to_string(credentials_path()) {
        Ok(text) => Ok(Some(toml::from_str(&text)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::anyhow!("read credentials: {e}")),
    }
}

/// Save credentials. The file is created with `0600` permissions.
pub fn save(creds: &Credentials) -> anyhow::Result<()> {
    let path = credentials_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let text = toml::to_string_pretty(creds)?;
    let mut file = open_private(&path)?;
    file.write_all(text.as_bytes())?;
    Ok(())
}

/// Remove the saved credentials. Missing file is not an error.
pub fn clear() -> anyhow::Result<()> {
    match fs::remove_file(credentials_path()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::anyhow!("remove credentials: {e}")),
    }
}

/// Open the credentials file for writing.
///
/// On Unix the file gets `0600` so the session never lands in a
/// group-readable file.
#[cfg(unix)]
fn open_private(path: &std::path::Path) -> anyhow::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    Ok(file)
}

#[cfg(not(unix))]
fn open_private(path: &std::path::Path) -> anyhow::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .map_err(Into::into)
}

/// Parse a pasted cookie string, e.g. `SESSDATA=…; bili_jct=…`.
pub fn parse_cookie_string(raw: &str) -> anyhow::Result<Credentials> {
    let mut cookies: Vec<(String, String)> = Vec::new();
    for part in raw.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((n, v)) = part.split_once('=') else {
            anyhow::bail!("cookie entry without '=': {part}");
        };
        cookies.push((n.trim().to_string(), v.trim().to_string()));
    }
    Credentials::from_cookies(&cookies).ok_or_else(|| {
        anyhow::anyhow!("cookie string needs SESSDATA, bili_jct, and DedeUserID entries")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Credentials {
        Credentials {
            sessdata: "sess".into(),
            bili_jct: "jct".into(),
            dede_user_id: "1234".into(),
            dede_user_id_ckmd5: Some("md5".into()),
            buvid3: Some("b3".into()),
            buvid4: Some("b4".into()),
            refresh_token: None,
        }
    }

    #[test]
    fn cookie_string_covers_all_cookies() {
        let c = sample();
        assert_eq!(
            c.cookie_string(),
            "SESSDATA=sess; bili_jct=jct; DedeUserID=1234; \
             DedeUserID__ckMd5=md5; buvid3=b3; buvid4=b4"
        );
        assert_eq!(c.uid(), Some(UID(1234)));
    }

    #[test]
    fn from_cookies_requires_the_three_core() {
        let ok = vec![
            ("SESSDATA".into(), "s".into()),
            ("bili_jct".into(), "j".into()),
            ("DedeUserID".into(), "9".into()),
        ];
        assert!(Credentials::from_cookies(&ok).is_some());
        let missing: Vec<(String, String)> = vec![("SESSDATA".into(), "s".into())];
        assert!(Credentials::from_cookies(&missing).is_none());
    }

    #[test]
    fn parse_cookie_string_round_trip() {
        let c = sample();
        let back = parse_cookie_string(&c.cookie_string()).unwrap();
        assert_eq!(back.sessdata, "sess");
        assert_eq!(back.dede_user_id, "1234");
        assert_eq!(back.buvid3.as_deref(), Some("b3"));
    }

    #[test]
    fn parse_cookie_string_rejects_garbage() {
        assert!(parse_cookie_string("").is_err());
        assert!(parse_cookie_string("SESSDATA=only-one").is_err());
        assert!(parse_cookie_string("no-equals-sign").is_err());
    }

    #[test]
    fn credentials_round_trip_through_toml() {
        let c = sample();
        let text = toml::to_string_pretty(&c).unwrap();
        let back: Credentials = toml::from_str(&text).unwrap();
        assert_eq!(back.buvid4.as_deref(), Some("b4"));
    }
}

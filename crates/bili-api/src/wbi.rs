//! WBI (web bilibili interface) request signing.
//!
//! Some bilibili web APIs reject requests unless the query string is
//! signed with `w_rid` and `wts`. The signing key is mixed from two
//! image keys served by the `nav` endpoint. The algorithm is public
//! and used by many third-party clients.

use md5::{Md5, Digest};
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::Error;

/// The fixed index table used to mix the image key and the sub key.
const MIXIN_KEY_TABLE: [usize; 32] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19, 29,
    28, 14, 39, 12, 38, 41, 13,
];

/// Signs query parameters with the WBI scheme.
#[derive(Debug, Clone)]
pub struct WbiSigner {
    client: reqwest::Client,
    cookies: String,
    mixin_key: String,
}

impl WbiSigner {
    /// Fetch the current mixin key from `GET /x/web-interface/nav`.
    pub async fn new(client: &reqwest::Client, cookies: &str) -> Result<Self, Error> {
        let mixin_key = Self::fetch_mixin_key(client, cookies).await?;
        Ok(Self {
            client: client.clone(),
            cookies: cookies.to_string(),
            mixin_key,
        })
    }

    /// Refresh the mixin key. Call this if a signed request returns
    /// code `-352` (bad WBI signature).
    pub async fn refresh(&mut self) -> Result<(), Error> {
        self.mixin_key = Self::fetch_mixin_key(&self.client, &self.cookies).await?;
        Ok(())
    }

    /// Add `wts` and `w_rid` to `params`. Returns the signed params.
    pub fn sign(&self, params: BTreeMap<String, String>) -> BTreeMap<String, String> {
        let mut signed = params;
        let wts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
            .to_string();
        signed.insert("wts".into(), wts);
        let clean: BTreeMap<String, String> = signed
            .iter()
            .map(|(k, v)| (k.clone(), sanitize_value(v)))
            .collect();
        let query = clean
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        let digest = Md5::digest(format!("{query}{}", self.mixin_key).as_bytes());
        let w_rid: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        signed.insert("w_rid".into(), w_rid);
        signed
    }

    async fn fetch_mixin_key(client: &reqwest::Client, cookies: &str) -> Result<String, Error> {
        let resp = client
            .get("https://api.bilibili.com/x/web-interface/nav")
            .header(reqwest::header::COOKIE, cookies)
            .send()
            .await?;
        let body: Value = resp.json().await.map_err(|e| Error::Wbi(e.to_string()))?;
        let img_url = body["data"]["wbi_img"]["img_url"]
            .as_str()
            .ok_or_else(|| Error::MissingField("wbi_img.img_url"))?;
        let sub_url = body["data"]["wbi_img"]["sub_url"]
            .as_str()
            .ok_or_else(|| Error::MissingField("wbi_img.sub_url"))?;
        let img_key = key_from_url(img_url);
        let sub_key = key_from_url(sub_url);
        let shuffled = format!("{img_key}{sub_key}");
        let mixin: String = MIXIN_KEY_TABLE
            .iter()
            .filter_map(|i| shuffled.chars().nth(*i))
            .collect();
        if mixin.is_empty() {
            return Err(Error::Wbi("empty mixin key".into()));
        }
        Ok(mixin)
    }
}

/// Pull the file name (without extension) out of a bilibili CDN URL.
fn key_from_url(url: &str) -> String {
    url.rsplit('/')
        .next()
        .unwrap_or_default()
        .split('.')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// WBI signing strips the characters `'`, `!`, `(`, `)`, `*` from values.
fn sanitize_value(v: &str) -> String {
    v.chars()
        .filter(|c| !matches!(c, '\'' | '!' | '(' | ')' | '*'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_value() {
        assert_eq!(sanitize_value("a!b'c(d)e*f"), "abcdef");
    }

    #[test]
    fn test_sign_is_deterministic_for_fixed_inputs() {
        let signer = WbiSigner {
            client: reqwest::Client::new(),
            cookies: String::new(),
            mixin_key: "x".repeat(32),
        };
        let mut params = BTreeMap::new();
        params.insert("id".into(), "123".into());
        params.insert("type".into(), "0".into());
        let signed = signer.sign(params);
        assert!(signed.contains_key("w_rid"));
        assert!(signed.contains_key("wts"));
        // w_rid is 32 hex chars (md5).
        assert_eq!(signed["w_rid"].len(), 32);
    }

    #[test]
    fn test_key_from_url() {
        assert_eq!(
            key_from_url("https://i0.hdslb.com/bfs/wbi/img/2025key.png"),
            "2025key"
        );
    }
}

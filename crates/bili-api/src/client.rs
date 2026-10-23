//! The bilibili HTTP API client.

use std::collections::BTreeMap;
use std::sync::Arc;

use bon::Builder;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::auth::{Qrcode, QrcodePoll, QrcodeStatus};
use crate::error::Error;
use crate::types::{
    DanmuServerConf, LiveHome, LiveRoomEntry, PlayUrl, RoomID, RoomInfo, UID, UpInfo,
};
use crate::wbi::WbiSigner;

/// A configured bilibili API client.
///
/// Hold one client per process and share it between tasks. All methods
/// are cheap to call concurrently.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    cookie: String,
    buvid3: Arc<RwLock<Option<String>>>,
    buvid4: Arc<RwLock<Option<String>>>,
    wbi: Arc<RwLock<Option<WbiSigner>>>,
    user_agent: &'static str,
}

/// The builder inputs for [`Client`].
///
/// ```
/// use bili_api::{Client, ClientConfig};
///
/// let config = ClientConfig::builder()
///     .cookie("SESSDATA=...; bili_jct=...")
///     .build();
/// let client = Client::from_config(config);
/// ```
#[derive(Debug, Builder)]
pub struct ClientConfig {
    /// The raw `Cookie` header value. May be empty for anonymous use.
    #[builder(default, into)]
    pub cookie: String,
    /// The `buvid3` cookie value. When absent, the client fetches a
    /// fingerprint from the `spi` endpoint on first use.
    pub buvid3: Option<String>,
    /// The `buvid4` cookie value.
    pub buvid4: Option<String>,
}

/// The quality tier requested from `playUrl`.
///
/// The server clamps to the lowest quality the room actually offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    /// Lowest quality, to save bandwidth. Default.
    Lowest,
}

impl Client {
    /// Start the builder: `Client::builder().cookie(..).build()`.
    ///
    /// The builder produces a [`ClientConfig`]. Call
    /// [`Client::from_config`] on it, or use [`Client::new`]:
    ///
    /// ```
    /// let client = bili_api::Client::new();
    /// ```
    pub fn new() -> Self {
        ClientConfig::builder().build().into()
    }

    pub fn from_config(config: ClientConfig) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(Self::USER_AGENT)
            .build()
            .expect("reqwest client build");
        Self {
            http,
            cookie: config.cookie,
            buvid3: Arc::new(RwLock::new(config.buvid3)),
            buvid4: Arc::new(RwLock::new(config.buvid4)),
            wbi: Arc::new(RwLock::new(None)),
            user_agent: Self::USER_AGENT,
        }
    }
}

impl From<ClientConfig> for Client {
    fn from(config: ClientConfig) -> Self {
        Self::from_config(config)
    }
}

impl ClientConfig {
    /// A config for a logged-in session: the cookie header plus the
    /// fingerprint cookies, when the login gave them.
    pub fn from_login(cookie: String, buvid3: Option<String>, buvid4: Option<String>) -> Self {
        match (buvid3, buvid4) {
            (Some(b3), Some(b4)) => Self::builder().cookie(cookie).buvid3(b3).buvid4(b4).build(),
            (Some(b3), None) => Self::builder().cookie(cookie).buvid3(b3).build(),
            (None, Some(b4)) => Self::builder().cookie(cookie).buvid4(b4).build(),
            (None, None) => Self::builder().cookie(cookie).build(),
        }
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    const USER_AGENT: &'static str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120 Safari/537.36";

    /// The raw cookie header value.
    pub fn cookie(&self) -> &str {
        &self.cookie
    }
    /// The `buvid3` value, when known.
    pub async fn buvid3(&self) -> Option<String> {
        self.buvid3.read().await.clone()
    }

    /// A WBI signer for this client, built lazily and cached.
    pub async fn wbi_signer(&self) -> Result<WbiSigner, Error> {
        {
            let guard = self.wbi.read().await;
            if let Some(signer) = &*guard {
                return Ok(signer.clone());
            }
        }
        let mut guard = self.wbi.write().await;
        if let Some(signer) = &*guard {
            return Ok(signer.clone());
        }
        let signer = WbiSigner::new(&self.http, &self.cookie).await?;
        *guard = Some(signer.clone());
        Ok(signer)
    }

    /// Resolve a room id candidate (short or real) to full room info.
    pub async fn room_info(&self, candidate: u64) -> Result<RoomInfo, Error> {
        let mut params = BTreeMap::new();
        params.insert("room_id".to_string(), candidate.to_string());
        let data: Value = self
            .get_api(
                "https://api.live.bilibili.com/room/v1/Room/get_info",
                &params,
            )
            .await?;
        let info: RoomInfo =
            serde_json::from_value(data).map_err(|e| Error::UnexpectedResponse(e.to_string()))?;
        Ok(info)
    }

    /// Fetch a playable stream URL for a room.
    ///
    /// Pass [`Quality::Lowest`] to request the lowest quality tier.
    /// The server clamps to what the room offers.
    pub async fn play_url(&self, room_id: RoomID, quality: Quality) -> Result<PlayUrl, Error> {
        let mut params = BTreeMap::new();
        params.insert("cid".to_string(), room_id.get().to_string());
        params.insert("platform".to_string(), "web".into());
        params.insert("protos".to_string(), "http-flv,http-m3u8".into());
        match quality {
            Quality::Lowest => {
                params.insert("qn".to_string(), "80".into());
            }
        }
        let data: Value = self
            .get_api(
                "https://api.live.bilibili.com/room/v1/Room/playUrl",
                &params,
            )
            .await?;
        let urls: Vec<Value> = data["durl"]
            .as_array()
            .cloned()
            .ok_or(Error::MissingField("durl"))?;
        if urls.is_empty() {
            return Err(Error::MissingField("durl[0]"));
        }
        // `durl` entries carry an `order` field. Pick the lowest order.
        let mut entries = urls;
        entries.sort_by_key(|u| u["order"].as_u64().unwrap_or(u64::MAX));
        let url = entries[0]["url"]
            .as_str()
            .ok_or(Error::MissingField("durl[0].url"))?
            .to_string();
        Ok(PlayUrl { url })
    }

    /// Fetch the danmaku WebSocket server list and token for a room.
    pub async fn danmu_server_conf(&self, room_id: RoomID) -> Result<DanmuServerConf, Error> {
        let signer = self.wbi_signer().await?;
        let mut params = BTreeMap::new();
        params.insert("id".to_string(), room_id.get().to_string());
        params.insert("type".to_string(), "0".into());
        let signed = signer.sign(params);
        let query = signed
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        let data: Value = self
            .get_api_raw(&format!(
                "https://api.live.bilibili.com/xlive/web-room/v1/index/getDanmuInfo?{query}"
            ))
            .await?;
        let servers: Vec<crate::types::DanmuServer> =
            serde_json::from_value(data["host_list"].clone())
                .map_err(|e| Error::UnexpectedResponse(e.to_string()))?;
        let token = data["token"].as_str().unwrap_or_default().to_string();
        Ok(DanmuServerConf { servers, token })
    }

    /// Fetch the display name of a UP.
    ///
    /// Fetches a `buvid3`/`buvid4` fingerprint first when the client
    /// was built without one. Bilibili applies risk control to some
    /// IPs; on failure the caller should fall back to showing the uid.
    pub async fn up_info(&self, uid: UID) -> Result<UpInfo, Error> {
        self.ensure_buvid().await?;
        let mut params = BTreeMap::new();
        params.insert("mid".to_string(), uid.get().to_string());
        params.insert("platform".to_string(), "web".into());
        let data: Value = self
            .get_api("https://api.bilibili.com/x/space/acc/info", &params)
            .await?;
        let uname = data["uname"]
            .as_str()
            .ok_or(Error::MissingField("uname"))?
            .to_string();
        Ok(UpInfo { uid, uname })
    }

    /// Fetch the live home rooms: followed livers' live rooms first,
    /// then recommended rooms.
    ///
    /// `GET /xlive/web-interface/v1/index/getList`, WBI signed. The
    /// followed module (type 8) only appears for logged-in sessions;
    /// anonymous clients get the recommended modules only.
    pub async fn live_home(&self) -> Result<LiveHome, Error> {
        let signer = self.wbi_signer().await?;
        let mut params = BTreeMap::new();
        params.insert("platform".into(), "web".into());
        params.insert("web_location".into(), "444.7".into());
        let signed = signer.sign(params);
        let query = signed
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        let url =
            format!("https://api.live.bilibili.com/xlive/web-interface/v1/index/getList?{query}");
        let data: Value = self.get_api_raw(&url).await?;
        parse_live_home(&data)
    }

    /// Start a QR login session.
    ///
    /// `GET /x/passport-login/web/qrcode/generate`, anonymous.
    pub async fn qrcode_generate(&self) -> Result<Qrcode, Error> {
        let data: Value = self
            .get_api(
                "https://passport.bilibili.com/x/passport-login/web/qrcode/generate",
                &BTreeMap::new(),
            )
            .await?;
        let qr: Qrcode = serde_json::from_value(data)
            .map_err(|e| Error::UnexpectedResponse(format!("qrcode generate: {e}")))?;
        Ok(qr)
    }

    /// Poll a QR login session once.
    ///
    /// `GET /x/passport-login/web/qrcode/poll`. Call this every two
    /// seconds until the status leaves `Waiting`/`Scanned`.
    pub async fn qrcode_poll(&self, qrcode_key: &str) -> Result<QrcodePoll, Error> {
        let url = format!(
            "https://passport.bilibili.com/x/passport-login/web/qrcode/poll?qrcode_key={qrcode_key}"
        );
        let resp = self
            .http
            .get(&url)
            .header(reqwest::header::USER_AGENT, self.user_agent)
            .header(reqwest::header::COOKIE, &self.cookie)
            .send()
            .await?;
        let cookies: Vec<(String, String)> = resp
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .filter_map(|raw| {
                // `Set-Cookie: name=value; Path=/; ...`: keep `name=value`.
                let name_value = raw.split(';').next()?.trim();
                let (n, v) = name_value.split_once('=')?;
                Some((n.trim().to_string(), v.trim().to_string()))
            })
            .collect();
        let body: Value = resp.json().await?;
        let code = body["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            let msg = body["msg"]
                .as_str()
                .or_else(|| body["message"].as_str())
                .unwrap_or("unknown")
                .to_string();
            return Err(Error::Api { code, msg });
        }
        let status_code = body["data"]["code"].as_i64().unwrap_or(86101);
        let status = QrcodeStatus::from_code(status_code as i32);
        let refresh_token = body["data"]["refresh_token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        Ok(QrcodePoll {
            status,
            cookies,
            refresh_token,
        })
    }

    /// Fetch a `buvid3`/`buvid4` fingerprint when the client has none.
    async fn ensure_buvid(&self) -> Result<(), Error> {
        {
            let guard = self.buvid3.read().await;
            if guard.is_some() {
                return Ok(());
            }
        }
        let resp = self
            .http
            .get("https://api.bilibili.com/x/frontend/finger/spi")
            .header(reqwest::header::USER_AGENT, self.user_agent)
            .header(reqwest::header::COOKIE, &self.cookie)
            .send()
            .await?;
        let body: Value = resp.json().await?;
        let b3 = body["data"]["b_3"]
            .as_str()
            .ok_or(Error::MissingField("b_3"))?
            .to_string();
        let b4 = body["data"]["b_4"]
            .as_str()
            .ok_or(Error::MissingField("b_4"))?
            .to_string();
        *self.buvid3.write().await = Some(b3);
        *self.buvid4.write().await = Some(b4);
        Ok(())
    }

    /// A GET that expects `{code, msg, data}` and returns `data`.
    async fn get_api(&self, url: &str, params: &BTreeMap<String, String>) -> Result<Value, Error> {
        let url = if params.is_empty() {
            url.to_string()
        } else {
            let query = params
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("&");
            format!("{url}?{query}")
        };
        self.get_api_raw(&url).await
    }

    async fn get_api_raw(&self, url: &str) -> Result<Value, Error> {
        let resp = self
            .http
            .get(url)
            .header(reqwest::header::USER_AGENT, self.user_agent)
            .header(reqwest::header::COOKIE, &self.cookie)
            .send()
            .await?;
        let body: Value = resp.json().await?;
        let code = body["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            let msg = body["msg"]
                .as_str()
                .or_else(|| body["message"].as_str())
                .unwrap_or("unknown")
                .to_string();
            return Err(Error::Api { code, msg });
        }
        Ok(body["data"].clone())
    }
}
/// One module in the live home `getList` response.
#[derive(Debug, serde::Deserialize)]
struct LiveHomeModule {
    module_info: LiveHomeModuleInfo,
    #[serde(default)]
    list: Vec<LiveRoomEntry>,
}

#[derive(Debug, serde::Deserialize)]
struct LiveHomeModuleInfo {
    /// The module type. 8 is the "my follows" module.
    #[serde(rename = "type")]
    module_type: i32,
}

/// Parse a `getList` `data` value into followed and recommended rooms.
///
/// A room that appears in several modules keeps its first
/// occurrence, and followed rooms keep priority over recommended
/// ones.
fn parse_live_home(data: &Value) -> Result<LiveHome, Error> {
    let modules: Vec<LiveHomeModule> = serde_json::from_value(data["room_list"].clone())
        .map_err(|e| Error::UnexpectedResponse(format!("live home room_list: {e}")))?;
    let mut followed: Vec<LiveRoomEntry> = Vec::new();
    let mut recommended: Vec<LiveRoomEntry> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    // The followed module comes first in the response, so a room that
    // appears in several modules lands in the first list it matches.
    for module in &modules {
        let is_followed = module.module_info.module_type == 8;
        for room in &module.list {
            if !seen.insert(room.room_id.get()) {
                continue;
            }
            if is_followed {
                followed.push(room.clone());
            } else {
                recommended.push(room.clone());
            }
        }
    }
    Ok(LiveHome {
        followed,
        recommended,
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_live_home_splits_followed_from_recommended() {
        let data = serde_json::json!({
            "room_list": [
                {
                    "module_info": {"type": 8, "title": "我的关注"},
                    "list": [
                        {"roomid": 111, "uid": 1, "uname": "liver a", "title": "t1", "online": 10},
                        {"roomid": 222, "uid": 2, "uname": "liver b", "title": "t2", "online": 20}
                    ]
                },
                {
                    "module_info": {"type": 9, "title": "网游"},
                    "list": [
                        {"roomid": 222, "uid": 2, "uname": "liver b", "title": "t2", "online": 20},
                        {"roomid": 333, "uid": 3, "uname": "liver c", "title": "t3", "online": 30}
                    ]
                }
            ]
        });
        let home = parse_live_home(&data).unwrap();
        assert_eq!(
            home.followed
                .iter()
                .map(|r| r.room_id.get())
                .collect::<Vec<_>>(),
            [111, 222]
        );
        // Room 222 is already in followed; the duplicate is dropped.
        assert_eq!(
            home.recommended
                .iter()
                .map(|r| r.room_id.get())
                .collect::<Vec<_>>(),
            [333]
        );
        assert_eq!(home.followed[0].uname, "liver a");
        assert_eq!(home.followed[0].online, 10);
    }

    #[test]
    fn parse_live_home_tolerates_missing_fields() {
        let data = serde_json::json!({
            "room_list": [
                {
                    "module_info": {"type": 9},
                    "list": [{"roomid": 7, "uid": 9}]
                }
            ]
        });
        let home = parse_live_home(&data).unwrap();
        assert!(home.followed.is_empty());
        assert_eq!(home.recommended.len(), 1);
        assert_eq!(home.recommended[0].room_id.get(), 7);
        assert!(home.recommended[0].uname.is_empty());
    }

    #[test]
    fn parse_live_home_rejects_bad_shape() {
        let data = serde_json::json!({"room_list": "nope"});
        assert!(parse_live_home(&data).is_err());
    }
}

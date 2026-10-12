//! Typed views over danmaku WS commands.

use serde_json::Value;

/// One command object from a business payload.
///
/// `value` is the full JSON object. Use the typed helpers below for
/// the commands `bdr` shows in the UI.
#[derive(Clone, Debug)]
pub struct Command {
    pub cmd: String,
    pub value: Value,
}

/// Split a business JSON document into commands.
///
/// Accepts both a single command object and the `{"message": [...]}`
/// envelope.
pub fn parse_commands(value: &Value) -> Vec<Command> {
    if let Some(items) = value.get("message").and_then(|m| m.as_array()) {
        return items
            .iter()
            .filter_map(|item| {
                let cmd = item["cmd"].as_str()?.to_string();
                Some(Command {
                    cmd,
                    value: item.clone(),
                })
            })
            .collect();
    }
    let Some(cmd) = value["cmd"].as_str() else {
        return Vec::new();
    };
    vec![Command {
        cmd: cmd.to_string(),
        value: value.clone(),
    }]
}

/// A danmaku line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Danmaku {
    pub user: String,
    pub text: String,
    pub color: u32,
    pub uid: u64,
}

/// Extract danmaku lines from a `DANMU_MSG` command.
///
/// Two layouts exist:
/// - Current web layout: `info: [[id, mode, font, color, ts, uid, ...],
///   ...]`. Entry 15 carries `extra` (a JSON string with `content`) and
///   `user` (with `base.name`).
/// - Old web layout: `danmus: [[text, [id, mode, color, ts, uid,
///   uname, ...], color], ...]`.
pub fn danmus(cmd: &Command) -> Vec<Danmaku> {
    if let Some(items) = cmd.value["info"].as_array() {
        let out: Vec<Danmaku> = items
            .iter()
            .filter_map(|item| {
                let parts = item.as_array()?;
                let uid = parts.get(5).and_then(|v| v.as_u64()).unwrap_or(0);
                let color = parts.get(3).and_then(|v| v.as_u64()).unwrap_or(0xFFFFFF) as u32;
                let meta = parts.get(15);
                let text = meta
                    .and_then(|m| m.get("extra"))
                    .and_then(|e| e.as_str())
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                    .and_then(|v| v["content"].as_str().map(String::from))
                    .unwrap_or_default();
                let user = meta
                    .and_then(|m| m.get("user"))
                    .and_then(|u| u.get("base"))
                    .and_then(|b| b.get("name"))
                    .and_then(|n| n.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| uid.to_string());
                // Heart-click danmaku carry no text. Skip them.
                if text.is_empty() {
                    return None;
                }
                Some(Danmaku { user, text, color, uid })
            })
            .collect();
        return out;
    }
    let Some(items) = cmd.value["danmus"].as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let parts = item.as_array()?;
            if parts.len() < 2 {
                return None;
            }
            let text = parts[0].as_str().unwrap_or_default();
            let info = parts.get(1)?.as_array()?;
            if text.is_empty() {
                return None;
            }
            Some(Danmaku {
                user: info.get(5).and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                text: text.to_string(),
                color: info.get(2).and_then(|v| v.as_u64()).unwrap_or(255) as u32,
                uid: info.get(4).and_then(|v| v.as_u64()).unwrap_or(0),
            })
        })
        .collect()
}

/// Extract a gift event from a `SEND_GIFT` command.
pub fn gift(cmd: &Command) -> Option<(String, String, u32)> {
    let data = &cmd.value["data"];
    let user = data["uname"].as_str()?.to_string();
    let gift = data["giftName"].as_str()?.to_string();
    let num = data["num"].as_u64()? as u32;
    Some((user, gift, num))
}

/// Extract the online count from `ONLINE_RANK_COUNT` or `ONLINE_CNT_RPK`.
pub fn online_count(cmd: &Command) -> Option<u64> {
    cmd.value["data"]["count"].as_u64()
}

/// Extract a room change from a `ROOM_CHANGE` command.
pub fn room_change(cmd: &Command) -> Option<(String, bool)> {
    let title = cmd.value["data"]["title"].as_str()?.to_string();
    let live = cmd.value["data"]["live"].as_bool().unwrap_or(true);
    Some((title, live))
}

/// Extract a system message from `INTERFERE` or `SYSTEM_MSG`.
pub fn system_msg(cmd: &Command) -> Option<String> {
    if cmd.cmd == "SYSTEM_MSG" {
        return cmd.value["data"]["msg"].as_str().map(|s| s.to_string());
    }
    cmd.value["data"]["msg"].as_str().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_new_info_layout() {
        let cmd = parse_commands(&json!({
            "cmd": "DANMU_MSG",
            "info": [[
                0, 1, 25, 16777215, 1700000000000u64, 99, 0, "hash", 0, 0, 0, "", 0, "{}",
                "{}",
                {"extra": "{\"content\": \"hello world\"}", "user": {"base": {"name": "alice"}, "uid": 99}},
                {}, 0
            ]],
            "dm_v2": ""
        }));
        let d = danmus(&cmd[0]);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].text, "hello world");
        assert_eq!(d[0].user, "alice");
        assert_eq!(d[0].uid, 99);
        assert_eq!(d[0].color, 16777215);
    }

    #[test]
    fn test_envelope_and_single() {
        let envelope = json!({
            "roomid": 1,
            "message": [
                {"cmd": "DANMU_MSG", "danmus": [["hi", [0, 255, 255, 1, 7, "alice"], 1]]},
                {"cmd": "SEND_GIFT", "data": {"uname": "bob", "giftName": "chili", "num": 3}}
            ]
        });
        let cmds = parse_commands(&envelope);
        assert_eq!(cmds.len(), 2);
        let d = danmus(&cmds[0]);
        assert_eq!(d[0].user, "alice");
        assert_eq!(d[0].text, "hi");
        assert_eq!(gift(&cmds[1]).unwrap(), ("bob".into(), "chili".into(), 3));
    }

    #[test]
    fn test_room_change_and_online() {
        let rc = parse_commands(&json!({"cmd": "ROOM_CHANGE", "data": {"title": "t", "live": false}}));
        assert_eq!(room_change(&rc[0]).unwrap(), ("t".into(), false));
        let oc = parse_commands(&json!({"cmd": "ONLINE_RANK_COUNT", "data": {"count": 9}}));
        assert_eq!(online_count(&oc[0]).unwrap(), 9);
    }

    #[test]
    fn test_system_msg() {
        let s = parse_commands(&json!({"cmd": "INTERFERE", "data": {"msg": "hello"}}));
        assert_eq!(system_msg(&s[0]).unwrap(), "hello");
    }
}

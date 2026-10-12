//! Parse user-typed room references (URLs, plain ids, short ids).

use crate::error::Error;

/// Extract a room id candidate from user input.
///
/// Accepted forms:
/// - `6` or `7734200`: a bare number (short id or real room id).
/// - `live.bilibili.com/6`: a host-and-path room URL without scheme.
/// - `https://live.bilibili.com/7734200`: a full room URL.
/// - `https://live.bilibili.com/bl1234`: the `BL` short-id URL form.
///
/// The result is a candidate id. Resolve it with
/// [`Client::room_info`](crate::Client::room_info), which accepts both
/// short and real ids.
pub fn parse_room_ref(input: &str) -> Result<u64, Error> {
    let input = input.trim();
    if input.is_empty() {
        return Err(Error::BadRoomRef("empty input".into()));
    }
    let bare = input
        .strip_prefix("https://")
        .or_else(|| input.strip_prefix("http://"))
        .unwrap_or(input);
    let bare = bare
        .trim_start_matches("live.bilibili.com")
        .trim_start_matches('/');
    let lower = bare.to_lowercase();
    let bare = lower.trim_start_matches("bl");
    let bare = bare.trim();
    if bare.is_empty() {
        // A bare host, no room segment.
        return Err(Error::BadRoomRef(
            "no room id found in the URL, expected live.bilibili.com/<id>".into(),
        ));
    }
    match bare.parse::<u64>() {
        Ok(id) if id > 0 => Ok(id),
        _ => Err(Error::BadRoomRef(format!(
            "expected a number, live.bilibili.com/<id>, or a full room URL, got {input:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bare_numbers() {
        assert_eq!(parse_room_ref("6").unwrap(), 6);
        assert_eq!(parse_room_ref(" 7734200 ").unwrap(), 7734200);
    }

    #[test]
    fn test_urls() {
        assert_eq!(
            parse_room_ref("https://live.bilibili.com/7734200").unwrap(),
            7734200
        );
        assert_eq!(
            parse_room_ref("http://live.bilibili.com/6").unwrap(),
            6
        );
        assert_eq!(parse_room_ref("live.bilibili.com/6").unwrap(), 6);
        assert_eq!(parse_room_ref("/6").unwrap(), 6);
        assert_eq!(
            parse_room_ref("https://live.bilibili.com/bl66").unwrap(),
            66
        );
    }

    #[test]
    fn test_bad_input() {
        assert!(parse_room_ref("").is_err());
        assert!(parse_room_ref("https://live.bilibili.com/").is_err());
        assert!(parse_room_ref("not a room").is_err());
        assert!(parse_room_ref("0").is_err());
    }
}

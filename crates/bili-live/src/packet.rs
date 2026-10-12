//! Bilibili live danmaku WebSocket packet framing.
//!
//! Every message on the socket is a 16-byte big-endian header followed
//! by the body.
//!
//! ```text
//! bytes  0..4   pack_len       total frame length, header + body
//! bytes  4..6   raw_header_len always 16
//! bytes  6..8   ver            0 = plain JSON, 2 = deflate, 3 = brotli
//! bytes  8..12  operation      see the OP_* constants
//! bytes 12..16  seq_id
//! ```
//!
//! A body with `ver = 2` or `ver = 3` is a compressed stream that
//! contains one or more inner frames. Each inner frame has its own
//! 16-byte header with `ver = 0` and a plain JSON body.

use thiserror::Error;

use crate::cmd::{parse_commands, Command};

/// A framing or payload error.
#[derive(Debug, Error)]
pub enum Error {
    #[error("frame too short: {0} bytes")]
    ShortFrame(usize),

    #[error("bad compressed payload (ver {0}): {1}")]
    BadPayload(u16, String),

    #[error("bad business payload: {0}")]
    #[allow(dead_code)]
    BadBusiness(String),
}

/// Header length in bytes.
pub const HEADER_LEN: usize = 16;

/// AUTH (client auth packet).
pub const OP_AUTH: u32 = 7;
/// HEARTBEAT.
pub const OP_HEARTBEAT: u32 = 2;
/// HEARTBEAT_REPLY (server heartbeat ack).
#[allow(dead_code)]
pub const OP_HEARTBEAT_REPLY: u32 = 3;
/// SEND_MSG (client send, used for danmaku in M3).
#[allow(dead_code)]
pub const OP_SEND_MSG: u32 = 4;
/// SEND_MSG_REPLY (server business messages).
pub const OP_SEND_MSG_REPLY: u32 = 5;
/// AUTH_REPLY.
pub const OP_AUTH_REPLY: u32 = 8;

/// One 16-byte packet header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketHeader {
    pub pack_len: u32,
    pub raw_header_len: u16,
    pub ver: u16,
    pub operation: u32,
    pub seq_id: u32,
}

impl PacketHeader {
    /// Read one header from the start of `buf`.
    pub fn read(buf: &[u8]) -> Result<Self, Error> {
        if buf.len() < HEADER_LEN {
            return Err(Error::ShortFrame(buf.len()));
        }
        Ok(Self {
            pack_len: u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]),
            raw_header_len: u16::from_be_bytes([buf[4], buf[5]]),
            ver: u16::from_be_bytes([buf[6], buf[7]]),
            operation: u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]),
            seq_id: u32::from_be_bytes([buf[12], buf[13], buf[14], buf[15]]),
        })
    }

    /// Build a packet (header + body) for sending.
    pub fn build(self, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + body.len());
        let pack_len = (HEADER_LEN + body.len()) as u32;
        out.extend_from_slice(&pack_len.to_be_bytes());
        out.extend_from_slice(&self.raw_header_len.to_be_bytes());
        out.extend_from_slice(&self.ver.to_be_bytes());
        out.extend_from_slice(&self.operation.to_be_bytes());
        out.extend_from_slice(&self.seq_id.to_be_bytes());
        out.extend_from_slice(body);
        out
    }
}

/// Build a JSON auth packet.
pub fn build_auth_packet(uid: u64, room_id: u64, buvid: &str, token: &str) -> Vec<u8> {
    let body = serde_json::json!({
        "uid": uid,
        "roomid": room_id,
        "protover": 2,
        "platform": "web",
        "type": 2,
        "buvid": buvid,
        "key": token,
    })
    .to_string();
    PacketHeader {
        pack_len: 0,
        raw_header_len: HEADER_LEN as u16,
        ver: 1,
        operation: OP_AUTH,
        seq_id: 1,
    }
    .build(body.as_bytes())
}

/// Build a heartbeat packet.
pub fn build_heartbeat_packet() -> Vec<u8> {
    PacketHeader {
        pack_len: 0,
        raw_header_len: HEADER_LEN as u16,
        ver: 1,
        operation: OP_HEARTBEAT,
        seq_id: 1,
    }
    .build(br"{}")
}


/// Walk all outer frames in a raw WebSocket payload.
///
/// For each frame, the callback gets the header and the body slice.
/// A body is valid from `raw_header_len` up to `pack_len`.
pub fn walk_frames(data: &[u8], mut cb: impl FnMut(&PacketHeader, &[u8])) -> Result<(), Error> {
    let mut off = 0usize;
    while off + HEADER_LEN <= data.len() {
        let header = PacketHeader::read(&data[off..])?;
        let end = (off + header.pack_len as usize).min(data.len());
        let body = &data[off + HEADER_LEN..end];
        cb(&header, body);
        if (header.pack_len as usize) <= 0 {
            break;
        }
        off = end;
    }
    Ok(())
}

/// Decode one business payload (`op = SEND_MSG_REPLY`) into commands.
///
/// The payload may be plain JSON, or a deflate/brotli stream that
/// contains one or more inner frames. Each inner frame is a 16-byte
/// header (ver 0) plus a JSON body. A JSON body is either a single
/// command object or an envelope `{"message": [...]}`.
pub fn decode_business(ver: u16, body: &[u8]) -> Result<Vec<Command>, Error> {
    let raw: Vec<u8> = match ver {
        0 => body.to_vec(),
        2 => {
            let mut out = Vec::new();
            use std::io::Read;
            let mut decoder =
                flate2::read::ZlibDecoder::new(std::io::BufReader::new(std::io::Cursor::new(body)));
            decoder
                .read_to_end(&mut out)
                .map_err(|e| Error::BadPayload(2, e.to_string()))?;
            out
        }
        3 => {
            let mut out = Vec::new();
            use std::io::Read;
            let mut decoder = brotli::Decompressor::new(std::io::Cursor::new(body), 4096);
            decoder
                .read_to_end(&mut out)
                .map_err(|e| Error::BadPayload(3, e.to_string()))?;
            out
        }
        other => return Err(Error::BadPayload(other, "unsupported ver".into())),
    };
    // If the payload starts with a plausible inner frame header, walk
    // the frames. Otherwise parse it as a plain JSON document.
    if raw.len() >= HEADER_LEN
        && let Ok(header) = PacketHeader::read(&raw)
        && header.raw_header_len as usize == HEADER_LEN
        && (header.pack_len as usize) <= raw.len()
        && header.pack_len as usize > HEADER_LEN
    {
        let mut commands = Vec::new();
        walk_frames(&raw, |header, body| {
            if header.operation == OP_SEND_MSG_REPLY || header.operation == 0 {
                commands.extend(parse_json_business(body));
            }
        })?;
        return Ok(commands);
    }
    Ok(parse_json_business(&raw))
}

fn parse_json_business(body: &[u8]) -> Vec<Command> {
    let value: serde_json::Value =
        match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
    parse_commands(&value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_roundtrip() {
        let body = b"hello";
        let header = PacketHeader {
            pack_len: 0,
            raw_header_len: 16,
            ver: 1,
            operation: OP_AUTH,
            seq_id: 1,
        };
        let packet = header.build(body);
        assert_eq!(packet.len(), HEADER_LEN + body.len());
        let read = PacketHeader::read(&packet).unwrap();
        assert_eq!(read.operation, OP_AUTH);
        assert_eq!(read.pack_len as usize, packet.len());
        assert_eq!(&packet[HEADER_LEN..], body);
    }

    #[test]
    fn test_auth_packet_shape() {
        let packet = build_auth_packet(0, 7734200, "", "TOK");
        let header = PacketHeader::read(&packet).unwrap();
        assert_eq!(header.operation, OP_AUTH);
        let body = serde_json::from_slice::<serde_json::Value>(&packet[HEADER_LEN..]).unwrap();
        assert_eq!(body["roomid"], 7734200);
        assert_eq!(body["protover"], 2);
        assert_eq!(body["key"], "TOK");
    }

    #[test]
    fn test_plain_json_business() {
        let json = br#"{"cmd":"DANMU_MSG","danmus":[["hi",[0,255,255,1700000000,123,"alice",1],255]]}"#;
        let cmds = decode_business(0, json).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].cmd, "DANMU_MSG");
    }

    #[test]
    fn test_deflated_inner_frames() {
        let inner = br#"{"cmd":"ROOM_CHANGE","data":{"title":"x","live":true}}"#;
        let header = PacketHeader {
            pack_len: 0,
            raw_header_len: 16,
            ver: 0,
            operation: OP_SEND_MSG_REPLY,
            seq_id: 0,
        };
        let frame = header.build(inner);
        // Wrap it in a zlib stream, as the server sends.
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        use std::io::Write;
        encoder.write_all(&frame).unwrap();
        let deflated = encoder.finish().unwrap();
        let cmds = decode_business(2, &deflated).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].cmd, "ROOM_CHANGE");
    }

    #[test]
    fn test_brotli_inner_frames() {
        let inner = br#"{"cmd":"ONLINE_RANK_COUNT","data":{"count":42}}"#;
        let frame = PacketHeader {
            pack_len: 0,
            raw_header_len: 16,
            ver: 0,
            operation: OP_SEND_MSG_REPLY,
            seq_id: 0,
        }
        .build(inner);
        let mut out = Vec::new();
        use brotli::CompressorWriter;
        let mut compressor = CompressorWriter::new(&mut out, 4096, 5, 22);
        use std::io::Write;
        compressor.write_all(&frame).unwrap();
        let compressed = compressor.into_inner();
        let cmds = decode_business(3, &compressed).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].cmd, "ONLINE_RANK_COUNT");
    }
}

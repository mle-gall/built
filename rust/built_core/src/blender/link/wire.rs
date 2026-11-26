// SPDX-License-Identifier: GPL-3.0-or-later
//! The Blender link's frames. One frame is an 8-byte prefix (the JSON
//! header's length and the payload's length, both big-endian u32), the UTF-8
//! JSON object, then the payload bytes. A message whose payload is larger
//! than `CHUNK` travels as several frames that share a `msg` number and carry
//! `part` / `parts`; the receiver joins them before handing the message on.
//! The add-on's `built_link/wire.py` is the same format in plain Python.
use serde_json::{Map, Value};
use std::collections::HashMap;

/// The protocol version both sides name in `hello` / `welcome`.
pub const PROTOCOL: u64 = 1;
/// Payload bytes per frame.
pub const CHUNK: usize = 1 << 20;
/// Longest JSON header accepted.
pub const MAX_HEADER: usize = 1 << 20;
/// Longest payload one frame may carry.
pub const MAX_FRAME_PAYLOAD: usize = 4 << 20;
/// Largest joined payload (a whole scene batch).
pub const MAX_MESSAGE: usize = 512 << 20;
const MAX_PARTS: u64 = (MAX_MESSAGE / CHUNK) as u64 + 1;

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub header: Map<String, Value>,
    pub payload: Vec<u8>,
}

impl Message {
    pub fn new(header: Map<String, Value>, payload: Vec<u8>) -> Self {
        Self { header, payload }
    }
    pub fn kind(&self) -> &str {
        self.header.get("type").and_then(Value::as_str).unwrap_or("")
    }
}

fn frame(header: &Map<String, Value>, payload: &[u8]) -> Vec<u8> {
    let json = serde_json::to_vec(header).expect("a JSON map always serialises");
    let mut out = Vec::with_capacity(8 + json.len() + payload.len());
    out.extend_from_slice(&(json.len() as u32).to_be_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(payload);
    out
}

/// The frames of one message: a single frame when the payload fits in
/// `chunk` bytes, else `parts` frames numbered from 0 under `msg`.
pub fn encode(message: &Message, msg: u64, chunk: usize) -> Vec<Vec<u8>> {
    let chunk = chunk.clamp(1, MAX_FRAME_PAYLOAD);
    if message.payload.len() <= chunk {
        return vec![frame(&message.header, &message.payload)];
    }
    let pieces: Vec<&[u8]> = message.payload.chunks(chunk).collect();
    let parts = pieces.len();
    pieces
        .into_iter()
        .enumerate()
        .map(|(i, piece)| {
            let mut h = if i == 0 { message.header.clone() } else { Map::new() };
            if i > 0 {
                h.insert("type".into(), message.header.get("type").cloned().unwrap_or(Value::Null));
            }
            h.insert("msg".into(), msg.into());
            h.insert("part".into(), i.into());
            h.insert("parts".into(), parts.into());
            frame(&h, piece)
        })
        .collect()
}

#[derive(Debug, PartialEq)]
pub enum WireError {
    Header(String),
    TooLarge(usize),
    Parts(String),
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireError::Header(e) => write!(f, "bad frame header: {e}"),
            WireError::TooLarge(n) => write!(f, "frame too large ({n} bytes)"),
            WireError::Parts(e) => write!(f, "bad message parts: {e}"),
        }
    }
}

struct Partial {
    header: Map<String, Value>,
    parts: u64,
    next: u64,
    payload: Vec<u8>,
}

/// Bytes in, whole messages out. Frames of different messages may
/// interleave; parts of one message arrive in order (one TCP stream).
#[derive(Default)]
pub struct Assembler {
    buffer: Vec<u8>,
    partial: HashMap<u64, Partial>,
}

impl Assembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bytes buffered but not yet a whole message.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn pending(&self) -> usize {
        self.buffer.len() + self.partial.values().map(|p| p.payload.len()).sum::<usize>()
    }

    /// Feed bytes; return every message they complete, in order.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Message>, WireError> {
        self.buffer.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            if self.buffer.len() < 8 {
                break;
            }
            let hlen = u32::from_be_bytes(self.buffer[0..4].try_into().unwrap()) as usize;
            let plen = u32::from_be_bytes(self.buffer[4..8].try_into().unwrap()) as usize;
            if hlen > MAX_HEADER {
                return Err(WireError::TooLarge(hlen));
            }
            if plen > MAX_FRAME_PAYLOAD {
                return Err(WireError::TooLarge(plen));
            }
            if self.buffer.len() < 8 + hlen + plen {
                break;
            }
            let header: Value = serde_json::from_slice(&self.buffer[8..8 + hlen])
                .map_err(|e| WireError::Header(e.to_string()))?;
            let Value::Object(header) = header else {
                return Err(WireError::Header("not an object".into()));
            };
            let payload = self.buffer[8 + hlen..8 + hlen + plen].to_vec();
            self.buffer.drain(..8 + hlen + plen);
            if let Some(m) = self.join(header, payload)? {
                out.push(m);
            }
        }
        Ok(out)
    }

    fn join(&mut self, mut header: Map<String, Value>, payload: Vec<u8>) -> Result<Option<Message>, WireError> {
        let Some(parts) = header.get("parts").and_then(Value::as_u64) else {
            return Ok(Some(Message::new(header, payload)));
        };
        let msg = header.get("msg").and_then(Value::as_u64).ok_or_else(|| WireError::Parts("no msg".into()))?;
        let part = header.get("part").and_then(Value::as_u64).ok_or_else(|| WireError::Parts("no part".into()))?;
        if parts == 0 || parts > MAX_PARTS {
            return Err(WireError::Parts(format!("{parts} parts")));
        }
        for k in ["msg", "part", "parts"] {
            header.remove(k);
        }
        if part == 0 {
            if self.partial.contains_key(&msg) {
                return Err(WireError::Parts(format!("message {msg} restarted")));
            }
            self.partial.insert(msg, Partial { header, parts, next: 0, payload: Vec::new() });
        }
        let entry = self.partial.get_mut(&msg).ok_or_else(|| WireError::Parts(format!("part {part} of unknown message {msg}")))?;
        if entry.next != part || entry.parts != parts {
            return Err(WireError::Parts(format!("message {msg}: part {part} out of order")));
        }
        if entry.payload.len() + payload.len() > MAX_MESSAGE {
            return Err(WireError::TooLarge(entry.payload.len() + payload.len()));
        }
        entry.payload.extend_from_slice(&payload);
        entry.next += 1;
        if entry.next < entry.parts {
            return Ok(None);
        }
        let done = self.partial.remove(&msg).unwrap();
        Ok(Some(Message::new(done.header, done.payload)))
    }
}

/// Compare two secrets in time independent of where they differ.
pub fn same_secret(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = (a.len() ^ b.len()) as u8 | u8::from(a.len() != b.len());
    for i in 0..a.len().max(b.len()) {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0xff);
    }
    diff == 0
}

/// A pairing token: 16 characters from an unambiguous alphabet, seeded by
/// the system's random keys (`RandomState`), about 80 bits.
pub fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    const ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
    let mut bits = Vec::new();
    for salt in 0..2u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(salt);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        bits.push(h.finish());
    }
    let mut out = String::new();
    for i in 0..16 {
        let word = bits[i / 8];
        let index = ((word >> ((i % 8) * 5)) & 31) as usize;
        if i > 0 && i % 4 == 0 {
            out.push('-');
        }
        out.push(ALPHABET[index] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(kind: &str, payload: Vec<u8>) -> Message {
        let Value::Object(h) = json!({"type": kind, "uid": "a"}) else { unreachable!() };
        Message::new(h, payload)
    }

    #[test]
    fn a_small_message_is_one_frame_and_comes_back_whole() {
        let m = msg("chunk", vec![1, 2, 3]);
        let frames = encode(&m, 7, CHUNK);
        assert_eq!(frames.len(), 1);
        let mut a = Assembler::new();
        assert_eq!(a.feed(&frames[0]).unwrap(), vec![m]);
        assert_eq!(a.pending(), 0);
    }

    #[test]
    fn a_large_payload_is_split_and_joined_byte_for_byte() {
        let payload: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        let m = msg("chunk", payload);
        let frames = encode(&m, 3, 1024);
        assert_eq!(frames.len(), 10);
        let bytes: Vec<u8> = frames.concat();
        let mut a = Assembler::new();
        let mut out = Vec::new();
        // Arbitrary TCP segment boundaries.
        for piece in bytes.chunks(777) {
            out.extend(a.feed(piece).unwrap());
        }
        assert_eq!(out, vec![m]);
        assert_eq!(a.pending(), 0);
    }

    #[test]
    fn interleaved_messages_are_kept_apart() {
        let a1 = msg("chunk", vec![1; 3000]);
        let b1 = msg("transform", vec![2; 2500]);
        let fa = encode(&a1, 1, 1000);
        let fb = encode(&b1, 2, 1000);
        let mut stream = Vec::new();
        for i in 0..3 {
            stream.extend_from_slice(&fa[i]);
            stream.extend_from_slice(&fb[i]);
        }
        let out = Assembler::new().feed(&stream).unwrap();
        assert_eq!(out, vec![a1, b1]);
    }

    #[test]
    fn malformed_frames_are_errors_not_panics() {
        let mut a = Assembler::new();
        let mut bad = (5u32).to_be_bytes().to_vec();
        bad.extend_from_slice(&0u32.to_be_bytes());
        bad.extend_from_slice(b"[1,2]");
        assert!(matches!(a.feed(&bad), Err(WireError::Header(_))));
        let mut huge = (u32::MAX).to_be_bytes().to_vec();
        huge.extend_from_slice(&0u32.to_be_bytes());
        assert!(matches!(Assembler::new().feed(&huge), Err(WireError::TooLarge(_))));
        let m = msg("chunk", vec![0; 3000]);
        let frames = encode(&m, 9, 1000);
        let mut a = Assembler::new();
        assert!(a.feed(&frames[1]).is_err(), "a part without its start");
    }

    #[test]
    fn secrets_compare_exactly() {
        assert!(same_secret("ABCD-EFGH", "ABCD-EFGH"));
        assert!(!same_secret("ABCD-EFGH", "ABCD-EFGX"));
        assert!(!same_secret("ABCD", "ABCD-"));
        assert!(!same_secret("", "A"));
        assert!(same_secret("", ""));
    }

    #[test]
    fn tokens_are_fresh_and_readable() {
        let a = new_token();
        let b = new_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 19);
        assert!(a.chars().all(|c| c == '-' || c.is_ascii_uppercase() || c.is_ascii_digit()));
        assert!(!a.contains('0') && !a.contains('O') && !a.contains('1') && !a.contains('I'));
    }
}

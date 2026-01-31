//! Shared protocol primitives for `audio-bridge` and `audio-send`.
//!
//! Protocol v2: framed stream over a single TCP connection.
//! Frames allow:
//! - streaming one or more files without reconnecting
//! - control messages (pause/resume/next) interleaved with file chunks
//!
//! Frame format:
//! - magic: 4 bytes "ABRD" (once, at connection start)
//! - version: u16 LE (once, at connection start)
//! - then repeated frames:
//!   - kind: u8
//!   - len: u32 LE
//!   - payload: [u8; len]

use std::io::{self, Read, Write};

pub const MAGIC: [u8; 4] = *b"ABRD";
pub const VERSION: u16 = 2;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    BeginFile = 0x10,
    FileChunk = 0x11,
    EndFile = 0x12,

    Pause = 0x20,
    Resume = 0x21,
    Next = 0x23,

    Error = 0x7F,
}

impl FrameKind {
    pub fn from_u8(b: u8) -> io::Result<Self> {
        let k = match b {
            0x10 => FrameKind::BeginFile,
            0x11 => FrameKind::FileChunk,
            0x12 => FrameKind::EndFile,
            0x20 => FrameKind::Pause,
            0x21 => FrameKind::Resume,
            0x23 => FrameKind::Next,
            0x7F => FrameKind::Error,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unknown frame kind {b:#x}"),
                ))
            }
        };
        Ok(k)
    }
}

/// Connection prelude: magic + version.
pub fn write_prelude(mut w: impl Write) -> io::Result<()> {
    w.write_all(&MAGIC)?;
    w.write_all(&VERSION.to_le_bytes())?;
    Ok(())
}

/// Read and validate the connection prelude.
pub fn read_prelude(mut r: impl Read) -> io::Result<()> {
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic)?;
    if magic != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad magic"));
    }

    let mut ver = [0u8; 2];
    r.read_exact(&mut ver)?;
    let version = u16::from_le_bytes(ver);
    if version != VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported version {version}"),
        ));
    }

    Ok(())
}

/// Write a frame header + payload.
pub fn write_frame(mut w: impl Write, kind: FrameKind, payload: &[u8]) -> io::Result<()> {
    w.write_all(&[kind as u8])?;
    let len: u32 = payload
        .len()
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too large"))?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(payload)?;
    Ok(())
}

/// Read a frame header and return `(kind, len)`.
///
/// The caller should then read exactly `len` bytes of payload.
pub fn read_frame_header(mut r: impl Read) -> io::Result<(FrameKind, u32)> {
    let mut kindb = [0u8; 1];
    r.read_exact(&mut kindb)?;
    let kind = FrameKind::from_u8(kindb[0])?;

    let mut lenb = [0u8; 4];
    r.read_exact(&mut lenb)?;
    let len = u32::from_le_bytes(lenb);
    Ok((kind, len))
}

/// Encode a `BEGIN_FILE` payload: `u16 ext_len` + UTF-8 extension bytes.
pub fn encode_begin_file_payload(extension: &str) -> io::Result<Vec<u8>> {
    let ext_bytes = extension.as_bytes();
    let ext_len: u16 = ext_bytes
        .len()
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "extension too long"))?;

    let mut out = Vec::with_capacity(2 + ext_bytes.len());
    out.extend_from_slice(&ext_len.to_le_bytes());
    out.extend_from_slice(ext_bytes);
    Ok(out)
}

/// Decode a `BEGIN_FILE` payload, returning the extension (may be empty).
pub fn decode_begin_file_payload(mut payload: &[u8]) -> io::Result<String> {
    if payload.len() < 2 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "short BEGIN_FILE"));
    }
    let ext_len = u16::from_le_bytes([payload[0], payload[1]]) as usize;
    payload = &payload[2..];

    if payload.len() != ext_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "BEGIN_FILE ext length mismatch",
        ));
    }

    let ext = std::str::from_utf8(payload)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "extension not utf-8"))?;

    Ok(ext.to_string())
}
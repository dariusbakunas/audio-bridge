//! Shared protocol primitives for `bridge` and `hub-cli`.
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
pub const VERSION: u16 = 4;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    BeginFile = 0x10,
    FileChunk = 0x11,
    EndFile = 0x12,

    Pause = 0x20,
    Resume = 0x21,
    Next = 0x23,

    /// Receiver → sender: basic track metadata (so UI can show duration/progress).
    TrackInfo = 0x30,

    /// Receiver → sender: playback position updates.
    PlaybackPos = 0x31,

    /// Sender → receiver: request device list.
    ListDevices = 0x40,
    /// Receiver → sender: device list response.
    DeviceList = 0x41,
    /// Sender → receiver: set output device by substring.
    SetDevice = 0x42,
    /// Receiver → sender: device set ack.
    DeviceSet = 0x43,

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
            0x30 => FrameKind::TrackInfo,
            0x31 => FrameKind::PlaybackPos,
            0x40 => FrameKind::ListDevices,
            0x41 => FrameKind::DeviceList,
            0x42 => FrameKind::SetDevice,
            0x43 => FrameKind::DeviceSet,
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
    let frame = encode_frame(kind, payload)?;
    w.write_all(&frame)?;
    Ok(())
}

/// Encode a frame into a single buffer (header + payload).
pub fn encode_frame(kind: FrameKind, payload: &[u8]) -> io::Result<Vec<u8>> {
    let len: u32 = payload
        .len()
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too large"))?;

    let mut out = Vec::with_capacity(1 + 4 + payload.len());
    out.push(kind as u8);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
    Ok(out)
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

/// Encode `TRACK_INFO` payload:
/// - sample_rate: u32 LE
/// - channels:    u16 LE
/// - duration_ms: u64 LE (0 means unknown)
pub fn encode_track_info(sample_rate: u32, channels: u16, duration_ms: Option<u64>) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 2 + 8);
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&duration_ms.unwrap_or(0).to_le_bytes());
    out
}

/// Decode `TRACK_INFO` payload.
pub fn decode_track_info(payload: &[u8]) -> io::Result<(u32, u16, Option<u64>)> {
    if payload.len() != 14 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad TRACK_INFO length"));
    }
    let sr = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let ch = u16::from_le_bytes([payload[4], payload[5]]);
    let dur = u64::from_le_bytes([
        payload[6], payload[7], payload[8], payload[9], payload[10], payload[11], payload[12], payload[13],
    ]);
    Ok((sr, ch, if dur == 0 { None } else { Some(dur) }))
}

/// Encode `PLAYBACK_POS` payload:
/// - played_frames: u64 LE (at device/output rate)
/// - paused:        u8 (0/1)
pub fn encode_playback_pos(played_frames: u64, paused: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + 1);
    out.extend_from_slice(&played_frames.to_le_bytes());
    out.push(if paused { 1 } else { 0 });
    out
}

/// Decode `PLAYBACK_POS` payload.
pub fn decode_playback_pos(payload: &[u8]) -> io::Result<(u64, bool)> {
    if payload.len() != 9 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad PLAYBACK_POS length"));
    }
    let frames = u64::from_le_bytes([
        payload[0], payload[1], payload[2], payload[3], payload[4], payload[5], payload[6], payload[7],
    ]);
    Ok((frames, payload[8] != 0))
}

/// Encode a list of device names: u16 count, then u16 len + bytes for each name.
pub fn encode_device_list(devices: &[String]) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let count: u16 = devices
        .len()
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "too many devices"))?;
    out.extend_from_slice(&count.to_le_bytes());
    for name in devices {
        let bytes = name.as_bytes();
        let len: u16 = bytes
            .len()
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "device name too long"))?;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(bytes);
    }
    Ok(out)
}

/// Decode a device list payload.
pub fn decode_device_list(payload: &[u8]) -> io::Result<Vec<String>> {
    if payload.len() < 2 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "short device list"));
    }
    let count = u16::from_le_bytes([payload[0], payload[1]]) as usize;
    let mut off = 2usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if off + 2 > payload.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated device list"));
        }
        let len = u16::from_le_bytes([payload[off], payload[off + 1]]) as usize;
        off += 2;
        if off + len > payload.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated device name"));
        }
        let name = std::str::from_utf8(&payload[off..off + len])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "device name not utf-8"))?;
        out.push(name.to_string());
        off += len;
    }
    Ok(out)
}

/// Encode a device selector string: u16 len + bytes.
pub fn encode_device_selector(name: &str) -> io::Result<Vec<u8>> {
    let bytes = name.as_bytes();
    let len: u16 = bytes
        .len()
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "device name too long"))?;
    let mut out = Vec::with_capacity(2 + bytes.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(out)
}

/// Decode a device selector string.
pub fn decode_device_selector(payload: &[u8]) -> io::Result<String> {
    if payload.len() < 2 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "short device selector"));
    }
    let len = u16::from_le_bytes([payload[0], payload[1]]) as usize;
    if payload.len() != 2 + len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "device selector length mismatch",
        ));
    }
    let name = std::str::from_utf8(&payload[2..])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "device selector not utf-8"))?;
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn prelude_roundtrip_ok() {
        let mut buf = Vec::new();
        write_prelude(&mut buf).unwrap();
        let mut cur = Cursor::new(buf);
        read_prelude(&mut cur).unwrap();
    }

    #[test]
    fn prelude_rejects_bad_magic() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"NOPE");
        buf.extend_from_slice(&VERSION.to_le_bytes());
        let mut cur = Cursor::new(buf);
        let err = read_prelude(&mut cur).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn prelude_rejects_bad_version() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&(VERSION + 1).to_le_bytes());
        let mut cur = Cursor::new(buf);
        let err = read_prelude(&mut cur).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn frame_encode_and_header_roundtrip() {
        let payload = b"hello";
        let frame = encode_frame(FrameKind::FileChunk, payload).unwrap();
        let mut cur = Cursor::new(frame);
        let (kind, len) = read_frame_header(&mut cur).unwrap();
        assert_eq!(kind, FrameKind::FileChunk);
        assert_eq!(len, payload.len() as u32);

        let mut read_payload = vec![0u8; len as usize];
        cur.read_exact(&mut read_payload).unwrap();
        assert_eq!(read_payload, payload);
    }

    #[test]
    fn write_frame_roundtrip() {
        let payload = b"abc123";
        let mut buf = Vec::new();
        write_frame(&mut buf, FrameKind::BeginFile, payload).unwrap();

        let mut cur = Cursor::new(buf);
        let (kind, len) = read_frame_header(&mut cur).unwrap();
        assert_eq!(kind, FrameKind::BeginFile);
        assert_eq!(len, payload.len() as u32);
        let mut read_payload = vec![0u8; len as usize];
        cur.read_exact(&mut read_payload).unwrap();
        assert_eq!(read_payload, payload);
    }

    #[test]
    fn begin_file_payload_roundtrip() {
        let ext = "flac";
        let payload = encode_begin_file_payload(ext).unwrap();
        let decoded = decode_begin_file_payload(&payload).unwrap();
        assert_eq!(decoded, ext);
    }

    #[test]
    fn begin_file_payload_rejects_short() {
        let err = decode_begin_file_payload(&[0x00]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn begin_file_payload_rejects_len_mismatch() {
        let payload = [0x02, 0x00, b'a']; // says len=2, only 1 byte provided
        let err = decode_begin_file_payload(&payload).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn track_info_roundtrip() {
        let payload = encode_track_info(48_000, 2, Some(123_456));
        let (sr, ch, dur) = decode_track_info(&payload).unwrap();
        assert_eq!(sr, 48_000);
        assert_eq!(ch, 2);
        assert_eq!(dur, Some(123_456));
    }

    #[test]
    fn track_info_unknown_duration() {
        let payload = encode_track_info(44_100, 1, None);
        let (_sr, _ch, dur) = decode_track_info(&payload).unwrap();
        assert_eq!(dur, None);
    }

    #[test]
    fn track_info_rejects_bad_len() {
        let err = decode_track_info(&[0u8; 10]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn playback_pos_roundtrip() {
        let payload = encode_playback_pos(987_654, true);
        let (frames, paused) = decode_playback_pos(&payload).unwrap();
        assert_eq!(frames, 987_654);
        assert!(paused);
    }

    #[test]
    fn playback_pos_rejects_bad_len() {
        let err = decode_playback_pos(&[0u8; 8]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}

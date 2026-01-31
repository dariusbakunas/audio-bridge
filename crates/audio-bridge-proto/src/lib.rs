//! Shared protocol primitives for `audio-bridge` and `audio-send`.
//!
//! This crate is intentionally tiny and dependency-free. It defines the on-the-wire
//! header used by the sender and receiver so both sides stay in lockstep.

use std::io::{self, Read, Write};

/// Protocol magic bytes.
pub const MAGIC: [u8; 4] = *b"ABRD";

/// Protocol version.
pub const VERSION: u16 = 1;

/// Write the stream header.
///
/// The sender writes this once, then streams raw file bytes until EOF.
///
/// `extension` should be something like `"flac"` or `"wav"`. It may be empty.
pub fn write_header(mut w: impl Write, extension: &str) -> io::Result<()> {
    w.write_all(&MAGIC)?;
    w.write_all(&VERSION.to_le_bytes())?;

    let ext_bytes = extension.as_bytes();
    let ext_len: u16 = ext_bytes
        .len()
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "extension too long"))?;

    w.write_all(&ext_len.to_le_bytes())?;
    w.write_all(ext_bytes)?;
    Ok(())
}

/// Read the stream header and return the file extension hint (may be empty).
pub fn read_header(mut r: impl Read) -> io::Result<String> {
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

    let mut lenb = [0u8; 2];
    r.read_exact(&mut lenb)?;
    let ext_len = u16::from_le_bytes(lenb) as usize;

    let mut ext = vec![0u8; ext_len];
    r.read_exact(&mut ext)?;

    let ext = std::str::from_utf8(&ext)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "extension not utf-8"))?;

    Ok(ext.to_string())
}
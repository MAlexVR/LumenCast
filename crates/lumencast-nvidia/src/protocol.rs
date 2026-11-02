//! The wire format of the matte server (`sidecar/nvidia-matte/matte_server.cpp`).
//!
//! request : u32 magic, u32 width, u32 height, width*height*3 RGB bytes
//! response: u32 magic, u32 status (0 = ok), u32 width, u32 height, then
//!           width*height matte bytes (only when the status is 0)
//! All integers are little endian.

use lumencast_fx::matte::Matte;
use std::io::{self, Read, Write};

/// "LCM1" read as a little endian integer.
pub const MAGIC: u32 = 0x314D_434C;
/// The server refuses sides above this; the client never sends them.
pub const MAX_SIDE: u32 = 4096;

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("the matte server connection failed: {0}")]
    Io(#[from] io::Error),
    #[error("the matte server sent something that is not a matte response")]
    BadMagic,
    #[error("the matte server could not process the frame (status {0})")]
    Refused(u32),
    #[error("the matte server answered with a matte of the wrong size")]
    WrongSize,
    #[error("the frame is empty or larger than {MAX_SIDE} pixels on a side")]
    BadFrame,
}

fn word(value: u32) -> [u8; 4] {
    value.to_le_bytes()
}

/// Sends one RGB frame.
pub fn write_request(
    out: &mut impl Write,
    width: u32,
    height: u32,
    rgb: &[u8],
) -> Result<(), ProtocolError> {
    let valid = width > 0
        && height > 0
        && width <= MAX_SIDE
        && height <= MAX_SIDE
        && rgb.len() == width as usize * height as usize * 3;
    if !valid {
        return Err(ProtocolError::BadFrame);
    }
    out.write_all(&word(MAGIC))?;
    out.write_all(&word(width))?;
    out.write_all(&word(height))?;
    out.write_all(rgb)?;
    out.flush()?;
    Ok(())
}

fn read_word(input: &mut impl Read) -> Result<u32, ProtocolError> {
    let mut bytes = [0u8; 4];
    input.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

/// Reads the answer to a frame of `width` x `height`.
pub fn read_response(
    input: &mut impl Read,
    width: u32,
    height: u32,
) -> Result<Matte, ProtocolError> {
    if read_word(input)? != MAGIC {
        return Err(ProtocolError::BadMagic);
    }
    let status = read_word(input)?;
    let (got_width, got_height) = (read_word(input)?, read_word(input)?);
    if status != 0 {
        return Err(ProtocolError::Refused(status));
    }
    if (got_width, got_height) != (width, height) {
        return Err(ProtocolError::WrongSize);
    }
    let mut alpha = vec![0u8; width as usize * height as usize];
    input.read_exact(&mut alpha)?;
    Matte::new(width as usize, height as usize, alpha).ok_or(ProtocolError::WrongSize)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u32, width: u32, height: u32, body: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for value in [MAGIC, status, width, height] {
            bytes.extend(word(value));
        }
        bytes.extend(body);
        bytes
    }

    #[test]
    fn a_request_is_the_header_followed_by_the_pixels() {
        let mut sent = Vec::new();
        write_request(&mut sent, 2, 1, &[1, 2, 3, 4, 5, 6]).unwrap();
        assert_eq!(&sent[..4], b"LCM1");
        assert_eq!(&sent[4..8], &2u32.to_le_bytes());
        assert_eq!(&sent[8..12], &1u32.to_le_bytes());
        assert_eq!(&sent[12..], &[1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn frames_that_the_server_would_refuse_are_never_sent() {
        let mut sink = Vec::new();
        assert!(matches!(
            write_request(&mut sink, 0, 1, &[]),
            Err(ProtocolError::BadFrame)
        ));
        assert!(matches!(
            write_request(&mut sink, 2, 1, &[0; 5]),
            Err(ProtocolError::BadFrame)
        ));
        assert!(matches!(
            write_request(&mut sink, MAX_SIDE + 1, 1, &[]),
            Err(ProtocolError::BadFrame)
        ));
        assert!(sink.is_empty());
    }

    #[test]
    fn a_good_response_becomes_a_matte() {
        let bytes = response(0, 2, 2, &[0, 255, 128, 7]);
        let matte = read_response(&mut bytes.as_slice(), 2, 2).unwrap();
        assert_eq!(matte.alpha(), &[0, 255, 128, 7]);
    }

    #[test]
    fn bad_responses_are_told_apart() {
        let mut wrong_magic = response(0, 1, 1, &[0]);
        wrong_magic[0] = b'X';
        assert!(matches!(
            read_response(&mut wrong_magic.as_slice(), 1, 1),
            Err(ProtocolError::BadMagic)
        ));
        assert!(matches!(
            read_response(&mut response(1, 0, 0, &[]).as_slice(), 1, 1),
            Err(ProtocolError::Refused(1))
        ));
        assert!(matches!(
            read_response(&mut response(0, 3, 3, &[0; 9]).as_slice(), 2, 2),
            Err(ProtocolError::WrongSize)
        ));
        assert!(matches!(
            read_response(&mut response(0, 2, 2, &[0; 3]).as_slice(), 2, 2),
            Err(ProtocolError::Io(_))
        ));
    }
}

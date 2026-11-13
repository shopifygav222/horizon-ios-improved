//! Checksums and the text forms of bytes: SHA-256, HMAC-SHA256 (RFC 2104), hex, and base64 in
//! the URL alphabet without padding (RFC 4648 section 5).
use crate::{Error, Result};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

pub type Hash = [u8; 32];

pub fn sha256(data: &[u8]) -> Hash {
    Sha256::digest(data).into()
}

/// A file's SHA-256 and length, read in pieces.
pub fn sha256_file(path: &Path) -> std::io::Result<(Hash, u64)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    let mut length = 0u64;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        length += n as u64;
    }
    Ok((hasher.finalize().into(), length))
}

pub fn hmac_sha256(key: &[u8], data: &[u8]) -> Hash {
    const BLOCK: usize = 64;
    let mut padded = [0u8; BLOCK];
    if key.len() > BLOCK {
        padded[..32].copy_from_slice(&sha256(key));
    } else {
        padded[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(padded.map(|b| b ^ 0x36));
    inner.update(data);
    let mut outer = Sha256::new();
    outer.update(padded.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        text.push(DIGITS[usize::from(b >> 4)] as char);
        text.push(DIGITS[usize::from(b & 15)] as char);
    }
    text
}

pub fn unhex(text: &str) -> Option<Vec<u8>> {
    let digits = text.as_bytes();
    if digits.len() & 1 == 1 {
        return None;
    }
    let value = |d: u8| (d as char).to_digit(16).map(|v| v as u8);
    (0..digits.len() / 2)
        .map(|i| Some(value(digits[2 * i])? << 4 | value(digits[2 * i + 1])?))
        .collect()
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn base64(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let n = group.iter().fold(0u32, |n, b| n << 8 | u32::from(*b)) << (8 * (3 - group.len()));
        for i in 0..=group.len() {
            text.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    text
}

pub fn unbase64(text: &str) -> Result<Vec<u8>> {
    let bad = || Error::Format("not base64".into());
    let symbols = text.as_bytes();
    if symbols.len() % 4 == 1 {
        return Err(bad());
    }
    let mut bytes = Vec::with_capacity(symbols.len() / 4 * 3 + 2);
    for group in symbols.chunks(4) {
        let mut n = 0u32;
        for symbol in group {
            let value = ALPHABET.iter().position(|a| a == symbol).ok_or_else(bad)?;
            n = n << 6 | value as u32;
        }
        n <<= 6 * (4 - group.len());
        // The bits past the last whole byte are zero in anything `base64` wrote.
        if n & ((1 << (8 * (4 - group.len()))) - 1) != 0 {
            return Err(bad());
        }
        for i in 0..group.len() - 1 {
            bytes.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Ok(bytes)
}

/// `N` bytes from the system random generator.
pub fn random<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|e| Error::Store(format!("no random bytes: {e}")))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_standard_vector() {
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hmac_matches_rfc_4231() {
        assert_eq!(
            hex(&hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // A key longer than a block is hashed first (test case 6).
        assert_eq!(
            hex(&hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn hex_and_base64_round_trip_every_length() {
        let bytes: Vec<u8> = (0..=255u8).collect();
        for length in 0..bytes.len() {
            let piece = &bytes[..length];
            assert_eq!(unhex(&hex(piece)).unwrap(), piece);
            assert_eq!(unbase64(&base64(piece)).unwrap(), piece);
        }
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(b"fo"), "Zm8");
        assert_eq!(base64(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn damaged_text_is_refused() {
        assert!(unhex("abc").is_none());
        assert!(unhex("zz").is_none());
        assert!(unbase64("Zm9vY").is_err());
        assert!(unbase64("Zm9v*mFy").is_err());
        // Bits set past the last byte: not something `base64` writes.
        assert!(unbase64("Zm9").is_err());
    }

    #[test]
    fn a_file_hashes_as_its_bytes() {
        let dir = std::env::temp_dir().join(format!("hsw-codec-{}", hex(&random::<8>().unwrap())));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f");
        let data: Vec<u8> = (0..200_000u32).map(|i| (i * 31 % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            (sha256(&data), data.len() as u64)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

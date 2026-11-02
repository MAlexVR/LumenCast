//! Verification of a downloaded update, failing closed: the file's SHA-256
//! must equal its entry in `SHA256SUMS.txt`, and its minisign signature must
//! verify against the public key built into the program. Nothing here runs
//! anything.

use base64::Engine;
use minisign_verify::{PublicKey, Signature};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

/// The public key of the update signatures: the base64 of the minisign public
/// key file exactly as `tauri signer generate` writes it. The private key
/// stays with the maintainer, outside the repository.
pub const UPDATE_PUBKEY: &str = include_str!("../update.pub");

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    #[error("checksum mismatch: {0}")]
    ChecksumMismatch(String),
    #[error("invalid signature: {0}")]
    SignatureInvalid(String),
    #[error("cannot read the file: {0}")]
    Io(String),
}

const CHUNK: usize = 64 * 1024;

fn is_sha256(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `<hash>  <name>` lines (also `<hash> *<name>`), hashes lowercased.
pub fn parse_sha256sums(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_end_matches('\r');
            let (hash, rest) = line.split_once(' ')?;
            let name = rest.trim_start_matches([' ', '*']);
            (is_sha256(hash) && !name.is_empty())
                .then(|| (name.to_owned(), hash.to_ascii_lowercase()))
        })
        .collect()
}

pub fn sha256_file(path: &Path) -> Result<String, VerifyError> {
    let mut file = std::fs::File::open(path).map_err(|e| VerifyError::Io(e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| VerifyError::Io(e.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// The file called `name` must hash to its entry in `sums`; no entry fails.
pub fn verify_checksum(path: &Path, name: &str, sums: &str) -> Result<(), VerifyError> {
    let expected = parse_sha256sums(sums)
        .remove(name)
        .ok_or_else(|| VerifyError::ChecksumMismatch(format!("no checksum entry for {name}")))?;
    if sha256_file(path)? == expected {
        Ok(())
    } else {
        Err(VerifyError::ChecksumMismatch(format!(
            "{name} does not match its checksum"
        )))
    }
}

fn decode_text(b64: &str, what: &str) -> Result<String, VerifyError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|_| VerifyError::SignatureInvalid(format!("the {what} is not valid base64")))?;
    String::from_utf8(bytes)
        .map_err(|_| VerifyError::SignatureInvalid(format!("the {what} is not text")))
}

/// Streams the whole file through the minisign verifier. Both inputs are the
/// base64 form `tauri signer` produces (base64 of the minisign file text).
pub fn verify_signature(
    pubkey_b64: &str,
    signature_b64: &str,
    path: &Path,
) -> Result<(), VerifyError> {
    let invalid = |m: &str| VerifyError::SignatureInvalid(m.to_owned());
    let key = PublicKey::decode(&decode_text(pubkey_b64, "public key")?)
        .map_err(|_| invalid("the public key is not valid"))?;
    let signature = Signature::decode(&decode_text(signature_b64, "signature")?)
        .map_err(|_| invalid("the signature is not valid"))?;
    let mut verifier = key
        .verify_stream(&signature)
        .map_err(|_| invalid("the signature does not belong to this key"))?;
    let mut file = std::fs::File::open(path).map_err(|e| VerifyError::Io(e.to_string()))?;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| VerifyError::Io(e.to_string()))?;
        if n == 0 {
            break;
        }
        verifier.update(&buf[..n]);
    }
    verifier
        .finalize()
        .map_err(|_| invalid("the file does not match its signature"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_PUB: &str = include_str!("../tests/fixtures/test.pub");
    const OTHER_PUB: &str = include_str!("../tests/fixtures/other.pub");
    const SIG: &str = include_str!("../tests/fixtures/payload.bin.sig");

    fn payload() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/payload.bin")
    }

    #[test]
    fn the_production_key_is_a_valid_minisign_public_key() {
        let text = decode_text(UPDATE_PUBKEY, "key").unwrap();
        assert!(text.starts_with("untrusted comment: minisign public key"));
        assert!(PublicKey::decode(&text).is_ok());
    }

    #[test]
    fn a_genuine_signature_verifies_and_every_tamper_fails_closed() {
        verify_signature(GOOD_PUB.trim(), SIG.trim(), &payload()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let tampered = dir.path().join("t.bin");
        let mut bytes = std::fs::read(payload()).unwrap();
        bytes.push(b'!');
        std::fs::write(&tampered, &bytes).unwrap();
        let rejected = |key: &str, signature: &str, file: &Path| {
            matches!(
                verify_signature(key, signature, file),
                Err(VerifyError::SignatureInvalid(_))
            )
        };
        assert!(
            rejected(GOOD_PUB.trim(), SIG.trim(), &tampered),
            "changed file"
        );
        assert!(
            rejected(OTHER_PUB.trim(), SIG.trim(), &payload()),
            "another key"
        );
        assert!(rejected("not base64!", SIG.trim(), &payload()));
        assert!(rejected(GOOD_PUB.trim(), "AAAA", &payload()));
        assert!(matches!(
            verify_signature(GOOD_PUB.trim(), SIG.trim(), &dir.path().join("missing")),
            Err(VerifyError::Io(_))
        ));
    }

    #[test]
    fn checksums_are_read_from_the_sums_file_and_must_match() {
        let hash = sha256_file(&payload()).unwrap();
        let sums = format!("{hash}  payload.bin\r\n{}  *other.bin\n", "0".repeat(64));
        verify_checksum(&payload(), "payload.bin", &sums).unwrap();
        assert!(verify_checksum(&payload(), "other.bin", &sums).is_err());
        assert!(verify_checksum(&payload(), "missing.bin", &sums).is_err());
        assert!(verify_checksum(&payload(), "payload.bin", "garbage").is_err());
        assert_eq!(parse_sha256sums(&sums).len(), 2);
    }

    /// Run after `scripts/release.sh`: every file of the release folder must
    /// verify against the key the program carries, and against its checksum.
    ///
    ///   LUMENCAST_RELEASE_DIR=$PWD/target/release/release-1.0.0 \
    ///       cargo test -p lumencast-update -- --ignored --nocapture
    #[test]
    #[ignore = "needs a release folder made by scripts/release.sh"]
    fn the_release_files_verify_against_the_key_built_into_the_program() {
        let dir = std::env::var("LUMENCAST_RELEASE_DIR").expect("set LUMENCAST_RELEASE_DIR");
        let sums = std::fs::read_to_string(Path::new(&dir).join("SHA256SUMS.txt")).unwrap();
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name.ends_with(".sig") || name == "SHA256SUMS.txt" {
                continue;
            }
            let signature = std::fs::read_to_string(format!("{}.sig", path.display())).unwrap();
            verify_signature(UPDATE_PUBKEY.trim(), signature.trim(), &path).unwrap();
            verify_checksum(&path, &name, &sums).unwrap();
            println!("{name}: signature and checksum are good");
            checked += 1;
        }
        assert!(checked > 0, "no packages in {dir}");
    }
}

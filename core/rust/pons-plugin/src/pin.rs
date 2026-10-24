use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginPin {
    path: PathBuf,
    sha256: [u8; 32],
}

impl PluginPin {
    pub fn new(path: PathBuf, checksum: &str) -> Result<Self, String> {
        let sha256 = parse_checksum(checksum)?;
        Ok(Self { path, sha256 })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn verify(&self) -> Result<(), String> {
        let bytes = std::fs::read(&self.path).map_err(|error| {
            format!(
                "pinned plugin {} cannot be read: {error}",
                self.path.display()
            )
        })?;
        let actual: [u8; 32] = Sha256::digest(&bytes).into();
        if actual != self.sha256 {
            return Err(format!(
                "pinned plugin {} has checksum {}, expected {}",
                self.path.display(),
                hex(&actual),
                hex(&self.sha256)
            ));
        }
        Ok(())
    }
}

fn parse_checksum(checksum: &str) -> Result<[u8; 32], String> {
    if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "sha256 is {checksum:?}; it is 64 hexadecimal characters"
        ));
    }
    let mut digest = [0; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        let high = hex_value(checksum.as_bytes()[index * 2]);
        let low = hex_value(checksum.as_bytes()[index * 2 + 1]);
        *byte = high << 4 | low;
    }
    Ok(digest)
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => unreachable!("the checksum was checked as hexadecimal"),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

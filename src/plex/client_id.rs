//! The id Plex keys a playback session by: random, stored beside the config.

use std::io::Read;
use std::path::Path;

const FILE: &str = "client-id";

fn valid(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn generate() -> std::io::Result<String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The stored id, or a new one written to `dir` when the file is missing or
/// does not hold one.
pub fn load_or_create(dir: &Path) -> std::io::Result<String> {
    let path = dir.join(FILE);
    if let Ok(text) = std::fs::read_to_string(&path) {
        let id = text.trim();
        if valid(id) {
            return Ok(id.to_string());
        }
    }
    let id = generate()?;
    crate::cache::write_private_atomic(&path, id.as_bytes())?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn creates_a_private_hex_id_and_returns_it_again() {
        let dir = tempfile::tempdir().unwrap();
        let id = load_or_create(dir.path()).unwrap();
        assert!(valid(&id), "{id}");
        let mode = std::fs::metadata(dir.path().join(FILE)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(load_or_create(dir.path()).unwrap(), id);
    }

    #[test]
    fn a_file_with_other_content_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), "not an id").unwrap();
        let id = load_or_create(dir.path()).unwrap();
        assert!(valid(&id), "{id}");
        assert_eq!(std::fs::read_to_string(dir.path().join(FILE)).unwrap(), id);
    }
}

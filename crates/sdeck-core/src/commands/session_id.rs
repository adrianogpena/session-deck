//! Session ids are plain UUID-shaped tokens. Enforced wherever one is interpolated into a terminal
//! command or built into a filesystem path, so a malformed id from an untrusted or corrupted source
//! fails loudly instead of being used as-is.

fn is_safe(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

pub fn is_safe_session_id(session_id: &str) -> bool {
    is_safe(session_id)
}

pub fn assert_safe_session_id(session_id: &str) -> Result<&str, String> {
    if is_safe(session_id) {
        Ok(session_id)
    } else {
        Err(format!(
            "Refusing to use an unexpected session id: {session_id:?}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_uuid_shaped_ids() {
        assert!(is_safe_session_id("3f2c9a1e-8b7d-4c11-9e55-0a1b2c3d4e5f"));
        assert_eq!(assert_safe_session_id("abc-123"), Ok("abc-123"));
    }

    #[test]
    fn rejects_empty_traversal_and_shell_metacharacters() {
        for bad in ["", "..", "a/b", "a\\b", "a b", "a;b", "a$(x)", "\u{e9}"] {
            assert!(!is_safe_session_id(bad), "{bad:?}");
            assert!(assert_safe_session_id(bad).is_err());
        }
    }

    #[test]
    fn rejects_a_trailing_newline() {
        assert!(!is_safe_session_id("abc\n"));
    }
}

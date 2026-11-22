//! How a secret too large for one credential is laid out (ADR-025).

/// Bytes per piece. Windows caps a credential blob at 2560 bytes
/// (`CRED_MAX_CREDENTIAL_BLOB_SIZE`); a round number leaves headroom.
pub(crate) const CHUNK_BYTES: usize = 2048;

const PREFIX: &str = "senvakara-chunks:";

/// The text stored under the reference of a split secret.
pub(crate) fn manifest(count: usize) -> String {
    format!("{PREFIX}{count}")
}

/// Number of pieces if `text` is a manifest, `None` for a plain secret.
pub(crate) fn parse_manifest(text: &str) -> Option<usize> {
    text.strip_prefix(PREFIX)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_reads_back_its_count() {
        assert_eq!(parse_manifest(&manifest(3)), Some(3));
        assert_eq!(parse_manifest(&manifest(0)), Some(0));
    }

    #[test]
    fn ordinary_secrets_are_not_mistaken_for_a_manifest() {
        assert_eq!(parse_manifest("hunter2"), None);
        assert_eq!(parse_manifest("senvakara-chunks:"), None);
        assert_eq!(parse_manifest("senvakara-chunks:x"), None);
    }
}

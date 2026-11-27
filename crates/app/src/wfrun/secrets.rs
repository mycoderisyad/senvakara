//! Secret settings in a run (ADR-033): read from Credential Manager right
//! before the node runs, handed to the launcher on stdin (never the script
//! file, argv, the database or a log), and masked in everything the run
//! shows or writes.

use super::RunCtx;
use crate::state::CmdResult;
use senvakara_adapters_ssh::Secret;
use senvakara_core::{CoreError, ErrorCode};
use senvakara_workflow::{secret_block, secret_refs, CheckedNode};

/// Like the plain launcher (ADR-030), but first reads the secret block from
/// stdin: lines before its marker (a password sudo did not ask for) are
/// dropped, each `SV_<NAME> <base64>` line is decoded and exported, and the
/// script still starts with stdin from /dev/null. `$1` pidfile, `$2`
/// script, `$3` marker nonce. Reviewed text, not built from input.
pub(super) const LAUNCHER_WITH_SECRETS: &str = r#"echo $$ > "$1" || exit 97
while IFS= read -r line; do [ "$line" = "SV-SECRETS $3" ] && break; done
while IFS=' ' read -r name value; do
  [ "$name" = "SV-SECRETS-END" ] && [ "$value" = "$3" ] && break
  case $name in SV_*) ;; *) echo "bad secret block" >&2; exit 98;; esac
  case ${name#SV_} in ''|*[!A-Z0-9_]*) echo "bad secret name" >&2; exit 98;; esac
  v=$(printf '%s' "$value" | base64 -d && printf x) || { echo "bad secret value" >&2; exit 98; }
  export "$name=${v%x}"
done
unset line name value v
exec bash -euo pipefail "$2" </dev/null"#;

/// Shortest value masked in logs: shorter ones would blank out ordinary
/// words.
const MIN_MASK: usize = 4;

/// A node's secrets as the launcher receives them.
pub(super) struct NodeSecrets {
    /// The stdin block, `None` when the node has no secret setting.
    pub block: Option<Secret>,
    pub nonce: String,
}

/// Reads every secret setting of `node` from Credential Manager and
/// registers the values for masking. A reference that is gone fails the step
/// before anything runs.
pub(super) async fn resolve(ctx: &RunCtx, node: &CheckedNode) -> CmdResult<NodeSecrets> {
    let nonce = uuid::Uuid::new_v4().to_string();
    let refs = secret_refs(&node.params);
    if refs.is_empty() {
        return Ok(NodeSecrets { block: None, nonce });
    }
    let app = ctx.app.clone();
    let values = ctx
        .app
        .rt
        .spawn_blocking(move || -> CmdResult<Vec<(String, String)>> {
            let store = app.state.secrets.as_ref().ok_or_else(|| {
                CoreError::new(
                    ErrorCode::UnsupportedCapability,
                    "credential store unavailable",
                )
            })?;
            refs.into_iter()
                .map(|(name, reference)| {
                    let value = store
                        .get(&reference)
                        .map_err(|e| crate::state::to_core("read a secret setting", e.into()))?
                        .ok_or_else(|| {
                            CoreError::new(
                                ErrorCode::InvalidInput,
                                format!(
                                    "the secret for `{name}` is no longer in Credential Manager: enter it again"
                                ),
                            )
                        })?;
                    Ok((name, value))
                })
                .collect()
        })
        .await
        .map_err(|e| CoreError::internal(format!("background task failed: {e}")))??;
    for (_, value) in &values {
        ctx.mask(value);
    }
    let block =
        secret_block(&values, &nonce).map_err(|e| CoreError::new(ErrorCode::InvalidInput, e))?;
    Ok(NodeSecrets {
        block: Some(Secret::new(block)),
        nonce,
    })
}

/// Longest secret setting accepted: one credential entry holds 2,560 bytes
/// (ADR-025), and a token or password is far shorter.
const MAX_SECRET_CHARS: usize = 1024;

fn store(state: &crate::AppState) -> CmdResult<&senvakara_secrets::SecretStore> {
    state.secrets.as_ref().ok_or_else(|| {
        CoreError::new(
            ErrorCode::UnsupportedCapability,
            "credential store unavailable",
        )
    })
}

/// Stores a secret setting typed by the user; returns the reference the
/// graph keeps. Blocking: run it off the UI thread.
pub(crate) fn secret_put(state: &crate::AppState, value: &str) -> CmdResult<String> {
    if value.is_empty() || value.contains('\0') || value.chars().count() > MAX_SECRET_CHARS {
        return Err(CoreError::new(
            ErrorCode::InvalidInput,
            format!("a secret setting holds 1 to {MAX_SECRET_CHARS} characters"),
        ));
    }
    store(state)?
        .put(value)
        .map_err(|e| crate::state::to_core("save a secret setting", e.into()))
}

/// Deletes a stored secret setting. A reference that is already gone is
/// fine. Blocking: run it off the UI thread.
pub(crate) fn secret_delete(state: &crate::AppState, reference: &str) -> CmdResult<()> {
    if !reference.starts_with("secret-") {
        return Err(CoreError::new(
            ErrorCode::InvalidInput,
            "not a secret reference",
        ));
    }
    store(state)?
        .delete(reference)
        .map_err(|e| crate::state::to_core("delete a secret setting", e.into()))
}

/// The pieces of a secret worth masking: the whole value and each of its
/// lines, longest first so a longer match is never cut by a shorter one.
pub(crate) fn mask_parts(value: &str) -> Vec<String> {
    let mut parts: Vec<String> = std::iter::once(value)
        .chain(value.lines())
        .map(str::trim_end)
        .filter(|p| p.chars().count() >= MIN_MASK)
        .map(str::to_string)
        .collect();
    parts.sort_by_key(|p| std::cmp::Reverse(p.len()));
    parts.dedup();
    parts
}

/// `text` with every mask replaced by `[secret]`.
pub(crate) fn apply_masks(text: &str, masks: &[String]) -> String {
    let mut out = text.to_string();
    for m in masks {
        if out.contains(m.as_str()) {
            out = out.replace(m.as_str(), "[secret]");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_and_their_lines_are_masked_but_short_values_are_not() {
        let mut masks = mask_parts("hunter2-long\nsecond-line");
        masks.extend(mask_parts("abc"));
        assert_eq!(masks[0], "hunter2-long\nsecond-line");
        assert!(!masks.iter().any(|m| m == "abc"));
        assert_eq!(
            apply_masks("pw=hunter2-long then second-line", &masks),
            "pw=[secret] then [secret]"
        );
        assert_eq!(apply_masks("abc stays", &masks), "abc stays");
    }

    #[test]
    fn the_launcher_reads_only_its_own_block() {
        // The marker check is what keeps a password line sudo did not read
        // from being taken as a secret.
        assert!(LAUNCHER_WITH_SECRETS.contains(r#"[ "$line" = "SV-SECRETS $3" ] && break"#));
        assert!(LAUNCHER_WITH_SECRETS.ends_with(r#"exec bash -euo pipefail "$2" </dev/null"#));
    }
}

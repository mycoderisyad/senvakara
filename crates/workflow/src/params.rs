//! Parameter values: checked against the module's schema, then handed to a
//! script as `export SV_<NAME>='…'` lines. No template engine: values never
//! become script text, only single-quoted shell words.

use crate::module::{ParamKind, ParamSpec};
use serde_json::Value;
use std::collections::BTreeMap;

const DEFAULT_TEXT_LEN: usize = 256;
const DEFAULT_MULTILINE_LEN: usize = 4096;
const MAX_PATH_LEN: usize = 4096;

/// A checked value, ready for a script or an executor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamValue {
    Text(String),
    Int(i64),
    Bool(bool),
    /// Credential Manager reference; the app resolves it right before a run.
    SecretRef(String),
}

/// Checks every parameter of a node against its specs: unknown names are
/// refused, missing ones take the default, required ones must be present.
/// Returns the values by name, or the first problem as a sentence.
pub fn check_params(
    specs: &[ParamSpec],
    given: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, ParamValue>, String> {
    if let Some(unknown) = given.keys().find(|k| !specs.iter().any(|s| &s.name == *k)) {
        return Err(format!("unknown setting `{unknown}`"));
    }
    let mut out = BTreeMap::new();
    for spec in specs {
        let value = given
            .get(&spec.name)
            .filter(|v| !is_blank(v))
            .or(spec.default.as_ref());
        match value {
            Some(v) => {
                out.insert(spec.name.clone(), check_one(spec, v)?);
            }
            None if spec.required => return Err(format!("{} is required", spec.label)),
            None => {}
        }
    }
    Ok(out)
}

fn is_blank(v: &Value) -> bool {
    v.is_null() || v.as_str().is_some_and(|s| s.trim().is_empty())
}

fn text<'a>(spec: &ParamSpec, v: &'a Value) -> Result<&'a str, String> {
    v.as_str()
        .ok_or_else(|| format!("{} must be text", spec.label))
}

fn single_line(spec: &ParamSpec, s: &str, max: usize) -> Result<(), String> {
    if s.contains(['\n', '\r', '\0']) {
        return Err(format!("{} must be one line", spec.label));
    }
    if s.len() > max {
        return Err(format!("{} is longer than {max} characters", spec.label));
    }
    Ok(())
}

fn check_one(spec: &ParamSpec, v: &Value) -> Result<ParamValue, String> {
    let label = &spec.label;
    match spec.kind {
        ParamKind::Text => {
            let s = text(spec, v)?.trim();
            single_line(spec, s, spec.max_len.unwrap_or(DEFAULT_TEXT_LEN))?;
            Ok(ParamValue::Text(s.into()))
        }
        ParamKind::Multiline => {
            let s = text(spec, v)?;
            let max = spec.max_len.unwrap_or(DEFAULT_MULTILINE_LEN);
            if s.contains('\0') || s.len() > max {
                return Err(format!("{label} is longer than {max} characters"));
            }
            Ok(ParamValue::Text(s.replace("\r\n", "\n")))
        }
        ParamKind::Int | ParamKind::Port => {
            let n = v
                .as_i64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
                .ok_or_else(|| format!("{label} must be a whole number"))?;
            let (min, max) = match spec.kind {
                ParamKind::Port => (1, 65535),
                _ => (spec.min.unwrap_or(i64::MIN), spec.max.unwrap_or(i64::MAX)),
            };
            if !(min..=max).contains(&n) {
                return Err(format!("{label} must be between {min} and {max}"));
            }
            Ok(ParamValue::Int(n))
        }
        ParamKind::Bool => v
            .as_bool()
            .map(ParamValue::Bool)
            .ok_or_else(|| format!("{label} must be on or off")),
        ParamKind::Choice => {
            let s = text(spec, v)?.trim();
            if spec.options.iter().any(|o| o == s) {
                Ok(ParamValue::Text(s.into()))
            } else {
                Err(format!(
                    "{label} must be one of: {}",
                    spec.options.join(", ")
                ))
            }
        }
        ParamKind::Path => {
            let s = text(spec, v)?.trim();
            single_line(spec, s, MAX_PATH_LEN)?;
            if let Some(suffix) = &spec.suffix {
                if !s
                    .to_ascii_lowercase()
                    .ends_with(&suffix.to_ascii_lowercase())
                {
                    return Err(format!("{label} must point to a {suffix} file"));
                }
            }
            Ok(ParamValue::Text(s.into()))
        }
        ParamKind::Package => {
            let s = text(spec, v)?.trim();
            if valid_package(s) {
                Ok(ParamValue::Text(s.into()))
            } else {
                Err(format!(
                    "{label} is not a valid package name (for example com.example.app)"
                ))
            }
        }
        ParamKind::UnixUser => {
            let s = text(spec, v)?.trim();
            let ok = s.len() <= 32
                && s.chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
            if ok {
                Ok(ParamValue::Text(s.into()))
            } else {
                Err(format!(
                    "{label} must be a Linux user name (lowercase letters, digits, - and _)"
                ))
            }
        }
        ParamKind::Hostname => {
            let s = text(spec, v)?.trim();
            let ok = !s.is_empty()
                && s.len() <= 253
                && s.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && !label.starts_with('-')
                        && !label.ends_with('-')
                        && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                });
            if ok {
                Ok(ParamValue::Text(s.to_ascii_lowercase()))
            } else {
                Err(format!(
                    "{label} must be a host name such as app.example.com"
                ))
            }
        }
        ParamKind::Secret => {
            let s = text(spec, v)?.trim();
            if s.starts_with("secret-") && single_line(spec, s, 128).is_ok() {
                Ok(ParamValue::SecretRef(s.into()))
            } else {
                Err(format!(
                    "{label} must be stored in Credential Manager first"
                ))
            }
        }
    }
}

/// Android application id, as accepted by the device actions.
pub fn valid_package(name: &str) -> bool {
    name.contains('.')
        && name.len() <= 255
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

/// POSIX single-quoted word: safe for any byte except NUL, which the
/// validators above already refuse.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The lines a script starts with: strict mode, then one export per value.
/// `secrets` maps a secret reference to its resolved value; a reference
/// without one is an error so a run never starts half-configured.
pub fn preamble(
    values: &BTreeMap<String, ParamValue>,
    secrets: &BTreeMap<String, String>,
) -> Result<String, String> {
    let mut out = String::from("set -euo pipefail\n");
    for (name, value) in values {
        let word = match value {
            ParamValue::Text(s) => shell_quote(s),
            ParamValue::Int(n) => n.to_string(),
            ParamValue::Bool(b) => if *b { "1" } else { "0" }.to_string(),
            ParamValue::SecretRef(r) => {
                let s = secrets
                    .get(r)
                    .ok_or_else(|| format!("the secret for `{name}` is missing"))?;
                if s.contains('\0') {
                    return Err(format!("the secret for `{name}` holds a NUL byte"));
                }
                shell_quote(s)
            }
        };
        out.push_str(&format!("export SV_{}={word}\n", name.to_ascii_uppercase()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests;

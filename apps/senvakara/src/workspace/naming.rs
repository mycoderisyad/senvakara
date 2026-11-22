//! Names: the check on a workspace name and the tmux session name of a panel.

const NAME_LIMIT: usize = 80;

/// Why the name cannot be saved, or "" when it can.
pub fn name_problem(name: &str) -> &'static str {
    let name = name.trim();
    if name.is_empty() {
        "Give the workspace a name to save it."
    } else if name.len() > NAME_LIMIT {
        "The name can be at most 80 characters."
    } else {
        ""
    }
}

/// `senvakara-<base36 time>`: the tmux session name v0.1.0 used.
pub fn tmux_name(millis: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut value = millis;
    let mut out = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    if out.is_empty() {
        out.push(b'0');
    }
    out.reverse();
    format!("senvakara-{}", String::from_utf8(out).unwrap_or_default())
}

pub(super) fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_checked() {
        assert!(name_problem("  ").contains("name"));
        assert!(name_problem(&"x".repeat(81)).contains("80"));
        assert_eq!(name_problem(" Bench "), "");
    }

    #[test]
    fn tmux_names_match_v010() {
        // (1732200000000).toString(36) in JavaScript.
        assert_eq!(tmux_name(1_732_200_000_000), "senvakara-m3rf5m2o");
        assert_eq!(tmux_name(0), "senvakara-0");
    }
}

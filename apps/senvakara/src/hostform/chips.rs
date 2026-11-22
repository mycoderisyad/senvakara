//! The chips of the host form: packing tags into lines, and which groups to suggest.

/// The width chips may fill: the inspector (340) less its padding and the card's.
const LINE_WIDTH: f32 = 276.0;
/// Space between two chips on a line.
const GAP: f32 = 4.0;
/// A chip's padding, and the remove button of a chip that has one.
const CHIP_PADDING: f32 = 18.0;
const REMOVE_BUTTON: f32 = 20.0;
/// An average character of the small interface face. Wide text is cut with an ellipsis by the
/// chip itself, so an estimate that is a little short does no harm.
const CHAR_WIDTH: f32 = 7.0;
/// Groups offered under the Parent Group field.
const MAX_SUGGESTIONS: usize = 6;

fn chip_width(label: &str, removable: bool) -> f32 {
    let button = if removable { REMOVE_BUTTON } else { 0.0 };
    label.chars().count() as f32 * CHAR_WIDTH + CHIP_PADDING + button
}

/// Pack chips into lines, in order: each line holds the positions (in `labels`) of its chips.
/// The view cannot measure a wrapped row of chips, so the lines are decided here.
pub fn lines(labels: &[String], removable: bool) -> Vec<Vec<usize>> {
    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut used = 0.0;
    for (index, label) in labels.iter().enumerate() {
        let width = chip_width(label, removable);
        match lines.last_mut() {
            Some(line) if used + GAP + width <= LINE_WIDTH => {
                line.push(index);
                used += GAP + width;
            }
            _ => {
                lines.push(vec![index]);
                used = width;
            }
        }
    }
    lines
}

/// The groups the hosts are filed under, each once (ignoring case, first spelling wins), by
/// name.
pub fn known_groups<'a>(groups: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut known: Vec<String> = Vec::new();
    for group in groups.map(str::trim).filter(|group| !group.is_empty()) {
        if !known.iter().any(|have| have.eq_ignore_ascii_case(group)) {
            known.push(group.to_string());
        }
    }
    known.sort_by_key(|group| group.to_lowercase());
    known
}

/// The existing groups to offer for what is typed in the Parent Group field: those that
/// contain it, ignoring case, without the one already typed in full.
pub fn group_suggestions(known: &[String], typed: &str) -> Vec<String> {
    let typed = typed.trim().to_lowercase();
    known
        .iter()
        .filter(|group| {
            let group = group.to_lowercase();
            group != typed && group.contains(&typed)
        })
        .take(MAX_SUGGESTIONS)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(list: &[&str]) -> Vec<String> {
        list.iter().map(|label| label.to_string()).collect()
    }

    #[test]
    fn short_chips_share_a_line_and_long_ones_wrap_in_order() {
        assert!(lines(&[], true).is_empty());
        assert_eq!(lines(&labels(&["web", "eu", "db"]), true), [[0, 1, 2]]);
        let long = "x".repeat(32);
        let packed = lines(&labels(&["web", &long, "eu", "db"]), true);
        assert_eq!(packed, [vec![0], vec![1], vec![2, 3]]);
    }

    #[test]
    fn a_chip_wider_than_the_line_still_gets_a_line_of_its_own() {
        let huge = "w".repeat(64);
        assert_eq!(lines(&labels(&[&huge, "a"]), false), [[0], [1]]);
    }

    #[test]
    fn groups_are_listed_once_by_name_whatever_their_case() {
        let groups = known_groups(["prod", " Lab ", "", "Prod", "lab", "Edge"].into_iter());
        assert_eq!(groups, ["Edge", "Lab", "prod"]);
    }

    #[test]
    fn suggestions_match_what_is_typed_and_leave_out_the_group_itself() {
        let known = labels(&["Edge", "Lab", "Production", "Prototype"]);
        assert_eq!(group_suggestions(&known, ""), known);
        assert_eq!(
            group_suggestions(&known, " pro"),
            ["Production", "Prototype"]
        );
        assert_eq!(group_suggestions(&known, "LAB"), Vec::<String>::new());
        assert_eq!(group_suggestions(&known, "zzz"), Vec::<String>::new());
        let many: Vec<String> = (0..10).map(|n| format!("group{n}")).collect();
        assert_eq!(group_suggestions(&many, "group").len(), 6);
    }
}

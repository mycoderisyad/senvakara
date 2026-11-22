//! The Shell tool: asking about, running and showing one shell command on the phone.

use std::rc::Rc;

use slint::{ComponentHandle, SharedString};

use crate::inventory::{fact, friendly};
use crate::{App, DeviceTools, Pending, Question};

use super::target_facts;

/// Shell output longer than this is cut in the inspector; Copy output still copies all of it.
const OUTPUT_LINES: usize = 200;
const OUTPUT_CHARS: usize = 16_000;

/// The part of a long output the inspector shows, and whether anything was left out.
pub fn shown_output(output: &str) -> (String, bool) {
    let mut shown = String::new();
    for (index, line) in output.lines().enumerate() {
        if index == OUTPUT_LINES || shown.len() + line.len() > OUTPUT_CHARS {
            if shown.is_empty() {
                // One enormous line: show its beginning.
                shown = line.chars().take(OUTPUT_CHARS).collect();
            }
            return (shown, true);
        }
        if index > 0 {
            shown.push('\n');
        }
        shown.push_str(line);
    }
    (shown, false)
}

impl App {
    pub(super) fn shell_ask(self: &Rc<Self>) {
        let (Some(ui), Some(device)) = (self.ui.upgrade(), self.selected_device()) else {
            return;
        };
        let command = ui.global::<DeviceTools>().get_command().trim().to_string();
        if command.is_empty() || device.record.state != "available" {
            return;
        }
        let mut facts = target_facts(&device);
        facts.push(fact("Command", command.as_str(), true));
        self.ask_with(
            Question {
                title: "Run this command on the phone?".into(),
                body: "The command runs with ADB shell permissions, on this phone only.".into(),
                confirm: "Run command".into(),
                danger: false,
            },
            facts,
            "",
            Pending::RunShell {
                route_id: device.record.route_id.clone(),
                serial: device.record.serial.clone(),
                command,
            },
        );
    }

    /// The user confirmed the command for exactly this route and serial.
    pub fn shell_run(self: &Rc<Self>, route_id: String, serial: String, command: String) {
        let Some(ui) = self.tools_of(&route_id) else {
            return;
        };
        let tools = ui.global::<DeviceTools>();
        tools.set_shell_busy(true);
        tools.set_shell_error(SharedString::new());
        let backend = self.backend.clone();
        let route = route_id.clone();
        self.run(
            async move { backend.device_shell(route_id, serial, command).await },
            move |app, result| {
                let Some(ui) = app.tools_of(&route) else {
                    return;
                };
                let tools = ui.global::<DeviceTools>();
                tools.set_shell_busy(false);
                match result {
                    Ok(done) => {
                        let (shown, cut) = shown_output(&done.output);
                        let mut note = format!("Ran on {}, serial {}.", done.host_name, done.serial);
                        if done.truncated {
                            note.push_str(" The output was truncated: the command printed more than Senvakara keeps.");
                        }
                        if cut {
                            note.push_str(" Only the beginning is shown here. Copy output copies everything.");
                        }
                        *app.dev.output.borrow_mut() = done.output;
                        tools.set_shell_note(note.into());
                        tools.set_shell_output(shown.into());
                        tools.set_shell_ran(true);
                    }
                    Err(error) => tools.set_shell_error(friendly(&error).into()),
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_output_is_shown_whole() {
        assert_eq!(shown_output("a\nb\n"), ("a\nb".to_string(), false));
        assert_eq!(shown_output(""), (String::new(), false));
    }

    #[test]
    fn long_output_is_cut_at_a_line() {
        let output: String = (0..OUTPUT_LINES + 50)
            .map(|n| format!("line {n}\n"))
            .collect();
        let (shown, cut) = shown_output(&output);
        assert!(cut);
        assert_eq!(shown.lines().count(), OUTPUT_LINES);
        assert!(shown.ends_with(&format!("line {}", OUTPUT_LINES - 1)));
    }

    #[test]
    fn one_huge_line_is_cut_by_length() {
        let (shown, cut) = shown_output(&"x".repeat(OUTPUT_CHARS * 2));
        assert!(cut);
        assert_eq!(shown.len(), OUTPUT_CHARS);
    }
}

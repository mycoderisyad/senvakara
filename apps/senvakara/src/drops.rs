//! Files dragged from Windows Explorer onto the window. The window hears about them as a whole
//! (`platform::on_file_events`); this module decides which part of the app takes them, shows
//! that part that files are held over it, and hands it the paths of a drop.
//!
//! To add a consumer: give it a [`Target`], name when it is on screen in [`target`], and
//! handle it in `file_drag_show` and `file_drop_deliver`.

use std::rc::Rc;

use slint::ComponentHandle;

use crate::platform::{on_file_events, FileEvent};
use crate::{App, Confirm, Keychain, Nav, Palette, Wireless};

/// The part of the app that takes dropped files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The Keychain section: the first file is imported as a new key.
    Keychain,
    /// The Files tab in front: the files are uploaded to the host or phone it shows.
    Files,
}

/// Who takes files dropped right now, from what is on screen: `on_home` when no session tab is
/// in front, `section` the section of Home, `files` when the tab in front is a Files tab,
/// `modal` when a dialog or the palette covers the window. None means a drop is ignored.
pub fn target(on_home: bool, section: &str, files: bool, modal: bool) -> Option<Target> {
    match section {
        _ if modal => None,
        _ if files => Some(Target::Files),
        _ if !on_home => None,
        "keychain" => Some(Target::Keychain),
        _ => None,
    }
}

impl App {
    pub fn init_file_drops(self: &Rc<Self>, ui: &crate::MainWindow) {
        on_file_events(ui.window(), |event| {
            if let Some(app) = crate::app() {
                app.file_event(event);
            }
        });
    }

    fn file_drop_target(&self) -> Option<Target> {
        let ui = self.ui.upgrade()?;
        let modal = ui.global::<Confirm>().get_open()
            || ui.global::<Palette>().get_open()
            || ui.global::<Wireless>().get_open();
        target(
            self.active.get() < 0,
            &ui.global::<Nav>().get_section(),
            self.files_in_front(),
            modal,
        )
    }

    fn file_event(self: &Rc<Self>, event: FileEvent) {
        match event {
            FileEvent::Hovered => {
                if self.drops.hovered() {
                    self.file_drag_show(true);
                }
            }
            FileEvent::Left => {
                if self.drops.ended() {
                    self.file_drag_show(false);
                }
            }
            FileEvent::Dropped(path) => {
                if self.drops.ended() {
                    self.file_drag_show(false);
                }
                // The other files of this drop follow as events of their own, before the
                // event loop gets to a timer: deliver them all in one go then.
                if self.drops.dropped(path) {
                    let app = self.clone();
                    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                        app.file_drop_deliver()
                    });
                }
            }
        }
    }

    /// Tell whoever would take a drop that files are held over the window, or no longer.
    fn file_drag_show(&self, hovering: bool) {
        let Some(ui) = self.ui.upgrade() else { return };
        // Switched off everywhere, on only where a drop would land.
        let target = self.file_drop_target().filter(|_| hovering);
        ui.global::<Keychain>()
            .set_drop_hover(target == Some(Target::Keychain));
        self.files_explorer_hover(target == Some(Target::Files));
    }

    fn file_drop_deliver(self: &Rc<Self>) {
        let paths = self.drops.take();
        match self.file_drop_target() {
            Some(Target::Keychain) => self.keychain_files_dropped(paths),
            Some(Target::Files) => self.files_explorer_dropped(paths),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_files_go_to_the_keychain_only_while_it_is_showing() {
        assert_eq!(
            target(true, "keychain", false, false),
            Some(Target::Keychain)
        );
        assert_eq!(target(true, "hosts", false, false), None);
        assert_eq!(target(true, "settings", false, false), None);
    }

    #[test]
    fn a_drop_is_ignored_behind_a_session_tab_or_a_dialog() {
        assert_eq!(target(false, "keychain", false, false), None);
        assert_eq!(target(true, "keychain", false, true), None);
    }

    #[test]
    fn dropped_files_go_to_the_files_tab_in_front_unless_a_dialog_covers_it() {
        // Home's section does not matter while a Files tab is in front.
        assert_eq!(target(false, "keychain", true, false), Some(Target::Files));
        assert_eq!(target(false, "hosts", true, false), Some(Target::Files));
        assert_eq!(target(false, "hosts", true, true), None);
    }
}

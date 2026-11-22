//! Input from the session views: terminal, mirror, desktop and logcat callbacks and their panes.

use std::rc::Rc;

use slint::Model;

use crate::{desktop, logcat, mirror, term, App, Desktop, Logcat, Mirror, Pane, Place, Term};

impl App {
    pub(crate) fn with_term<R>(&self, id: i32, f: impl FnOnce(&term::Session) -> R) -> Option<R> {
        self.with_pane(id, |pane, _| match pane {
            Pane::Term(session) => Some(f(session)),
            _ => None,
        })
        .flatten()
    }

    fn with_mirror(&self, id: i32, f: impl FnOnce(&mirror::MirrorPane)) {
        self.with_pane(id, |pane, _| {
            if let Pane::Mirror(pane) = pane {
                f(pane);
            }
        });
    }

    fn with_desktop(&self, id: i32, f: impl FnOnce(&desktop::DesktopPane)) {
        self.with_pane(id, |pane, _| {
            if let Pane::Desktop(pane) = pane {
                f(pane);
            }
        });
    }

    /// Run `f` on a logcat pane, then redraw its status line.
    fn with_logcat(&self, id: i32, f: impl FnOnce(&logcat::LogcatPane)) {
        let Some(ui) = self.ui.upgrade() else { return };
        let found = self.with_pane(id, |pane, place| {
            let Pane::Logcat(pane) = pane else {
                return None;
            };
            f(pane);
            if self.on_screen(place) {
                self.present_logcat(&ui, pane, place, false);
            }
            Some((place, !pane.active()))
        });
        if let Some(Some((Place::Tab(index), exited))) = found {
            if let Some(mut tab) = self.tabs.row_data(index) {
                if tab.exited != exited {
                    tab.exited = exited;
                    self.tabs.set_row_data(index, tab);
                }
            }
        }
    }
}

/// Connect the terminal view: keys, resizing, scrolling and the pointer.
pub(crate) fn wire_terminal(view: &Term<'_>, app: &Rc<App>) {
    view.on_key({
        let app = app.clone();
        move |id, text, control, alt, shift| {
            app.focus_term.set(id);
            app.with_term(id, |session| session.key(&text, control, alt, shift))
                .unwrap_or(false)
        }
    });
    view.on_resized({
        let app = app.clone();
        move |id, cols, lines| {
            let grid = (cols.clamp(2, 1000) as u16, lines.clamp(1, 1000) as u16);
            // A tab's terminal sets the size new terminals start with; a small workspace
            // panel does not.
            if matches!(app.locate(id), Some(Place::Tab(_))) {
                app.grid.set(grid);
            }
            app.with_term(id, |session| session.resize(grid.0, grid.1));
        }
    });
    view.on_scrolled({
        let app = app.clone();
        move |id, lines, col, row, shift, control, alt| {
            let modifiers = term::Modifiers {
                shift,
                alt,
                control,
            };
            app.with_term(id, |session| {
                session.scroll(lines, col.max(0) as usize, row.max(0) as usize, modifiers)
            });
        }
    });
    view.on_pointer({
        let app = app.clone();
        move |id, kind, button, col, row, right_half, shift, control, alt| {
            let event = match kind {
                0 => term::Pointer::Down,
                1 => term::Pointer::Up,
                2 => term::Pointer::Move,
                _ => term::Pointer::Word,
            };
            if event == term::Pointer::Down {
                app.focus_term.set(id);
            }
            let modifiers = term::Modifiers {
                shift,
                alt,
                control,
            };
            app.with_term(id, |session| {
                session.pointer(
                    event,
                    button.clamp(0, 2) as u8,
                    col.max(0) as usize,
                    row.max(0) as usize,
                    right_half,
                    modifiers,
                )
            });
        }
    });
}

/// Connect the mirror view: touch, wheel, keys and the tool buttons.
pub(crate) fn wire_mirror(screen: &Mirror<'_>, app: &Rc<App>) {
    screen.on_touch({
        let app = app.clone();
        move |id, action, x, y| {
            app.with_mirror(id, |pane| pane.touch(action.clamp(0, 2) as u8, x, y))
        }
    });
    screen.on_wheel({
        let app = app.clone();
        move |id, x, y, up| app.with_mirror(id, |pane| pane.wheel(x, y, up))
    });
    screen.on_key({
        let app = app.clone();
        move |id, text, control, alt| {
            let mut handled = false;
            app.with_mirror(id, |pane| handled = pane.key(&text, control, alt));
            handled
        }
    });
    screen.on_tool({
        let app = app.clone();
        move |id, tool| app.with_mirror(id, |pane| pane.tool(&tool))
    });
}

/// Connect the desktop view: pointer, wheel, keys, blur and its two controls.
pub(crate) fn wire_desktop(remote: &Desktop<'_>, app: &Rc<App>) {
    remote.on_pointer({
        let app = app.clone();
        move |id, kind, button, x, y| app.with_desktop(id, |pane| pane.pointer(kind, button, x, y))
    });
    remote.on_wheel({
        let app = app.clone();
        move |id, x, y, up| app.with_desktop(id, |pane| pane.wheel(x, y, up))
    });
    remote.on_key({
        let app = app.clone();
        move |id, text, down| {
            let mut handled = false;
            app.with_desktop(id, |pane| handled = pane.key(&text, down));
            handled
        }
    });
    remote.on_blur({
        let app = app.clone();
        move |id| app.with_desktop(id, |pane| pane.release_all())
    });
    remote.on_set_view_only({
        let app = app.clone();
        move |id, on| {
            app.with_desktop(id, |pane| pane.set_view_only(on));
            // The box follows at once; the picture and state are redrawn as they are.
            app.refresh(id);
        }
    });
    remote.on_ctrl_alt_del({
        let app = app.clone();
        move |id| app.with_desktop(id, |pane| pane.send_ctrl_alt_del())
    });
}

/// Connect the logcat view: start and stop, clear, the filter and follow.
pub(crate) fn wire_logcat(log: &Logcat<'_>, app: &Rc<App>) {
    log.on_toggle({
        let app = app.clone();
        move |id| {
            app.with_logcat(id, |pane| {
                if pane.active() {
                    pane.stop();
                } else {
                    pane.resume();
                }
            })
        }
    });
    log.on_clear({
        let app = app.clone();
        move |id| app.with_logcat(id, |pane| pane.clear())
    });
    log.on_filter_edited({
        let app = app.clone();
        move |id, text| app.with_logcat(id, |pane| pane.set_filter(&text))
    });
    log.on_follow_changed({
        let app = app.clone();
        move |id, follow| {
            app.with_logcat(id, |pane| {
                pane.follow.set(follow);
                if follow {
                    // Jump to the newest line.
                    pane.revision.set(pane.revision.get().wrapping_add(1));
                }
            })
        }
    });
}

//! Platform hooks: the Slint backend the window uses, and files dragged in from Windows
//! Explorer.

/// Pick the Slint backend before the first window exists.
///
/// - Renderer: Slint's software renderer unless `SLINT_BACKEND` names another. With the
///   FemtoVG/OpenGL renderer an idle window cost about 0.5 % of a core in a time-critical
///   thread of the Intel OpenGL driver even with no frames drawn; the software renderer idled
///   at 0.16 % in total (TESTING §9, 2026-11-22). `SLINT_BACKEND=winit-femtovg` brings
///   OpenGL back.
/// - Title bar: always dark (dark theme only, ADR-023), whatever the Windows setting says.
pub(crate) fn select_backend() -> anyhow::Result<()> {
    let mut selector =
        slint::BackendSelector::new().with_winit_window_attributes_hook(|attributes| {
            attributes.with_theme(Some(slint::winit_030::winit::window::Theme::Dark))
        });
    if std::env::var_os("SLINT_BACKEND").is_none() {
        selector = selector
            .backend_name("winit".into())
            .renderer_name("software".into());
    }
    selector.select()?;
    Ok(())
}

/// What a drag of files from Windows Explorer is doing over the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FileEvent {
    /// A file is held over the window. Reported once per file of the drag.
    Hovered,
    /// The files were taken away again without being dropped.
    Left,
    /// One file was dropped. A drop of several files reports each one.
    Dropped(std::path::PathBuf),
}

/// Hear about files dragged onto `window` from Explorer. Slint 1.18.1 has `DropArea` for drags
/// that start inside the app, but its winit backend does not hand drops from other programs
/// to it, so they are read from the winit window events instead. The events go on to Slint
/// untouched. With a backend other than winit nothing is reported.
///
/// `handler` runs on the UI thread, inside the event loop: it should only note the event.
pub(crate) fn on_file_events(window: &slint::Window, handler: impl Fn(FileEvent) + 'static) {
    use slint::winit_030::{winit::event::WindowEvent, EventResult, WinitWindowAccessor};
    window.on_winit_window_event(move |_, event| {
        match event {
            WindowEvent::HoveredFile(_) => handler(FileEvent::Hovered),
            WindowEvent::HoveredFileCancelled => handler(FileEvent::Left),
            WindowEvent::DroppedFile(path) => handler(FileEvent::Dropped(path.clone())),
            _ => {}
        }
        EventResult::Propagate
    });
}

/// Puts the per-file events of one drag back together: whether files are being held over the
/// window, and the paths of one drop.
#[derive(Default)]
pub(crate) struct FileDrops {
    hovering: std::cell::Cell<bool>,
    dropped: std::cell::RefCell<Vec<std::path::PathBuf>>,
}

impl FileDrops {
    /// A file is held over the window. True for the first file of a drag.
    pub(crate) fn hovered(&self) -> bool {
        !self.hovering.replace(true)
    }

    /// The drag ended, by leaving or by dropping. True when one was going on.
    pub(crate) fn ended(&self) -> bool {
        self.hovering.replace(false)
    }

    /// A file was dropped. True for the first file of a drop: the caller then arranges for
    /// [`FileDrops::take`] to run once the rest of the drop has been reported.
    pub(crate) fn dropped(&self, path: std::path::PathBuf) -> bool {
        let mut dropped = self.dropped.borrow_mut();
        dropped.push(path);
        dropped.len() == 1
    }

    /// The files of the drop, in the order they were reported.
    pub(crate) fn take(&self) -> Vec<std::path::PathBuf> {
        std::mem::take(&mut *self.dropped.borrow_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_files_of_one_drop_are_delivered_together_and_only_once() {
        let drops = FileDrops::default();
        assert!(drops.dropped("C:/keys/id_ed25519".into()));
        assert!(!drops.dropped("C:/keys/id_rsa".into()));
        let files = drops.take();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0], std::path::PathBuf::from("C:/keys/id_ed25519"));
        assert!(drops.take().is_empty());
        // The next drop starts a new batch.
        assert!(drops.dropped("C:/keys/other".into()));
    }

    #[test]
    fn a_drag_of_several_files_enters_once_and_ends_once() {
        let drops = FileDrops::default();
        assert!(drops.hovered());
        assert!(!drops.hovered());
        assert!(drops.ended());
        assert!(!drops.ended());
        assert!(drops.hovered());
    }
}

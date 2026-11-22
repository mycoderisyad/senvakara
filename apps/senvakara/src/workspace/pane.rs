//! A workspace tab and its panel slots.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use slint::VecModel;

use crate::{Pane, SlotView};

use super::{Layout, Spec};

// ------------------------------------------------------------------- pane

/// One panel position of a workspace tab.
#[derive(Default)]
pub struct Slot {
    pub spec: Option<Spec>,
    /// The live session, while the panel can run.
    pub pane: Option<Pane>,
    /// Which entry of the picker is chosen while the slot is empty.
    pub(super) choice: usize,
    /// A "Connect host" for this slot is in flight.
    pub(super) connecting: bool,
}

/// Name, layout and one optional panel per slot, as last saved.
type Stored = (String, Layout, Vec<Option<Spec>>);

/// A workspace tab.
pub struct WorkspacePane {
    pub id: i32,
    /// The database id, once saved.
    pub(super) saved_id: RefCell<Option<String>>,
    pub(super) name: RefCell<String>,
    pub(super) layout: Cell<Layout>,
    pub slots: RefCell<Vec<Slot>>,
    /// What the view draws, one row per slot.
    pub view: Rc<VecModel<SlotView>>,
    /// Name, layout and panels as last saved; `None` for a workspace never saved.
    pub(super) stored: RefCell<Option<Stored>>,
    /// The picker entries, in the order shown.
    pub(super) options: RefCell<Vec<(String, Spec)>>,
    /// (is good news, text) next to Save.
    pub(super) message: RefCell<(bool, String)>,
}

impl WorkspacePane {
    pub(super) fn new(id: i32, name: &str, layout: Layout, specs: Vec<Option<Spec>>) -> Self {
        let mut slots: Vec<Slot> = specs
            .into_iter()
            .map(|spec| Slot {
                spec,
                ..Slot::default()
            })
            .collect();
        slots.resize_with(layout.slots(), Slot::default);
        Self {
            id,
            saved_id: RefCell::new(None),
            name: RefCell::new(name.to_string()),
            layout: Cell::new(layout),
            slots: RefCell::new(slots),
            view: Rc::new(VecModel::default()),
            stored: RefCell::new(None),
            options: RefCell::new(Vec::new()),
            message: RefCell::new((true, String::new())),
        }
    }

    pub(super) fn specs(&self) -> Vec<Option<Spec>> {
        self.slots
            .borrow()
            .iter()
            .map(|slot| slot.spec.clone())
            .collect()
    }

    /// Whether anything differs from what is stored.
    pub(super) fn dirty(&self) -> bool {
        match &*self.stored.borrow() {
            Some((name, layout, specs)) => {
                *name != self.name.borrow().trim()
                    || *layout != self.layout.get()
                    || *specs != self.specs()
            }
            None => true,
        }
    }

    /// The slot holding the session `id`.
    pub fn slot_of(&self, id: i32) -> Option<usize> {
        self.slots
            .borrow()
            .iter()
            .position(|slot| slot.pane.as_ref().is_some_and(|pane| pane.id() == id))
    }

    /// Whether closing the tab would end a session that is still going.
    pub fn running(&self) -> bool {
        self.slots
            .borrow()
            .iter()
            .any(|slot| slot.pane.as_ref().is_some_and(Pane::running))
    }

    pub fn closing_note(&self) -> String {
        let mut note = "Every session in its panels will end.".to_string();
        if self.dirty() {
            note.push_str(" Changes that were not saved are lost.");
        }
        note
    }
}

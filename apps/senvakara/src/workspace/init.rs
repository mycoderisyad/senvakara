//! Connects the callbacks of the Workspaces view to their handlers.

use std::rc::Rc;

use slint::ComponentHandle;

use crate::{App, Workspaces};

use super::Layout;

impl App {
    pub fn init_workspaces(self: &Rc<Self>, ui: &crate::MainWindow) {
        let view = ui.global::<Workspaces>();
        view.set_saved(self.ws.items.clone().into());
        view.on_open({
            let app = self.clone();
            move |id| app.workspace_open(&id)
        });
        view.on_create({
            let app = self.clone();
            move || app.workspace_create()
        });
        view.on_remove({
            let app = self.clone();
            move |id| app.workspace_remove_ask(&id)
        });
        view.on_name_edited({
            let app = self.clone();
            move |text| {
                if let Some(tab) = app.workspace_tab() {
                    app.with_workspace(tab, |pane| *pane.name.borrow_mut() = text.to_string());
                    app.workspace_publish(tab);
                }
            }
        });
        view.on_set_layout({
            let app = self.clone();
            move |key| {
                if let (Some(tab), Some(layout)) = (app.workspace_tab(), Layout::from_key(&key)) {
                    app.workspace_layout_ask(tab, layout);
                }
            }
        });
        view.on_save({
            let app = self.clone();
            move || app.workspace_save()
        });
        view.on_choose({
            let app = self.clone();
            move |slot, index| {
                if let Some(tab) = app.workspace_tab() {
                    app.with_workspace(tab, |pane| {
                        if let Some(slot) = pane.slots.borrow_mut().get_mut(slot.max(0) as usize) {
                            slot.choice = index.max(0) as usize;
                        }
                    });
                    app.workspace_publish(tab);
                }
            }
        });
        view.on_add({
            let app = self.clone();
            move |slot| app.workspace_add(slot.max(0) as usize)
        });
        view.on_close_panel({
            let app = self.clone();
            move |slot| app.workspace_close_ask(slot.max(0) as usize)
        });
        view.on_connect({
            let app = self.clone();
            move |slot| app.workspace_connect(slot.max(0) as usize)
        });
        self.workspaces_reload();
    }
}

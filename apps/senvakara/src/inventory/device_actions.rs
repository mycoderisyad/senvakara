//! What the user can do to a phone: open its screen, logcat or files, or hand off to a device tool.

use std::rc::Rc;

use slint::{Model, SharedString};

use crate::{App, TabInfo};

use super::text::transport_label;

impl App {
    pub fn device_action(self: &Rc<Self>, route_id: &str, action: &str) {
        let Some(device) = self.inv.device(route_id) else {
            return;
        };
        let record = &device.record;
        if !matches!(action, "mirror" | "logcat" | "files") {
            self.device_tool_action(&device, action);
            return;
        }
        if record.state != "available" {
            return;
        }
        if action == "files" {
            self.open_files(crate::files::Source::Device(record.route_id.clone()));
            return;
        }
        let id = self.take_id();
        let route = format!(
            "{}  >  {}",
            record.host_name,
            transport_label(&record.connection_type)
        );
        let hue = self.inv.hue(&record.host_id);
        let pane = if action == "mirror" {
            crate::Pane::Mirror(crate::mirror::MirrorPane::start(
                id,
                self.backend.clone(),
                record.route_id.clone(),
                record.display_name.clone(),
                route,
                hue,
                crate::notifier(id),
            ))
        } else {
            crate::Pane::Logcat(crate::logcat::LogcatPane::start(
                id,
                self.backend.clone(),
                record.route_id.clone(),
                record.display_name.clone(),
                route,
                hue,
                crate::notifier(id),
            ))
        };
        self.tabs.push(TabInfo {
            id,
            kind: pane.kind().into(),
            title: if action == "mirror" {
                record.display_name.as_str().into()
            } else {
                format!("{} logcat", record.display_name).into()
            },
            detail: SharedString::new(),
            hue,
            exited: false,
        });
        self.sessions.borrow_mut().push(pane);
        self.select(self.tabs.row_count() as i32 - 1);
    }
}

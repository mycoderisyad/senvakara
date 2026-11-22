//! The phone a run targets: its label in a picker, its facts in a confirmation, and the run tab.

use std::rc::Rc;

use slint::{Model, SharedString};

use crate::inventory::{fact, transport_label};
use crate::run::{Job, RunPane};
use crate::{App, KeyValue, Pane, TabInfo};

use super::DeviceView;

pub(super) fn device_label(device: &DeviceView) -> String {
    format!(
        "{} ({} > {})",
        device.record.display_name,
        device.record.host_name,
        transport_label(&device.record.connection_type)
    )
}

/// Host, route and serial of the phone a run will touch, for a confirmation.
pub(super) fn target_facts(device: &DeviceView) -> Vec<KeyValue> {
    let record = &device.record;
    vec![
        fact("Phone", record.display_name.as_str(), false),
        fact(
            "Route",
            format!(
                "{} over {}",
                record.host_name,
                transport_label(&record.connection_type)
            ),
            false,
        ),
        fact("Serial", record.serial.as_str(), true),
    ]
}

impl App {
    pub(super) fn open_run(
        self: &Rc<Self>,
        title: String,
        device: &DeviceView,
        labels: Vec<String>,
        job: Job,
    ) {
        let id = self.take_id();
        let record = &device.record;
        let hue = self.inv.hue(&record.host_id);
        let target = format!(
            "{}, through {} over {}",
            record.display_name,
            record.host_name,
            transport_label(&record.connection_type)
        );
        let pane = RunPane::start(
            id,
            self.backend.clone(),
            title.clone(),
            target,
            hue,
            labels,
            job,
            crate::notifier(id),
        );
        self.sessions.borrow_mut().push(Pane::Run(pane));
        self.tabs.push(TabInfo {
            id,
            kind: "run".into(),
            title: title.into(),
            detail: SharedString::new(),
            hue,
            exited: false,
        });
        self.select(self.tabs.row_count() as i32 - 1);
    }
}

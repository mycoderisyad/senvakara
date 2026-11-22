//! What the user can do to a host: connect, check its key, open terminals, files and the desktop.

use std::rc::Rc;
use std::sync::Arc;

use senvakara_app::{HostView, ShellRequest, Sink, TermEvent};
use slint::{Model, SharedString};

use crate::{term, App, Pending, Question, TabInfo};

use super::text::{fact, friendly};

impl App {
    pub fn host_action(self: &Rc<Self>, host_id: &str, action: &str) {
        let Some(host) = self
            .inv
            .hosts
            .borrow()
            .iter()
            .find(|host| host.id == host_id)
            .cloned()
        else {
            return;
        };
        match action {
            "terminal" if host.is_local => self.open_shell("powershell", ""),
            "terminal" => self.open_ssh(&host, None),
            "tmux" => {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0);
                self.open_ssh(&host, Some(format!("senvakara-{}", base36(stamp))));
            }
            "connect" | "disconnect" => {
                self.inv.busy.borrow_mut().insert(host.id.clone());
                self.inv.errors.borrow_mut().remove(&host.id);
                self.rebuild();
                let backend = self.backend.clone();
                let (id, connect) = (host.id.clone(), action == "connect");
                self.run(
                    async move {
                        if connect {
                            backend.host_connect(id).await
                        } else {
                            backend.host_disconnect(id).await
                        }
                    },
                    move |app, result| {
                        app.inv.busy.borrow_mut().remove(&host.id);
                        if let Err(error) = result {
                            app.inv
                                .errors
                                .borrow_mut()
                                .insert(host.id.clone(), friendly(&error));
                        }
                        app.reload();
                        app.sample_metrics();
                    },
                );
            }
            "trust" => {
                let backend = self.backend.clone();
                let id = host.id.clone();
                self.run(async move { backend.host_check_key(id).await }, move |app, result| match result {
                    Ok(check) => {
                        let mut facts = vec![fact("Host", host.display_name.as_str(), false), fact("Presented key", check.presented.as_str(), true)];
                        if let Some(pinned) = &check.pinned {
                            facts.push(fact("Trusted key", pinned.as_str(), true));
                        }
                        let (title, body, warning, confirm) = if check.matches {
                            ("The host key matches", "The host presented the key you already trust. Nothing needs to change.", "", "")
                        } else if check.pinned.is_some() {
                            (
                                "The host key has changed",
                                "Only continue if you know why the key changed, for example after reinstalling the host.",
                                "A different key can mean another machine is answering at this address.",
                                "Trust new key",
                            )
                        } else {
                            (
                                "Trust this host key?",
                                "Compare the key with the one shown on the host itself (ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub).",
                                "",
                                "Trust this key",
                            )
                        };
                        let question = Question {
                            title: title.into(),
                            body: body.into(),
                            confirm: confirm.into(),
                            danger: check.pinned.is_some() && !check.matches,
                        };
                        app.ask_with(question, facts, warning, Pending::TrustKey(host.id.clone(), check.presented));
                    }
                    Err(error) => {
                        app.inv.errors.borrow_mut().insert(host.id.clone(), friendly(&error));
                        app.rebuild();
                    }
                });
            }
            "files" if host.connected => self.open_files(crate::files::Source::Host(host.id)),
            "desktop" => self.open_desktop(&host),
            "edit" if !host.is_local => {
                if self.host_form_leave(crate::hostform::Next::Open(Some(host.id.clone()))) {
                    self.host_form_open(Some(&host.id));
                }
            }
            "remove" if !host.is_local => self.host_remove_ask(&host),
            wsl if wsl.starts_with("wsl:") => self.open_shell("wsl", &wsl[4..]),
            _ => {}
        }
    }

    /// The user accepted a host key in the dialog.
    pub fn trust_key(self: &Rc<Self>, host_id: String, fingerprint: String) {
        let backend = self.backend.clone();
        let id = host_id.clone();
        self.run(
            async move { backend.host_trust_key(id, fingerprint).await },
            move |app, result| {
                if let Err(error) = result {
                    app.inv
                        .errors
                        .borrow_mut()
                        .insert(host_id, friendly(&error));
                }
                app.reload();
            },
        );
    }

    /// Open a terminal tab on a remote host. The tab appears at once and shows progress.
    fn open_ssh(self: &Rc<Self>, host: &HostView, tmux: Option<String>) {
        let id = self.take_id();
        let session = self.start_ssh(id, host, tmux);
        self.sessions.borrow_mut().push(crate::Pane::Term(session));
        self.tabs.push(TabInfo {
            id,
            kind: "terminal".into(),
            title: host.display_name.as_str().into(),
            detail: SharedString::new(),
            hue: self.inv.hue(&host.id),
            exited: false,
        });
        self.select(self.tabs.row_count() as i32 - 1);
    }

    /// Start a terminal on a remote host, for a tab or a workspace panel. The session exists
    /// at once and shows progress; `tmux` names the tmux session to attach to or create.
    pub fn start_ssh(
        self: &Rc<Self>,
        id: i32,
        host: &HostView,
        tmux: Option<String>,
    ) -> term::Session {
        let (cols, lines) = self.grid.get();
        let (session, feeder, commands) =
            term::Session::remote(id, cols, lines, crate::notifier(id));
        feeder.feed(format!("Connecting to {}...\r\n", host.display_name).as_bytes());

        let backend = self.backend.clone();
        let request = ShellRequest::Ssh {
            host_id: host.id.clone(),
            tmux,
        };
        let sink: Sink<TermEvent> = {
            let feeder = feeder.clone();
            Arc::new(move |event| {
                match event {
                    TermEvent::Data(bytes) => feeder.feed(&bytes),
                    TermEvent::Exit(_) => feeder.end(),
                }
                true
            })
        };
        self.backend.rt.spawn(async move {
            match backend.term_open(request, cols, lines, sink).await {
                Ok(remote) => {
                    feeder.feed(b"\x1b[2J\x1b[H");
                    // Apply input and resizes in the order the user made them.
                    std::thread::spawn(move || {
                        for command in commands {
                            match command {
                                term::Cmd::Input(bytes) => {
                                    let _ = backend.term_write(remote, bytes);
                                }
                                term::Cmd::Resize(cols, lines) => {
                                    let _ = backend.term_resize(remote, cols, lines);
                                }
                                term::Cmd::Close => break,
                            }
                        }
                        let _ = backend.term_close(remote);
                    });
                    let _ = slint::invoke_from_event_loop(|| {
                        if let Some(app) = crate::app() {
                            app.reload();
                        }
                    });
                }
                Err(error) => {
                    feeder.feed(format!("\r\n{}\r\n", friendly(&error)).as_bytes());
                    feeder.end();
                }
            }
        });
        session
    }
}

fn base36(mut value: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

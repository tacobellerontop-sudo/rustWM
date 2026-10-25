//! Serving `rwm` and status bars over the named pipe.
//!
//! Client threads never touch the window manager. Commands go to the core
//! thread as [`Event::Request`]s and the client waits until the core reports
//! them done; queries read the snapshot the core publishes after every event;
//! subscribers get a line whenever that snapshot changes.

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Condvar, Mutex, RwLock};
use std::time::Duration;

use rwm_core::state::{State, TOPICS};
use rwm_core::{Command, Event};
use rwm_ipc::{EventMessage, PROTOCOL_VERSION, Request, Response, read_message, write_message};
use serde_json::json;

/// How long a client waits for the core to run its command.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

struct Subscriber {
    topics: Vec<String>,
    tx: Sender<EventMessage>,
}

/// State shared between the core thread and IPC client threads.
pub struct Shared {
    events: Mutex<Sender<Event>>,
    state: RwLock<Option<State>>,
    config: RwLock<serde_json::Value>,
    subscribers: Mutex<Vec<Subscriber>>,
    next_id: AtomicU64,
    /// The id of the last request the core finished.
    done: Mutex<u64>,
    done_cv: Condvar,
}

impl Shared {
    pub fn new(events: Sender<Event>) -> Self {
        Self {
            events: Mutex::new(events),
            state: RwLock::new(None),
            config: RwLock::new(serde_json::Value::Null),
            subscribers: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
            done: Mutex::new(0),
            done_cv: Condvar::new(),
        }
    }

    /// Called by the core thread after it handled request `id`.
    pub fn finished(&self, id: u64) {
        if let Ok(mut done) = self.done.lock() {
            *done = (*done).max(id);
            self.done_cv.notify_all();
        }
    }

    /// Records what `rwm query config` reports.
    pub fn set_config(&self, path: &std::path::Path) {
        if let Ok(mut c) = self.config.write() {
            *c = json!({ "path": PathBuf::from(path) });
        }
    }

    /// Publishes a new snapshot and notifies subscribers of what changed.
    pub fn publish(&self, state: State) {
        let Ok(mut current) = self.state.write() else {
            return;
        };
        let topics = match current.as_ref() {
            Some(old) if *old == state => return,
            Some(old) => state.changes(old),
            None => Vec::new(),
        };
        if !topics.is_empty() {
            let value = serde_json::to_value(&state).unwrap_or_default();
            if let Ok(mut subs) = self.subscribers.lock() {
                for topic in &topics {
                    let msg = EventMessage {
                        v: PROTOCOL_VERSION,
                        event: (*topic).into(),
                        state: value.clone(),
                    };
                    subs.retain(|s| {
                        !s.topics.iter().any(|t| t == topic) || s.tx.send(msg.clone()).is_ok()
                    });
                }
            }
        }
        *current = Some(state);
    }

    fn state_json(&self) -> serde_json::Value {
        self.state
            .read()
            .ok()
            .and_then(|s| serde_json::to_value(s.as_ref()?).ok())
            .unwrap_or_default()
    }

    fn run_command(&self, cmd: &str) -> Response {
        if let Err(e) = cmd.parse::<Command>() {
            return Response::error(e.to_string());
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let sent = self.events.lock().map(|tx| {
            tx.send(Event::Request {
                id,
                command: cmd.into(),
            })
        });
        if !matches!(sent, Ok(Ok(()))) {
            return Response::error("rustWM is shutting down");
        }
        let Ok(done) = self.done.lock() else {
            return Response::error("internal error");
        };
        match self
            .done_cv
            .wait_timeout_while(done, COMMAND_TIMEOUT, |d| *d < id)
        {
            Ok((_, r)) if !r.timed_out() => Response::ok(),
            _ => Response::error("timed out waiting for rustWM"),
        }
    }

    fn query(&self, what: &str) -> Response {
        match what {
            "state" => Response::data(self.state_json()),
            "focused" => Response::data(self.state_json()["focused_window"].clone()),
            "windows" => {
                let state = self.state_json();
                let windows: Vec<_> = state["monitors"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|m| {
                        let monitor = m["id"].clone();
                        m["workspaces"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .flat_map(move |ws| {
                                let (monitor, workspace) = (monitor.clone(), ws["name"].clone());
                                ws["windows"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .map(move |w| {
                                        let mut w = w.clone();
                                        w["monitor"] = monitor.clone();
                                        w["workspace"] = workspace.clone();
                                        w
                                    })
                            })
                    })
                    .collect();
                Response::data(json!(windows))
            }
            "config" => Response::data(self.config.read().map(|c| c.clone()).unwrap_or_default()),
            "version" => Response::data(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "protocol": PROTOCOL_VERSION,
            })),
            other => Response::error(format!(
                "unknown query `{other}` (try state, windows, focused, config or version)"
            )),
        }
    }

    /// Serves one client until it disconnects.
    pub fn serve_client(&self, pipe: File) {
        let Ok(mut writer) = pipe.try_clone() else {
            return;
        };
        let mut reader = BufReader::new(pipe);
        loop {
            let request: Request = match read_message(&mut reader) {
                Ok(Some(r)) => r,
                Ok(None) => return,
                Err(e) => {
                    let _ =
                        write_message(&mut writer, &Response::error(format!("bad request: {e}")));
                    return;
                }
            };
            let response = match request {
                Request::Command { cmd } => self.run_command(&cmd),
                Request::Query { query } => self.query(&query),
                Request::Subscribe { subscribe } => {
                    self.stream(subscribe, writer);
                    return;
                }
            };
            if write_message(&mut writer, &response).is_err() {
                return;
            }
        }
    }

    /// Keeps the pipe open and writes an event per change until the client
    /// goes away.
    fn stream(&self, mut topics: Vec<String>, mut writer: File) {
        if topics.is_empty() || topics.iter().any(|t| t == "all") {
            topics = TOPICS.iter().map(|t| t.to_string()).collect();
        }
        if let Some(bad) = topics.iter().find(|t| !TOPICS.contains(&t.as_str())) {
            let msg = format!("unknown topic `{bad}` (try {})", TOPICS.join(", "));
            let _ = write_message(&mut writer, &Response::error(msg));
            return;
        }
        let (tx, rx) = mpsc::channel();
        if let Ok(mut subs) = self.subscribers.lock() {
            subs.push(Subscriber { topics, tx });
        }
        let init = EventMessage {
            v: PROTOCOL_VERSION,
            event: "init".into(),
            state: self.state_json(),
        };
        if write_message(&mut writer, &Response::ok()).is_err()
            || write_message(&mut writer, &init).is_err()
        {
            return;
        }
        for msg in rx {
            if write_message(&mut writer, &msg).is_err() {
                return;
            }
        }
    }
}

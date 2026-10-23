//! A serialisable snapshot of the window manager, for `rwm query` and for
//! status bars subscribed over IPC.

use serde::{Deserialize, Serialize};

use crate::Rect;
use crate::layout::LayoutKind;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub focused_monitor: usize,
    pub focused_window: Option<WindowState>,
    /// The special workspace currently shown, if any.
    pub special: Option<String>,
    pub monitors: Vec<MonitorState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorState {
    pub id: String,
    pub rect: Rect,
    pub work_area: Rect,
    pub scale: f32,
    pub active_workspace: String,
    pub workspaces: Vec<WorkspaceState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub name: String,
    pub layout: LayoutKind,
    pub focused: Option<isize>,
    pub windows: Vec<WindowState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub handle: isize,
    pub title: String,
    pub exe: String,
    pub class: String,
    pub floating: bool,
    pub fullscreen: bool,
}

/// Topics a subscriber can ask for.
pub const TOPICS: &[&str] = &["focus", "workspace", "window"];

impl State {
    /// Which subscription topics differ between `old` and `self`.
    ///
    /// - `focus`: the focused window or monitor changed
    /// - `workspace`: active workspaces, layouts or the shown
    ///   special workspace changed
    /// - `window`: windows opened, closed, moved between workspaces or retitled
    pub fn changes(&self, old: &State) -> Vec<&'static str> {
        let mut topics = Vec::new();
        let focus_id = |s: &State| {
            (
                s.focused_monitor,
                s.focused_window.as_ref().map(|w| w.handle),
            )
        };
        if focus_id(self) != focus_id(old) {
            topics.push("focus");
        }
        type Shape<'a> = (
            Vec<(&'a str, &'a str, Vec<(&'a str, LayoutKind)>)>,
            Option<&'a str>,
        );
        fn ws_shape(s: &State) -> Shape<'_> {
            let monitors = s
                .monitors
                .iter()
                .map(|m| {
                    let layouts = m
                        .workspaces
                        .iter()
                        .map(|w| (w.name.as_str(), w.layout))
                        .collect();
                    (m.id.as_str(), m.active_workspace.as_str(), layouts)
                })
                .collect();
            (monitors, s.special.as_deref())
        }
        if ws_shape(self) != ws_shape(old) {
            topics.push("workspace");
        }
        let windows = |s: &State| {
            s.monitors
                .iter()
                .flat_map(|m| m.workspaces.iter().map(|w| &w.windows))
                .cloned()
                .collect::<Vec<_>>()
        };
        if windows(self) != windows(old) {
            topics.push("window");
        }
        topics
    }
}

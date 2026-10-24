//! The rustWM IPC protocol: newline-delimited JSON over the named pipe
//! `\\.\pipe\rustwm`.
//!
//! A client writes one [`Request`] per line and reads one [`Response`] per
//! line. After a `subscribe` request the daemon keeps the pipe open and
//! streams [`EventMessage`]s, one per line, until the client disconnects.

mod client;
#[cfg(windows)]
mod server;

use std::io::{self, BufRead, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub use client::Client;
#[cfg(windows)]
pub use server::{AlreadyRunning, Server};

pub const PIPE_NAME: &str = r"\\.\pipe\rustwm";

/// Protocol version carried in every message as `"v"`.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Request {
    /// `{"cmd": "focus left"}`: the same strings as `[bindings]`.
    Command { cmd: String },
    /// `{"query": "state"}`: also `windows`, `focused`, `config`, `version`.
    Query { query: String },
    /// `{"subscribe": ["workspace", "focus", "window"]}`
    Subscribe { subscribe: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub v: u32,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl Response {
    pub fn ok() -> Self {
        Self {
            v: PROTOCOL_VERSION,
            ok: true,
            error: None,
            data: None,
        }
    }

    pub fn data(data: serde_json::Value) -> Self {
        Self {
            data: Some(data),
            ..Self::ok()
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            ok: false,
            error: Some(message.into()),
            data: None,
        }
    }
}

/// One streamed notification for a subscriber.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventMessage {
    pub v: u32,
    /// The topic that changed (`focus`, `workspace`, `window`), or `init` for
    /// the first message after subscribing.
    pub event: String,
    /// The full window-manager state after the change.
    pub state: serde_json::Value,
}

/// Writes `msg` as one JSON line and flushes.
pub fn write_message<W: Write, T: Serialize>(w: &mut W, msg: &T) -> io::Result<()> {
    let mut line = serde_json::to_vec(msg)?;
    line.push(b'\n');
    w.write_all(&line)?;
    w.flush()
}

/// Reads one JSON line. `Ok(None)` means the other side closed the pipe.
pub fn read_message<R: BufRead, T: DeserializeOwned>(r: &mut R) -> io::Result<Option<T>> {
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if !line.trim().is_empty() {
            break;
        }
    }
    serde_json::from_str(line.trim())
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_request_kind() {
        let cmd: Request = serde_json::from_str(r#"{"cmd":"focus left"}"#).unwrap();
        assert_eq!(
            cmd,
            Request::Command {
                cmd: "focus left".into()
            }
        );

        let q: Request = serde_json::from_str(r#"{"v":1,"query":"state"}"#).unwrap();
        assert_eq!(
            q,
            Request::Query {
                query: "state".into()
            }
        );

        let s: Request = serde_json::from_str(r#"{"subscribe":["focus"]}"#).unwrap();
        assert_eq!(
            s,
            Request::Subscribe {
                subscribe: vec!["focus".into()]
            }
        );
    }

    #[test]
    fn ok_response_omits_empty_fields() {
        assert_eq!(
            serde_json::to_string(&Response::ok()).unwrap(),
            r#"{"v":1,"ok":true}"#
        );
    }

    #[test]
    fn messages_round_trip_over_a_stream() {
        let mut buf = Vec::new();
        write_message(
            &mut buf,
            &Request::Command {
                cmd: "promote".into(),
            },
        )
        .unwrap();
        buf.extend_from_slice(b"\n\n");
        write_message(&mut buf, &Response::error("nope")).unwrap();

        let mut r = io::Cursor::new(buf);
        let req: Request = read_message(&mut r).unwrap().unwrap();
        assert_eq!(
            req,
            Request::Command {
                cmd: "promote".into()
            }
        );
        let resp: Response = read_message(&mut r).unwrap().unwrap();
        assert_eq!(resp.error.as_deref(), Some("nope"));
        assert!(read_message::<_, Response>(&mut r).unwrap().is_none());
    }

    #[test]
    fn garbage_is_invalid_data() {
        let mut r = io::Cursor::new(b"{not json\n".to_vec());
        let err = read_message::<_, Request>(&mut r).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}

//! The rustWM IPC protocol: newline-delimited JSON over a named pipe.
//!
//! The pipe server and client land in M4; the message types live here from
//! the start so the daemon and CLI agree on them.

use serde::{Deserialize, Serialize};

pub const PIPE_NAME: &str = r"\\.\pipe\rustwm";

/// Protocol version carried in every message as `"v"`.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Request {
    /// `{"cmd": "focus left"}`: the same strings as `[bindings]`.
    Command { cmd: String },
    /// `{"query": "state"}`
    Query { query: String },
    /// `{"subscribe": ["workspace", "focus"]}`
    Subscribe { subscribe: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub v: u32,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
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

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            ok: false,
            error: Some(message.into()),
            data: None,
        }
    }
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

        let q: Request = serde_json::from_str(r#"{"query":"state"}"#).unwrap();
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
}

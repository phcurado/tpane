use serde::{Deserialize, Serialize};

use crate::process::ProcessInfo;

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Ping,
    Info,
    Shutdown,
    Refresh,
    Reload,
    Status,
    Panes,
    Panels,
    SelectPane { id: String },
    ExpandPane { id: String },
    SetState { id: String, state: String },
    Doctor { clean: bool },
    Command { name: String, args: Vec<String> },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PaneSnapshot {
    pub id: String,
    pub pid: i32,
    pub kind: String,
    pub label: String,
    pub cwd: String,
    pub cwd_basename: String,
    pub command: String,
    pub session: String,
    pub window: String,
    pub active: bool,
    pub zoomed: bool,
    pub tag: Option<String>,
    pub home: Option<String>,
    pub state: Option<String>,
    pub processes: Vec<ProcessInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PanelView {
    pub id: String,
    pub title: String,
    pub cards: Vec<PanelCard>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DaemonInfo {
    pub version: String,
    pub exe_hash: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PanelCard {
    pub title: String,
    pub subtitle: Option<String>,
    pub state: Option<String>,
    pub tag: Option<String>,
    pub pane: Option<String>,
    pub enter: Option<Vec<String>>,
    pub expand: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    pub fn ok(data: impl Into<Option<String>>) -> Self {
        Self {
            ok: true,
            data: data.into(),
            error: None,
        }
    }

    pub fn error(error: impl ToString) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_request_round_trips_as_snake_case_json() {
        let request = Request::Command {
            name: "hello".to_string(),
            args: vec!["a".to_string(), "b".to_string()],
        };

        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(
            json,
            r#"{"type":"command","name":"hello","args":["a","b"]}"#
        );

        let decoded: Request = serde_json::from_str(&json).unwrap();
        match decoded {
            Request::Command { name, args } => {
                assert_eq!(name, "hello");
                assert_eq!(args, ["a", "b"]);
            }
            other => panic!("expected command request, got {other:?}"),
        }
    }
}

use serde::{Deserialize, Serialize};

pub const DAEMON_SIGNATURE: &str = concat!("castr/", env!("CARGO_PKG_VERSION"), "/protocol-3");

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Ping,
    Refresh,
    Reload,
    Status,
    Pick,
    Panes,
    SelectPane { id: String },
    Command { name: String, args: Vec<String> },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PaneSnapshot {
    pub id: String,
    pub pid: i32,
    pub kind: String,
    pub label: String,
    pub cwd: String,
    pub session: String,
    pub window: String,
    pub active: bool,
    pub zoomed: bool,
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

use serde::{Deserialize, Serialize};

pub const DAEMON_SIGNATURE: &str = concat!("castr/", env!("CARGO_PKG_VERSION"), "/plugin-2");

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Ping,
    Refresh,
    Pick,
    Panes,
    SelectPane { id: String },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PaneSnapshot {
    pub id: String,
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

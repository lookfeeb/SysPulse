use serde::{Deserialize, Serialize};

// Wire types matching hw-helper/Protocol.cs.

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperRequest {
    pub id: u64,
    pub op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<RequestParams>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fan_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pwm: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperResponse {
    pub id: i64, // -1 means "we couldn't parse the inbound id"
    pub ok: bool,
    #[serde(default)]
    pub data: serde_json::Value,
    #[serde(default)]
    pub error: Option<HelperError>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperError {
    pub code: String,
    pub message: String,
}

/// Either a `Response` or an unsolicited `Event`. We deserialize each line
/// untyped first, then dispatch based on which key is present.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperEvent {
    pub event: String,
    #[serde(default)]
    pub data: serde_json::Value,
}

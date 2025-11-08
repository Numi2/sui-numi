// JSON-RPC transport layer implementation
// This file implements the JSON-RPC client communication protocol
// for interacting with Sui network nodes via HTTP/WebSocket
//
// Numan Thabit 2025 Nov

use crate::errors::AggrError;
use base64::{engine::general_purpose::STANDARD_NO_PAD as B64, Engine as _};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Clone)]
pub struct JsonRpc {
    http: Client,
    url: String,
}

impl JsonRpc {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            http: Client::new(),
            url: url.into(),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.url
    }

    pub async fn execute_tx_block(
        &self,
        tx_bcs: &[u8],
        signatures_b64: &[String],
    ) -> Result<ExecuteResp, AggrError> {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sui_executeTransactionBlock",
            "params": [
                B64.encode(tx_bcs),
                signatures_b64,
                { "showEffects": true, "showEvents": true },
                "WaitForLocalExecution"
            ]
        });
        let resp = self
            .http
            .post(&self.url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| AggrError::Transport(format!("jsonrpc send: {e}")))?;
        if !resp.status().is_success() {
            return Err(AggrError::Provider(format!("http {}", resp.status())));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AggrError::Transport(format!("json parse: {e}")))?;
        if let Some(err) = body.get("error") {
            return Err(AggrError::Provider(err.to_string()));
        }
        serde_json::from_value(body["result"].clone())
            .map_err(|e| AggrError::Provider(format!("decode result: {e}")))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecuteResp {
    pub digest: Option<String>,
    pub effects: Option<serde_json::Value>,
    pub events: Option<serde_json::Value>,
}

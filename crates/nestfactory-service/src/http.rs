//! API explorer: requests against the running app and endpoints from its Swagger spec.

use std::io::Read;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApiRequest {
    pub id: String,
    pub name: String,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApiResponse {
    pub status: u32,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub time_ms: f64,
    pub size: u32,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryEntry {
    pub request: ApiRequest,
    pub status: u32,
    pub time_ms: f64,
    pub at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Endpoint {
    pub method: String,
    pub path: String,
    pub summary: Option<String>,
    pub tag: Option<String>,
    /// `code` (static scan), `swagger` (running app) or `both`.
    pub source: String,
    pub controller: Option<String>,
    pub handler: Option<String>,
    pub file: Option<String>,
    pub line: u32,
    pub body_example: Option<String>,
}

const MAX_BODY: u64 = 10 * 1024 * 1024;

pub fn send(req: &ApiRequest) -> ApiResponse {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(60)).redirects(5).build();
    let mut r = agent.request(&req.method.to_uppercase(), &req.url);
    let mut has_type = false;
    for (k, v) in &req.headers {
        if k.trim().is_empty() {
            continue;
        }
        has_type |= k.eq_ignore_ascii_case("content-type");
        r = r.set(k.trim(), v);
    }
    let started = Instant::now();
    let result = if req.body.trim().is_empty() || matches!(req.method.to_uppercase().as_str(), "GET" | "HEAD") {
        r.call()
    } else {
        if !has_type {
            r = r.set("Content-Type", "application/json");
        }
        r.send_string(&req.body)
    };
    let response = match result {
        Ok(resp) | Err(ureq::Error::Status(_, resp)) => resp,
        Err(e) => {
            return ApiResponse { error: Some(e.to_string()), time_ms: started.elapsed().as_secs_f64() * 1000.0, ..Default::default() };
        }
    };
    let status = response.status() as u32;
    let status_text = response.status_text().to_string();
    let headers: Vec<(String, String)> = response.headers_names().into_iter().map(|n| (n.clone(), response.header(&n).unwrap_or("").to_string())).collect();
    let mut bytes = vec![];
    let _ = response.into_reader().take(MAX_BODY).read_to_end(&mut bytes);
    let time_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut body = String::from_utf8_lossy(&bytes).to_string();
    if let Ok(v) = serde_json::from_str::<Value>(&body) {
        body = serde_json::to_string_pretty(&v).unwrap_or(body);
    }
    ApiResponse { status, status_text, headers, size: bytes.len() as u32, body, time_ms, error: None }
}

/// Reads endpoints from the running app's Swagger JSON, trying the usual paths.
pub fn swagger_endpoints(port: u32) -> Result<Vec<Endpoint>> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_millis(1500)).build();
    for path in ["docs-json", "api-json", "swagger-json", "api-docs-json", "docs/json"] {
        let url = format!("http://localhost:{port}/{path}");
        let Ok(resp) = agent.get(&url).call() else { continue };
        let Ok(spec) = resp.into_json::<Value>() else { continue };
        let Some(paths) = spec.get("paths").and_then(Value::as_object) else { continue };
        let mut out = vec![];
        for (p, ops) in paths {
            let Some(ops) = ops.as_object() else { continue };
            for (method, op) in ops {
                if !matches!(method.as_str(), "get" | "post" | "put" | "patch" | "delete" | "options" | "head") {
                    continue;
                }
                let body_example = op
                    .pointer("/requestBody/content/application~1json/schema/$ref")
                    .and_then(Value::as_str)
                    .and_then(|r| r.rsplit('/').next())
                    .and_then(|name| spec.pointer(&format!("/components/schemas/{name}")))
                    .map(example_from_schema);
                out.push(Endpoint {
                    method: method.to_uppercase(),
                    path: p.replace('{', ":").replace('}', ""),
                    summary: op.get("summary").and_then(Value::as_str).or_else(|| op.get("operationId").and_then(Value::as_str)).map(String::from),
                    tag: op.get("tags").and_then(|t| t.get(0)).and_then(Value::as_str).map(String::from),
                    source: "swagger".into(),
                    controller: None,
                    handler: None,
                    file: None,
                    line: 0,
                    body_example,
                });
            }
        }
        return Ok(out);
    }
    anyhow::bail!("no Swagger JSON found on port {port}")
}

fn example_from_schema(schema: &Value) -> String {
    let mut obj = serde_json::Map::new();
    if let Some(props) = schema.get("properties").and_then(Value::as_object) {
        for (k, p) in props {
            let v = match p.get("type").and_then(Value::as_str) {
                Some("number") | Some("integer") => Value::from(0),
                Some("boolean") => Value::from(false),
                Some("array") => Value::Array(vec![]),
                Some("object") => Value::Object(Default::default()),
                _ => p.get("example").cloned().unwrap_or_else(|| Value::from("")),
            };
            obj.insert(k.clone(), v);
        }
    }
    serde_json::to_string_pretty(&Value::Object(obj)).unwrap_or_default()
}

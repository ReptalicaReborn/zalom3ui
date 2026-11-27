use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chrono::Utc;
use reqwest::Method;
use serde_json::{json, Map, Value};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};

use crate::crypto::{decode_aes, parse_json_keeping_bigints};
use crate::http::ZaloHttp;
use crate::types::ZaloContext;

const MIN_GAP_MS: u64 = 1_000;
const EARLY_EMPTY_MS: u64 = 9_000;
const EARLY_EMPTY_WAIT_MS: u64 = 5_000;
const BACKOFF_MS: &[u64] = &[5_000, 15_000, 30_000, 60_000];
const REPEAT_WAIT_MS: u64 = 30_000;
const SEEN_CAP: usize = 500;

/// Parsed response from one `zpw_ctrs_new` control long-poll request.
#[derive(Debug, Clone)]
pub enum PollAnswer {
    Empty,
    Data(Map<String, Value>),
    Error { code: i64, message: String },
}

/// Events emitted by [`ControlPoller`].
#[derive(Debug, Clone)]
pub enum ControlPollEvent {
    Controls(Vec<Value>),
    Refused { code: i64, message: String },
    Error { message: String, failures: usize },
    Unavailable(String),
}

/// Builds Zalo PC's control poll cursor: `lastActionId,lastActionIdOther,<urlenc(b64(queueIds)+",1")>`.
pub fn poll_cursor(
    last_action_id: &str,
    last_action_id_other: &str,
    queue_ids: &Value,
) -> String {
    let ids_b64 = B64.encode(queue_ids.to_string().as_bytes());
    let suffix = format!("{ids_b64},1");
    format!(
        "{last_action_id},{last_action_id_other},{}",
        percent_encoding::utf8_percent_encode(&suffix, percent_encoding::NON_ALPHANUMERIC)
    )
}

/// Parses one `zpw_ctrs_new` HTTP body (exact port of `src/ctrlpoll.ts:readPollAnswer`).
pub fn read_poll_answer(body: &str, secret_key: &str) -> PollAnswer {
    if body.trim().is_empty() {
        return PollAnswer::Empty;
    }
    let Ok(outer) = parse_json_keeping_bigints(body) else {
        return PollAnswer::Error {
            code: -1,
            message: "Malformed poll answer".into(),
        };
    };
    let Some(outer_obj) = outer.as_object() else {
        return PollAnswer::Error {
            code: -1,
            message: "Malformed poll answer".into(),
        };
    };
    let code = outer_obj
        .get("error_code")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if code != 0 {
        let message = outer_obj
            .get("error_message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        return PollAnswer::Error { code, message };
    }
    let Some(raw_data) = outer_obj.get("data") else {
        return PollAnswer::Empty;
    };
    if raw_data.is_null() || raw_data.as_str() == Some("") {
        return PollAnswer::Empty;
    }

    let inner: Value = match raw_data {
        Value::String(s) => {
            let text = if s.trim_start().starts_with('{') {
                s.clone()
            } else {
                match decode_aes(secret_key, s) {
                    Ok(pt) => pt,
                    Err(_) => {
                        return PollAnswer::Error {
                            code: -1,
                            message: "Could not decrypt the poll answer".into(),
                        };
                    }
                }
            };
            match parse_json_keeping_bigints(&text) {
                Ok(v) => v,
                Err(_) => {
                    return PollAnswer::Error {
                        code: -1,
                        message: "Invalid poll answer data".into(),
                    };
                }
            }
        }
        other => other.clone(),
    };

    let Some(inner_obj) = inner.as_object() else {
        return PollAnswer::Error {
            code: -1,
            message: "Invalid poll answer data".into(),
        };
    };
    if let Some(ic) = inner_obj.get("error_code").and_then(|v| v.as_i64()) {
        if ic != 0 {
            let message = inner_obj
                .get("error_message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            return PollAnswer::Error { code: ic, message };
        }
    }

    let candidate = inner_obj.get("data").filter(|v| !v.is_null());
    let data_obj = match candidate {
        Some(Value::Object(m)) => m.clone(),
        Some(Value::String(s)) if s.is_empty() => inner_obj.clone(),
        None => inner_obj.clone(),
        _ => {
            return PollAnswer::Error {
                code: -1,
                message: "Invalid poll answer data".into(),
            };
        }
    };
    PollAnswer::Data(data_obj)
}

/// Recursively finds every control entry (`{ content: { act_type, ... } }` or top-level `act_type`) up to depth 4.
pub fn controls_in(value: &Value, depth: usize) -> Vec<Value> {
    if depth > 4 {
        return Vec::new();
    }
    match value {
        Value::Array(arr) => {
            let mut out = Vec::new();
            for item in arr {
                if let Some(obj) = item.as_object() {
                    if obj
                        .get("content")
                        .and_then(|c| c.as_object())
                        .map(|c| c.contains_key("act_type"))
                        .unwrap_or(false)
                    {
                        out.push(item.clone());
                        continue;
                    }
                    if obj.contains_key("act_type") {
                        let mut wrapped = Map::new();
                        if let Some(cid) = obj.get("controlId") {
                            wrapped.insert("controlId".to_string(), cid.clone());
                        }
                        if let Some(aid) = obj.get("actionId") {
                            wrapped.insert("actionId".to_string(), aid.clone());
                        }
                        wrapped.insert("content".to_string(), item.clone());
                        out.push(Value::Object(wrapped));
                        continue;
                    }
                }
                out.extend(controls_in(item, depth + 1));
            }
            out
        }
        Value::Object(map) => {
            let mut out = Vec::new();
            for v in map.values() {
                out.extend(controls_in(v, depth + 1));
            }
            out
        }
        _ => Vec::new(),
    }
}

/// Filters a list of HTTPS control server URLs from `zpw_ctrs_new`.
pub fn server_list(urls: &[String]) -> Vec<String> {
    urls.iter()
        .filter(|u| u.starts_with("https://"))
        .map(|u| u.trim_end_matches('/').to_string())
        .collect()
}

/// Handle to stop a running [`ControlPoller`].
pub struct ControlPollerHandle {
    stop_tx: Option<oneshot::Sender<()>>,
}

impl ControlPollerHandle {
    pub fn stop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for ControlPollerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Long-poller for Zalo's `zpw_ctrs_new` control channel (used to receive VoIP call controls and sync controls).
pub struct ControlPoller {
    ctx: Arc<ZaloContext>,
    http: ZaloHttp,
    hold_seconds: u64,
}

impl ControlPoller {
    pub fn new(ctx: Arc<ZaloContext>, http: ZaloHttp, hold_seconds: u64) -> Self {
        Self {
            ctx,
            http,
            hold_seconds: hold_seconds.max(10),
        }
    }

    /// Starts the control long-poll background task.
    pub fn start(self) -> (mpsc::UnboundedReceiver<ControlPollEvent>, ControlPollerHandle) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (stop_tx, stop_rx) = oneshot::channel();
        tokio::spawn(async move {
            self.run_loop(event_tx, stop_rx).await;
        });
        (
            event_rx,
            ControlPollerHandle {
                stop_tx: Some(stop_tx),
            },
        )
    }

    async fn run_loop(
        self,
        event_tx: mpsc::UnboundedSender<ControlPollEvent>,
        mut stop_rx: oneshot::Receiver<()>,
    ) {
        let servers = server_list(&self.ctx.ctrs_urls);
        if servers.is_empty() {
            let _ = event_tx.send(ControlPollEvent::Unavailable(
                "no control servers in login answer".into(),
            ));
            return;
        }
        info!(
            servers = servers.len(),
            hold_seconds = self.hold_seconds,
            "Starting Zalo control poller"
        );

        let mut last_action_id = "0".to_string();
        let mut last_action_id_other = "1".to_string();
        let mut server_idx: usize = 0;
        let mut failures: usize = 0;
        let mut seen_set: HashSet<String> = HashSet::new();
        let mut seen_order: VecDeque<String> = VecDeque::new();
        let empty_queues = json!({});

        loop {
            if stop_rx.try_recv().is_ok() {
                return;
            }
            let started = Instant::now();
            let base = &servers[server_idx % servers.len()];
            let cursor = poll_cursor(&last_action_id, &last_action_id_other, &empty_queues);
            let url = format!(
                "{base}/?zpw_ver={}&zpw_type={}&params={cursor}&ts={}",
                self.ctx.api_version,
                self.ctx.api_type,
                Utc::now().timestamp()
            );

            let req_fut = self.http.request_with_cookies(
                &self.ctx.cookies,
                &self.ctx.user_agent,
                Method::GET,
                &url,
                None,
                None,
            );
            let timeout = Duration::from_secs(self.hold_seconds + 20);
            let res_result = tokio::select! {
                _ = &mut stop_rx => return,
                r = tokio::time::timeout(timeout, req_fut) => r,
            };

            let answer = match res_result {
                Ok(Ok(resp)) => {
                    if !resp.status().is_success() {
                        Err(format!("HTTP {}", resp.status().as_u16()))
                    } else {
                        match resp.text().await {
                            Ok(body) => Ok(read_poll_answer(&body, &self.ctx.secret_key)),
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
                Ok(Err(e)) => Err(e.to_string()),
                Err(_) => Err("control poll request timeout".into()),
            };

            let answer = match answer {
                Ok(a) => a,
                Err(err_msg) => {
                    failures += 1;
                    server_idx = server_idx.wrapping_add(1);
                    warn!(error = %err_msg, failures, "Control poll error");
                    let _ = event_tx.send(ControlPollEvent::Error {
                        message: err_msg,
                        failures,
                    });
                    let backoff = BACKOFF_MS[(failures - 1).min(BACKOFF_MS.len() - 1)];
                    tokio::select! {
                        _ = &mut stop_rx => return,
                        _ = tokio::time::sleep(Duration::from_millis(backoff)) => continue,
                    }
                }
            };

            match answer {
                PollAnswer::Error { code, message } => {
                    warn!(code, message = %message, "Control poll refused by Zalo");
                    let _ = event_tx.send(ControlPollEvent::Refused { code, message });
                    return;
                }
                PollAnswer::Empty => {
                    failures = 0;
                    if started.elapsed() < Duration::from_millis(EARLY_EMPTY_MS) {
                        tokio::select! {
                            _ = &mut stop_rx => return,
                            _ = tokio::time::sleep(Duration::from_millis(EARLY_EMPTY_WAIT_MS)) => {}
                        }
                    }
                }
                PollAnswer::Data(data) => {
                    failures = 0;
                    if let Some(reset_id) = non_empty_id(data.get("resetLastActionId")) {
                        last_action_id = reset_id;
                        last_action_id_other = "1".to_string();
                    } else if let Some(next_id) = non_empty_id(data.get("lastActionId")) {
                        last_action_id = next_id;
                    }

                    let all_controls = controls_in(&Value::Object(data), 0);
                    let mut fresh = Vec::new();
                    for ctrl in &all_controls {
                        let id = non_empty_id(ctrl.get("controlId"))
                            .or_else(|| non_empty_id(ctrl.get("actionId")));
                        if let Some(cid) = id {
                            if seen_set.contains(&cid) {
                                continue;
                            }
                            seen_set.insert(cid.clone());
                            seen_order.push_back(cid);
                            if seen_order.len() > SEEN_CAP {
                                if let Some(old) = seen_order.pop_front() {
                                    seen_set.remove(&old);
                                }
                            }
                        }
                        fresh.push(ctrl.clone());
                    }

                    if !fresh.is_empty() {
                        debug!(count = fresh.len(), "Control poll yielded fresh controls");
                        let _ = event_tx.send(ControlPollEvent::Controls(fresh));
                    } else if !all_controls.is_empty() {
                        tokio::select! {
                            _ = &mut stop_rx => return,
                            _ = tokio::time::sleep(Duration::from_millis(REPEAT_WAIT_MS)) => {}
                        }
                    }
                }
            }

            let elapsed = started.elapsed();
            if elapsed < Duration::from_millis(MIN_GAP_MS) {
                let wait = Duration::from_millis(MIN_GAP_MS) - elapsed;
                tokio::select! {
                    _ = &mut stop_rx => return,
                    _ = tokio::time::sleep(wait) => {}
                }
            }
        }
    }
}

fn non_empty_id(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::encode_aes;

    #[test]
    fn poll_cursor_and_read_answer() {
        let cursor = poll_cursor("0", "1", &json!({}));
        assert_eq!(cursor, "0,1,e30%3D%2C1");

        let key = B64.encode(b"0123456789abcdef");
        let inner = json!({
            "error_code": 0,
            "data": {
                "lastActionId": "999",
                "controls": [
                    { "controlId": "c1", "content": { "act_type": "voip", "act": "request", "data": "{}" } }
                ]
            }
        });
        let enc = encode_aes(&key, &inner.to_string()).unwrap();
        let body = json!({ "error_code": 0, "data": enc }).to_string();

        match read_poll_answer(&body, &key) {
            PollAnswer::Data(map) => {
                assert_eq!(map["lastActionId"], "999");
                let ctrls = controls_in(&Value::Object(map), 0);
                assert_eq!(ctrls.len(), 1);
                assert_eq!(ctrls[0]["content"]["act_type"], "voip");
            }
            other => panic!("expected Data, got {other:?}"),
        }
    }
}

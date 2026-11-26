pub mod account;
pub mod conversations;
pub mod ctrlpoll;
pub mod friends;
pub mod groups;
pub mod messaging;
pub mod polls;
pub mod voicecall;

use std::collections::HashMap;
use std::sync::Arc;

use reqwest::Method;
use serde_json::{json, Value};

use crate::crypto::{decode_aes, encode_aes, parse_json_keeping_bigints};
use crate::error::{ProtoError, Result};
use crate::http::ZaloHttp;
use crate::listener::ZaloListener;
use crate::types::ZaloContext;

pub const FILE_SERVICE_FALLBACK: &str = "https://tt-files-wpa.chat.zalo.me";
pub const PROFILE_SERVICE_FALLBACK: &str = "https://tt-profile-wpa.chat.zalo.me";
pub const MAX_BACKUP_BYTES: usize = 256 * 1024 * 1024;

/// Reply from `/api/znoise` mapping plain mobile user/group IDs to Zalo Web noised IDs.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct NoisedIdsReply {
    pub users: HashMap<String, String>,
    pub groups: HashMap<String, String>,
    pub reply_shape: String,
}

/// Logged-in Zalo Web protocol client exposing all HTTP APIs and WebSocket listener creation.
#[derive(Clone, Debug)]
pub struct ZaloClient {
    pub(crate) ctx: Arc<ZaloContext>,
    pub(crate) http: ZaloHttp,
}

impl ZaloClient {
    pub fn new(ctx: Arc<ZaloContext>, http: ZaloHttp) -> Self {
        Self { ctx, http }
    }

    pub fn ctx(&self) -> &Arc<ZaloContext> {
        &self.ctx
    }

    pub fn http(&self) -> &ZaloHttp {
        &self.http
    }

    pub fn own_id(&self) -> &str {
        &self.ctx.uid
    }

    /// Creates a [`ZaloListener`] bound to this session's context.
    pub fn listener(&self, self_listen: bool) -> ZaloListener {
        ZaloListener::new(Arc::clone(&self.ctx), self_listen)
    }

    /// Creates a [`voicecall::VoiceCallApi`] bound to this client.
    pub fn voice_call(&self) -> voicecall::VoiceCallApi {
        voicecall::VoiceCallApi::new(self.clone())
    }

    /// Resolves a service base URL from `zpw_service_map_v3`, returning an error if missing.
    pub(crate) fn require_service(&self, name: &str) -> Result<String> {
        self.ctx
            .service_url(name)
            .ok_or_else(|| ProtoError::InvalidArg(format!("service '{name}' missing from zpw_service_map_v3")))
    }

    /// Generic encrypted GET to `/api/message/<endpoint>` on the file service (used by history sync).
    pub async fn get_file_service(&self, endpoint: &str, params: Value) -> Result<Value> {
        let base = self
            .ctx
            .service_url("file")
            .filter(|s| s.starts_with("https://"))
            .unwrap_or_else(|| FILE_SERVICE_FALLBACK.to_string());
        let url = format!("{base}/api/message/{}", endpoint.trim_start_matches('/'));
        self.http.get_encrypted(&self.ctx, &url, &params, &[]).await
    }

    /// Calls `/api/znoise` on the profile service to translate plain mobile user/group IDs to
    /// Zalo Web noised IDs (preserving 19-digit BigInt IDs as strings).
    pub async fn get_znoise(
        &self,
        user_ids: &[String],
        group_ids: &[String],
    ) -> Result<NoisedIdsReply> {
        let base = self
            .ctx
            .service_url("profile")
            .filter(|s| s.starts_with("https://"))
            .unwrap_or_else(|| PROFILE_SERVICE_FALLBACK.to_string());
        let payload = json!({
            "fids": user_ids,
            "gids": group_ids,
        });
        let encrypted = encode_aes(&self.ctx.secret_key, &payload.to_string())?;
        let url = ZaloHttp::make_ctx_url(
            &self.ctx,
            &format!("{base}/api/znoise"),
            &[("params", &encrypted)],
            true,
        )?;
        let resp = self
            .http
            .request_with_cookies(
                &self.ctx.cookies,
                &self.ctx.user_agent,
                Method::GET,
                &url,
                None,
                None,
            )
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(ProtoError::HttpStatus {
                status: status.as_u16(),
                message: format!("znoise HTTP {}", status.as_u16()),
            });
        }

        let outer = parse_json_keeping_bigints(&text)?;
        let outer_code = outer
            .get("error_code")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        if outer_code != 0 {
            let msg = outer
                .get("error_message")
                .and_then(|v| v.as_str())
                .unwrap_or("znoise error")
                .to_string();
            return Err(ProtoError::Api {
                code: outer_code,
                message: msg,
            });
        }

        let inner_data = match outer.get("data") {
            Some(Value::String(enc)) if !self.ctx.secret_key.is_empty() => {
                let plain = decode_aes(&self.ctx.secret_key, enc)?;
                let inner = parse_json_keeping_bigints(&plain)?;
                let inner_code = inner
                    .get("error_code")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(-1);
                if inner_code != 0 {
                    let msg = inner
                        .get("error_message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("znoise decrypted error")
                        .to_string();
                    return Err(ProtoError::Api {
                        code: inner_code,
                        message: msg,
                    });
                }
                inner.get("data").cloned().unwrap_or(Value::Null)
            }
            Some(other) => other.clone(),
            None => Value::Null,
        };

        Ok(parse_noised_ids_reply(user_ids, group_ids, &inner_data))
    }

    /// Downloads a history-sync backup blob (enforcing `MAX_BACKUP_BYTES` and Zalo-host-only cookies).
    pub async fn download_backup(&self, url: &str) -> Result<Vec<u8>> {
        self.http
            .download_backup_bytes(&self.ctx, url, MAX_BACKUP_BYTES)
            .await
    }
}

/// Parses `/api/znoise`'s `fids` and `gids` fields, supporting both parallel arrays and `{ plain: noised }` maps.
pub fn parse_noised_ids_reply(
    asked_users: &[String],
    asked_groups: &[String],
    data: &Value,
) -> NoisedIdsReply {
    fn maybe_parse(v: &Value) -> Value {
        if let Some(s) = v.as_str() {
            let trimmed = s.trim();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                if let Ok(p) = parse_json_keeping_bigints(trimmed) {
                    return p;
                }
            }
        }
        v.clone()
    }

    fn val_str(v: &Value) -> String {
        match v {
            Value::String(s) => s.trim().to_string(),
            Value::Number(n) => n.to_string(),
            _ => String::new(),
        }
    }

    fn apply(asked: &[String], raw: Option<&Value>, out: &mut HashMap<String, String>) {
        let Some(raw_v) = raw else { return };
        let parsed = maybe_parse(raw_v);
        match parsed {
            Value::Array(arr) => {
                if arr.len() != asked.len() {
                    return;
                }
                for (plain, item) in asked.iter().zip(arr.iter()) {
                    let noised = val_str(item);
                    if !noised.is_empty() && noised != "0" {
                        out.insert(plain.clone(), noised);
                    }
                }
            }
            Value::Object(map) => {
                for plain in asked {
                    if let Some(v) = map.get(plain) {
                        let noised = val_str(v);
                        if !noised.is_empty() && noised != "0" {
                            out.insert(plain.clone(), noised);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let unpacked = maybe_parse(data);
    let mut users = HashMap::new();
    let mut groups = HashMap::new();
    let mut reply_shape = "{}".to_string();

    if let Some(obj) = unpacked.as_object() {
        apply(asked_users, obj.get("fids"), &mut users);
        apply(asked_groups, obj.get("gids"), &mut groups);
        let mut keys: Vec<&String> = obj.keys().collect();
        keys.sort();
        let parts: Vec<String> = keys
            .into_iter()
            .map(|k| {
                let kind = match &obj[k] {
                    Value::Array(a) => format!("arr({})", a.len()),
                    Value::Object(o) => format!("obj({})", o.len()),
                    Value::String(_) => "string".to_string(),
                    Value::Number(_) => "number".to_string(),
                    Value::Bool(_) => "boolean".to_string(),
                    Value::Null => "null".to_string(),
                };
                format!("{k}:{kind}")
            })
            .collect();
        reply_shape = format!("{{{}}}", parts.join(","));
    }

    NoisedIdsReply {
        users,
        groups,
        reply_shape,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_noised_ids_supports_arrays_and_maps() {
        let u = vec!["111".to_string(), "222".to_string()];
        let g = vec!["999".to_string()];

        let arr_data = json!({
            "fids": ["7000000000000000111", "0"],
            "gids": ["8000000000000000999"]
        });
        let r1 = parse_noised_ids_reply(&u, &g, &arr_data);
        assert_eq!(r1.users.get("111").map(|s| s.as_str()), Some("7000000000000000111"));
        assert!(!r1.users.contains_key("222"));
        assert_eq!(r1.groups.get("999").map(|s| s.as_str()), Some("8000000000000000999"));

        let map_data = json!({
            "fids": "{\"111\":7000000000000000111,\"222\":7000000000000000222}",
            "gids": { "999": "8000000000000000999" }
        });
        let r2 = parse_noised_ids_reply(&u, &g, &map_data);
        assert_eq!(r2.users.len(), 2);
        assert_eq!(r2.users.get("222").map(|s| s.as_str()), Some("7000000000000000222"));
        assert_eq!(r2.groups.get("999").map(|s| s.as_str()), Some("8000000000000000999"));
    }
}


use crate::time_util::{now_ms, now_secs, seconds_until_next_local_8am};
use serde_json::{json, Value};

use crate::api::ZaloClient;
use crate::error::Result;
use crate::types::{LastMessageInfo, MuteAction, MuteDuration, ThreadType};

impl ZaloClient {
    /// Marks a conversation as unread (`POST /api/conv/addUnreadMark`).
    pub async fn add_unread_mark(&self, thread_id: &str, thread_type: ThreadType) -> Result<Value> {
        let base = self.require_service("conversation")?;
        let ts = now_ms();
        let is_group = thread_type.is_group();
        let entry = json!([{
            "id": thread_id,
            "cliMsgId": ts.to_string(),
            "fromUid": "0",
            "ts": ts,
        }]);
        let inner = if is_group {
            json!({
                "convsGroup": entry,
                "convsUser": [],
                "imei": self.ctx.imei,
            })
        } else {
            json!({
                "convsUser": entry,
                "convsGroup": [],
                "imei": self.ctx.imei,
            })
        };
        let params = json!({ "param": inner.to_string() });
        let data = self
            .http
            .post_encrypted(
                &self.ctx,
                &format!("{base}/api/conv/addUnreadMark"),
                &params,
                &[],
            )
            .await?;
        Ok(unwrap_string_data_field(data))
    }

    /// Removes unread mark from a conversation (`POST /api/conv/removeUnreadMark`).
    pub async fn remove_unread_mark(
        &self,
        thread_id: &str,
        thread_type: ThreadType,
    ) -> Result<Value> {
        let base = self.require_service("conversation")?;
        let ts = now_ms();
        let is_group = thread_type.is_group();
        let inner = if is_group {
            json!({
                "convsGroup": [thread_id],
                "convsUser": [],
                "convsGroupData": [{ "id": thread_id, "ts": ts }],
                "convsUserData": [],
            })
        } else {
            json!({
                "convsUser": [thread_id],
                "convsGroup": [],
                "convsUserData": [{ "id": thread_id, "ts": ts }],
                "convsGroupData": [],
            })
        };
        let params = json!({ "param": inner.to_string() });
        let data = self
            .http
            .post_encrypted(
                &self.ctx,
                &format!("{base}/api/conv/removeUnreadMark"),
                &params,
                &[],
            )
            .await?;
        Ok(unwrap_string_data_field(data))
    }

    /// Pins or unpins one or more conversations (`POST /api/pinconvers/updatev2`).
    pub async fn set_pinned_conversations(
        &self,
        pinned: bool,
        thread_ids: &[String],
        thread_type: ThreadType,
    ) -> Result<Value> {
        let base = self.require_service("conversation")?;
        let prefix = if thread_type.is_group() { "g" } else { "u" };
        let conversations: Vec<String> = thread_ids
            .iter()
            .map(|id| format!("{prefix}{id}"))
            .collect();
        let params = json!({
            "actionType": if pinned { 1 } else { 2 },
            "conversations": conversations,
        });
        self.http
            .post_encrypted(
                &self.ctx,
                &format!("{base}/api/pinconvers/updatev2"),
                &params,
                &[],
            )
            .await
    }

    /// Lists pinned conversations (`GET /api/pinconvers/list`).
    pub async fn get_pin_conversations(&self) -> Result<Value> {
        let base = self.require_service("conversation")?;
        let params = json!({ "imei": self.ctx.imei });
        self.http
            .get_encrypted(&self.ctx, &format!("{base}/api/pinconvers/list"), &params, &[])
            .await
    }

    /// Mutes or unmutes a conversation (`POST /api/social/profile/setmute`).
    pub async fn set_mute(
        &self,
        action: MuteAction,
        duration: MuteDuration,
        thread_id: &str,
        thread_type: ThreadType,
    ) -> Result<Value> {
        let base = self.require_service("profile")?;
        let mute_duration: i64 = if action == MuteAction::Unmute {
            -1
        } else {
            match duration {
                MuteDuration::OneHour => 3600,
                MuteDuration::FourHours => 14400,
                MuteDuration::Forever => -1,
                MuteDuration::Seconds(s) => s,
                MuteDuration::Until8Am => seconds_until_next_8am(),
            }
        };
        let params = json!({
            "toid": thread_id,
            "duration": mute_duration,
            "action": action as i64,
            "startTime": now_secs(),
            "muteType": if thread_type.is_group() { 2 } else { 1 },
            "imei": self.ctx.imei,
        });
        self.http
            .post_encrypted(
                &self.ctx,
                &format!("{base}/api/social/profile/setmute"),
                &params,
                &[],
            )
            .await
    }

    /// Fetches currently muted conversations (`GET /api/social/profile/getmute`).
    pub async fn get_mute(&self) -> Result<Value> {
        let base = self.require_service("profile")?;
        let params = json!({ "imei": self.ctx.imei });
        self.http
            .get_encrypted(
                &self.ctx,
                &format!("{base}/api/social/profile/getmute"),
                &params,
                &[],
            )
            .await
    }

    /// Deletes a chat conversation locally (`POST /api/message/deleteconver` or `/api/group/deleteconver`).
    pub async fn delete_chat(
        &self,
        last_message: &LastMessageInfo,
        thread_id: &str,
        thread_type: ThreadType,
    ) -> Result<Value> {
        let ts_str = now_ms().to_string();
        let (url, params) = match thread_type {
            ThreadType::User => {
                let base = self.require_service("chat")?;
                (
                    format!("{base}/api/message/deleteconver"),
                    json!({
                        "toid": thread_id,
                        "cliMsgId": ts_str,
                        "conver": last_message,
                        "onlyMe": 1,
                        "imei": self.ctx.imei,
                    }),
                )
            }
            ThreadType::Group => {
                let base = self.require_service("group")?;
                (
                    format!("{base}/api/group/deleteconver"),
                    json!({
                        "grid": thread_id,
                        "cliMsgId": ts_str,
                        "conver": last_message,
                        "onlyMe": 1,
                        "imei": self.ctx.imei,
                    }),
                )
            }
        };
        self.http
            .post_encrypted(&self.ctx, &url, &params, &[("nretry", "0")])
            .await
    }
}

fn unwrap_string_data_field(data: Value) -> Value {
    if let Some(obj) = data.as_object() {
        if let Some(Value::String(inner_s)) = obj.get("data") {
            if let Ok(parsed) = serde_json::from_str::<Value>(inner_s) {
                return json!({
                    "data": parsed,
                    "status": obj.get("status").cloned().unwrap_or(Value::Null),
                });
            }
        }
    }
    data
}

fn seconds_until_next_8am() -> i64 {
    seconds_until_next_local_8am(now_ms())
}

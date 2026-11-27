use chrono::Utc;
use reqwest::Method;
use serde_json::{json, Map, Value};

use crate::api::ZaloClient;
use crate::error::{ProtoError, Result};
use crate::http::{handle_zalo_response, ZaloHttp};
use crate::types::CreateGroupOptions;

impl ZaloClient {
    /// Lists all group IDs and versions (`GET /api/group/getlg/v4`).
    pub async fn get_all_groups(&self) -> Result<Value> {
        let base = self.require_service("group_poll")?;
        let url = ZaloHttp::make_ctx_url(
            &self.ctx,
            &format!("{base}/api/group/getlg/v4"),
            &[],
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
        handle_zalo_response(&self.ctx.secret_key, resp, true).await
    }

    /// Fetches group metadata for one or more group IDs (`POST /api/group/getmg-v2`).
    pub async fn get_group_info(&self, group_ids: &[String]) -> Result<Value> {
        let base = self.require_service("group")?;
        let mut grid_map = Map::new();
        for id in group_ids {
            grid_map.insert(id.clone(), Value::Number(0.into()));
        }
        let params = json!({
            "gridVerMap": Value::Object(grid_map).to_string(),
        });
        self.http
            .post_encrypted(
                &self.ctx,
                &format!("{base}/api/group/getmg-v2"),
                &params,
                &[],
            )
            .await
    }

    /// Fetches member profiles for a list of member IDs (`GET /api/social/group/members`).
    pub async fn get_group_members_info(&self, member_ids: &[String]) -> Result<Value> {
        let base = self.require_service("profile")?;
        let mapped: Vec<String> = member_ids
            .iter()
            .map(|id| {
                if id.ends_with("_0") {
                    id.clone()
                } else {
                    format!("{id}_0")
                }
            })
            .collect();
        let params = json!({
            "friend_pversion_map": mapped,
        });
        self.http
            .get_encrypted(
                &self.ctx,
                &format!("{base}/api/social/group/members"),
                &params,
                &[],
            )
            .await
    }

    /// Creates a new group (`POST /api/group/create/v2` with encrypted `params` in query).
    pub async fn create_group(&self, options: &CreateGroupOptions) -> Result<Value> {
        if options.members.is_empty() {
            return Err(ProtoError::InvalidArg(
                "Group must have at least one member".into(),
            ));
        }
        let base = self.require_service("group")?;
        let now = Utc::now().timestamp_millis();
        let (gname, name_changed) = match options.name.as_deref() {
            Some(n) if !n.is_empty() => (n.to_string(), 1),
            _ => (now.to_string(), 0),
        };
        let member_types: Vec<i32> = vec![-1; options.members.len()];
        let params = json!({
            "clientId": now,
            "gname": gname,
            "gdesc": Value::Null,
            "members": options.members,
            "membersTypes": member_types,
            "nameChanged": name_changed,
            "createLink": 1,
            "clientLang": self.ctx.language,
            "imei": self.ctx.imei,
            "zsource": 601,
        });
        self.http
            .post_encrypted_query(
                &self.ctx,
                &format!("{base}/api/group/create/v2"),
                &params,
                &[],
            )
            .await
    }

    /// Invites one or more users to a group (`POST /api/group/invite/v2`).
    pub async fn add_user_to_group(&self, member_ids: &[String], group_id: &str) -> Result<Value> {
        let base = self.require_service("group")?;
        let member_types: Vec<i32> = vec![-1; member_ids.len()];
        let params = json!({
            "grid": group_id,
            "members": member_ids,
            "memberTypes": member_types,
            "imei": self.ctx.imei,
            "clientLang": self.ctx.language,
        });
        self.http
            .post_encrypted(
                &self.ctx,
                &format!("{base}/api/group/invite/v2"),
                &params,
                &[],
            )
            .await
    }

    /// Removes one or more members from a group (`POST /api/group/kickout`).
    pub async fn remove_user_from_group(
        &self,
        member_ids: &[String],
        group_id: &str,
    ) -> Result<Value> {
        let base = self.require_service("group")?;
        let params = json!({
            "grid": group_id,
            "members": member_ids,
            "imei": self.ctx.imei,
        });
        self.http
            .post_encrypted(
                &self.ctx,
                &format!("{base}/api/group/kickout"),
                &params,
                &[],
            )
            .await
    }

    /// Renames a group (`POST /api/group/updateinfo`).
    pub async fn change_group_name(&self, name: &str, group_id: &str) -> Result<Value> {
        let base = self.require_service("group")?;
        let gname = if name.is_empty() {
            Utc::now().timestamp_millis().to_string()
        } else {
            name.to_string()
        };
        let params = json!({
            "grid": group_id,
            "gname": gname,
            "imei": self.ctx.imei,
        });
        self.http
            .post_encrypted(
                &self.ctx,
                &format!("{base}/api/group/updateinfo"),
                &params,
                &[],
            )
            .await
    }

    /// Leaves a group (`POST /api/group/leave`).
    pub async fn leave_group(&self, group_id: &str, silent: bool) -> Result<Value> {
        let base = self.require_service("group")?;
        let params = json!({
            "grids": [group_id],
            "imei": self.ctx.imei,
            "silent": if silent { 1 } else { 0 },
            "language": self.ctx.language,
        });
        self.http
            .post_encrypted(&self.ctx, &format!("{base}/api/group/leave"), &params, &[])
            .await
    }

    /// Gets group invite link details (`GET /api/group/link/detail` on `group` service).
    pub async fn get_group_link_detail(&self, group_id: &str) -> Result<Value> {
        let base = self.require_service("group")?;
        let params = json!({
            "grid": group_id,
            "imei": self.ctx.imei,
        });
        self.http
            .get_encrypted(
                &self.ctx,
                &format!("{base}/api/group/link/detail"),
                &params,
                &[],
            )
            .await
    }

    /// Enables and creates a new group invite link (`GET /api/group/link/new` on `group` service).
    pub async fn enable_group_link(&self, group_id: &str) -> Result<Value> {
        let base = self.require_service("group")?;
        let params = json!({
            "grid": group_id,
            "imei": self.ctx.imei,
        });
        self.http
            .get_encrypted(
                &self.ctx,
                &format!("{base}/api/group/link/new"),
                &params,
                &[],
            )
            .await
    }

    /// Disables the group invite link (`GET /api/group/link/disable` on `group` service).
    pub async fn disable_group_link(&self, group_id: &str) -> Result<Value> {
        let base = self.require_service("group")?;
        let params = json!({
            "grid": group_id,
        });
        self.http
            .get_encrypted(
                &self.ctx,
                &format!("{base}/api/group/link/disable"),
                &params,
                &[],
            )
            .await
    }

    /// Joins a group via invite link (`GET /api/group/link/join` on `group` service).
    pub async fn join_group_link(&self, link: &str) -> Result<Value> {
        let base = self.require_service("group")?;
        let params = json!({
            "link": link,
            "clientLang": self.ctx.language,
        });
        self.http
            .get_encrypted(
                &self.ctx,
                &format!("{base}/api/group/link/join"),
                &params,
                &[],
            )
            .await
    }

    /// Promotes a group member to deputy/admin (`GET /api/group/admins/add` on `group` service).
    pub async fn add_group_deputy(&self, member_id: &str, group_id: &str) -> Result<Value> {
        let base = self.require_service("group")?;
        let params = json!({
            "grid": group_id,
            "members": [member_id],
            "imei": self.ctx.imei,
        });
        self.http
            .get_encrypted(
                &self.ctx,
                &format!("{base}/api/group/admins/add"),
                &params,
                &[],
            )
            .await
    }

    /// Removes a group deputy/admin (`GET /api/group/admins/remove` on `group` service).
    pub async fn remove_group_deputy(&self, member_id: &str, group_id: &str) -> Result<Value> {
        let base = self.require_service("group")?;
        let params = json!({
            "grid": group_id,
            "members": [member_id],
            "imei": self.ctx.imei,
        });
        self.http
            .get_encrypted(
                &self.ctx,
                &format!("{base}/api/group/admins/remove"),
                &params,
                &[],
            )
            .await
    }

    /// Lists board items (notes, pinned messages, polls) in a group (`GET /api/board/list` on `group_board` service).
    pub async fn get_list_board(
        &self,
        group_id: &str,
        page: Option<u32>,
        count: Option<u32>,
    ) -> Result<Value> {
        let base = self.require_service("group_board")?;
        let params = json!({
            "group_id": group_id,
            "board_type": 0,
            "page": page.unwrap_or(1),
            "count": count.unwrap_or(20),
            "last_id": 0,
            "last_type": 0,
            "imei": self.ctx.imei,
        });
        let mut data = self
            .http
            .get_encrypted(
                &self.ctx,
                &format!("{base}/api/board/list"),
                &params,
                &[],
            )
            .await?;
        if let Some(items) = data.get_mut("items").and_then(|v| v.as_array_mut()) {
            for item in items.iter_mut() {
                let board_type = item.get("boardType").and_then(|v| v.as_i64()).unwrap_or(0);
                if board_type != 3 {
                    if let Some(detail) = item.get_mut("data").and_then(|v| v.as_object_mut()) {
                        if let Some(params_str) = detail.get("params").and_then(|v| v.as_str()) {
                            if let Ok(parsed) = serde_json::from_str::<Value>(params_str) {
                                detail.insert("params".to_string(), parsed);
                            }
                        }
                    }
                }
            }
        }
        Ok(data)
    }

    /// Creates a note on the group board (`POST /api/board/topic/createv2` on `group_board` service).
    pub async fn create_note(
        &self,
        group_id: &str,
        title: &str,
        pin_act: bool,
    ) -> Result<Value> {
        let base = self.require_service("group_board")?;
        let params = json!({
            "grid": group_id,
            "type": 0,
            "color": -16777216,
            "emoji": "",
            "startTime": -1,
            "duration": -1,
            "params": json!({ "title": title }).to_string(),
            "repeat": 0,
            "src": 1,
            "imei": self.ctx.imei,
            "pinAct": if pin_act { 1 } else { 0 },
        });
        let mut data = self
            .http
            .post_encrypted(
                &self.ctx,
                &format!("{base}/api/board/topic/createv2"),
                &params,
                &[],
            )
            .await?;
        if let Some(obj) = data.as_object_mut() {
            if let Some(params_str) = obj.get("params").and_then(|v| v.as_str()) {
                if let Ok(parsed) = serde_json::from_str::<Value>(params_str) {
                    obj.insert("params".to_string(), parsed);
                }
            }
        }
        Ok(data)
    }
}

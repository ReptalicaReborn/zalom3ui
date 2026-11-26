use serde_json::{json, Value};

use crate::api::ZaloClient;
use crate::error::{ProtoError, Result};
use crate::types::AvatarSize;

impl ZaloClient {
    /// Lists all friends (`GET /api/social/friend/getfriends`).
    pub async fn get_all_friends(
        &self,
        count: Option<u32>,
        page: Option<u32>,
        avatar_size: Option<AvatarSize>,
    ) -> Result<Value> {
        let base = self.require_service("profile")?;
        let params = json!({
            "incInvalid": 1,
            "page": page.unwrap_or(1),
            "count": count.unwrap_or(20000),
            "avatar_size": avatar_size.unwrap_or(AvatarSize::Small) as u32,
            "actiontime": 0,
            "imei": self.ctx.imei,
        });
        self.http
            .get_encrypted(
                &self.ctx,
                &format!("{base}/api/social/friend/getfriends"),
                &params,
                &[],
            )
            .await
    }

    /// Fetches profile info for one or more user IDs (`POST /api/social/friend/getprofiles/v2`).
    pub async fn get_user_info(
        &self,
        user_ids: &[String],
        avatar_size: Option<AvatarSize>,
    ) -> Result<Value> {
        if user_ids.is_empty() {
            return Err(ProtoError::InvalidArg("Missing user id".into()));
        }
        let base = self.require_service("profile")?;
        let mapped: Vec<String> = user_ids
            .iter()
            .map(|id| {
                if id.contains('_') {
                    id.clone()
                } else {
                    format!("{id}_0")
                }
            })
            .collect();
        let phonebook_ver = self
            .ctx
            .extra_ver
            .get("phonebook")
            .cloned()
            .unwrap_or(Value::Null);
        let params = json!({
            "phonebook_version": phonebook_ver,
            "friend_pversion_map": mapped,
            "avatar_size": avatar_size.unwrap_or(AvatarSize::Small) as u32,
            "language": self.ctx.language,
            "show_online_status": 1,
            "imei": self.ctx.imei,
        });
        self.http
            .post_encrypted(
                &self.ctx,
                &format!("{base}/api/social/friend/getprofiles/v2"),
                &params,
                &[],
            )
            .await
    }

    /// Fetches last online status for a user (`GET /api/social/profile/lastOnline`).
    pub async fn last_online(&self, uid: &str) -> Result<Value> {
        let base = self.require_service("profile")?;
        let params = json!({
            "uid": uid,
            "conv_type": 1,
            "imei": self.ctx.imei,
        });
        self.http
            .get_encrypted(
                &self.ctx,
                &format!("{base}/api/social/profile/lastOnline"),
                &params,
                &[],
            )
            .await
    }

    /// Finds a user by phone number (`GET /api/friend/profile/get`).
    /// Note: code `216` (not found) resolves to `Value::Null` matching `zca-js`.
    pub async fn find_user(
        &self,
        phone_number: &str,
        avatar_size: Option<AvatarSize>,
    ) -> Result<Value> {
        if phone_number.is_empty() {
            return Err(ProtoError::InvalidArg("Missing phoneNumber".into()));
        }
        let base = self.require_service("friend")?;
        let normalized_phone = if phone_number.starts_with('0') && self.ctx.language == "vi" {
            format!("84{}", &phone_number[1..])
        } else {
            phone_number.to_string()
        };
        let params = json!({
            "phone": normalized_phone,
            "avatar_size": avatar_size.unwrap_or(AvatarSize::Large) as u32,
            "language": self.ctx.language,
            "imei": self.ctx.imei,
            "reqSrc": 40,
        });
        match self
            .http
            .get_encrypted(
                &self.ctx,
                &format!("{base}/api/friend/profile/get"),
                &params,
                &[],
            )
            .await
        {
            Ok(v) => Ok(v),
            Err(ProtoError::Api { code: 216, .. }) => Ok(Value::Null),
            Err(e) => Err(e),
        }
    }

    /// Lists friend recommendations and incoming friend requests (`GET /api/friend/recommendsv2/list`).
    pub async fn get_friend_recommendations(&self) -> Result<Value> {
        let base = self.require_service("friend")?;
        let params = json!({ "imei": self.ctx.imei });
        self.http
            .get_encrypted(
                &self.ctx,
                &format!("{base}/api/friend/recommendsv2/list"),
                &params,
                &[],
            )
            .await
    }

    /// Lists outgoing sent friend requests (`GET /api/friend/requested/list`).
    pub async fn get_sent_friend_request(&self) -> Result<Value> {
        let base = self.require_service("friend")?;
        let params = json!({ "imei": self.ctx.imei });
        self.http
            .get_encrypted(
                &self.ctx,
                &format!("{base}/api/friend/requested/list"),
                &params,
                &[],
            )
            .await
    }

    /// Accepts an incoming friend request (`POST /api/friend/accept`).
    pub async fn accept_friend_request(&self, friend_id: &str) -> Result<Value> {
        let base = self.require_service("friend")?;
        let params = json!({
            "fid": friend_id,
            "language": self.ctx.language,
        });
        self.http
            .post_encrypted(&self.ctx, &format!("{base}/api/friend/accept"), &params, &[])
            .await
    }

    /// Rejects an incoming friend request (`POST /api/friend/reject`).
    pub async fn reject_friend_request(&self, friend_id: &str) -> Result<Value> {
        let base = self.require_service("friend")?;
        let params = json!({ "fid": friend_id });
        self.http
            .post_encrypted(&self.ctx, &format!("{base}/api/friend/reject"), &params, &[])
            .await
    }

    /// Sends a friend request (`POST /api/friend/sendreq`).
    pub async fn send_friend_request(&self, msg: &str, user_id: &str) -> Result<Value> {
        let base = self.require_service("friend")?;
        let params = json!({
            "toid": user_id,
            "msg": msg,
            "reqsrc": 30,
            "imei": self.ctx.imei,
            "language": self.ctx.language,
            "srcParams": json!({ "uidTo": user_id }).to_string(),
        });
        self.http
            .post_encrypted(&self.ctx, &format!("{base}/api/friend/sendreq"), &params, &[])
            .await
    }

    /// Cancels/undoes a sent friend request (`POST /api/friend/undo`).
    pub async fn undo_friend_request(&self, friend_id: &str) -> Result<Value> {
        let base = self.require_service("friend")?;
        let params = json!({ "fid": friend_id });
        self.http
            .post_encrypted(&self.ctx, &format!("{base}/api/friend/undo"), &params, &[])
            .await
    }
}

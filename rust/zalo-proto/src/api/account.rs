use reqwest::Method;
use serde_json::Value;

use crate::api::ZaloClient;
use crate::error::Result;
use crate::http::{handle_zalo_response, ZaloHttp};

impl ZaloClient {
    /// Fetches the logged-in user's account profile (`GET /api/social/profile/me-v2`).
    pub async fn fetch_account_info(&self) -> Result<Value> {
        let base = self.require_service("profile")?;
        let url = ZaloHttp::make_ctx_url(
            &self.ctx,
            &format!("{base}/api/social/profile/me-v2"),
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
}

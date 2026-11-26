use std::sync::{Arc, RwLock};
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, Response};
use serde_json::Value;
use url::Url;

use crate::cookies::{is_zalo_cookie_host, ZaloCookieJar};
use crate::crypto::{decode_aes, encode_aes};
use crate::error::{ProtoError, Result};
use crate::types::ZaloContext;

const MAX_REDIRECTS: usize = 10;

/// HTTP client tailored for Zalo Web API requests, manual redirect handling, and cookie jar sync.
#[derive(Clone)]
pub struct ZaloHttp {
    client: reqwest::Client,
}

impl std::fmt::Debug for ZaloHttp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZaloHttp").finish_non_exhaustive()
    }
}

impl ZaloHttp {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(20))
            .build()?;
        Ok(Self { client })
    }

    pub fn inner(&self) -> &reqwest::Client {
        &self.client
    }

    /// Builds a URL by appending query pairs and optionally `zpw_ver` & `zpw_type` (matching `zca-js`'s `makeURL`).
    pub fn make_url(
        base: &str,
        query: &[(&str, &str)],
        with_api_version: bool,
        api_version: u32,
        api_type: u32,
    ) -> Result<String> {
        let mut url = Url::parse(base)?;
        {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in query {
                pairs.append_pair(k, v);
            }
        }
        if with_api_version {
            let has_ver = url.query_pairs().any(|(k, _)| k == "zpw_ver");
            let has_type = url.query_pairs().any(|(k, _)| k == "zpw_type");
            let mut pairs = url.query_pairs_mut();
            if !has_ver {
                pairs.append_pair("zpw_ver", &api_version.to_string());
            }
            if !has_type {
                pairs.append_pair("zpw_type", &api_type.to_string());
            }
        }
        Ok(url.to_string())
    }

    /// Convenience helper using `ctx.api_version` and `ctx.api_type`.
    pub fn make_ctx_url(
        ctx: &ZaloContext,
        base: &str,
        query: &[(&str, &str)],
        with_api_version: bool,
    ) -> Result<String> {
        Self::make_url(base, query, with_api_version, ctx.api_version, ctx.api_type)
    }

    /// Sends a raw request without Zalo default headers or cookie jar interaction.
    pub async fn request_raw(
        &self,
        method: Method,
        url: &str,
        headers: Option<HeaderMap>,
        body: Option<Vec<u8>>,
    ) -> Result<Response> {
        let mut req = self.client.request(method, url);
        if let Some(h) = headers {
            req = req.headers(h);
        }
        if let Some(b) = body {
            req = req.body(b);
        }
        Ok(req.send().await?)
    }

    /// Sends an HTTP request with Zalo default headers, syncing `Set-Cookie` into `jar` and
    /// manually following `Location` redirects with `GET` and `Referer: https://id.zalo.me/`.
    pub async fn request_with_cookies(
        &self,
        jar: &Arc<RwLock<ZaloCookieJar>>,
        user_agent: &str,
        method: Method,
        url: &str,
        extra_headers: Option<&[(&str, &str)]>,
        body: Option<Vec<u8>>,
    ) -> Result<Response> {
        let mut current_url = Url::parse(url)?;
        let mut current_method = method;
        let mut current_body = body;
        let mut referer_override: Option<String> = None;

        for _hop in 0..=MAX_REDIRECTS {
            let cookie_header = jar
                .read()
                .map(|j| j.cookie_header_for(&current_url))
                .unwrap_or_default();

            let mut headers = default_zalo_headers(user_agent, &cookie_header)?;
            if let Some(extra) = extra_headers {
                for (k, v) in extra {
                    if let (Ok(name), Ok(val)) = (
                        HeaderName::from_bytes(k.as_bytes()),
                        HeaderValue::from_str(v),
                    ) {
                        headers.insert(name, val);
                    }
                }
            }
            if let Some(ref ref_val) = referer_override {
                if let Ok(val) = HeaderValue::from_str(ref_val) {
                    headers.insert(reqwest::header::REFERER, val);
                }
            }
            // Refresh Cookie header for this hop in case extra_headers or redirects changed host.
            if !cookie_header.is_empty() {
                if let Ok(val) = HeaderValue::from_str(&cookie_header) {
                    headers.insert(reqwest::header::COOKIE, val);
                }
            } else {
                headers.remove(reqwest::header::COOKIE);
            }

            let mut req = self
                .client
                .request(current_method.clone(), current_url.as_str())
                .headers(headers);
            if let Some(b) = current_body.take() {
                req = req.body(b);
            }

            let resp = req.send().await?;

            // Capture all Set-Cookie headers on every hop (including 3xx redirects).
            let set_cookies: Vec<String> = resp
                .headers()
                .get_all(reqwest::header::SET_COOKIE)
                .iter()
                .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
                .collect();
            if !set_cookies.is_empty() {
                if let Ok(mut guard) = jar.write() {
                    for sc in &set_cookies {
                        guard.set_cookie_header(sc, &current_url);
                    }
                }
            }

            if let Some(loc) = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
            {
                let next_url = current_url.join(loc)?;
                current_url = next_url;
                current_method = Method::GET;
                current_body = None;
                referer_override = Some("https://id.zalo.me/".to_string());
                continue;
            }

            return Ok(resp);
        }

        Err(ProtoError::HttpStatus {
            status: 310,
            message: "Too many redirects".into(),
        })
    }

    /// Sends a form-encoded POST request with Zalo cookies and returns the response.
    pub async fn post_form(
        &self,
        jar: &Arc<RwLock<ZaloCookieJar>>,
        user_agent: &str,
        url: &str,
        extra_headers: Option<&[(&str, &str)]>,
        form: &[(&str, &str)],
    ) -> Result<Response> {
        let encoded = form_urlencoded_body(form);
        self.request_with_cookies(
            jar,
            user_agent,
            Method::POST,
            url,
            extra_headers,
            Some(encoded.into_bytes()),
        )
        .await
    }

    /// Standard encrypted GET API call: encrypts `params` with `ctx.secret_key`, sends as `?params=...`,
    /// and decrypts/resolves the Zalo JSON envelope.
    pub async fn get_encrypted(
        &self,
        ctx: &ZaloContext,
        base_url: &str,
        params: &Value,
        extra_query: &[(&str, &str)],
    ) -> Result<Value> {
        let encrypted = encode_aes(&ctx.secret_key, &params.to_string())?;
        let mut query: Vec<(&str, &str)> = Vec::with_capacity(extra_query.len() + 1);
        query.extend_from_slice(extra_query);
        query.push(("params", &encrypted));
        let url = Self::make_ctx_url(ctx, base_url, &query, true)?;
        let resp = self
            .request_with_cookies(&ctx.cookies, &ctx.user_agent, Method::GET, &url, None, None)
            .await?;
        handle_zalo_response(&ctx.secret_key, resp, true).await
    }

    /// Standard encrypted POST API call: encrypts `params` with `ctx.secret_key`, sends as
    /// `application/x-www-form-urlencoded` body `params=<enc>`, and decrypts/resolves the Zalo JSON envelope.
    pub async fn post_encrypted(
        &self,
        ctx: &ZaloContext,
        base_url: &str,
        params: &Value,
        extra_query: &[(&str, &str)],
    ) -> Result<Value> {
        let encrypted = encode_aes(&ctx.secret_key, &params.to_string())?;
        let url = Self::make_ctx_url(ctx, base_url, extra_query, true)?;
        let body = form_urlencoded_body(&[("params", &encrypted)]);
        let resp = self
            .request_with_cookies(
                &ctx.cookies,
                &ctx.user_agent,
                Method::POST,
                &url,
                None,
                Some(body.into_bytes()),
            )
            .await?;
        handle_zalo_response(&ctx.secret_key, resp, true).await
    }

    /// Encrypted POST API call where `params` is passed in the URL query string (used by e.g. `createGroup`).
    pub async fn post_encrypted_query(
        &self,
        ctx: &ZaloContext,
        base_url: &str,
        params: &Value,
        extra_query: &[(&str, &str)],
    ) -> Result<Value> {
        let encrypted = encode_aes(&ctx.secret_key, &params.to_string())?;
        let mut query: Vec<(&str, &str)> = Vec::with_capacity(extra_query.len() + 1);
        query.extend_from_slice(extra_query);
        query.push(("params", &encrypted));
        let url = Self::make_ctx_url(ctx, base_url, &query, true)?;
        let resp = self
            .request_with_cookies(
                &ctx.cookies,
                &ctx.user_agent,
                Method::POST,
                &url,
                None,
                None,
            )
            .await?;
        handle_zalo_response(&ctx.secret_key, resp, true).await
    }

    /// Sends a multipart/form-data chunk upload with Zalo cookies and resolves the encrypted JSON response.
    pub async fn post_multipart_chunk(
        &self,
        ctx: &ZaloContext,
        url: &str,
        field_name: &str,
        filename: &str,
        content_type: &str,
        data: Vec<u8>,
    ) -> Result<Value> {
        let current_url = Url::parse(url)?;
        let cookie_header = ctx
            .cookies
            .read()
            .map(|j| j.cookie_header_for(&current_url))
            .unwrap_or_default();

        let mut headers = default_zalo_headers(&ctx.user_agent, &cookie_header)?;
        // Remove form-urlencoded content-type so reqwest sets multipart boundary header.
        headers.remove(reqwest::header::CONTENT_TYPE);

        let part = reqwest::multipart::Part::bytes(data)
            .file_name(filename.to_string())
            .mime_str(content_type)
            .map_err(|e| ProtoError::InvalidArg(format!("invalid mime type: {e}")))?;
        let form = reqwest::multipart::Form::new().part(field_name.to_string(), part);

        let resp = self
            .client
            .post(url)
            .headers(headers)
            .multipart(form)
            .send()
            .await?;

        let set_cookies: Vec<String> = resp
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
            .collect();
        if !set_cookies.is_empty() {
            if let Ok(mut guard) = ctx.cookies.write() {
                for sc in &set_cookies {
                    guard.set_cookie_header(sc, &current_url);
                }
            }
        }

        handle_zalo_response(&ctx.secret_key, resp, true).await
    }

    /// Downloads a backup file following HTTPS redirects manually and sending Zalo cookies only to Zalo hosts.
    pub async fn download_backup_bytes(
        &self,
        ctx: &ZaloContext,
        url: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>> {
        let mut target = Url::parse(url)?;
        if target.scheme() != "https" {
            return Err(ProtoError::InvalidArg("backup URL must be https".into()));
        }

        let mut resp_opt: Option<Response> = None;
        for _hop in 0..5 {
            let mut headers = HeaderMap::new();
            headers.insert(
                reqwest::header::USER_AGENT,
                HeaderValue::from_str(&ctx.user_agent)
                    .map_err(|e| ProtoError::InvalidArg(e.to_string()))?,
            );
            headers.insert(reqwest::header::ACCEPT, HeaderValue::from_static("*/*"));
            headers.insert(
                reqwest::header::ACCEPT_LANGUAGE,
                HeaderValue::from_static("vi-VN,vi;q=0.9,en-US;q=0.8,en;q=0.7"),
            );
            if let Some(host) = target.host_str() {
                if is_zalo_cookie_host(host) {
                    let cookie = ctx
                        .cookies
                        .read()
                        .map(|j| j.backup_cookie_header(&target, &ctx.uid))
                        .unwrap_or_default();
                    if !cookie.is_empty() {
                        if let Ok(val) = HeaderValue::from_str(&cookie) {
                            headers.insert(reqwest::header::COOKIE, val);
                        }
                    }
                    headers.insert(
                        reqwest::header::REFERER,
                        HeaderValue::from_static("https://chat.zalo.me/"),
                    );
                    headers.insert(
                        reqwest::header::ORIGIN,
                        HeaderValue::from_static("https://chat.zalo.me"),
                    );
                }
            }

            let resp = self.client.get(target.as_str()).headers(headers).send().await?;
            let status = resp.status().as_u16();
            if (300..400).contains(&status) {
                if let Some(loc) = resp
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                {
                    target = target.join(loc)?;
                    if target.scheme() != "https" {
                        return Err(ProtoError::InvalidArg("redirected off https".into()));
                    }
                    continue;
                }
            }
            resp_opt = Some(resp);
            break;
        }

        let mut resp = resp_opt.ok_or_else(|| ProtoError::HttpStatus {
            status: 310,
            message: "Too many redirects".into(),
        })?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(ProtoError::HttpStatus {
                status,
                message: text.chars().take(200).collect(),
            });
        }
        if let Some(cl) = resp.content_length() {
            if cl as usize > max_bytes {
                return Err(ProtoError::HttpStatus {
                    status: 413,
                    message: "Backup exceeds size cap".into(),
                });
            }
        }
        let mut out = Vec::new();
        while let Some(chunk) = resp.chunk().await? {
            if out.len().saturating_add(chunk.len()) > max_bytes {
                return Err(ProtoError::HttpStatus {
                    status: 413,
                    message: "Backup exceeds size cap".into(),
                });
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }
}

/// Parses a standard Zalo API response, decrypting `data` with AES-CBC if `is_encrypted` is true.
pub async fn handle_zalo_response(
    secret_key: &str,
    resp: Response,
    is_encrypted: bool,
) -> Result<Value> {
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        return Err(ProtoError::HttpStatus {
            status: status.as_u16(),
            message: format!("Request failed with status code {}", status.as_u16()),
        });
    }
    parse_zalo_response_text(secret_key, &text, is_encrypted)
}

/// Synchronous inner response parser (also unit-testable without HTTP).
pub fn parse_zalo_response_text(
    secret_key: &str,
    text: &str,
    is_encrypted: bool,
) -> Result<Value> {
    let outer: Value = serde_json::from_str(text)?;
    let outer_code = outer
        .get("error_code")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if outer_code != 0 {
        let msg = outer
            .get("error_message")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Zalo error")
            .to_string();
        return Err(ProtoError::Api {
            code: outer_code,
            message: msg,
        });
    }

    if !is_encrypted {
        return Ok(outer.get("data").cloned().unwrap_or(Value::Null));
    }

    let enc_data = match outer.get("data") {
        Some(Value::String(s)) => s.as_str(),
        Some(other) => return Ok(other.clone()),
        None => return Ok(Value::Null),
    };

    let decoded_str = decode_aes(secret_key, enc_data)?;
    let inner: Value = serde_json::from_str(&decoded_str)?;
    let inner_code = inner
        .get("error_code")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if inner_code != 0 {
        let msg = inner
            .get("error_message")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Zalo error")
            .to_string();
        return Err(ProtoError::Api {
            code: inner_code,
            message: msg,
        });
    }
    Ok(inner.get("data").cloned().unwrap_or(Value::Null))
}

fn default_zalo_headers(user_agent: &str, cookie_header: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(
        reqwest::header::ACCEPT,
        HeaderValue::from_static("application/json, text/plain, */*"),
    );
    headers.insert(
        reqwest::header::ACCEPT_LANGUAGE,
        HeaderValue::from_static("en-US,en;q=0.9"),
    );
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );
    headers.insert(
        reqwest::header::ORIGIN,
        HeaderValue::from_static("https://chat.zalo.me"),
    );
    headers.insert(
        reqwest::header::REFERER,
        HeaderValue::from_static("https://chat.zalo.me/"),
    );
    headers.insert(
        reqwest::header::USER_AGENT,
        HeaderValue::from_str(user_agent)
            .map_err(|e| ProtoError::InvalidArg(format!("invalid user_agent: {e}")))?,
    );
    if !cookie_header.is_empty() {
        headers.insert(
            reqwest::header::COOKIE,
            HeaderValue::from_str(cookie_header)
                .map_err(|e| ProtoError::InvalidArg(format!("invalid cookie header: {e}")))?,
        );
    }
    Ok(headers)
}

pub fn form_urlencoded_body(pairs: &[(&str, &str)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        serializer.append_pair(k, v);
    }
    serializer.finish()
}

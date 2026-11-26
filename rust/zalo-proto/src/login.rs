use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use chrono::Utc;
use regex::Regex;
use reqwest::Method;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};

use crate::api::ZaloClient;
use crate::cookies::ZaloCookieJar;
use crate::crypto::{
    decode_aes_utf8_key, encode_aes_utf8_key, generate_zalo_uuid, get_sign_key, AesOutputEncoding,
    ParamsEncryptor,
};
use crate::error::{ProtoError, Result};
use crate::http::ZaloHttp;
use crate::types::{
    Credentials, LoginQrEvent, ZaloContext, DEFAULT_API_TYPE, DEFAULT_API_VERSION,
};

pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:133.0) Gecko/20100101 Firefox/133.0";

const QR_TIMEOUT_SECS: u64 = 100;

const QR_PAGE_HEADERS: &[(&str, &str)] = &[
    (
        "accept",
        "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7",
    ),
    (
        "accept-language",
        "vi-VN,vi;q=0.9,fr-FR;q=0.8,fr;q=0.7,en-US;q=0.6,en;q=0.5",
    ),
    ("cache-control", "max-age=0"),
    ("priority", "u=0, i"),
    (
        "sec-ch-ua",
        "\"Chromium\";v=\"130\", \"Google Chrome\";v=\"130\", \"Not?A_Brand\";v=\"99\"",
    ),
    ("sec-ch-ua-mobile", "?0"),
    ("sec-ch-ua-platform", "\"Windows\""),
    ("sec-fetch-dest", "document"),
    ("sec-fetch-mode", "navigate"),
    ("sec-fetch-site", "same-site"),
    ("sec-fetch-user", "?1"),
    ("upgrade-insecure-requests", "1"),
    ("referer", "https://chat.zalo.me/"),
    ("referrer-policy", "strict-origin-when-cross-origin"),
];

const QR_API_HEADERS_PC: &[(&str, &str)] = &[
    ("accept", "*/*"),
    (
        "accept-language",
        "vi-VN,vi;q=0.9,fr-FR;q=0.8,fr;q=0.7,en-US;q=0.6,en;q=0.5",
    ),
    ("content-type", "application/x-www-form-urlencoded"),
    ("priority", "u=1, i"),
    (
        "sec-ch-ua",
        "\"Chromium\";v=\"130\", \"Google Chrome\";v=\"130\", \"Not?A_Brand\";v=\"99\"",
    ),
    ("sec-ch-ua-mobile", "?0"),
    ("sec-ch-ua-platform", "\"Windows\""),
    ("sec-fetch-dest", "empty"),
    ("sec-fetch-mode", "cors"),
    ("sec-fetch-site", "same-origin"),
    (
        "referer",
        "https://id.zalo.me/account?continue=https%3A%2F%2Fzalo.me%2Fpc",
    ),
    ("referrer-policy", "strict-origin-when-cross-origin"),
];

const QR_API_HEADERS_CHAT: &[(&str, &str)] = &[
    ("accept", "*/*"),
    (
        "accept-language",
        "vi-VN,vi;q=0.9,fr-FR;q=0.8,fr;q=0.7,en-US;q=0.6,en;q=0.5",
    ),
    ("content-type", "application/x-www-form-urlencoded"),
    ("priority", "u=1, i"),
    (
        "sec-ch-ua",
        "\"Chromium\";v=\"130\", \"Google Chrome\";v=\"130\", \"Not?A_Brand\";v=\"99\"",
    ),
    ("sec-ch-ua-mobile", "?0"),
    ("sec-ch-ua-platform", "\"Windows\""),
    ("sec-fetch-dest", "empty"),
    ("sec-fetch-mode", "cors"),
    ("sec-fetch-site", "same-origin"),
    (
        "referer",
        "https://id.zalo.me/account?continue=https%3A%2F%2Fchat.zalo.me%2F",
    ),
    ("referrer-policy", "strict-origin-when-cross-origin"),
];

/// Runs the exact 9-step Zalo Web QR login flow (port of `zca-js/dist/apis/loginQR.js`),
/// auto-retrying on 100s QR expiry until scanned/confirmed, declined (`-13`), or aborted via `abort_rx`.
pub async fn login_qr(
    http: ZaloHttp,
    user_agent: Option<&str>,
    event_tx: mpsc::UnboundedSender<LoginQrEvent>,
    mut abort_rx: oneshot::Receiver<()>,
) -> Result<(Credentials, ZaloClient)> {
    let ua = user_agent
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_USER_AGENT)
        .to_string();
    let version_re = Regex::new(r"https://stc-zlogin\.zdn\.vn/main-([\d.]+)\.js")
        .expect("valid static regex");

    loop {
        if abort_rx.try_recv().is_ok() {
            return Err(ProtoError::QrAborted);
        }

        let jar = Arc::new(RwLock::new(ZaloCookieJar::new()));

        // Step 1: Load login page & extract stc-zlogin version
        let login_version = {
            let fut = http.request_with_cookies(
                &jar,
                &ua,
                Method::GET,
                "https://id.zalo.me/account?continue=https%3A%2F%2Fchat.zalo.me%2F",
                Some(QR_PAGE_HEADERS),
                None,
            );
            let resp = tokio::select! {
                _ = &mut abort_rx => return Err(ProtoError::QrAborted),
                r = fut => r?,
            };
            let html = resp.text().await?;
            version_re
                .captures(&html)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_string())
                .ok_or_else(|| ProtoError::Login(-1, "Cannot get API login version".into()))?
        };
        info!(version = %login_version, "Got Zalo QR login version");

        // Step 2: logininfo
        {
            let form = [("continue", "https://zalo.me/pc"), ("v", login_version.as_str())];
            let fut = http.post_form(
                &jar,
                &ua,
                "https://id.zalo.me/account/logininfo",
                Some(QR_API_HEADERS_PC),
                &form,
            );
            let _ = tokio::select! {
                _ = &mut abort_rx => return Err(ProtoError::QrAborted),
                r = fut => r,
            };
        }

        // Step 3: verify-client
        {
            let form = [
                ("type", "device"),
                ("continue", "https://zalo.me/pc"),
                ("v", login_version.as_str()),
            ];
            let fut = http.post_form(
                &jar,
                &ua,
                "https://id.zalo.me/account/verify-client",
                Some(QR_API_HEADERS_PC),
                &form,
            );
            let _ = tokio::select! {
                _ = &mut abort_rx => return Err(ProtoError::QrAborted),
                r = fut => r,
            };
        }

        // Step 4: generate QR
        let (code, image_base64) = {
            let form = [("continue", "https://zalo.me/pc"), ("v", login_version.as_str())];
            let fut = http.post_form(
                &jar,
                &ua,
                "https://id.zalo.me/account/authen/qr/generate",
                Some(QR_API_HEADERS_PC),
                &form,
            );
            let resp = tokio::select! {
                _ = &mut abort_rx => return Err(ProtoError::QrAborted),
                r = fut => r?,
            };
            let val: Value = resp.json().await?;
            let data = val.get("data").ok_or_else(|| {
                ProtoError::Login(-1, format!("Unable to generate QRCode: {val}"))
            })?;
            let code = data
                .get("code")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ProtoError::Login(-1, "QR response missing code".into()))?
                .to_string();
            let raw_img = data
                .get("image")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ProtoError::Login(-1, "QR response missing image".into()))?;
            let stripped = raw_img
                .strip_prefix("data:image/png;base64,")
                .unwrap_or(raw_img)
                .to_string();
            (code, stripped)
        };

        let _ = event_tx.send(LoginQrEvent::QrCodeGenerated {
            image_base64,
            code: code.clone(),
        });

        let deadline = Instant::now() + Duration::from_secs(QR_TIMEOUT_SECS);

        // Step 5: waiting-scan (poll while error_code == 8)
        let scan_data = match poll_waiting_scan(&http, &jar, &ua, &login_version, &code, deadline, &mut abort_rx).await {
            Ok(Some(data)) => data,
            Ok(None) => {
                // QR expired -> emit event and auto-retry with a fresh QR code
                info!("QR expired, generating fresh QR code");
                let _ = event_tx.send(LoginQrEvent::QrCodeExpired);
                continue;
            }
            Err(e) => return Err(e),
        };

        let display_name = scan_data
            .get("display_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let avatar = scan_data
            .get("avatar")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let _ = event_tx.send(LoginQrEvent::QrCodeScanned {
            display_name: display_name.clone(),
            avatar,
        });

        // Step 6: waiting-confirm (poll while error_code == 8)
        let confirm_val = match poll_waiting_confirm(&http, &jar, &ua, &login_version, &code, deadline, &mut abort_rx).await {
            Ok(Some(v)) => v,
            Ok(None) => {
                info!("QR expired while waiting for confirmation, retrying");
                let _ = event_tx.send(LoginQrEvent::QrCodeExpired);
                continue;
            }
            Err(e) => return Err(e),
        };

        let confirm_code = confirm_val
            .get("error_code")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        if confirm_code == -13 {
            let _ = event_tx.send(LoginQrEvent::QrCodeDeclined);
            return Err(ProtoError::QrDeclined);
        } else if confirm_code != 0 {
            let msg = confirm_val
                .get("error_message")
                .and_then(|v| v.as_str())
                .unwrap_or("QR confirm failed")
                .to_string();
            return Err(ProtoError::Login(confirm_code, msg));
        }

        // Step 7: checksession (follows redirects manually, capturing zpsid & zpw_sek on every hop)
        {
            const CHECK_HEADERS: &[(&str, &str)] = &[
                (
                    "accept",
                    "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7",
                ),
                (
                    "accept-language",
                    "vi-VN,vi;q=0.9,fr-FR;q=0.8,fr;q=0.7,en-US;q=0.6,en;q=0.5",
                ),
                ("priority", "u=0, i"),
                (
                    "sec-ch-ua",
                    "\"Chromium\";v=\"130\", \"Google Chrome\";v=\"130\", \"Not?A_Brand\";v=\"99\"",
                ),
                ("sec-ch-ua-mobile", "?0"),
                ("sec-ch-ua-platform", "\"Windows\""),
                ("sec-fetch-dest", "document"),
                ("sec-fetch-mode", "navigate"),
                ("sec-fetch-site", "same-origin"),
                ("upgrade-insecure-requests", "1"),
                (
                    "referer",
                    "https://id.zalo.me/account?continue=https%3A%2F%2Fchat.zalo.me%2F",
                ),
                ("referrer-policy", "strict-origin-when-cross-origin"),
            ];
            let _resp = http
                .request_with_cookies(
                    &jar,
                    &ua,
                    Method::GET,
                    "https://id.zalo.me/account/checksession?continue=https%3A%2F%2Fchat.zalo.me%2Findex.html",
                    Some(CHECK_HEADERS),
                    None,
                )
                .await?;
        }

        // Step 8: jr/userinfo verify
        {
            const USERINFO_HEADERS: &[(&str, &str)] = &[
                ("accept", "*/*"),
                (
                    "accept-language",
                    "vi-VN,vi;q=0.9,fr-FR;q=0.8,fr;q=0.7,en-US;q=0.6,en;q=0.5",
                ),
                ("priority", "u=1, i"),
                (
                    "sec-ch-ua",
                    "\"Chromium\";v=\"130\", \"Google Chrome\";v=\"130\", \"Not?A_Brand\";v=\"99\"",
                ),
                ("sec-ch-ua-mobile", "?0"),
                ("sec-ch-ua-platform", "\"Windows\""),
                ("sec-fetch-dest", "empty"),
                ("sec-fetch-mode", "cors"),
                ("sec-fetch-site", "same-site"),
                ("referer", "https://chat.zalo.me/"),
                ("referrer-policy", "strict-origin-when-cross-origin"),
            ];
            let resp = http
                .request_with_cookies(
                    &jar,
                    &ua,
                    Method::GET,
                    "https://jr.chat.zalo.me/jr/userinfo",
                    Some(USERINFO_HEADERS),
                    None,
                )
                .await?;
            let info: Value = resp.json().await?;
            let logged = info
                .get("data")
                .and_then(|d| d.get("logged"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !logged {
                return Err(ProtoError::Login(-1, "Can't login: jr/userinfo reported not logged".into()));
            }
        }

        // Step 9: Build Credentials and call login_cookie
        let cookies = jar
            .read()
            .map(|j| j.export_cookies())
            .unwrap_or_default();
        let imei = generate_zalo_uuid(&ua);
        let creds = Credentials {
            imei,
            cookie: cookies,
            user_agent: ua.clone(),
            language: Some("vi".to_string()),
        };
        let _ = event_tx.send(LoginQrEvent::GotLoginInfo(creds.clone()));

        let client = login_cookie_with_http(http, creds.clone()).await?;
        info!(account = %display_name, uid = %client.ctx().uid, "QR login succeeded");
        return Ok((creds, client));
    }
}

async fn poll_waiting_scan(
    http: &ZaloHttp,
    jar: &Arc<RwLock<ZaloCookieJar>>,
    ua: &str,
    version: &str,
    code: &str,
    deadline: Instant,
    abort_rx: &mut oneshot::Receiver<()>,
) -> Result<Option<Value>> {
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Ok(None);
        };
        let form = [
            ("code", code),
            ("continue", "https://chat.zalo.me/"),
            ("v", version),
        ];
        let req_fut = http.post_form(
            jar,
            ua,
            "https://id.zalo.me/account/authen/qr/waiting-scan",
            Some(QR_API_HEADERS_CHAT),
            &form,
        );
        let resp = tokio::select! {
            _ = &mut *abort_rx => return Err(ProtoError::QrAborted),
            _ = tokio::time::sleep(remaining) => return Ok(None),
            r = req_fut => r?,
        };
        let val: Value = resp.json().await?;
        let err_code = val.get("error_code").and_then(|v| v.as_i64()).unwrap_or(-1);
        if err_code == 8 {
            debug!("waiting-scan returned error_code=8, continuing poll");
            continue;
        }
        if let Some(data) = val.get("data") {
            if !data.is_null() {
                return Ok(Some(data.clone()));
            }
        }
        let msg = val
            .get("error_message")
            .and_then(|v| v.as_str())
            .unwrap_or("Cannot get scan result")
            .to_string();
        return Err(ProtoError::Login(err_code, msg));
    }
}

async fn poll_waiting_confirm(
    http: &ZaloHttp,
    jar: &Arc<RwLock<ZaloCookieJar>>,
    ua: &str,
    version: &str,
    code: &str,
    deadline: Instant,
    abort_rx: &mut oneshot::Receiver<()>,
) -> Result<Option<Value>> {
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Ok(None);
        };
        let form = [
            ("code", code),
            ("gToken", ""),
            ("gAction", "CONFIRM_QR"),
            ("continue", "https://chat.zalo.me/"),
            ("v", version),
        ];
        let req_fut = http.post_form(
            jar,
            ua,
            "https://id.zalo.me/account/authen/qr/waiting-confirm",
            Some(QR_API_HEADERS_CHAT),
            &form,
        );
        let resp = tokio::select! {
            _ = &mut *abort_rx => return Err(ProtoError::QrAborted),
            _ = tokio::time::sleep(remaining) => return Ok(None),
            r = req_fut => r?,
        };
        let val: Value = resp.json().await?;
        let err_code = val.get("error_code").and_then(|v| v.as_i64()).unwrap_or(-1);
        if err_code == 8 {
            debug!("waiting-confirm returned error_code=8, continuing poll");
            continue;
        }
        return Ok(Some(val));
    }
}

/// Logs in using saved cookies (`getLoginInfo` + `getServerInfo`), returning a ready [`ZaloClient`].
pub async fn login_cookie(credentials: Credentials) -> Result<ZaloClient> {
    let http = ZaloHttp::new()?;
    login_cookie_with_http(http, credentials).await
}

/// Same as [`login_cookie`] using an existing [`ZaloHttp`] instance.
pub async fn login_cookie_with_http(http: ZaloHttp, credentials: Credentials) -> Result<ZaloClient> {
    if credentials.imei.is_empty()
        || credentials.cookie.is_empty()
        || credentials.user_agent.is_empty()
    {
        return Err(ProtoError::InvalidArg(
            "Missing required credentials (imei, cookie, user_agent)".into(),
        ));
    }

    let api_type = DEFAULT_API_TYPE;
    let api_version = DEFAULT_API_VERSION;
    let language = credentials
        .language
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "vi".to_string());
    let jar = Arc::new(RwLock::new(ZaloCookieJar::from_saved(credentials.cookie)));

    // 1. getLoginInfo
    let now_ms = Utc::now().timestamp_millis();
    let encryptor = ParamsEncryptor::new(api_type, &credentials.imei, now_ms)?;
    let login_payload = json!({
        "computer_name": "Web",
        "imei": credentials.imei,
        "language": language,
        "ts": now_ms,
    });
    let encrypted_data = encode_aes_utf8_key(
        encryptor.encrypt_key(),
        &login_payload.to_string(),
        AesOutputEncoding::Base64,
        false,
    )?;

    let type_str = api_type.to_string();
    let ver_str = api_version.to_string();
    let mut sign_params = encryptor.get_params();
    sign_params.insert("params".to_string(), encrypted_data.clone());
    sign_params.insert("type".to_string(), type_str.clone());
    sign_params.insert("client_version".to_string(), ver_str.clone());
    let signkey = get_sign_key("getlogininfo", &sign_params);

    let login_query: Vec<(&str, &str)> = vec![
        ("zcid", &encryptor.zcid),
        ("zcid_ext", &encryptor.zcid_ext),
        ("enc_ver", &encryptor.enc_ver),
        ("params", &encrypted_data),
        ("type", &type_str),
        ("client_version", &ver_str),
        ("signkey", &signkey),
        ("nretry", "0"),
    ];
    let login_url = ZaloHttp::make_url(
        "https://wpa.chat.zalo.me/api/login/getLoginInfo",
        &login_query,
        true,
        api_version,
        api_type,
    )?;
    let login_resp = http
        .request_with_cookies(
            &jar,
            &credentials.user_agent,
            Method::GET,
            &login_url,
            None,
            None,
        )
        .await?;
    if !login_resp.status().is_success() {
        return Err(ProtoError::Login(
            login_resp.status().as_u16() as i64,
            format!("Failed to fetch login info: HTTP {}", login_resp.status()),
        ));
    }
    let outer_login: Value = login_resp.json().await?;
    let outer_code = outer_login
        .get("error_code")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if outer_code != 0 {
        let msg = outer_login
            .get("error_message")
            .and_then(|v| v.as_str())
            .unwrap_or("Login failed")
            .to_string();
        return Err(ProtoError::Login(outer_code, msg));
    }
    let enc_resp = outer_login
        .get("data")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ProtoError::Login(-1, "getLoginInfo missing encrypted data".into()))?;
    let decrypted_login_str = decode_aes_utf8_key(encryptor.encrypt_key(), enc_resp)?;
    let decrypted_login: Value = serde_json::from_str(&decrypted_login_str)?;
    let inner_code = decrypted_login
        .get("error_code")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if inner_code != 0 {
        let msg = decrypted_login
            .get("error_message")
            .and_then(|v| v.as_str())
            .unwrap_or("Login failed")
            .to_string();
        return Err(ProtoError::Login(inner_code, msg));
    }
    let login_info = decrypted_login
        .get("data")
        .cloned()
        .ok_or_else(|| ProtoError::Login(-1, "getLoginInfo decrypted data is empty".into()))?;

    // 2. getServerInfo
    let server_signkey = get_sign_key(
        "getserverinfo",
        [
            ("imei", credentials.imei.as_str()),
            ("type", type_str.as_str()),
            ("client_version", ver_str.as_str()),
            ("computer_name", "Web"),
        ],
    );
    let server_url = ZaloHttp::make_url(
        "https://wpa.chat.zalo.me/api/login/getServerInfo",
        &[
            ("imei", &credentials.imei),
            ("type", &type_str),
            ("client_version", &ver_str),
            ("computer_name", "Web"),
            ("signkey", &server_signkey),
        ],
        false,
        api_version,
        api_type,
    )?;
    let server_resp = http
        .request_with_cookies(
            &jar,
            &credentials.user_agent,
            Method::GET,
            &server_url,
            None,
            None,
        )
        .await?;
    if !server_resp.status().is_success() {
        return Err(ProtoError::Login(
            server_resp.status().as_u16() as i64,
            format!("Failed to fetch server info: HTTP {}", server_resp.status()),
        ));
    }
    let server_json: Value = server_resp.json().await?;
    let server_data = server_json
        .get("data")
        .filter(|v| !v.is_null())
        .cloned()
        .ok_or_else(|| {
            let code = server_json
                .get("error_code")
                .and_then(|v| v.as_i64())
                .unwrap_or(-1);
            let msg = server_json
                .get("error_message")
                .and_then(|v| v.as_str())
                .unwrap_or("Failed to fetch server info")
                .to_string();
            ProtoError::Login(code, msg)
        })?;

    // Extract session fields
    let secret_key = login_info
        .get("zpw_enk")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ProtoError::Login(-1, "Login response missing zpw_enk".into()))?
        .to_string();
    let uid = value_to_string(login_info.get("uid"))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ProtoError::Login(-1, "Login response missing uid".into()))?;

    let service_map = parse_service_map(login_info.get("zpw_service_map_v3"));
    let ws_urls = parse_string_list(login_info.get("zpw_ws"));
    let ctrs_urls = parse_string_list(login_info.get("zpw_ctrs_new"));

    // Zalo historically sends `setttings` (3 t's); fall back to `settings`.
    let settings = server_data
        .get("setttings")
        .filter(|v| !v.is_null())
        .or_else(|| server_data.get("settings").filter(|v| !v.is_null()))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let extra_ver = server_data
        .get("extra_ver")
        .cloned()
        .unwrap_or_else(|| json!({}));

    if ws_urls.is_empty() {
        warn!("Login response contained no zpw_ws WebSocket URLs");
    }

    let ctx = Arc::new(ZaloContext {
        uid,
        imei: credentials.imei,
        user_agent: credentials.user_agent,
        language,
        api_type,
        api_version,
        secret_key,
        service_map,
        ws_urls,
        ctrs_urls,
        settings,
        extra_ver,
        login_info,
        cookies: jar,
        upload_callbacks: Arc::new(Mutex::new(HashMap::new())),
    });

    Ok(ZaloClient::new(ctx, http))
}

fn value_to_string(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn parse_string_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|item| item.as_str().map(|s| s.to_string()))
            .collect(),
        Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn parse_service_map(v: Option<&Value>) -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    let Some(obj) = v.and_then(|val| val.as_object()) else {
        return map;
    };
    for (k, val) in obj {
        let list = parse_string_list(Some(val));
        if !list.is_empty() {
            map.insert(k.clone(), list);
        }
    }
    map
}

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use url::Url;

/// Serializable cookie compatible with `tough-cookie` JSON (`credentials.json` from `zca-js`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SavedCookie {
    #[serde(alias = "name")]
    pub key: String,
    #[serde(default)]
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secure: Option<bool>,
    #[serde(
        rename = "httpOnly",
        alias = "http_only",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub http_only: Option<bool>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl SavedCookie {
    pub fn new(key: impl Into<String>, value: impl Into<String>, domain: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            domain: Some(domain.into()),
            path: Some("/".to_string()),
            expires: None,
            secure: None,
            http_only: None,
            extra: HashMap::new(),
        }
    }

    /// Returns whether this cookie is expired relative to `now`.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        let Some(ref exp) = self.expires else {
            return false;
        };
        match exp {
            Value::String(s) => {
                let trimmed = s.trim();
                if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("infinity") {
                    return false;
                }
                if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
                    return dt.with_timezone(&Utc) <= now;
                }
                if let Ok(dt) = DateTime::parse_from_rfc2822(trimmed) {
                    return dt.with_timezone(&Utc) <= now;
                }
                // Try common HTTP-date format: "Wed, 18 Mar 2026 12:34:56 GMT"
                if let Ok(naive) =
                    chrono::NaiveDateTime::parse_from_str(trimmed, "%a, %d %b %Y %H:%M:%S GMT")
                {
                    return DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc) <= now;
                }
                if let Ok(naive) =
                    chrono::NaiveDateTime::parse_from_str(trimmed, "%a, %d-%b-%Y %H:%M:%S GMT")
                {
                    return DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc) <= now;
                }
                false
            }
            Value::Number(n) => {
                if let Some(ms) = n.as_i64() {
                    let secs = if ms > 10_000_000_000 { ms / 1000 } else { ms };
                    if let Some(dt) = DateTime::from_timestamp(secs, 0) {
                        return dt <= now;
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// Normalized domain without a leading dot, lowercase.
    pub fn normalized_domain(&self) -> String {
        self.domain
            .as_deref()
            .unwrap_or("chat.zalo.me")
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase()
    }

    pub fn normalized_path(&self) -> &str {
        match self.path.as_deref() {
            Some(p) if !p.is_empty() => p,
            _ => "/",
        }
    }

    fn is_host_only(&self) -> bool {
        self.extra
            .get("hostOnly")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }
}

/// Custom serde deserializer that accepts both `[SavedCookie, ...]` and `{ "cookies": [SavedCookie, ...] }`.
pub fn deserialize_saved_cookies<'de, D>(deserializer: D) -> Result<Vec<SavedCookie>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum CookieOrJar {
        List(Vec<SavedCookie>),
        Jar { cookies: Vec<SavedCookie> },
    }

    match CookieOrJar::deserialize(deserializer)? {
        CookieOrJar::List(list) => Ok(list),
        CookieOrJar::Jar { cookies } => Ok(cookies),
    }
}

/// Returns true if `host` belongs to Zalo's cookie domain family (matching `src/historysync.ts`).
pub fn is_zalo_cookie_host(host: &str) -> bool {
    let h = host.trim().to_ascii_lowercase();
    if h.is_empty() {
        return false;
    }
    const SUFFIXES: &[&str] = &[
        "zalo.me",
        "zaloapp.com",
        "zdn.vn",
        "zadn.vn",
        "zalo.vn",
        "dlmd.me",
    ];
    SUFFIXES.iter().any(|suffix| {
        h == *suffix
            || (h.len() > suffix.len()
                && h.ends_with(suffix)
                && h.as_bytes()[h.len() - suffix.len() - 1] == b'.')
    })
}

/// Pure-Rust cookie jar matching `tough-cookie` semantics for Zalo domains.
#[derive(Debug, Clone, Default)]
pub struct ZaloCookieJar {
    cookies: Vec<SavedCookie>,
}

impl ZaloCookieJar {
    pub fn new() -> Self {
        Self {
            cookies: Vec::new(),
        }
    }

    /// Imports cookies (matching `zca-js`'s `Zalo.parseCookies` normalization).
    pub fn from_saved(saved: Vec<SavedCookie>) -> Self {
        let mut jar = Self::new();
        jar.import_cookies(saved);
        jar
    }

    pub fn import_cookies(&mut self, saved: Vec<SavedCookie>) {
        for mut c in saved {
            if let Some(ref d) = c.domain {
                let trimmed = d.trim();
                if let Some(stripped) = trimmed.strip_prefix('.') {
                    c.domain = Some(stripped.to_ascii_lowercase());
                } else if !trimmed.is_empty() {
                    c.domain = Some(trimmed.to_ascii_lowercase());
                }
            }
            if c.domain.as_deref().unwrap_or("").is_empty() {
                c.domain = Some("chat.zalo.me".to_string());
            }
            if c.path.as_deref().unwrap_or("").is_empty() {
                c.path = Some("/".to_string());
            }
            self.upsert(c);
        }
    }

    pub fn export_cookies(&self) -> Vec<SavedCookie> {
        self.cookies.clone()
    }

    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    pub fn len(&self) -> usize {
        self.cookies.len()
    }

    /// Parses a single `Set-Cookie` header value received from `request_url` and inserts/updates it.
    pub fn set_cookie_header(&mut self, header_value: &str, request_url: &Url) {
        let Some(cookie) = parse_set_cookie(header_value, request_url) else {
            return;
        };
        let now = Utc::now();
        if cookie.is_expired(now) {
            self.remove(&cookie.key, &cookie.normalized_domain(), cookie.normalized_path());
        } else {
            self.upsert(cookie);
        }
    }

    /// Upserts a cookie by `(key, domain, path)`.
    pub fn upsert(&mut self, cookie: SavedCookie) {
        let dom = cookie.normalized_domain();
        let path = cookie.normalized_path().to_string();
        if let Some(existing) = self.cookies.iter_mut().find(|c| {
            c.key == cookie.key
                && c.normalized_domain() == dom
                && c.normalized_path() == path
        }) {
            *existing = cookie;
        } else {
            self.cookies.push(cookie);
        }
    }

    pub fn remove(&mut self, key: &str, domain: &str, path: &str) {
        let dom = domain.trim_start_matches('.').to_ascii_lowercase();
        self.cookies.retain(|c| {
            !(c.key == key && c.normalized_domain() == dom && c.normalized_path() == path)
        });
    }

    /// Returns all non-expired cookies matching `url`, ordered most-specific path first.
    pub fn matching_cookies(&self, url: &Url) -> Vec<&SavedCookie> {
        let Some(host) = url.host_str() else {
            return Vec::new();
        };
        let host_lower = host.to_ascii_lowercase();
        let req_path = if url.path().is_empty() { "/" } else { url.path() };
        let is_secure = url.scheme().eq_ignore_ascii_case("https")
            || url.scheme().eq_ignore_ascii_case("wss");
        let now = Utc::now();

        let mut matched: Vec<&SavedCookie> = self
            .cookies
            .iter()
            .filter(|c| {
                if c.is_expired(now) {
                    return false;
                }
                if c.secure.unwrap_or(false) && !is_secure {
                    return false;
                }
                let dom = c.normalized_domain();
                if !domain_matches(&host_lower, &dom, c.is_host_only()) {
                    return false;
                }
                path_matches(req_path, c.normalized_path())
            })
            .collect();

        // RFC 6265: longer path first; tie-break by domain specificity (longer domain first).
        matched.sort_by(|a, b| {
            b.normalized_path()
                .len()
                .cmp(&a.normalized_path().len())
                .then_with(|| b.normalized_domain().len().cmp(&a.normalized_domain().len()))
        });
        matched
    }

    /// Formats the `Cookie:` request header value for `url`.
    pub fn cookie_header_for(&self, url: &Url) -> String {
        let matched = self.matching_cookies(url);
        let mut seen = HashSet::new();
        let mut parts = Vec::with_capacity(matched.len());
        for c in matched {
            if seen.insert(c.key.as_str()) {
                parts.push(format!("{}={}", c.key, c.value));
            }
        }
        parts.join("; ")
    }

    /// Formats the `Cookie:` header for a string URL.
    pub fn cookie_header_for_str(&self, url_str: &str) -> String {
        match Url::parse(url_str) {
            Ok(u) => self.cookie_header_for(&u),
            Err(_) => String::new(),
        }
    }

    /// Builds the cookie header for backup downloads (merging `chat.zalo.me` + target host cookies,
    /// plus `zpw_uin` fallback, matching `src/historysync.ts:backupCookieHeader`).
    pub fn backup_cookie_header(&self, target: &Url, uid: &str) -> String {
        let chat_url = Url::parse("https://chat.zalo.me/").expect("valid static url");
        let mut order: Vec<(String, String)> = Vec::new();
        let mut seen = HashSet::new();

        for c in self
            .matching_cookies(&chat_url)
            .into_iter()
            .chain(self.matching_cookies(target))
        {
            if c.value.is_empty() && (c.key == "zpw_uin" || c.key == "zpw_sek") {
                continue;
            }
            if seen.insert(c.key.clone()) {
                order.push((c.key.clone(), c.value.clone()));
            }
        }
        if !seen.contains("zpw_uin") && !uid.is_empty() {
            order.push(("zpw_uin".to_string(), uid.to_string()));
        }
        order
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

fn domain_matches(host: &str, cookie_domain: &str, host_only: bool) -> bool {
    if cookie_domain.is_empty() {
        return false;
    }
    if host == cookie_domain {
        return true;
    }
    if host_only {
        return false;
    }
    host.len() > cookie_domain.len()
        && host.ends_with(cookie_domain)
        && host.as_bytes()[host.len() - cookie_domain.len() - 1] == b'.'
}

fn path_matches(req_path: &str, cookie_path: &str) -> bool {
    if cookie_path == "/" {
        return true;
    }
    if req_path == cookie_path {
        return true;
    }
    if req_path.starts_with(cookie_path) {
        if cookie_path.ends_with('/') {
            return true;
        }
        return req_path.as_bytes().get(cookie_path.len()) == Some(&b'/');
    }
    false
}

/// Parses a single `Set-Cookie` header string into a `SavedCookie`.
pub fn parse_set_cookie(header: &str, request_url: &Url) -> Option<SavedCookie> {
    let mut parts = header.split(';');
    let name_val = parts.next()?.trim();
    let eq_idx = name_val.find('=')?;
    let key = name_val[..eq_idx].trim().to_string();
    if key.is_empty() {
        return None;
    }
    let value = name_val[eq_idx + 1..].trim().to_string();

    let default_host = request_url
        .host_str()
        .unwrap_or("chat.zalo.me")
        .to_ascii_lowercase();
    let mut domain: Option<String> = None;
    let mut path: Option<String> = None;
    let mut expires: Option<Value> = None;
    let mut secure: Option<bool> = None;
    let mut http_only: Option<bool> = None;
    let mut host_only = true;
    let mut extra = HashMap::new();

    for attr in parts {
        let attr = attr.trim();
        if attr.is_empty() {
            continue;
        }
        if let Some((k, v)) = attr.split_once('=') {
            let k_lower = k.trim().to_ascii_lowercase();
            let v_trim = v.trim();
            match k_lower.as_str() {
                "domain" => {
                    let dom = v_trim.trim_start_matches('.').to_ascii_lowercase();
                    if !dom.is_empty() {
                        domain = Some(dom);
                        host_only = false;
                    }
                }
                "path" => {
                    if !v_trim.is_empty() {
                        path = Some(v_trim.to_string());
                    }
                }
                "expires" => {
                    if let Ok(dt) = DateTime::parse_from_rfc2822(v_trim) {
                        expires = Some(Value::String(dt.with_timezone(&Utc).to_rfc3339()));
                    } else if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(
                        v_trim,
                        "%a, %d %b %Y %H:%M:%S GMT",
                    ) {
                        expires = Some(Value::String(
                            DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc).to_rfc3339(),
                        ));
                    } else if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(
                        v_trim,
                        "%a, %d-%b-%Y %H:%M:%S GMT",
                    ) {
                        expires = Some(Value::String(
                            DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc).to_rfc3339(),
                        ));
                    } else {
                        expires = Some(Value::String(v_trim.to_string()));
                    }
                }
                "max-age" => {
                    if let Ok(secs) = v_trim.parse::<i64>() {
                        let exp_dt = Utc::now() + chrono::Duration::seconds(secs);
                        expires = Some(Value::String(exp_dt.to_rfc3339()));
                    }
                }
                "samesite" => {
                    extra.insert(
                        "sameSite".to_string(),
                        Value::String(v_trim.to_ascii_lowercase()),
                    );
                }
                _ => {}
            }
        } else {
            let k_lower = attr.to_ascii_lowercase();
            match k_lower.as_str() {
                "secure" => secure = Some(true),
                "httponly" => http_only = Some(true),
                _ => {}
            }
        }
    }

    // Match zca-js utils.js:283: cookies with Domain=zalo.me or subdomains are stored under that domain
    let final_domain = domain.unwrap_or(default_host);
    if host_only {
        extra.insert("hostOnly".to_string(), Value::Bool(true));
    } else {
        extra.insert("hostOnly".to_string(), Value::Bool(false));
    }

    Some(SavedCookie {
        key,
        value,
        domain: Some(final_domain),
        path: Some(path.unwrap_or_else(|| "/".to_string())),
        expires,
        secure,
        http_only,
        extra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Credentials;

    #[test]
    fn tough_cookie_json_roundtrip_array_and_object() {
        let json_array = r#"{
            "imei": "abc-123",
            "userAgent": "Mozilla/5.0",
            "cookie": [
                {
                    "key": "zpw_sek",
                    "value": "secret123",
                    "domain": "zalo.me",
                    "path": "/",
                    "expires": "2030-01-01T00:00:00.000Z",
                    "secure": true,
                    "httpOnly": true,
                    "hostOnly": false,
                    "creation": "2025-01-01T00:00:00.000Z"
                },
                {
                    "name": "zpsid",
                    "value": "sid456",
                    "domain": ".chat.zalo.me",
                    "path": "/"
                }
            ]
        }"#;
        let creds: Credentials = serde_json::from_str(json_array).unwrap();
        assert_eq!(creds.imei, "abc-123");
        assert_eq!(creds.user_agent, "Mozilla/5.0");
        assert_eq!(creds.cookie.len(), 2);
        assert_eq!(creds.cookie[0].key, "zpw_sek");
        assert_eq!(creds.cookie[0].http_only, Some(true));
        assert_eq!(
            creds.cookie[0].extra.get("creation").and_then(|v| v.as_str()),
            Some("2025-01-01T00:00:00.000Z")
        );
        assert_eq!(creds.cookie[1].key, "zpsid");

        // Round-trip lossless
        let serialized = serde_json::to_string(&creds).unwrap();
        let creds2: Credentials = serde_json::from_str(&serialized).unwrap();
        assert_eq!(creds.cookie, creds2.cookie);

        // Also accept { "cookies": [...] }
        let json_obj = r#"{
            "imei": "abc-123",
            "user_agent": "Mozilla/5.0",
            "cookie": {
                "version": "tough-cookie@5",
                "cookies": [
                    { "key": "zpw_sek", "value": "v1", "domain": "zalo.me", "path": "/" }
                ]
            }
        }"#;
        let creds_obj: Credentials = serde_json::from_str(json_obj).unwrap();
        assert_eq!(creds_obj.cookie.len(), 1);
        assert_eq!(creds_obj.cookie[0].key, "zpw_sek");
    }

    #[test]
    fn set_cookie_parsing_and_domain_matching() {
        let mut jar = ZaloCookieJar::new();
        let id_url = Url::parse("https://id.zalo.me/account/checksession").unwrap();
        jar.set_cookie_header(
            "zpsid=ABC.123; Path=/; Domain=.zalo.me; Expires=Wed, 18 Mar 2035 12:00:00 GMT; Secure; HttpOnly",
            &id_url,
        );
        jar.set_cookie_header("zpw_sek=XYZ; Path=/; Domain=chat.zalo.me; Secure", &id_url);
        jar.set_cookie_header("host_only_c=HO; Path=/", &id_url);

        let chat_hdr = jar.cookie_header_for_str("https://wpa.chat.zalo.me/api/login/getLoginInfo");
        assert!(chat_hdr.contains("zpsid=ABC.123"));
        assert!(chat_hdr.contains("zpw_sek=XYZ"));
        assert!(!chat_hdr.contains("host_only_c"));

        let id_hdr = jar.cookie_header_for_str("https://id.zalo.me/account");
        assert!(id_hdr.contains("zpsid=ABC.123"));
        assert!(id_hdr.contains("host_only_c=HO"));
        assert!(!id_hdr.contains("zpw_sek=XYZ"));

        let other_hdr = jar.cookie_header_for_str("https://evilzalo.me/");
        assert!(other_hdr.is_empty());

        // Expired cookie removes existing entry
        jar.set_cookie_header(
            "zpsid=deleted; Path=/; Domain=.zalo.me; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
            &id_url,
        );
        let after = jar.cookie_header_for_str("https://chat.zalo.me/");
        assert!(!after.contains("zpsid="));
    }

    #[test]
    fn zalo_cookie_host_matching() {
        assert!(is_zalo_cookie_host("chat.zalo.me"));
        assert!(is_zalo_cookie_host("zalo.me"));
        assert!(is_zalo_cookie_host("files.zdn.vn"));
        assert!(is_zalo_cookie_host("a.b.zadn.vn"));
        assert!(is_zalo_cookie_host("dlmd.me"));
        assert!(!is_zalo_cookie_host("notzalo.me"));
        assert!(!is_zalo_cookie_host("example.com"));
    }
}

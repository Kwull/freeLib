//! OAuth Client ID Metadata Documents (draft-ietf-oauth-client-id-metadata-document): a client
//! whose `client_id` is an `https://` URL is described by the JSON document at that URL.
//!
//! Fetching a URL a request names is a server-side request forgery risk, so:
//! * only hosts in `FREELIB_OAUTH_CLIENT_HOSTS` are fetched (default `claude.ai`, `claude.com`);
//! * the host must resolve to public addresses only (no loopback, private, link-local, CGNAT,
//!   multicast or unique-local ranges), and the connection is pinned to the checked address;
//! * `https` only, no redirects, 5 s timeout, 64 KiB at most;
//! * documents are cached (their `Cache-Control: max-age`, 1 min … 1 day, default 5 min),
//!   failures for 30 s.
//!
//! The documents of Claude (web, Desktop, mobile) and Claude Code are built in as a fallback,
//! for servers that cannot reach claude.ai (the published documents are identical).

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;

/// Claude (claude.ai, Claude Desktop, mobile) and Claude Code, as published.
const BUILT_IN: &[(&str, &str)] = &[
    (
        "https://claude.ai/oauth/mcp-oauth-client-metadata",
        r#"{"client_id":"https://claude.ai/oauth/mcp-oauth-client-metadata","client_name":"Claude","client_uri":"https://claude.ai","redirect_uris":["https://claude.ai/api/mcp/auth_callback"],"grant_types":["authorization_code","refresh_token","urn:ietf:params:oauth:grant-type:jwt-bearer"],"response_types":["code"],"token_endpoint_auth_method":"none"}"#,
    ),
    (
        "https://claude.ai/oauth/claude-code-client-metadata",
        r#"{"client_id":"https://claude.ai/oauth/claude-code-client-metadata","client_name":"Claude Code","client_uri":"https://claude.ai","redirect_uris":["http://localhost/callback","http://127.0.0.1/callback"],"grant_types":["authorization_code","refresh_token"],"response_types":["code"],"token_endpoint_auth_method":"none"}"#,
    ),
];

const MAX_BYTES: usize = 64 * 1024;
const FAIL_TTL: Duration = Duration::from_secs(30);
const MAX_CACHED: usize = 200;

/// The fields of a metadata document that matter here.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ClientDoc {
    pub client_id: String,
    pub client_name: Option<String>,
    pub client_uri: Option<String>,
    pub redirect_uris: Vec<String>,
    pub token_endpoint_auth_method: Option<String>,
    pub grant_types: Option<Vec<String>>,
    pub response_types: Option<Vec<String>>,
}

/// Parses and checks a document fetched from `url`.
pub fn parse_doc(url: &str, body: &str) -> Result<ClientDoc, String> {
    let d: ClientDoc =
        serde_json::from_str(body).map_err(|e| format!("not a client metadata document: {e}"))?;
    if d.client_id != url {
        return Err("the document's client_id is not its URL".into());
    }
    if d.redirect_uris.is_empty() || d.redirect_uris.len() > 20 {
        return Err("redirect_uris must list 1 to 20 URIs".into());
    }
    if let Some(m) = &d.token_endpoint_auth_method
        && m != "none"
    {
        return Err(format!(
            "token_endpoint_auth_method {m} is not supported (public clients only)"
        ));
    }
    if d.grant_types
        .as_ref()
        .is_some_and(|g| !g.iter().any(|x| x == "authorization_code"))
    {
        return Err("grant_types must include authorization_code".into());
    }
    if d.response_types
        .as_ref()
        .is_some_and(|g| !g.iter().any(|x| x == "code"))
    {
        return Err("response_types must include code".into());
    }
    Ok(d)
}

/// Whether an address is on the public internet.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_broadcast()
                || v.is_documentation()
                || v.is_unspecified()
                || v.is_multicast()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1])) // CGNAT
                || (o[0] == 192 && o[1] == 0 && o[2] == 0) // IETF protocol assignments
                || (o[0] == 198 && (18..20).contains(&o[1])) // benchmarking
                || o[0] >= 240)
        }
        IpAddr::V6(v) => {
            if let Some(m) = v.to_ipv4_mapped() {
                return is_public(IpAddr::V4(m));
            }
            let s = v.segments();
            !(v.is_loopback()
                || v.is_unspecified()
                || v.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80 // link local
                || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
                || (s[0] == 0x0064 && s[1] == 0xff9b)) // NAT64
        }
    }
}

enum Entry {
    Ok(ClientDoc, Instant, Duration),
    Failed(String, Instant),
}

/// Fetched documents.
pub struct Cache {
    entries: Mutex<HashMap<String, Entry>>,
    /// Documents known without fetching (tests), checked first.
    fixed: HashMap<String, String>,
}

impl Cache {
    pub fn new(fixed: &[(String, String)]) -> Cache {
        Cache {
            entries: Mutex::new(HashMap::new()),
            fixed: fixed.iter().cloned().collect(),
        }
    }

    /// The document of `url` (already checked to be an allowed `https` URL).
    pub async fn get(&self, url: &str) -> Result<ClientDoc, String> {
        if let Some(body) = self.fixed.get(url) {
            return parse_doc(url, body);
        }
        {
            let g = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            match g.get(url) {
                Some(Entry::Ok(d, at, ttl)) if at.elapsed() < *ttl => return Ok(d.clone()),
                Some(Entry::Failed(e, at)) if at.elapsed() < FAIL_TTL => return Err(e.clone()),
                _ => {}
            }
        }
        let fetched = fetch(url).await.and_then(|(body, ttl)| {
            let d = parse_doc(url, &body)?;
            Ok((d, ttl))
        });
        let result = match fetched {
            Ok((d, ttl)) => Ok((d, ttl)),
            Err(e) => match BUILT_IN.iter().find(|(u, _)| *u == url) {
                Some((_, body)) => {
                    tracing::warn!(
                        "client metadata {url} could not be fetched ({e}); using the built-in copy"
                    );
                    parse_doc(url, body).map(|d| (d, Duration::from_secs(300)))
                }
                None => Err(e),
            },
        };
        let mut g = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if g.len() >= MAX_CACHED {
            g.clear();
        }
        match result {
            Ok((d, ttl)) => {
                g.insert(url.to_string(), Entry::Ok(d.clone(), Instant::now(), ttl));
                Ok(d)
            }
            Err(e) => {
                tracing::info!("client metadata {url} refused: {e}");
                g.insert(url.to_string(), Entry::Failed(e.clone(), Instant::now()));
                Err(e)
            }
        }
    }
}

fn max_age(h: &reqwest::header::HeaderMap) -> Duration {
    let secs = h
        .get(reqwest::header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.split(',')
                .filter_map(|d| d.trim().strip_prefix("max-age="))
                .find_map(|n| n.trim().parse::<u64>().ok())
        })
        .unwrap_or(300);
    Duration::from_secs(secs.clamp(60, 86_400))
}

/// GET `url` with the SSRF precautions above; (body, cache lifetime).
async fn fetch(url: &str) -> Result<(String, Duration), String> {
    let u = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
    if u.scheme() != "https" {
        return Err("client_id must be an https URL".into());
    }
    let host = u.host_str().ok_or("client_id has no host")?.to_string();
    let port = u.port_or_known_default().unwrap_or(443);
    let addrs: Vec<SocketAddr> = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host.as_str(), port)),
    )
    .await
    .map_err(|_| "DNS lookup timed out".to_string())?
    .map_err(|e| format!("DNS lookup failed: {e}"))?
    .collect();
    if addrs.is_empty() {
        return Err("host has no address".into());
    }
    if let Some(a) = addrs.iter().find(|a| !is_public(a.ip())) {
        return Err(format!("host resolves to a non-public address {}", a.ip()));
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(5))
        .no_proxy()
        .resolve_to_addrs(&host, &addrs)
        .user_agent(concat!("freelib-server/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let mut resp = client
        .get(u)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let ttl = max_age(resp.headers());
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_BYTES {
            return Err("document too large".into());
        }
    }
    let text = String::from_utf8(body).map_err(|_| "document is not UTF-8".to_string())?;
    Ok((text, ttl))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_documents_parse() {
        for (u, b) in BUILT_IN {
            let d = parse_doc(u, b).unwrap();
            assert_eq!(d.client_id, *u);
        }
    }

    #[test]
    fn doc_checks() {
        let u = "https://app.example/client.json";
        let ok = r#"{"client_id":"https://app.example/client.json","client_name":"X","redirect_uris":["https://app.example/cb"]}"#;
        assert!(parse_doc(u, ok).is_ok());
        assert!(parse_doc("https://other.example/client.json", ok).is_err());
        assert!(
            parse_doc(
                u,
                &ok.replace(
                    r#""redirect_uris":["https://app.example/cb"]"#,
                    r#""redirect_uris":[]"#
                )
            )
            .is_err()
        );
        assert!(
            parse_doc(
                u,
                &ok.replace(
                    r#""client_name""#,
                    r#""token_endpoint_auth_method":"client_secret_basic","client_name""#
                )
            )
            .is_err()
        );
        assert!(parse_doc(u, "not json").is_err());
    }

    #[test]
    fn public_addresses() {
        for bad in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
        ] {
            assert!(!is_public(bad.parse().unwrap()), "{bad}");
        }
        for good in ["160.79.104.10", "8.8.8.8", "2606:4700::1111"] {
            assert!(is_public(good.parse().unwrap()), "{good}");
        }
    }
}

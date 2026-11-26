//! Sign-in through plex.tv for setup: a code for plex.tv/link, then the account's servers.

use super::client::{read_capped, MAX_XML_BYTES, PRODUCT};
use super::PlexClient;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::time::Duration;

const PLEX_TV: &str = "https://plex.tv";

pub const POLL_EVERY: Duration = Duration::from_secs(2);

pub const ANSWER_WITHIN: Duration = Duration::from_secs(3);

pub struct PlexTv {
    http: reqwest::Client,
    base: String,
    client_id: String,
}

pub struct Pin {
    pub id: u64,
    pub code: String,
    pub expires_in: Duration,
}

pub struct Server {
    pub name: String,
    pub owned: bool,
    /// For this server only; a shared server's differs from the account's.
    pub token: String,
    https_required: bool,
    on_its_network: bool,
    connections: Vec<Connection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    pub uri: String,
    pub local: bool,
    pub https: bool,
    pub relay: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PinReply {
    id: u64,
    code: String,
    expires_in: u64,
    auth_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Resource {
    #[serde(default)]
    name: String,
    #[serde(default)]
    provides: String,
    #[serde(default)]
    owned: bool,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    https_required: bool,
    #[serde(default)]
    public_address_matches: bool,
    #[serde(default)]
    connections: Vec<Connection>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Connection {
    uri: String,
    #[serde(default)]
    address: String,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    local: bool,
    #[serde(default)]
    relay: bool,
}

impl PlexTv {
    pub fn new(client_id: &str) -> Self {
        Self::with_base(PLEX_TV, client_id)
    }

    fn with_base(base: &str, client_id: &str) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("reqwest client"),
            base: base.to_string(),
            client_id: client_id.to_string(),
        }
    }

    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        token: Option<&str>,
    ) -> Result<T> {
        let url = format!("{}{path}", self.base);
        let mut req = self
            .http
            .request(method, &url)
            .header("Accept", "application/json")
            .header("X-Plex-Product", PRODUCT)
            .header("X-Plex-Version", env!("CARGO_PKG_VERSION"))
            .header("X-Plex-Client-Identifier", &self.client_id)
            .header("X-Plex-Device-Name", PRODUCT);
        if let Some(token) = token {
            req = req.header("X-Plex-Token", token);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("cannot reach plex.tv: {}", e.without_url()))?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            bail!("plex.tv refused the sign-in (HTTP {status})");
        }
        if !status.is_success() {
            bail!("plex.tv returned HTTP {status}");
        }
        let body = read_capped(resp, MAX_XML_BYTES, &url).await?;
        serde_json::from_slice(&body).context("plex.tv sent a reply shmc cannot read")
    }

    pub async fn create_pin(&self) -> Result<Pin> {
        let pin: PinReply =
            self.call(reqwest::Method::POST, "/api/v2/pins?strong=false", None).await?;
        Ok(Pin { id: pin.id, code: pin.code, expires_in: Duration::from_secs(pin.expires_in) })
    }

    pub async fn wait_for_token(&self, pin: &Pin, every: Duration) -> Result<String> {
        let deadline = tokio::time::Instant::now() + pin.expires_in;
        let mut failures = 0;
        loop {
            tokio::time::sleep(every).await;
            if tokio::time::Instant::now() >= deadline {
                bail!("the code expired; run 'shmc setup' to get a new one");
            }
            let path = format!("/api/v2/pins/{}", pin.id);
            match self.call::<PinReply>(reqwest::Method::GET, &path, None).await {
                Ok(PinReply { auth_token: Some(token), .. }) if !token.is_empty() => {
                    return Ok(token)
                }
                Ok(_) => failures = 0,
                Err(e) if failures >= 4 => return Err(e),
                Err(_) => failures += 1,
            }
        }
    }

    pub async fn servers(&self, account_token: &str) -> Result<Vec<Server>> {
        let path = "/api/v2/resources?includeHttps=1&includeRelay=1";
        let resources: Vec<Resource> =
            self.call(reqwest::Method::GET, path, Some(account_token)).await?;
        Ok(resources
            .into_iter()
            .filter(|r| r.provides.split(',').any(|p| p == "server"))
            .filter_map(|r| {
                Some(Server {
                    name: r.name,
                    owned: r.owned,
                    token: r.access_token.filter(|t| !t.is_empty())?,
                    https_required: r.https_required,
                    on_its_network: r.public_address_matches,
                    connections: r.connections,
                })
            })
            .collect())
    }
}

impl Server {
    /// Best first. Never remote plain http: it would send the token in the clear.
    pub fn addresses(&self) -> Vec<Address> {
        let https =
            |c: &Connection| reqwest::Url::parse(&c.uri).is_ok_and(|u| u.scheme() == "https");
        let listed = |local: bool, relay: bool| {
            self.connections
                .iter()
                .filter(move |c| c.local == local && c.relay == relay && https(c))
                .map(move |c| Address { uri: c.uri.clone(), local, https: true, relay })
        };
        let local_http = self
            .connections
            .iter()
            .filter(|c| c.local && !c.relay && self.on_its_network && !self.https_required)
            .filter_map(|c| {
                let ip: std::net::IpAddr = c.address.parse().ok()?;
                let at = std::net::SocketAddr::new(ip, c.port);
                (c.port != 0).then(|| Address {
                    uri: format!("http://{at}"),
                    local: true,
                    https: false,
                    relay: false,
                })
            });
        let mut out: Vec<Address> = Vec::new();
        for a in listed(true, false).chain(local_http).chain(listed(false, false)) {
            if !out.contains(&a) {
                out.push(a);
            }
        }
        out.extend(listed(false, true).chain(listed(true, true)));
        out
    }
}

/// The best-ranked address that answers, with the server's machine id and version.
pub async fn first_answering(
    addresses: &[Address],
    token: &str,
    within: Duration,
) -> Option<(usize, String, String)> {
    let tries: Vec<_> = addresses
        .iter()
        .map(|a| {
            let client = PlexClient::new(&a.uri, token);
            tokio::spawn(async move { tokio::time::timeout(within, client.identity()).await })
        })
        .collect();
    for (i, attempt) in tries.into_iter().enumerate() {
        if let Ok(Ok(Ok((machine, version)))) = attempt.await {
            return Some((i, machine, version));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::client::testing::serve_full;

    const PIN: &str = r#"{"id":77,"code":"K7QZ","expiresIn":900,"authToken":null}"#;

    #[tokio::test]
    async fn a_code_is_shown_then_polled_until_approved() {
        let polls = std::sync::Mutex::new(0);
        let (base, seen, full) = serve_full(move |line| {
            if line.starts_with("POST /api/v2/pins?strong=false ") {
                return (201, PIN.into());
            }
            assert!(line.starts_with("GET /api/v2/pins/77 "), "{line}");
            let mut n = polls.lock().unwrap();
            *n += 1;
            let token = if *n < 3 { "null" } else { "\"fake-account-token\"" };
            (200, format!(r#"{{"id":77,"code":"K7QZ","expiresIn":890,"authToken":{token}}}"#))
        })
        .await;
        let tv = PlexTv::with_base(&base, "cid-test");
        let pin = tv.create_pin().await.unwrap();
        assert_eq!((pin.id, pin.code.as_str(), pin.expires_in.as_secs()), (77, "K7QZ", 900));
        let token = tv.wait_for_token(&pin, Duration::from_millis(5)).await.unwrap();
        assert_eq!(token, "fake-account-token");
        assert_eq!(seen.lock().unwrap().len(), 4);
        assert!(full.lock().unwrap()[0]
            .to_lowercase()
            .contains("x-plex-client-identifier: cid-test"));
    }

    #[tokio::test]
    async fn an_expired_code_stops_the_wait() {
        let (base, _, _) = serve_full(|_| (200, PIN.into())).await;
        let tv = PlexTv::with_base(&base, "cid-test");
        let pin = Pin { id: 77, code: "K7QZ".into(), expires_in: Duration::from_millis(30) };
        let err = tv.wait_for_token(&pin, Duration::from_millis(10)).await.unwrap_err();
        assert!(err.to_string().contains("expired"), "{err}");
    }

    #[tokio::test]
    async fn each_server_comes_with_its_own_token_and_players_are_left_out() {
        let resources = r#"[
            {"name":"Friend","provides":"server","owned":false,"accessToken":"fake-shared-token",
             "connections":[{"uri":"https://a.plex.direct:32400","protocol":"https","address":"10.0.0.2","port":32400,"local":false,"relay":false}]},
            {"name":"Phone","provides":"player,controller","owned":true,"accessToken":"fake-device-token"}
        ]"#;
        let (base, seen, full) = serve_full(move |_| (200, resources.into())).await;
        let tv = PlexTv::with_base(&base, "cid-test");
        let servers = tv.servers("fake-account-token").await.unwrap();
        let [server] = servers.as_slice() else { panic!("one server") };
        assert_eq!((server.name.as_str(), server.owned), ("Friend", false));
        assert_eq!(server.token, "fake-shared-token");
        assert!(!seen.lock().unwrap()[0].contains("fake-account-token"), "never in the URL");
        assert!(full.lock().unwrap()[0].contains("fake-account-token"), "sent as a header");
    }

    #[tokio::test]
    async fn a_redirect_is_not_followed_and_errors_name_no_token() {
        let (elsewhere, elsewhere_seen, _) = serve_full(|_| (200, "[]".into())).await;
        let base = redirect_server(format!("{elsewhere}/api/v2/resources")).await;
        let tv = PlexTv::with_base(&base, "cid-test");
        let Err(err) = tv.servers("fake-account-token").await else { panic!("followed") };
        let err = format!("{err:#}");
        assert!(err.contains("HTTP 302"), "{err}");
        assert!(!err.contains("fake-account-token"), "{err}");
        assert!(elsewhere_seen.lock().unwrap().is_empty(), "the token went to another host");
    }

    async fn redirect_server(location: String) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 8192];
                let _ = sock.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        base
    }

    fn connection(uri: &str, address: &str, local: bool, relay: bool) -> Connection {
        Connection { uri: uri.into(), address: address.into(), port: 32400, local, relay }
    }

    fn server(connections: Vec<Connection>, https_required: bool) -> Server {
        Server {
            name: "Home".into(),
            owned: true,
            token: "fake".into(),
            https_required,
            on_its_network: true,
            connections,
        }
    }

    #[test]
    fn local_https_comes_first_and_remote_http_never() {
        let connections = vec![
            connection("https://relay.plex.direct:8443", "203.0.113.9", false, true),
            connection("https://remote.plex.direct:32400", "203.0.113.5", false, false),
            connection("http://203.0.113.5:32400", "203.0.113.5", false, false),
            connection("https://local.plex.direct:32400", "10.0.0.3", true, false),
        ];
        let uris = |s: &Server| s.addresses().into_iter().map(|a| a.uri).collect::<Vec<_>>();
        assert_eq!(
            uris(&server(connections.clone(), false)),
            [
                "https://local.plex.direct:32400",
                "http://10.0.0.3:32400",
                "https://remote.plex.direct:32400",
                "https://relay.plex.direct:8443",
            ]
        );
        assert_eq!(
            uris(&server(connections, true)),
            [
                "https://local.plex.direct:32400",
                "https://remote.plex.direct:32400",
                "https://relay.plex.direct:8443",
            ],
            "no plain http when the server requires https"
        );
    }

    #[test]
    fn plain_http_is_left_out_away_from_the_servers_network() {
        let local = connection("https://local.plex.direct:32400", "10.0.0.3", true, false);
        let mut away = server(vec![local], false);
        away.on_its_network = false;
        let uris: Vec<_> = away.addresses().into_iter().map(|a| a.uri).collect();
        assert_eq!(uris, ["https://local.plex.direct:32400"]);
    }

    #[test]
    fn a_local_ipv6_address_is_bracketed() {
        let local = connection("https://v6.plex.direct:32400", "fd00::3", true, false);
        let http: Vec<_> =
            server(vec![local], false).addresses().into_iter().filter(|a| !a.https).collect();
        assert_eq!(http[0].uri, "http://[fd00::3]:32400");
    }

    #[test]
    fn an_http_uri_listed_as_https_is_left_out() {
        let listed = serde_json::from_str(
            r#"{"uri":"http://203.0.113.5:32400","protocol":"https","address":"203.0.113.5","port":32400,"local":false,"relay":false}"#,
        )
        .unwrap();
        assert_eq!(server(vec![listed], false).addresses(), []);
    }

    #[tokio::test]
    async fn the_best_address_that_answers_wins() {
        let identity = r#"<MediaContainer machineIdentifier="m1" version="1.41.0"/>"#;
        let (answers, _, _) = serve_full(move |_| (200, identity.into())).await;
        let silent = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let silent_uri = format!("http://{}", silent.local_addr().unwrap());
        tokio::spawn(async move {
            let mut held = Vec::new();
            loop {
                held.push(silent.accept().await.unwrap());
            }
        });
        let address =
            |uri: &str| Address { uri: uri.into(), local: true, https: false, relay: false };
        let within = Duration::from_millis(300);

        let picked =
            first_answering(&[address(&silent_uri), address(&answers)], "fake", within).await;
        assert_eq!(picked, Some((1, "m1".into(), "1.41.0".into())));
        let picked =
            first_answering(&[address(&answers), address(&silent_uri)], "fake", within).await;
        assert_eq!(picked.map(|p| p.0), Some(0));
        assert_eq!(first_answering(&[address(&silent_uri)], "fake", within).await, None);
    }
}

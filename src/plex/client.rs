use super::models::*;
use super::report::Report;
use anyhow::{bail, Context, Result};
use url::Url;

pub const PRODUCT: &str = "shmc";

#[derive(Debug)]
pub struct PlaylistPage {
    pub tracks: Vec<Track>,
    pub total: Option<usize>,
}

const LIST_PAGE_SIZE: usize = 500;

/// Tracks per create/append request; the `uri` has to stay a sane length.
const SAVE_CHUNK: usize = 100;

/// Keys per `/library/metadata/<k1>,<k2>,…` request, for the same reason.
const MULTI_KEY_BATCH: usize = 300;

const PING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved {
        name: String,
        count: usize,
        repeats_skipped: usize,
    },
    /// The playlist exists but holds only `saved` of `total` tracks.
    Partial {
        name: String,
        saved: usize,
        total: usize,
        reason: String,
    },
    NameTaken {
        name: String,
    },
    /// Nothing was created.
    Failed {
        reason: String,
    },
}

/// Servers differ, so a listing ends at a short page, an overlong one, `totalSize` or a repeat.
#[derive(Debug, Default)]
pub(crate) struct PageWalk {
    /// Items accepted so far, i.e. the next `Container-Start`.
    pub offset: usize,
    pub total: Option<usize>,
    pub done: bool,
    pub whole: bool,
    last_ends: Option<(String, String)>,
}

impl PageWalk {
    /// The items to keep from a page fetched when `size` were asked for.
    pub fn accept<T>(
        &mut self,
        page: Vec<T>,
        total: Option<usize>,
        size: usize,
        key: impl Fn(&T) -> String,
    ) -> Vec<T> {
        self.total = total.or(self.total);
        let n = page.len();
        if n == 0 {
            self.done = true;
            return page;
        }
        if n > size {
            self.done = true;
            self.whole = true;
            return page;
        }
        let ends = (key(&page[0]), key(&page[n - 1]));
        if n == size && self.last_ends.as_ref() == Some(&ends) {
            tracing::warn!(offset = self.offset, "plex ignores X-Plex-Container-Start; stopping");
            self.done = true;
            return Vec::new();
        }
        self.offset += n;
        if n < size || self.total.is_some_and(|t| self.offset >= t) {
            self.done = true;
        }
        self.last_ends = Some(ends);
        page
    }
}

/// A playlist entry's own id: the same track can be in a playlist twice.
pub(crate) fn entry_key(t: &Track) -> String {
    t.playlist_item_id.clone().unwrap_or_else(|| t.rating_key.clone())
}

/// A playlist load starts playing after this many tracks.
pub const PLAYLIST_FIRST_PAGE: usize = 200;

/// Later playlist pages, ~4 MB of XML each.
pub const PLAYLIST_PAGE_SIZE: usize = 2500;

/// Backstop for `get_all_paged` against a server that never runs out of items.
pub(crate) const MAX_PAGES: usize = 2000;

/// reqwest's default limit lives in `Policy::limited`; a custom policy keeps it.
const MAX_REDIRECTS: usize = 10;

/// reqwest keeps `X-Plex-Token` on a redirect to another host, so only same-origin ones follow.
fn may_follow(previous: &Url, next: &Url) -> bool {
    if previous.host_str() != next.host_str() {
        return false;
    }
    let previous_port = previous.port_or_known_default();
    let next_port = next.port_or_known_default();
    if previous.scheme() == next.scheme() {
        return previous_port == next_port;
    }
    previous.scheme() == "http"
        && next.scheme() == "https"
        && (next_port == previous_port || next_port == Some(443))
}

/// Names where a stopped redirect pointed. Origin only: a proxy can echo the
/// query string, token included, in `Location`.
pub(super) fn redirect_error(resp: &reqwest::Response) -> Option<String> {
    if !resp.status().is_redirection() {
        return None;
    }
    let location = resp.headers().get(reqwest::header::LOCATION)?.to_str().ok()?;
    let next = resp.url().join(location).ok()?;
    if may_follow(resp.url(), &next) {
        return None;
    }
    Some(format!(
        "Plex redirected to another address ({}); set server to that address",
        crate::redact::escape_field(&next.origin().ascii_serialization())
    ))
}

fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        let too_many = attempt.previous().len() > MAX_REDIRECTS;
        let same_origin =
            attempt.previous().last().is_some_and(|previous| may_follow(previous, attempt.url()));
        if same_origin && !too_many {
            attempt.follow()
        } else {
            attempt.stop()
        }
    })
}

/// Ceiling for an XML response; a 2,500-track page is ~4 MB.
pub(super) const MAX_XML_BYTES: u64 = 32 * 1024 * 1024;

/// Ceiling for an image download; a cover is a few hundred kilobytes.
const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

/// The response body, or an error once it is known to exceed `max_bytes`.
pub(super) async fn read_capped(
    mut resp: reqwest::Response,
    max_bytes: u64,
    url: &str,
) -> Result<Vec<u8>> {
    let too_big = || {
        anyhow::anyhow!(
            "response from {} is larger than {} MiB; refusing it",
            crate::redact::escape_field(url),
            max_bytes / (1024 * 1024)
        )
    };
    if resp.content_length().is_some_and(|n| n > max_bytes) {
        return Err(too_big());
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .with_context(|| format!("reading {}", crate::redact::escape_field(url)))?
    {
        if body.len() as u64 + chunk.len() as u64 > max_bytes {
            return Err(too_big());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// The attributes sit on MediaContainer, which the wire model skips.
#[derive(serde::Deserialize)]
struct Id {
    #[serde(rename = "@machineIdentifier", default)]
    machine: Option<String>,
    #[serde(rename = "@version", default)]
    version: Option<String>,
}

fn playlist_reply(c: MediaContainer) -> Result<(String, Option<usize>)> {
    let p = c.playlists.into_iter().next().context("Plex's reply has no playlist")?;
    let id = p.rating_key.context("Plex's playlist has no id")?;
    Ok((id, p.leaf_count.map(|n| n as usize)))
}

/// Without a playlist id nothing is known to exist.
fn stopped(
    name: &str,
    id: Option<String>,
    saved: usize,
    total: usize,
    reason: String,
) -> SaveOutcome {
    match id {
        None => SaveOutcome::Failed { reason },
        Some(_) => SaveOutcome::Partial { name: name.to_string(), saved, total, reason },
    }
}

#[derive(Clone)]
pub struct PlexClient {
    http: reqwest::Client,
    base: String,
    token: String,
}

/// A 404 from a read, so a caller can tell a missing item from a failure.
#[derive(Debug)]
pub(crate) struct NotFound(String);

impl std::fmt::Display for NotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Plex returned HTTP 404 Not Found for {}", self.0)
    }
}

impl std::error::Error for NotFound {}

impl PlexClient {
    pub fn new(base: &str, token: &str) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .redirect(redirect_policy())
                .build()
                .expect("reqwest client"),
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }

    /// Server paths are untrusted: absolute ones and `@evil.example/x` are refused.
    pub(super) fn url_for(&self, path: &str) -> Result<Url> {
        let base = Url::parse(&self.base).context("parsing server URL")?;
        // A "://" in the query is not an origin change.
        let path_only = path.split('?').next().unwrap_or(path);
        if path_only.starts_with("//") || path_only.contains("://") {
            bail!("refusing a URL outside the Plex server: {}", crate::redact::escape_field(path));
        }
        let joined = Url::parse(&format!("{}{}", self.base, path))
            .with_context(|| format!("building a URL from {path:?}"))?;
        if joined.origin() != base.origin() {
            bail!(
                "refusing a URL outside the Plex server: {}",
                crate::redact::escape_field(joined.as_str())
            );
        }
        Ok(joined)
    }

    pub(super) async fn get_xml(&self, path: &str) -> Result<MediaContainer> {
        let url = self.url_for(path)?;
        let bytes = self.send(url.as_str(), "application/xml", MAX_XML_BYTES).await?;
        quick_xml::de::from_reader::<_, MediaContainer>(&bytes[..])
            .with_context(|| format!("parsing XML from {url}"))
    }

    pub(super) fn request(
        &self,
        method: reqwest::Method,
        url: &str,
        accept: &str,
    ) -> reqwest::RequestBuilder {
        self.http
            .request(method, url)
            .header("X-Plex-Token", &self.token)
            .header("X-Plex-Product", PRODUCT)
            .header("X-Plex-Version", env!("CARGO_PKG_VERSION"))
            .header("Accept", accept)
    }

    /// GET with auth headers, retried on transport errors.
    async fn send(&self, url: &str, accept: &str, max_bytes: u64) -> Result<Vec<u8>> {
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            // Server-supplied paths: escaped so a newline cannot forge a log line.
            tracing::debug!(
                url = %crate::redact::escape_field(url),
                attempt,
                "plex GET"
            );
            let resp = self.request(reqwest::Method::GET, url, accept).send().await;
            match resp {
                Ok(resp) => {
                    let status = resp.status();
                    if status == reqwest::StatusCode::UNAUTHORIZED
                        || status == reqwest::StatusCode::FORBIDDEN
                    {
                        bail!("Plex rejected the token (HTTP {status}); check token in the config");
                    }
                    if !status.is_success() {
                        if let Some(msg) = redirect_error(&resp) {
                            bail!(msg);
                        }
                        let url = crate::redact::escape_field(url);
                        if status == reqwest::StatusCode::NOT_FOUND {
                            return Err(NotFound(url.to_string()).into());
                        }
                        bail!("Plex returned HTTP {status} for {url}");
                    }
                    return read_capped(resp, max_bytes, url).await;
                }
                Err(e) if attempt < 3 => {
                    tracing::warn!(
                        url = %crate::redact::escape_field(url),
                        attempt,
                        "transport error, retrying: {e}"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(300 * attempt as u64))
                        .await;
                }
                Err(e) => {
                    return Err(e).with_context(|| format!("GET {url}"));
                }
            }
        }
    }

    async fn write_request(&self, method: reqwest::Method, url: &Url) -> Result<reqwest::Response> {
        tracing::debug!(
            url = %crate::redact::escape_field(url.as_str()),
            %method,
            "plex write"
        );
        self.request(method, url.as_str(), "application/xml")
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("request failed: {}", e.without_url()))
    }

    /// POST or PUT with the same headers as a read. Never retried: a second
    /// POST could create a second playlist.
    async fn write_xml(&self, method: reqwest::Method, url: Url) -> Result<MediaContainer> {
        let resp = self.write_request(method, &url).await?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            bail!(
                "Plex refused to create the playlist (HTTP {status}); this account may not be allowed to create playlists"
            );
        }
        if !status.is_success() {
            if let Some(msg) = redirect_error(&resp) {
                bail!(msg);
            }
            bail!("Plex returned HTTP {status}");
        }
        let bytes = read_capped(resp, MAX_XML_BYTES, url.as_str()).await?;
        quick_xml::de::from_reader::<_, MediaContainer>(&bytes[..]).context("parsing Plex's reply")
    }

    /// Plex's 0 to 10, or `None` to clear.
    pub async fn rate(&self, rating_key: &str, rating: Option<u8>) -> Result<()> {
        let mut url = self.url_for("/:/rate")?;
        url.query_pairs_mut()
            .append_pair("key", rating_key)
            .append_pair("identifier", "com.plexapp.plugins.library")
            .append_pair("rating", &rating.map_or(-1, i32::from).to_string());
        let resp = self.write_request(reqwest::Method::PUT, &url).await?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            bail!("Plex refused the rating (HTTP {status})");
        }
        if !status.is_success() {
            if let Some(msg) = redirect_error(&resp) {
                bail!(msg);
            }
            bail!("Plex returned HTTP {status}");
        }
        Ok(())
    }

    /// One attempt: a late report is worth less than the next one.
    pub async fn timeline(&self, client_id: &str, r: &Report) -> Result<()> {
        let mut url = self.url_for("/:/timeline")?;
        url.query_pairs_mut()
            .append_pair("ratingKey", &r.rating_key)
            .append_pair("key", &format!("/library/metadata/{}", r.rating_key))
            .append_pair("state", r.state.as_str())
            .append_pair("time", &r.time_ms.to_string())
            .append_pair("duration", &r.duration_ms.to_string())
            .append_pair("identifier", "com.plexapp.plugins.library");
        let platform = match std::env::consts::OS {
            "macos" => "macOS",
            "linux" => "Linux",
            other => other,
        };
        let resp = self
            .request(reqwest::Method::GET, url.as_str(), "application/xml")
            .header("X-Plex-Client-Identifier", client_id)
            .header("X-Plex-Device-Name", "shmc")
            .header("X-Plex-Platform", platform)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("request failed: {}", e.without_url()))?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            bail!("Plex refused the playback report (HTTP {status})");
        }
        if !status.is_success() {
            if let Some(msg) = redirect_error(&resp) {
                bail!(msg);
            }
            bail!("Plex returned HTTP {status}");
        }
        Ok(())
    }

    /// Saves `keys` as a new audio playlist; never touches an existing one and
    /// never deletes, even when a later request fails.
    pub async fn save_playlist(&self, name: &str, keys: &[String]) -> SaveOutcome {
        self.save_playlist_chunked(name, keys, SAVE_CHUNK).await
    }

    async fn save_playlist_chunked(
        &self,
        name: &str,
        keys: &[String],
        chunk: usize,
    ) -> SaveOutcome {
        let failed = |reason: String| SaveOutcome::Failed { reason };
        if keys.is_empty() {
            return failed("the queue is empty".into());
        }
        // Joined into one query value, so only plain ids may pass.
        if keys.iter().any(|k| k.is_empty() || !k.bytes().all(|b| b.is_ascii_digit())) {
            return failed("a track has an unexpected id".into());
        }
        let machine = match self.machine_id().await {
            Ok(m) => m,
            Err(e) => return failed(format!("{e:#}")),
        };
        match self.playlists().await {
            Ok(list) if list.iter().any(|p| p.title == name) => {
                return SaveOutcome::NameTaken { name: name.to_string() }
            }
            Ok(_) => {}
            Err(e) => return failed(format!("{e:#}")),
        }

        let mut seen = std::collections::HashSet::new();
        let unique: Vec<&str> =
            keys.iter().map(String::as_str).filter(|k| seen.insert(*k)).collect();
        let repeats_skipped = keys.len() - unique.len();
        let total = unique.len();

        let mut id: Option<String> = None;
        let mut confirmed = 0;
        for part in unique.chunks(chunk.max(1)) {
            let uri = format!(
                "server://{machine}/com.plexapp.plugins.library/library/metadata/{}",
                part.join(",")
            );
            let result = match &id {
                None => self.create_playlist(name, &uri).await,
                Some(id) => self.append_playlist(id, &uri).await,
            };
            let (pid, leaves) = match result {
                Ok(r) => r,
                Err(e) => return stopped(name, id, confirmed, total, format!("{e:#}")),
            };
            let leaves = match leaves {
                Some(n) => Some(n),
                None => self.leaf_count(&pid).await.ok().flatten(),
            };
            id = Some(pid);
            match leaves {
                Some(n) => confirmed = n.min(total),
                None => {
                    let reason = "Plex did not say how many tracks it added".into();
                    return stopped(name, id, confirmed, total, reason);
                }
            }
        }
        // A track removed from the library since it was queued is dropped by
        // Plex without an error; the other chunks are still worth saving.
        if confirmed < total {
            let reason = format!("Plex refused {} (removed from the library?)", total - confirmed);
            return stopped(name, id, confirmed, total, reason);
        }
        SaveOutcome::Saved { name: name.to_string(), count: total, repeats_skipped }
    }

    async fn create_playlist(&self, name: &str, uri: &str) -> Result<(String, Option<usize>)> {
        let mut url = self.url_for("/playlists")?;
        url.query_pairs_mut()
            .append_pair("type", "audio")
            .append_pair("title", name)
            .append_pair("smart", "0")
            .append_pair("uri", uri);
        playlist_reply(self.write_xml(reqwest::Method::POST, url).await?)
    }

    async fn append_playlist(&self, id: &str, uri: &str) -> Result<(String, Option<usize>)> {
        let mut url = self.url_for(&format!("/playlists/{id}/items"))?;
        url.query_pairs_mut().append_pair("uri", uri);
        playlist_reply(self.write_xml(reqwest::Method::PUT, url).await?)
    }

    async fn leaf_count(&self, id: &str) -> Result<Option<usize>> {
        let c = self.get_xml(&format!("/playlists/{id}")).await?;
        Ok(c.playlists.first().and_then(|p| p.leaf_count).map(|n| n as usize))
    }

    pub async fn music_sections(&self) -> Result<Vec<Section>> {
        let container = self.get_xml("/library/sections").await?;
        Ok(container
            .directories
            .into_iter()
            .filter(|d| d.type_.as_deref() == Some("artist"))
            .map(|d| Section {
                // sections carry their id in `key` (ratingKey is absent here)
                key: d.key,
                title: d.title.unwrap_or_default(),
            })
            .collect())
    }

    /// Every item of a listing, paged with `Container-Start`/`-Size`. `key`
    /// spots a repeated page.
    pub(crate) async fn get_all_paged<T>(
        &self,
        path: &str,
        size: usize,
        items: impl Fn(MediaContainer) -> Vec<T>,
        key: impl Fn(&T) -> String,
    ) -> Result<Vec<T>> {
        let sep = if path.contains('?') { '&' } else { '?' };
        let mut all: Vec<T> = Vec::new();
        let mut walk = PageWalk::default();
        for _ in 0..MAX_PAGES {
            let start = walk.offset;
            let c = self
                .get_xml(&format!(
                    "{path}{sep}X-Plex-Container-Start={start}&X-Plex-Container-Size={size}"
                ))
                .await
                .with_context(|| format!("loading the page at offset {start}"))?;
            let total = c.total_size;
            let kept = walk.accept(items(c), total, size, &key);
            if walk.whole {
                return Ok(kept);
            }
            all.extend(kept);
            if walk.done {
                return Ok(all);
            }
        }
        bail!("more than {MAX_PAGES} pages of {path}; giving up")
    }

    pub async fn artists(&self, section: &str) -> Result<Vec<Artist>> {
        let dirs = self
            .get_all_paged(
                &format!("/library/sections/{section}/all?type=8"),
                LIST_PAGE_SIZE,
                |c| c.directories,
                |d| d.key.clone(),
            )
            .await?;
        let mut artists: Vec<Artist> = dirs.into_iter().map(|d| d.into_artist()).collect();
        artists.sort_by_key(|a| a.title.to_lowercase());
        Ok(artists)
    }

    pub async fn albums(&self, section: &str, artist_key: &str) -> Result<Vec<Album>> {
        let dirs = self
            .get_all_paged(
                &format!("/library/sections/{section}/all?type=9&parent={artist_key}"),
                LIST_PAGE_SIZE,
                |c| c.directories,
                |d| d.key.clone(),
            )
            .await?;
        // Server order: the app sorts by `ui.album_sort` when they arrive.
        Ok(dirs.into_iter().map(|d| d.into_album()).collect())
    }

    pub async fn album_tracks(&self, album_key: &str) -> Result<Vec<Track>> {
        let mut c =
            self.get_xml(&format!("/library/metadata/{album_key}/children?type=10")).await?;
        // Disc first: track numbers restart on every disc.
        c.tracks.sort_by_key(|t| (t.parent_index.unwrap_or(0), t.index.unwrap_or(u32::MAX)));
        Ok(c.into_tracks())
    }

    /// Paged: a compilation "artist" can have tens of thousands of tracks.
    pub async fn artist_tracks(&self, section: &str, artist_key: &str) -> Result<Vec<Track>> {
        self.get_all_paged(
            &format!("/library/sections/{section}/all?type=10&artist.id={artist_key}"),
            PLAYLIST_PAGE_SIZE,
            |c| c.into_tracks(),
            |t: &Track| t.rating_key.clone(),
        )
        .await
    }

    pub async fn track(&self, rating_key: &str) -> Result<Track> {
        let c = self.get_xml(&format!("/library/metadata/{rating_key}")).await?;
        c.tracks.into_iter().next().map(|t| t.into_track()).context("no track found")
    }

    /// Missing keys are skipped.
    pub async fn tracks_by_keys(&self, keys: &[String]) -> Result<Vec<Track>> {
        let mut tracks = Vec::with_capacity(keys.len());
        for batch in keys.chunks(MULTI_KEY_BATCH) {
            let c = self.get_xml(&format!("/library/metadata/{}", batch.join(","))).await?;
            tracks.extend(c.into_tracks());
        }
        Ok(tracks)
    }

    pub async fn artist(&self, rating_key: &str) -> Result<Artist> {
        let c = self.get_xml(&format!("/library/metadata/{rating_key}")).await?;
        c.directories.into_iter().next().map(|d| d.into_artist()).context("no artist found")
    }

    /// Audio playlists, sorted by title. `?type=playlist` can return HTTP 500,
    /// so it is filtered here.
    pub async fn playlists(&self) -> Result<Vec<Playlist>> {
        let c = self.get_xml("/playlists").await?;
        let mut list: Vec<Playlist> = c
            .playlists
            .into_iter()
            .filter(|p| matches!(p.playlist_type.as_deref(), None | Some("audio")))
            .filter_map(|p| p.into_playlist())
            .collect();
        list.sort_by_key(|a| a.title.to_lowercase());
        Ok(list)
    }

    /// Tracks of a playlist, in playlist order (never re-sorted).
    pub async fn playlist_items(&self, rating_key: &str) -> Result<Vec<Track>> {
        self.get_all_paged(
            &format!("/playlists/{rating_key}/items"),
            PLAYLIST_PAGE_SIZE,
            |c| c.into_tracks(),
            entry_key,
        )
        .await
    }

    /// One page of playlist items for the hover preview.
    pub async fn playlist_items_page(
        &self,
        rating_key: &str,
        start: usize,
        size: usize,
    ) -> Result<Vec<Track>> {
        Ok(self.playlist_page(rating_key, start, size).await?.tracks)
    }

    /// A page and the `totalSize` (`leafCount` can be stale); Plex ignores `limit` here.
    pub async fn playlist_page(
        &self,
        rating_key: &str,
        start: usize,
        size: usize,
    ) -> Result<PlaylistPage> {
        let c = self
            .get_xml(&format!(
                "/playlists/{rating_key}/items?X-Plex-Container-Start={start}&X-Plex-Container-Size={size}"
            ))
            .await?;
        Ok(PlaylistPage { total: c.total_size, tracks: c.into_tracks() })
    }

    /// Full-text search. `GET /search` can return only `<Provider>`
    /// stubs, so this uses `/hubs/search`.
    pub async fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        let mut url = self.url_for("/hubs/search")?;
        url.query_pairs_mut().append_pair("query", query).append_pair("limit", "50");
        let bytes = self.send(url.as_str(), "application/xml", MAX_XML_BYTES).await?;
        let c: MediaContainer =
            quick_xml::de::from_reader(&bytes[..]).context("parsing search XML")?;
        Ok(c.into_search_hits())
    }

    pub async fn thumb_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let url = self.url_for(path)?;
        self.send(url.as_str(), "image/*", MAX_IMAGE_BYTES).await
    }

    async fn identity_raw(&self) -> Result<Id> {
        let url = self.url_for("/identity")?;
        let bytes = self.send(url.as_str(), "application/xml", MAX_XML_BYTES).await?;
        Ok(quick_xml::de::from_reader(&bytes[..])?)
    }

    /// Unlike `identity`, a missing id is an error.
    pub(crate) async fn machine_id(&self) -> Result<String> {
        self.identity_raw()
            .await?
            .machine
            .filter(|m| !m.is_empty())
            .context("Plex did not report its machine identifier")
    }

    /// Machine id and version, for `shmc config --check`.
    /// One try, timed out fast: on a dead kept-alive connection a request hangs.
    pub async fn ping(&self) -> bool {
        let Ok(url) = self.url_for("/identity") else { return false };
        self.request(reqwest::Method::GET, url.as_str(), "application/xml")
            .timeout(PING_TIMEOUT)
            .send()
            .await
            .is_ok_and(|resp| resp.status().is_success())
    }

    pub async fn identity(&self) -> Result<(String, String)> {
        let id = self.identity_raw().await?;
        Ok((
            id.machine.unwrap_or_else(|| "unknown".into()),
            id.version.unwrap_or_else(|| "unknown".into()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).expect("test url")
    }

    #[test]
    fn redirects_on_the_same_origin_are_followed() {
        assert!(may_follow(&url("https://plex:32400/a"), &url("https://plex:32400/b")));
        assert!(may_follow(&url("http://plex:32400/a"), &url("http://plex:32400/b")));
        // plain HTTP redirecting to TLS is a real setup
        assert!(may_follow(&url("http://plex:32400/a"), &url("https://plex:32400/a")));
        assert!(may_follow(&url("http://plex/a"), &url("https://plex/a")));
    }

    #[test]
    fn redirects_off_the_origin_are_stopped() {
        assert!(!may_follow(&url("https://plex:32400/a"), &url("http://plex:32400/a")));
        assert!(!may_follow(&url("https://plex:32400/a"), &url("https://evil.example/a")));
        assert!(!may_follow(&url("https://plex:32400/a"), &url("https://evil.example:32400/a")));
        assert!(!may_follow(&url("https://plex:32400/a"), &url("https://plex:8443/a")));
        assert!(!may_follow(&url("http://plex/a"), &url("http://plex:32400/a")));
    }

    #[tokio::test]
    async fn ping_gives_up_on_a_server_that_never_answers() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (_held, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let client = PlexClient::new(&base, "faketoken-ping");

        let started = std::time::Instant::now();
        assert!(!client.ping().await);
        assert!(started.elapsed() < PING_TIMEOUT * 2, "{:?}", started.elapsed());

        let (base, _, _) = testing::serve_full(|_| (200, "<MediaContainer/>".into())).await;
        assert!(PlexClient::new(&base, "faketoken-ping").ping().await);
    }

    #[tokio::test]
    async fn tracks_by_keys_asks_in_batches() {
        let (base, seen, _) = testing::serve_full(|line| {
            let keys = line.split(' ').nth(1).unwrap().trim_start_matches("/library/metadata/");
            let tracks: String = keys
                .split(',')
                .map(|k| {
                    format!(
                        "<Track ratingKey=\"{k}\" key=\"/library/metadata/{k}\" title=\"T{k}\"/>"
                    )
                })
                .collect();
            (200, format!("<MediaContainer>{tracks}</MediaContainer>"))
        })
        .await;
        let client = PlexClient::new(&base, "faketoken-keys");
        let keys: Vec<String> = (1..=MULTI_KEY_BATCH + 1).map(|k| k.to_string()).collect();

        let tracks = client.tracks_by_keys(&keys).await.unwrap();
        let got: Vec<String> = tracks.into_iter().map(|t| t.rating_key).collect();
        assert_eq!(got, keys);
        assert_eq!(seen.lock().unwrap().len(), 2);
    }
}

#[cfg(test)]
pub(crate) mod testing {
    /// Playlist entries `from..to`, each with its own `playlistItemID`.
    pub(crate) fn playlist_xml(
        from: usize,
        to: usize,
        total: Option<usize>,
        rating_key: impl Fn(usize) -> String,
    ) -> String {
        let total = total.map(|t| format!(" totalSize=\"{t}\"")).unwrap_or_default();
        let items: String = (from..to)
            .map(|i| {
                format!(
                    "<Track ratingKey=\"{k}\" key=\"/library/metadata/{k}\" title=\"T{i}\" playlistItemID=\"{i}\"><Media><Part id=\"{i}\" key=\"/library/parts/{i}/f.mp3\"/></Media></Track>",
                    k = rating_key(i)
                )
            })
            .collect();
        format!(
            "<?xml version=\"1.0\"?><MediaContainer size=\"{}\"{total}>{items}</MediaContainer>",
            to - from
        )
    }

    /// A fake PMS: `page(start, size, request_line)` returns the body (`None`
    /// is a 500). Returns the base URL and the request lines seen.
    pub(crate) async fn serve_pages(
        page: impl Fn(usize, usize, &str) -> Option<String> + Send + 'static,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = log.clone();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 8192];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let line = req.lines().next().unwrap_or("").to_string();
                let param = |name: &str| {
                    line.split(&format!("{name}="))
                        .nth(1)
                        .and_then(|v| v.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok())
                };
                let (start, size) = (
                    param("X-Plex-Container-Start").unwrap_or(0),
                    param("X-Plex-Container-Size").unwrap_or(0),
                );
                seen.lock().unwrap().push(line.clone());
                let resp = match page(start, size, &line) {
                    Some(body) => format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    ),
                    None => "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                };
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        (base, log)
    }

    pub(crate) type Seen = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

    /// `serve`, also keeping each whole request (headers included).
    pub(crate) async fn serve_full(
        reply: impl Fn(&str) -> (u16, String) + Send + 'static,
    ) -> (String, Seen, Seen) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen: Seen = Seen::default();
        let log = seen.clone();
        let full: Seen = Seen::default();
        let full_log = full.clone();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 16384];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let line = req.lines().next().unwrap_or("").to_string();
                log.lock().unwrap().push(line.clone());
                full_log.lock().unwrap().push(req);
                let (status, body) = reply(&line);
                let resp = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        (base, seen, full)
    }
}

#[cfg(test)]
mod url_for_tests {
    use super::testing::serve_pages;
    use super::*;

    /// A path from the server must not send the token elsewhere.
    #[test]
    fn other_origin_url_refused() {
        let client = PlexClient::new("http://plex:32400", "tok");
        assert!(client.url_for("/library/sections").is_ok());
        assert!(client.url_for("http://evil.example/x").is_err());
        assert!(client.url_for("//evil.example/x").is_err());
        // Concatenation would read this as userinfo.
        assert!(client.url_for("@evil.example/x").is_err());
    }

    #[test]
    fn proxied_base_path_survives() {
        let client = PlexClient::new("http://host/plex", "tok");
        let joined = client.url_for("/library/metadata/1").unwrap();
        assert_eq!(joined.as_str(), "http://host/plex/library/metadata/1");

        let paged = client.url_for("/library/sections?type=8&limit=500").unwrap();
        assert_eq!(paged.as_str(), "http://host/plex/library/sections?type=8&limit=500");
    }

    /// One canned response: the raw `head`, then `body_len` zero bytes.
    async fn serve_once(head: String, body_len: usize) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = sock.read(&mut buf).await;
            let _ = sock.write_all(head.as_bytes()).await;
            let block = vec![0u8; 16 * 1024];
            let mut sent = 0;
            while sent < body_len {
                let n = block.len().min(body_len - sent);
                if sock.write_all(&block[..n]).await.is_err() {
                    break;
                }
                sent += n;
            }
        });
        base
    }

    #[tokio::test]
    async fn redirect_to_another_host_names_only_its_origin() {
        let head = "HTTP/1.1 301 Moved\r\nLocation: http://other.example/library?X-Plex-Token=faketoken123\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let base = serve_once(head.into(), 0).await;
        let err = PlexClient::new(&base, "faketoken-bbbb").playlists().await.unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("other.example"), "{text}");
        assert!(!text.contains("faketoken123") && !text.contains("X-Plex-Token"), "{text}");
    }

    fn dirs_xml(from: usize, to: usize, total: Option<usize>) -> String {
        let total = total.map(|t| format!(" totalSize=\"{t}\"")).unwrap_or_default();
        let items: String = (from..to)
            .map(|i| {
                format!("<Directory ratingKey=\"{i}\" key=\"/library/metadata/{i}/children\" type=\"artist\" title=\"A{i:05}\"/>")
            })
            .collect();
        format!(
            "<?xml version=\"1.0\"?><MediaContainer size=\"{}\"{total}>{items}</MediaContainer>",
            to - from
        )
    }

    fn paged(n: usize) -> impl Fn(usize, usize, &str) -> Option<String> + Send + 'static {
        move |start, size, _| {
            let s = start.min(n);
            Some(dirs_xml(s, (s + size).min(n), None))
        }
    }

    use super::testing::playlist_xml;

    fn playlist_server(n: usize) -> impl Fn(usize, usize, &str) -> Option<String> + Send + 'static {
        move |start, size, _| {
            let s = start.min(n);
            Some(playlist_xml(s, (s + size).min(n), Some(n), |i| format!("k{i}")))
        }
    }

    #[tokio::test]
    async fn only_audio_playlists_are_listed() {
        let body = "<MediaContainer>\
            <Playlist ratingKey=\"1\" title=\"songs\" playlistType=\"audio\"/>\
            <Playlist ratingKey=\"2\" title=\"films\" playlistType=\"video\"/>\
            <Playlist ratingKey=\"3\" title=\"pics\" playlistType=\"photo\"/>\
            </MediaContainer>";
        let (base, _) = serve_pages(move |_, _, _| Some(body.to_string())).await;
        let lists = PlexClient::new(&base, "faketoken-bbbb").playlists().await.unwrap();
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].title, "songs");
    }

    #[tokio::test]
    async fn multi_disc_album_ordered_by_disc() {
        let track = |k: u32, disc: u32, no: u32| {
            format!("<Track ratingKey=\"{k}\" key=\"/library/metadata/{k}\" index=\"{no}\" parentIndex=\"{disc}\"/>")
        };
        let body = format!(
            "<MediaContainer>{}{}{}{}</MediaContainer>",
            track(1, 1, 1),
            track(2, 1, 2),
            track(3, 2, 1),
            track(4, 2, 2)
        );
        let (base, _) = serve_pages(move |_, _, _| Some(body.clone())).await;
        let tracks = PlexClient::new(&base, "faketoken-bbbb").album_tracks("7").await.unwrap();
        let keys: Vec<_> = tracks.iter().map(|t| t.rating_key.as_str()).collect();
        assert_eq!(keys, ["1", "2", "3", "4"]);
    }

    #[tokio::test]
    async fn playlist_fetched_in_pages() {
        let (base, log) = serve_pages(playlist_server(5237)).await;
        let client = PlexClient::new(&base, "faketoken-bbbb");
        let tracks = client.playlist_items("9").await.unwrap();
        assert_eq!(tracks.len(), 5237);
        assert_eq!(tracks[5236].rating_key, "k5236");
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 3, "{log:?}");
        assert!(log[1]
            .contains("/playlists/9/items?X-Plex-Container-Start=2500&X-Plex-Container-Size=2500"));
    }

    #[tokio::test]
    async fn artist_tracks_paged_under_query() {
        let (base, log) = serve_pages(playlist_server(3200)).await;
        let tracks =
            PlexClient::new(&base, "faketoken-bbbb").artist_tracks("5", "77").await.unwrap();
        assert_eq!(tracks.len(), 3200);
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 2);
        assert!(log[1].contains("type=10&artist.id=77&X-Plex-Container-Start=2500"));
    }

    /// Only a repeated *entry* means the server ignored the offset.
    #[tokio::test]
    async fn repeated_track_playlist_not_cut_short() {
        let (base, _) = serve_pages(|start, size, _| {
            let s = start.min(5100);
            Some(playlist_xml(s, (s + size).min(5100), None, |_| "same".into()))
        })
        .await;
        let tracks = PlexClient::new(&base, "faketoken-bbbb").playlist_items("9").await.unwrap();
        assert_eq!(tracks.len(), 5100);
    }

    async fn artist_titles(base: &str) -> Result<Vec<String>> {
        let client = PlexClient::new(base, "faketoken-bbbb");
        Ok(client.artists("5").await?.into_iter().map(|a| a.title).collect())
    }

    fn expected(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("A{i:05}")).collect()
    }

    #[tokio::test]
    async fn artists_are_paged_until_a_short_page() {
        let (base, log) = serve_pages(paged(1120)).await;
        assert_eq!(artist_titles(&base).await.unwrap(), expected(1120));
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 3);
        assert!(log[1].contains("type=8&X-Plex-Container-Start=500&X-Plex-Container-Size=500"));
        assert!(!log[1].contains("limit="));
    }

    #[tokio::test]
    async fn albums_keep_their_query_when_paged() {
        let (base, log) = serve_pages(paged(3)).await;
        let client = PlexClient::new(&base, "faketoken-bbbb");
        assert_eq!(client.albums("5", "77").await.unwrap().len(), 3);
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 1);
        assert!(
            log[0].contains("type=9&parent=77&X-Plex-Container-Start=0&X-Plex-Container-Size=500")
        );
    }

    #[tokio::test]
    async fn exact_multiple_ends_on_empty_page() {
        let (base, log) = serve_pages(paged(1000)).await;
        assert_eq!(artist_titles(&base).await.unwrap().len(), 1000);
        assert_eq!(log.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn paging_ignoring_server_asked_once() {
        let (base, log) = serve_pages(|_, _, _| Some(dirs_xml(0, 1200, None))).await;
        assert_eq!(artist_titles(&base).await.unwrap(), expected(1200));
        assert_eq!(log.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn repeating_server_does_not_loop() {
        let (base, log) = serve_pages(|_, _, _| Some(dirs_xml(0, 500, None))).await;
        assert_eq!(artist_titles(&base).await.unwrap(), expected(500));
        assert_eq!(log.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn total_size_ends_the_loop() {
        // Full pages forever, but the total says 700.
        let (base, log) =
            serve_pages(|start, size, _| Some(dirs_xml(start, start + size, Some(700)))).await;
        assert_eq!(artist_titles(&base).await.unwrap().len(), 1000);
        assert_eq!(log.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn failing_page_fails_call_without_token() {
        let (base, _) =
            serve_pages(|start, size, _| (start == 0).then(|| dirs_xml(0, size, None))).await;
        let err = format!("{:#}", artist_titles(&base).await.unwrap_err());
        assert!(err.contains("offset 500"), "{err}");
        assert!(!err.contains("faketoken-bbbb"), "{err}");
    }

    #[tokio::test]
    async fn oversized_response_refused_without_token() {
        let token = "faketoken-aaaa";
        let limit = 64 * 1024;

        let head = "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_string();
        let base = serve_once(head, 10 * limit as usize).await;
        let err = PlexClient::new(&base, token)
            .send(&format!("{base}/x"), "*/*", limit)
            .await
            .expect_err("a body past the ceiling fails");
        assert!(!format!("{err:#}").contains(token));

        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            100 * limit
        );
        let base = serve_once(head, 0).await;
        let err = PlexClient::new(&base, token)
            .send(&format!("{base}/x"), "*/*", limit)
            .await
            .expect_err("a lying length fails");
        assert!(format!("{err:#}").contains("larger than"), "{err:#}");

        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            limit / 2
        );
        let base = serve_once(head, limit as usize / 2).await;
        let body =
            PlexClient::new(&base, token).send(&format!("{base}/x"), "*/*", limit).await.unwrap();
        assert_eq!(body.len() as u64, limit / 2);
    }
}

#[cfg(test)]
mod save_tests {
    use super::testing::{serve_full, Seen};
    use super::*;
    use std::sync::Mutex;

    /// A fake PMS answering `reply(request_line)` with a status and body.
    async fn serve(reply: impl Fn(&str) -> (u16, String) + Send + 'static) -> (String, Seen) {
        let (base, seen, _) = serve_full(reply).await;
        (base, seen)
    }

    fn param(line: &str, name: &str) -> Option<String> {
        let path = line.split(' ').nth(1)?;
        let url = Url::parse(&format!("http://x{path}")).ok()?;
        url.query_pairs().find(|(k, _)| k == name).map(|(_, v)| v.into_owned())
    }

    fn uri_keys(line: &str) -> Vec<String> {
        let uri = param(line, "uri").unwrap();
        let (head, keys) = uri.rsplit_once('/').unwrap();
        assert_eq!(head, "server://m1/com.plexapp.plugins.library/library/metadata");
        keys.split(',').map(String::from).collect()
    }

    fn playlist(leaves: usize) -> String {
        format!(
            "<MediaContainer><Playlist ratingKey=\"77\" title=\"t\" playlistType=\"audio\" leafCount=\"{leaves}\"/></MediaContainer>"
        )
    }

    /// A server that accepts writes, with `existing` playlists and `fail` as
    /// the status for appends.
    async fn server(existing: &str, fail_append: u16) -> (String, Seen) {
        let existing = existing.to_string();
        let leaves = Mutex::new(0usize);
        serve(move |line| {
            // key 404 plays a track removed from the library: Plex drops it
            let n = || uri_keys(line).iter().filter(|k| *k != "404").count();
            if line.starts_with("GET /identity") {
                (200, "<MediaContainer machineIdentifier=\"m1\"/>".into())
            } else if line.starts_with("GET /playlists") {
                (200, format!("<MediaContainer>{existing}</MediaContainer>"))
            } else if line.starts_with("POST /playlists") {
                *leaves.lock().unwrap() = n();
                (200, playlist(n()))
            } else if line.starts_with("PUT /playlists/77/items") && fail_append != 200 {
                (fail_append, String::new())
            } else if line.starts_with("PUT /playlists/77/items") {
                let mut l = leaves.lock().unwrap();
                *l += n();
                (200, playlist(*l))
            } else {
                (404, String::new())
            }
        })
        .await
    }

    #[tokio::test]
    async fn rate_sends_the_rating_or_minus_one_without_the_token_in_the_url() {
        let (base, seen) = serve(|_| (200, String::new())).await;
        let client = PlexClient::new(&base, "faketoken-rate");
        client.rate("42", Some(8)).await.unwrap();
        client.rate("42", None).await.unwrap();
        let w = writes(&seen);
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w[0].starts_with("PUT /:/rate?"), "{}", w[0]);
        assert_eq!(param(&w[0], "key").as_deref(), Some("42"));
        assert_eq!(param(&w[0], "identifier").as_deref(), Some("com.plexapp.plugins.library"));
        assert_eq!(param(&w[0], "rating").as_deref(), Some("8"));
        assert_eq!(param(&w[1], "rating").as_deref(), Some("-1"));
        assert!(w.iter().all(|l| !l.contains("X-Plex-Token")), "{w:?}");
    }

    #[tokio::test]
    async fn timeline_sends_the_state_and_identity_without_the_token_in_the_url() {
        use crate::plex::report::{PlayState, Report};
        let (base, seen, full) = serve_full(|_| (200, String::new())).await;
        let client = PlexClient::new(&base, "faketoken-timeline");
        let report = Report {
            rating_key: "42".into(),
            state: PlayState::Paused,
            time_ms: 31_000,
            duration_ms: 257_000,
        };
        client.timeline("abc123", &report).await.unwrap();
        let line = seen.lock().unwrap()[0].clone();
        assert!(line.starts_with("GET /:/timeline?"), "{line}");
        assert_eq!(param(&line, "ratingKey").as_deref(), Some("42"));
        assert_eq!(param(&line, "key").as_deref(), Some("/library/metadata/42"));
        assert_eq!(param(&line, "state").as_deref(), Some("paused"));
        assert_eq!(param(&line, "time").as_deref(), Some("31000"));
        assert_eq!(param(&line, "duration").as_deref(), Some("257000"));
        assert_eq!(param(&line, "identifier").as_deref(), Some("com.plexapp.plugins.library"));
        assert!(!line.contains("X-Plex-Token"), "{line}");
        let head = full.lock().unwrap()[0].to_lowercase();
        assert!(head.contains("x-plex-client-identifier: abc123"), "{head}");
        assert!(head.contains("x-plex-device-name: shmc"), "{head}");
        assert!(head.contains("x-plex-platform: "), "{head}");
        assert!(head.contains("x-plex-token: faketoken-timeline"), "{head}");
    }

    fn keys(ks: &[&str]) -> Vec<String> {
        ks.iter().map(|k| k.to_string()).collect()
    }

    fn writes(seen: &Seen) -> Vec<String> {
        seen.lock().unwrap().iter().filter(|l| !l.starts_with("GET ")).cloned().collect()
    }

    #[tokio::test]
    async fn long_queue_created_then_appended_in_order() {
        let (base, seen) = server("", 200).await;
        let name = "Road & trip #1 ö";
        let outcome = PlexClient::new(&base, "faketoken-save")
            .save_playlist_chunked(name, &keys(&["1", "2", "2", "3", "4", "1", "5"]), 2)
            .await;
        assert_eq!(outcome, SaveOutcome::Saved { name: name.into(), count: 5, repeats_skipped: 2 });
        let w = writes(&seen);
        assert_eq!(w.len(), 3, "{w:?}");
        assert!(w[0].starts_with("POST /playlists?"));
        assert_eq!(param(&w[0], "title").as_deref(), Some(name));
        assert_eq!(param(&w[0], "type").as_deref(), Some("audio"));
        assert_eq!(uri_keys(&w[0]), ["1", "2"]);
        assert!(w[1].starts_with("PUT /playlists/77/items?"));
        assert_eq!(uri_keys(&w[1]), ["3", "4"]);
        assert_eq!(uri_keys(&w[2]), ["5"]);
    }

    #[tokio::test]
    async fn failed_append_leaves_partial_playlist() {
        let (base, seen) = server("", 500).await;
        let outcome = PlexClient::new(&base, "faketoken-save")
            .save_playlist_chunked("Mix", &keys(&["1", "2", "3", "4", "5"]), 2)
            .await;
        let SaveOutcome::Partial { saved, total, .. } = outcome else { panic!("{outcome:?}") };
        assert_eq!((saved, total), (2, 5));
        let w = writes(&seen);
        assert_eq!(w.len(), 2, "one create and one failed append, no retry or delete: {w:?}");
        assert!(w.iter().all(|l| l.starts_with("POST ") || l.starts_with("PUT ")));
    }

    #[tokio::test]
    async fn dropped_track_does_not_stop_later_chunks() {
        let (base, seen) = server("", 200).await;
        let outcome = PlexClient::new(&base, "faketoken-save")
            .save_playlist_chunked("Mix", &keys(&["1", "404", "3", "4", "5"]), 2)
            .await;
        let SaveOutcome::Partial { saved, total, .. } = outcome else { panic!("{outcome:?}") };
        assert_eq!((saved, total), (4, 5));
        assert_eq!(writes(&seen).len(), 3, "every chunk is still sent");
    }

    #[tokio::test]
    async fn existing_name_writes_nothing() {
        let existing =
            "<Playlist ratingKey=\"5\" title=\"Mix\" playlistType=\"audio\" leafCount=\"3\"/>";
        let (base, seen) = server(existing, 200).await;
        let outcome =
            PlexClient::new(&base, "faketoken-save").save_playlist("Mix", &keys(&["1"])).await;
        assert_eq!(outcome, SaveOutcome::NameTaken { name: "Mix".into() });
        assert!(writes(&seen).is_empty());
    }

    #[tokio::test]
    async fn refused_create_fails_without_token() {
        let (base, seen) = serve(|line| {
            if line.starts_with("GET /identity") {
                (200, "<MediaContainer machineIdentifier=\"m1\"/>".into())
            } else if line.starts_with("GET ") {
                (200, "<MediaContainer/>".into())
            } else {
                (403, String::new())
            }
        })
        .await;
        let token = "faketoken-save";
        let outcome = PlexClient::new(&base, token).save_playlist("Mix", &keys(&["1"])).await;
        let SaveOutcome::Failed { reason } = outcome else { panic!("{outcome:?}") };
        assert!(reason.contains("403"), "{reason}");
        assert!(!reason.contains("token"), "{reason}");
        assert!(!reason.contains(token));
        assert_eq!(writes(&seen).len(), 1);
    }
}

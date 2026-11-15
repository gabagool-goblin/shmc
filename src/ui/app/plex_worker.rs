use crate::plex::PlexClient;
use crate::ui::playlist_load::PlaylistRequest;
use crate::ui::queue_seq::{QueueOutcome, QueueReplyGuard};
use image::DynamicImage;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug)]
pub enum PlexCmd {
    LoadArtists,
    LoadAlbums(String),
    LoadTracks(String),
    Search(String),
    LoadPlaylists,
    /// Creates a new playlist; never edits or deletes one.
    SavePlaylist {
        name: String,
        keys: Vec<String>,
    },
    LoadPlaylistPreview {
        key: String,
        size: usize,
    },
    ResolvePlaylist {
        seq: u64,
        key: String,
        confirm: Confirm,
        action: QueueAction,
        /// An `entry_key`: start from that entry.
        start: Option<String>,
        stream: bool,
        /// The total to show when the server gives none.
        expected: usize,
        cancel: Arc<AtomicBool>,
    },
    ResolveTrack {
        seq: u64,
        key: String,
        confirm: Confirm,
        action: QueueAction,
        rest_of_album: bool,
    },
    ResolveAlbum {
        seq: u64,
        key: String,
        confirm: Confirm,
        action: QueueAction,
    },
    ResolveArtistTracks {
        seq: u64,
        key: String,
        confirm: Confirm,
        action: QueueAction,
    },
    ResolveArtist(String),
    LoadAlbumDetail {
        key: String,
        label: String,
    },
    LoadThumb {
        path: String,
    },
    /// Sample rate and bit depth come only from the full metadata.
    LoadTrackInfo(String),
}

pub struct PlexHandle {
    pub tx: UnboundedSender<PlexCmd>,
}

pub fn spawn_plex_worker(
    client: PlexClient,
    section: String,
    tx: UnboundedSender<Msg>,
    art_cache_mb: u64,
    art_max_size_px: [u32; 2],
) -> UnboundedSender<PlexCmd> {
    let (ctx, mut crx) = tokio::sync::mpsc::unbounded_channel::<PlexCmd>();
    tokio::spawn(async move {
        while let Some(cmd) = crx.recv().await {
            let client = client.clone();
            let tx = tx.clone();
            let section = section.clone();
            tokio::spawn(async move {
                match cmd {
                    PlexCmd::LoadArtists => match client.artists(&section).await {
                        Ok(a) => {
                            let _ = tx.send(Msg::ArtistsLoaded(a));
                        }
                        Err(e) => {
                            let _ = tx.send(Msg::Error(format!("loading artists: {e:#}")));
                        }
                    },
                    PlexCmd::LoadAlbums(key) => match client.albums(&section, &key).await {
                        Ok(albums) => {
                            let _ = tx.send(Msg::AlbumsLoaded { artist_key: key, albums });
                        }
                        Err(e) => {
                            let _ = tx.send(Msg::LoadFailed {
                                key,
                                error: format!("loading albums: {e:#}"),
                            });
                        }
                    },
                    PlexCmd::LoadTracks(key) => match client.album_tracks(&key).await {
                        Ok(tracks) => {
                            let _ = tx.send(Msg::TracksLoaded { album_key: key, tracks });
                        }
                        Err(e) => {
                            let _ = tx.send(Msg::LoadFailed {
                                key,
                                error: format!("loading tracks: {e:#}"),
                            });
                        }
                    },
                    PlexCmd::Search(q) => match client.search(&q).await {
                        Ok(hits) => {
                            let _ = tx.send(Msg::SearchResults { query: q, hits });
                        }
                        Err(e) => {
                            let _ = tx.send(Msg::Error(format!("searching: {e:#}")));
                        }
                    },
                    PlexCmd::LoadPlaylists => match client.playlists().await {
                        Ok(list) => {
                            let _ = tx.send(Msg::PlaylistsLoaded(list));
                        }
                        Err(e) => {
                            let _ = tx.send(Msg::Error(format!("loading playlists: {e:#}")));
                        }
                    },
                    PlexCmd::SavePlaylist { name, keys } => {
                        let outcome = client.save_playlist(&name, &keys).await;
                        let _ = tx.send(Msg::PlaylistSaved(outcome));
                    }
                    PlexCmd::LoadPlaylistPreview { key, size } => {
                        match client.playlist_items_page(&key, 0, size).await {
                            Ok(tracks) => {
                                let _ =
                                    tx.send(Msg::PlaylistItemsLoaded { rating_key: key, tracks });
                            }
                            Err(e) => {
                                tracing::warn!("playlist preview: {e:#}");
                                let _ = tx.send(Msg::PreviewFailed { key });
                            }
                        }
                    }
                    PlexCmd::ResolvePlaylist {
                        seq,
                        key,
                        confirm,
                        action,
                        start,
                        stream,
                        expected,
                        cancel,
                    } => {
                        crate::ui::playlist_load::run(
                            client,
                            tx,
                            PlaylistRequest {
                                seq,
                                key,
                                confirm,
                                action,
                                start,
                                stream,
                                expected,
                                cancel,
                            },
                        )
                        .await;
                    }
                    PlexCmd::ResolveTrack { seq, key, confirm, action, rest_of_album } => {
                        let reply = QueueReplyGuard::new(tx, seq);
                        match client.track(&key).await {
                            Ok(t) => {
                                let mut tracks = vec![t.clone()];
                                if rest_of_album {
                                    if let Some(album_key) = t.parent_rating_key.as_deref() {
                                        if let Ok(mut album) = client.album_tracks(album_key).await
                                        {
                                            if let Some(pos) = album
                                                .iter()
                                                .position(|x| x.rating_key == t.rating_key)
                                            {
                                                tracks = album.split_off(pos);
                                            }
                                        }
                                    }
                                }
                                reply.send(QueueOutcome::Tracks { tracks, confirm, action });
                            }
                            Err(e) => {
                                reply.send(QueueOutcome::Failed(format!("resolving track: {e:#}")));
                            }
                        }
                    }
                    PlexCmd::ResolveArtistTracks { seq, key, confirm, action } => {
                        let reply = QueueReplyGuard::new(tx, seq);
                        match client.artist_tracks(&section, &key).await {
                            Ok(tracks) => {
                                reply.send(QueueOutcome::Tracks { tracks, confirm, action });
                            }
                            Err(e) => {
                                reply
                                    .send(QueueOutcome::Failed(format!("resolving artist: {e:#}")));
                            }
                        }
                    }
                    PlexCmd::ResolveAlbum { seq, key, confirm, action } => {
                        let reply = QueueReplyGuard::new(tx, seq);
                        match client.album_tracks(&key).await {
                            Ok(tracks) => {
                                reply.send(QueueOutcome::Tracks { tracks, confirm, action });
                            }
                            Err(e) => {
                                reply.send(QueueOutcome::Failed(format!("resolving album: {e:#}")));
                            }
                        }
                    }
                    PlexCmd::ResolveArtist(key) => match client.artist(&key).await {
                        Ok(a) => {
                            let _ = tx.send(Msg::GotoArtist(a));
                        }
                        Err(e) => {
                            let _ = tx.send(Msg::LoadFailed {
                                key,
                                error: format!("resolving artist: {e:#}"),
                            });
                        }
                    },
                    PlexCmd::LoadAlbumDetail { key, label } => {
                        match client.album_tracks(&key).await {
                            Ok(tracks) => {
                                let _ = tx.send(Msg::SearchAlbumLoaded { key, label, tracks });
                            }
                            Err(e) => {
                                let _ = tx.send(Msg::Error(format!("loading album: {e:#}")));
                            }
                        }
                    }
                    PlexCmd::LoadThumb { path } => {
                        match load_thumb(&client, &path, art_cache_mb, art_max_size_px).await {
                            Ok(img) => {
                                let _ = tx.send(Msg::ArtReady { thumb_path: path, img });
                            }
                            Err(e) => {
                                tracing::warn!("album art: {e:#}");
                                let _ = tx.send(Msg::ArtFailed { thumb_path: path });
                            }
                        }
                    }
                    PlexCmd::LoadTrackInfo(key) => match client.track(&key).await {
                        Ok(t) => {
                            let _ = tx.send(Msg::TrackInfo { rating_key: key, audio: t.audio });
                        }
                        Err(e) => tracing::warn!("track info: {e:#}"),
                    },
                }
            });
        }
    });
    ctx
}

async fn load_thumb(
    client: &PlexClient,
    path: &str,
    cache_mb: u64,
    max_size_px: [u32; 2],
) -> anyhow::Result<Arc<DynamicImage>> {
    // no private cache dir means no art cache, never a shared one
    let cache_mb = match crate::cache::try_ensure_cache_dir() {
        Ok(_) => cache_mb,
        Err(e) => {
            tracing::warn!("album art cache disabled, cache dir is not private: {e}");
            0
        }
    };
    // None only without a home directory, when the cache is already off.
    load_cached_art(
        &crate::cache::art_path(path).unwrap_or_default(),
        cache_mb,
        max_size_px,
        || client.thumb_bytes(path),
    )
    .await
}

/// Decoded before it is cached, so a bad response never lands there; a cached
/// file that fails to decode is fetched again. `cache_mb == 0` disables it.
async fn load_cached_art<F, Fut>(
    cache_file: &std::path::Path,
    cache_mb: u64,
    max_size_px: [u32; 2],
    fetch: F,
) -> anyhow::Result<Arc<DynamicImage>>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<Vec<u8>>>,
{
    async fn decode(bytes: Vec<u8>, max_size_px: [u32; 2]) -> anyhow::Result<DynamicImage> {
        tokio::task::spawn_blocking(move || decode_image(&bytes, max_size_px)).await?
    }

    if cache_mb > 0 {
        if let Ok(bytes) = tokio::fs::read(cache_file).await {
            match decode(bytes, max_size_px).await {
                Ok(img) => {
                    crate::cache::touch(cache_file); // LRU: keep recently used art
                    return Ok(Arc::new(img));
                }
                Err(e) => {
                    tracing::warn!("cached cover is unusable, fetching it again: {e:#}");
                    let _ = tokio::fs::remove_file(cache_file).await;
                }
            }
        }
    }
    let bytes = fetch().await?;
    if cache_mb == 0 {
        return Ok(Arc::new(decode(bytes, max_size_px).await?));
    }
    let to_cache = bytes.clone();
    let img = decode(bytes, max_size_px).await?;
    if let Some(dir) = cache_file.parent() {
        let dest = cache_file.to_path_buf();
        let dir = dir.to_path_buf();
        let _ = tokio::task::spawn_blocking(move || {
            let written = crate::cache::ensure_private_dir(&dir)
                .and_then(|()| crate::cache::write_private_atomic(&dest, &to_cache));
            if let Err(e) = written {
                tracing::warn!("cannot cache the cover: {e}");
            }
            crate::cache::prune_art_at(&dir, cache_mb);
        })
        .await;
    }
    Ok(Arc::new(img))
}

/// The decoder's allocation ceiling, applied before it allocates: a hostile
/// server must not make shmc reserve gigabytes before `max_size_px` applies.
const MAX_IMAGE_ALLOC: u64 = 64 * 1024 * 1024;

/// The budget follows `album_art.max_size_px` (RGBA at that size), with a floor
/// for the "unlimited" default.
fn decode_limits(max_size_px: [u32; 2]) -> image::Limits {
    let pixels = u64::from(max_size_px[0]) * u64::from(max_size_px[1]);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(pixels.saturating_mul(4).max(MAX_IMAGE_ALLOC));
    limits
}

fn decode_image(bytes: &[u8], max_size_px: [u32; 2]) -> anyhow::Result<DynamicImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .context("guessing image format")?;
    reader.limits(decode_limits(max_size_px));
    reader.decode().context("decoding image")
}

use super::{Confirm, Msg, QueueAction};
use anyhow::Context;

#[cfg(test)]
mod tests {
    use super::*;

    /// An IHDR claiming `w` x `h` RGBA and no pixels (the empty IDAT is there
    /// because the decoder will not start without one).
    fn png_claiming(w: u32, h: u32) -> Vec<u8> {
        fn crc32(bytes: &[u8]) -> u32 {
            let mut crc = !0u32;
            for &b in bytes {
                crc ^= u32::from(b);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
                }
            }
            !crc
        }
        fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
            out.extend((data.len() as u32).to_be_bytes());
            let mut body = kind.to_vec();
            body.extend(data);
            out.extend(&body);
            out.extend(crc32(&body).to_be_bytes());
        }
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend(w.to_be_bytes());
        ihdr.extend(h.to_be_bytes());
        ihdr.extend([8, 6, 0, 0, 0]); // 8 bit, RGBA, deflate, no filter, no interlace
        chunk(&mut png, b"IHDR", &ihdr);
        chunk(&mut png, b"IDAT", &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01]);
        chunk(&mut png, b"IEND", &[]);
        png
    }

    #[test]
    fn art_decoding_bounded_and_works() {
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let decoded = decode_image(&png, [0, 0]).expect("a real cover decodes");
        assert_eq!(decoded.width(), 1);

        // 128 MB: over shmc's ceiling, under the image crate's default
        let bomb = png_claiming(8000, 4000);
        let err = decode_image(&bomb, [0, 0]).expect_err("a decompression bomb is refused");
        assert!(
            matches!(err.downcast_ref::<image::ImageError>(), Some(image::ImageError::Limits(_))),
            "refused by the limits check, not by a malformed file: {err:#}"
        );

        assert_eq!(decode_limits([0, 0]).max_alloc, Some(MAX_IMAGE_ALLOC));
        // a configured cap gets room for RGBA at that size
        assert_eq!(decode_limits([5000, 5000]).max_alloc, Some(5000u64 * 5000 * 4));
    }

    fn tiny_png() -> Vec<u8> {
        let mut out = Vec::new();
        DynamicImage::new_rgb8(4, 4)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[tokio::test]
    async fn corrupt_cached_cover_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let cache_file = dir.path().join("art").join("c.img");
        std::fs::create_dir_all(cache_file.parent().unwrap()).unwrap();
        std::fs::write(&cache_file, b"<html>proxy page</html>").unwrap();

        let good = tiny_png();
        let img = load_cached_art(&cache_file, 10, [0, 0], || {
            let good = good.clone();
            async move { Ok(good) }
        })
        .await
        .expect("fetched again after the cached copy failed to decode");
        assert_eq!(img.width(), 4);
        assert_eq!(std::fs::read(&cache_file).unwrap(), good);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&cache_file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        // a 200 that is not an image never reaches the cache
        let other = dir.path().join("art").join("d.img");
        let res =
            load_cached_art(&other, 10, [0, 0], || async { Ok(b"not an image".to_vec()) }).await;
        assert!(res.is_err());
        assert!(!other.exists());
    }
}

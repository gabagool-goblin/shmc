use super::*;
use shmc::plex::models::{Album, Artist, HitKind, SearchHit};
use shmc::ui::app::BrowserCol;

/// Under a filter the cursor is a position in the filtered view, not a list
/// index: Enter must open the matching artist, not the one at row 0.
#[tokio::test]
async fn enter_on_filtered_artist_opens_it() {
    let (mut app, mut plex_rx) = new_app_with_plex().await;
    app.tab = Tab::Browser;
    app.browser.focus = BrowserCol::Artists;
    app.browser.artists = (0..40)
        .map(|i| Artist {
            rating_key: format!("a{i}"),
            title: format!("Northwind Sounds {i:02}"),
            thumb: None,
        })
        .collect();

    // '/sounds 07' leaves one row: artist a7 at filtered position 0
    app.handle_msg(key('/'));
    for c in "sounds 07".chars() {
        app.handle_msg(key(c));
    }
    app.handle_msg(enter());
    assert_eq!(app.browser_filters[0].text, "sounds 07");
    assert_eq!(
        app.browser.artists_state.selected(),
        Some(0),
        "the cursor is on the filtered row 0"
    );

    app.handle_msg(enter());

    assert_eq!(app.browser.focus, BrowserCol::Albums, "Enter must drill into the albums");
    assert_eq!(
        app.browser.albums_for.as_deref(),
        Some("a7"),
        "the artist that matched is the one that opens"
    );
    assert!(app.browser_filters[0].text.is_empty(), "navigating on clears the filter");
    match plex_rx.try_recv() {
        Ok(PlexCmd::LoadAlbums(k)) => assert_eq!(k, "a7", "the worker is asked for a7's albums"),
        other => panic!("expected LoadAlbums(a7), got {other:?}"),
    }

    // unfiltered Enter still works
    app.browser.focus = BrowserCol::Artists;
    app.browser.artists_state.select(Some(3));
    app.handle_msg(enter());
    assert_eq!(app.browser.albums_for.as_deref(), Some("a3"));
}

/// Right on an artist opens its albums, without refetching ones already on
/// screen, and opens the new artist's after moving.
#[tokio::test]
async fn right_on_an_artist_opens_its_albums() {
    let (mut app, mut plex_rx) = new_app_with_plex().await;
    app.tab = Tab::Browser;
    app.browser.focus = BrowserCol::Artists;
    app.browser.artists = (0..3)
        .map(|i| Artist { rating_key: format!("a{i}"), title: format!("Artist {i}"), thumb: None })
        .collect();
    app.browser.artists_state.select(Some(0));

    app.handle_msg(key_code(KeyCode::Right));
    assert_eq!(app.browser.focus, BrowserCol::Albums);
    assert_eq!(app.browser.albums_for.as_deref(), Some("a0"));
    match plex_rx.try_recv() {
        Ok(PlexCmd::LoadAlbums(k)) => assert_eq!(k, "a0"),
        other => panic!("expected LoadAlbums(a0), got {other:?}"),
    }

    app.handle_msg(key_code(KeyCode::Left));
    assert_eq!(app.browser.focus, BrowserCol::Artists);
    app.handle_msg(key_code(KeyCode::Right));
    assert_eq!(app.browser.focus, BrowserCol::Albums);
    assert!(plex_rx.try_recv().is_err(), "the albums on screen are not fetched again");

    app.handle_msg(key_code(KeyCode::Left));
    app.handle_msg(key_code(KeyCode::Down));
    app.handle_msg(key_code(KeyCode::Right));
    assert_eq!(app.browser.focus, BrowserCol::Albums);
    assert_eq!(app.browser.albums_for.as_deref(), Some("a1"));
    match plex_rx.try_recv() {
        Ok(PlexCmd::LoadAlbums(k)) => assert_eq!(k, "a1"),
        other => panic!("expected LoadAlbums(a1), got {other:?}"),
    }
}

/// Show in Browser jumps to a real artist index, so it must drop the Artists filter.
#[tokio::test]
async fn goto_artist_clears_the_artists_filter() {
    let mut app = new_app().await;
    app.browser.artists = (0..10)
        .map(|i| Artist { rating_key: format!("a{i}"), title: format!("Artist {i}"), thumb: None })
        .collect();
    let titles: Vec<String> = app.browser.artists.iter().map(|a| a.title.clone()).collect();
    app.browser_filters[0].text = "Artist 1".into();
    app.browser_filters[0].apply(titles.len(), |i| titles[i].contains("Artist 1"));
    app.queue = vec![Track { grandparent_rating_key: Some("a5".into()), ..track(0) }];
    app.queue_state.select(Some(0));
    app.tab = Tab::Queue;
    app.handle_msg(key('b'));

    app.handle_msg(Msg::GotoArtist(Artist {
        rating_key: "a5".into(),
        title: "Artist 5".into(),
        thumb: None,
    }));

    assert!(app.browser_filters[0].text.is_empty(), "the filter must not hide the target");
    let sel = app.browser.artists_state.selected().expect("an artist is highlighted");
    assert_eq!(app.browser.artists[sel].rating_key, "a5");
}

/// Highlighting a playlist requests a bounded preview of its tracks.
#[tokio::test]
async fn highlighting_a_playlist_previews_its_tracks() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.playlists.playlists = (0..3).map(playlist).collect();
    app.playlists.state.select(Some(0));
    app.playlists.focus = PlaylistsCol::List;
    app.tab = Tab::Playlists;
    app.preview_debounce_ms = 0;

    app.handle_msg(key('j'));

    let previews: Vec<_> = sent(&mut plex)
        .into_iter()
        .filter_map(|cmd| match cmd {
            PlexCmd::LoadPlaylistPreview { key, size } => Some((key, size)),
            _ => None,
        })
        .collect();
    assert!(previews.iter().all(|(_, size)| *size == 500), "preview must be a bounded page");
    assert_eq!(previews.last().map(|(key, _)| key.as_str()), Some("p1"));
}

#[tokio::test]
async fn cached_playlist_preview_skips_the_request() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.playlists.playlists = (0..3).map(playlist).collect();
    app.playlists.preview_cache.insert("p1".into(), vec![track(0)]);
    app.playlists.state.select(Some(0));
    app.playlists.focus = PlaylistsCol::List;
    app.tab = Tab::Playlists;
    app.preview_debounce_ms = 0;

    app.handle_msg(key('j'));

    assert_eq!(app.playlists.items.len(), 1);
    let requested =
        sent(&mut plex).into_iter().any(|cmd| matches!(cmd, PlexCmd::LoadPlaylistPreview { .. }));
    assert!(!requested, "a cache hit must not fetch again");
}

/// A failed preview is retried on the next highlight, not on every message.
#[tokio::test]
async fn failed_playlist_preview_retried_on_highlight() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.playlists.playlists = (0..3).map(playlist).collect();
    app.playlists.state.select(Some(0));
    app.playlists.focus = PlaylistsCol::List;
    app.tab = Tab::Playlists;
    app.preview_debounce_ms = 0;
    let requests = |plex: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>| {
        sent(plex)
            .into_iter()
            .filter(|cmd| matches!(cmd, PlexCmd::LoadPlaylistPreview { .. }))
            .count()
    };

    app.handle_msg(key('j'));
    assert_eq!(requests(&mut plex), 1);
    app.handle_msg(Msg::PreviewFailed { key: "p1".into() });
    app.handle_msg(Msg::ClearStatus(0));
    assert_eq!(requests(&mut plex), 0, "a failing server is not asked on every message");

    app.playlists.focus = PlaylistsCol::Items;
    app.handle_msg(Msg::ClearStatus(0));
    app.handle_msg(key('h')); // back to the list, p1 highlighted again
    assert_eq!(requests(&mut plex), 1, "the preview is asked for again");
}

/// An unrelated error leaves a pending Browser jump alone; only its own failure ends it.
#[tokio::test]
async fn unrelated_error_keeps_goto_jump() {
    let mut app = new_app().await;
    app.queue = vec![track(0)];
    app.queue_state.select(Some(0));
    app.tab = Tab::Queue;

    app.handle_msg(key('b')); // the track is on album al1 of artist ar1
    assert_eq!(app.tab, Tab::Browser, "the jump has started");

    app.handle_msg(Msg::LoadFailed { key: "p9".into(), error: "loading playlists: boom".into() });
    app.handle_msg(Msg::Error("searching: boom".into()));
    assert_eq!(app.tab, Tab::Browser, "the jump is still going");
    assert!(app.status.contains("searching"), "the error shows itself: {:?}", app.status);

    app.handle_msg(Msg::LoadFailed { key: "ar1".into(), error: "resolving artist: boom".into() });
    assert_eq!(app.tab, Tab::Queue, "the jump's own failure returns to the queue");
    assert!(app.status.contains("not found"), "{:?}", app.status);
}

fn artist_ar1() -> Artist {
    Artist { rating_key: "ar1".into(), title: "Northwind Sounds".into(), thumb: None }
}

/// Leaving the tab before Plex answers drops the jump; a late reply must not
/// pull the user back.
#[tokio::test]
async fn late_reply_ignored_after_cancelled_jump() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.queue = vec![track(0)];
    app.queue_state.select(Some(0));
    app.tab = Tab::Queue;
    app.handle_msg(key('b'));
    app.handle_msg(key('3'));
    sent(&mut plex);

    app.handle_msg(Msg::GotoArtist(artist_ar1()));

    assert_eq!(app.tab, Tab::Playlists);
    assert!(app.browser.artists.is_empty());
    assert!(sent(&mut plex).is_empty(), "nothing is loaded for a dropped jump");
}

/// Moving in the Browser while the jump loads also drops it, `g` jumps included.
#[tokio::test]
async fn browser_move_drops_pending_jump() {
    for keys in ["j", "gg", "ga"] {
        let (mut app, mut plex) = new_app_with_plex().await;
        app.queue = vec![track(0)];
        app.queue_state.select(Some(0));
        app.tab = Tab::Queue;
        app.handle_msg(key('g'));
        app.handle_msg(Msg::GotoArtist(artist_ar1()));
        for c in keys.chars() {
            app.handle_msg(key(c));
        }
        sent(&mut plex);

        app.handle_msg(Msg::AlbumsLoaded {
            artist_key: "ar1".into(),
            albums: vec![album_by("al1", "Evening Satellite", "Northwind Sounds", None)],
        });

        assert_eq!(app.browser.focus, BrowserCol::Artists, "after {keys}");
        assert!(sent(&mut plex).is_empty(), "no tracks are fetched for a dropped jump ({keys})");
    }
}

fn hit(kind: HitKind, key: &str, album: Option<&str>, artist: Option<&str>) -> SearchHit {
    SearchHit {
        kind,
        rating_key: key.into(),
        title: "Hit".into(),
        artist: "Artist".into(),
        subtitle: String::new(),
        duration_ms: None,
        album_key: album.map(Into::into),
        artist_key: artist.map(Into::into),
    }
}

fn reveal_hit(
    app: &mut TestApp,
    plex: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>,
    h: SearchHit,
) -> Vec<PlexCmd> {
    app.search.hits = vec![h];
    app.search.state.select(Some(0));
    app.tab = Tab::Search;
    app.handle_msg(key('b'));
    sent(plex)
}

fn browser_target(app: &TestApp) -> (String, Option<String>, Option<String>) {
    let artist =
        app.browser.artists[app.browser.artists_state.selected().unwrap()].rating_key.clone();
    let album =
        app.browser.albums_state.selected().map(|i| app.browser.albums[i].rating_key.clone());
    let track =
        app.browser.tracks_state.selected().map(|i| app.browser.tracks[i].rating_key.clone());
    (artist, album, track)
}

/// `b` on a result lands on the artist, the album or the track.
#[tokio::test]
async fn search_reveal_lands_on_each_kind() {
    let hits = [
        hit(HitKind::Artist, "ar1", None, None),
        hit(HitKind::Album, "al1", None, Some("ar1")),
        hit(HitKind::Track, "t1", Some("al1"), Some("ar1")),
    ];
    for h in hits {
        let kind = h.kind;
        let (mut app, mut plex) = new_app_with_plex().await;
        let cmds = reveal_hit(&mut app, &mut plex, h);
        assert!(matches!(cmds.as_slice(), [PlexCmd::ResolveArtist(k)] if k == "ar1"), "{kind:?}");
        assert_eq!(app.tab, Tab::Browser);

        app.handle_msg(Msg::GotoArtist(artist_ar1()));
        app.handle_msg(Msg::AlbumsLoaded {
            artist_key: "ar1".into(),
            albums: vec![
                album_by("al0", "Other", "Northwind Sounds", None),
                album_by("al1", "Evening Satellite", "Northwind Sounds", None),
            ],
        });
        if kind == HitKind::Artist {
            assert_eq!(app.browser.focus, BrowserCol::Artists);
            assert_eq!(browser_target(&app).0, "ar1");
            continue;
        }
        assert_eq!(app.browser.focus, BrowserCol::Tracks, "{kind:?}");
        app.handle_msg(Msg::TracksLoaded {
            album_key: "al1".into(),
            tracks: vec![track(0), Track { rating_key: "t1".into(), ..track(1) }],
        });
        let (artist, album, track) = browser_target(&app);
        assert_eq!((artist.as_str(), album.as_deref()), ("ar1", Some("al1")));
        let want = if kind == HitKind::Track { "t1" } else { "t0" };
        assert_eq!(track.as_deref(), Some(want), "{kind:?}");
    }
}

/// A failed reveal returns to Search; a hit without keys never asks Plex.
#[tokio::test]
async fn failed_reveal_returns_to_search() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let cmds = reveal_hit(&mut app, &mut plex, hit(HitKind::Track, "t1", Some("al1"), Some("ar1")));
    assert_eq!(cmds.len(), 1);
    app.handle_msg(Msg::LoadFailed { key: "ar1".into(), error: "boom".into() });
    assert_eq!(app.tab, Tab::Search);
    assert!(app.status.contains("not found"), "{:?}", app.status);

    let cmds = reveal_hit(&mut app, &mut plex, hit(HitKind::Track, "t1", None, None));
    assert!(cmds.is_empty());
    assert_eq!(app.tab, Tab::Search);
}

/// A reveal from Search obeys the same cancellation as the Queue jump.
#[tokio::test]
async fn late_reply_ignored_after_cancelled_reveal() {
    let (mut app, mut plex) = new_app_with_plex().await;
    reveal_hit(&mut app, &mut plex, hit(HitKind::Artist, "ar1", None, None));
    app.handle_msg(key('1'));

    app.handle_msg(Msg::GotoArtist(artist_ar1()));

    assert_eq!(app.tab, Tab::Queue);
    assert!(app.browser.artists.is_empty());
    assert!(sent(&mut plex).is_empty());
}

/// In the album view `b` reveals the selected track.
#[tokio::test]
async fn search_album_view_reveals_the_selected_track() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let mut state = ratatui::widgets::TableState::default();
    state.select(Some(1));
    app.search.detail = Some(shmc::ui::app::SearchDetail {
        key: "al1".into(),
        label: "Album".into(),
        tracks: vec![track(0), track(1)],
        state,
        matches: None,
    });
    app.tab = Tab::Search;
    app.handle_msg(key('b'));
    assert!(matches!(sent(&mut plex).as_slice(), [PlexCmd::ResolveArtist(k)] if k == "ar1"));
    assert_eq!(app.tab, Tab::Browser);
}

/// In the similar list `b` opens the selected track's own album, not the seed's key.
#[tokio::test]
async fn similar_list_reveals_the_tracks_album() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let mut state = ratatui::widgets::TableState::default();
    state.select(Some(1));
    app.search.detail = Some(shmc::ui::app::SearchDetail {
        key: "t2".into(),
        label: "Sonically similar to Starfall! 2 — Northwind Sounds".into(),
        tracks: vec![track(10), track(11)],
        state,
        matches: Some(vec![Some(96), Some(95)]),
    });
    app.tab = Tab::Search;
    app.handle_msg(key('b'));
    assert!(matches!(sent(&mut plex).as_slice(), [PlexCmd::ResolveArtist(k)] if k == "ar1"));
    app.handle_msg(Msg::GotoArtist(artist_ar1()));
    app.handle_msg(Msg::AlbumsLoaded {
        artist_key: "ar1".into(),
        albums: vec![album_by("al1", "Evening Satellite", "Northwind Sounds", None)],
    });
    assert_eq!(app.tab, Tab::Browser);
    assert_eq!(app.browser.focus, BrowserCol::Tracks, "{}", app.status);
}

#[tokio::test]
async fn large_playlist_add_asks_for_confirmation() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let mut large = playlist(0);
    large.item_count = Some(5000);
    app.playlists.playlists = vec![large];
    app.playlists.state.select(Some(0));
    app.playlists.focus = PlaylistsCol::List;
    app.tab = Tab::Playlists;
    app.confirm_playlist_over = 1000;

    app.handle_msg(enter());
    assert!(resolve_seqs(&mut plex).is_empty(), "a large playlist must ask before queueing");

    app.handle_msg(key('y'));
    let resolved = last_sent(&mut plex, |cmd| match cmd {
        PlexCmd::ResolvePlaylist { key, action, .. } => Some((key, action)),
        _ => None,
    });
    assert_eq!(
        resolved,
        Some(("p0".to_string(), QueueAction::Replace)),
        "Enter on a large playlist replaces the queue"
    );
}

#[tokio::test]
async fn enter_on_a_playlist_replaces_the_queue() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.playlists.playlists = (0..3).map(playlist).collect();
    app.playlists.state.select(Some(1));
    app.playlists.focus = PlaylistsCol::List;
    app.tab = Tab::Playlists;

    app.handle_msg(enter());

    let resolved = last_sent(&mut plex, |cmd| match cmd {
        PlexCmd::ResolvePlaylist { key, action, .. } => Some((key, action)),
        _ => None,
    });
    assert_eq!(resolved, Some(("p1".to_string(), QueueAction::Replace)));
    assert_eq!(
        app.playlists.focus,
        PlaylistsCol::List,
        "Enter must not drill into the playlist any more"
    );
}

/// `a` on an artist queues the artist's whole catalog.
#[tokio::test]
async fn add_on_artist_queues_its_tracks() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.browser.artists =
        vec![Artist { rating_key: "ar1".into(), title: "Artist".into(), thumb: None }];
    app.browser.artists_state.select(Some(0));
    app.browser.focus = BrowserCol::Artists;
    app.tab = Tab::Browser;

    app.handle_msg(key('a'));

    let request = last_sent(&mut plex, |cmd| match cmd {
        PlexCmd::ResolveArtistTracks { key, action, .. } => Some((key, action)),
        _ => None,
    });
    assert_eq!(request, Some(("ar1".to_string(), QueueAction::Append)));
}

/// A capped preview is announced in the status, fresh or from the cache (a
/// revisit fires no arrival); a complete one stays quiet.
#[tokio::test]
async fn capped_preview_is_announced() {
    let mut app = new_app().await;
    app.tab = Tab::Playlists;
    app.playlists.playlists = vec![listed_playlist("p0", "Big Mix", 20_000)];
    app.playlists.items_for = Some("p0".into());
    app.playlists.items_total = Some(20_000);
    app.handle_msg(Msg::PlaylistItemsLoaded {
        rating_key: "p0".into(),
        tracks: (0..100).map(track).collect(),
    });
    assert!(app.status.contains("20000"), "{}", app.status);

    drop(app); // one app at a time: it holds the suite's lock

    // from the cache
    let mut app = new_app().await;
    app.tab = Tab::Playlists;
    app.playlists.playlists = vec![listed_playlist("p0", "Big Mix", 20_000)];
    app.playlists.focus = PlaylistsCol::List;
    app.playlists.state.select(Some(0));
    app.playlists.preview_cache.insert("p0".into(), (0..100).map(track).collect());
    app.handle_msg(key('j'));
    assert!(app.status.contains("20000"), "{}", app.status);

    drop(app);

    let mut app = new_app().await;
    app.tab = Tab::Playlists;
    app.playlists.playlists = vec![playlist(0)];
    app.playlists.items_for = Some("p0".into());
    app.playlists.items_total = Some(12);
    app.handle_msg(Msg::PlaylistItemsLoaded {
        rating_key: "p0".into(),
        tracks: (0..12).map(track).collect(),
    });
    assert!(app.status.is_empty(), "a complete preview needs no notice");
}

#[tokio::test]
async fn browser_track_enter_replaces_with_album_remainder() {
    let mut app = new_app().await;
    app.queue = vec![track(9)];
    app.current = Some(0);
    app.stopped = false;
    app.browser.tracks = (0..5).map(track).collect();
    app.browser.artists_state.select(Some(0));
    app.browser.tracks_state.select(Some(2));
    app.browser.focus = BrowserCol::Tracks;
    app.tab = Tab::Browser;

    app.handle_msg(enter());

    assert_eq!(
        app.queue.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
        vec!["t2", "t3", "t4"]
    );
    assert_eq!(app.current, Some(0));
    assert!(!app.stopped);
}

#[tokio::test]
async fn browser_album_enter_replaces_queue_with_album() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.browser.albums = vec![Album {
        rating_key: "al1".into(),
        title: "Album".into(),
        artist: "Artist".into(),
        year: Some(2024),
        thumb: None,
    }];
    app.browser.albums_state.select(Some(0));
    app.browser.focus = BrowserCol::Albums;
    app.tab = Tab::Browser;

    app.handle_msg(enter());

    let request = last_sent(&mut plex, |cmd| match cmd {
        PlexCmd::ResolveAlbum { key, action, .. } => Some((key, action)),
        _ => None,
    });
    assert_eq!(request, Some(("al1".to_string(), QueueAction::Replace)));
}

#[tokio::test]
async fn playlist_track_enter_replaces_from_that_track() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.playlists.playlists = vec![playlist(0)];
    app.playlists.items_for = Some("p0".into());
    app.playlists.items = (0..4).map(track).collect();
    app.playlists.items_state.select(Some(1));
    app.playlists.focus = PlaylistsCol::Items;
    app.tab = Tab::Playlists;

    app.handle_msg(enter());

    let request = last_sent(&mut plex, |cmd| match cmd {
        PlexCmd::ResolvePlaylist { key, action, start, .. } => Some((key, action, start)),
        _ => None,
    });
    assert_eq!(request, Some(("p0".to_string(), QueueAction::Replace, Some("t1".to_string()))));
}

/// A track row asks before replacing or appending a huge playlist, counting
/// the remainder from that track, and keeps the start track once confirmed.
#[tokio::test]
async fn large_playlist_track_asks_remainder_or_start() {
    let cases: [(fn() -> Msg, QueueAction); 2] =
        [(enter, QueueAction::Replace), (|| key('a'), QueueAction::Append)];
    for (press, want) in cases {
        let (mut app, mut plex) = new_app_with_plex().await;
        let mut large = playlist(0);
        large.item_count = Some(5000);
        app.playlists.playlists = vec![large];
        app.playlists.items_for = Some("p0".into());
        app.playlists.items = (0..4).map(track).collect();
        app.playlists.items_state.select(Some(1));
        app.playlists.focus = PlaylistsCol::Items;
        app.tab = Tab::Playlists;
        app.confirm_playlist_over = 1000;

        app.handle_msg(press());

        assert!(resolve_seqs(&mut plex).is_empty(), "{want:?}: a large playlist must ask first");
        assert!(
            app.status.contains("4999"),
            "{want:?}: the prompt must count the remainder: {:?}",
            app.status
        );

        app.handle_msg(key('y'));
        let request = last_sent(&mut plex, |cmd| match cmd {
            PlexCmd::ResolvePlaylist { key, action, start, .. } => Some((key, action, start)),
            _ => None,
        });
        assert_eq!(
            request,
            Some(("p0".to_string(), want, Some("t1".to_string()))),
            "y must keep the start track, not use the whole playlist"
        );
    }
}

/// `i` inserts one track, so nothing to confirm.
#[tokio::test]
async fn play_next_large_playlist_inserts_one_track() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let mut large = playlist(0);
    large.item_count = Some(5000);
    app.playlists.playlists = vec![large];
    app.playlists.items_for = Some("p0".into());
    app.playlists.items = (0..4).map(track).collect();
    app.playlists.items_state.select(Some(1));
    app.playlists.focus = PlaylistsCol::Items;
    app.tab = Tab::Playlists;
    app.confirm_playlist_over = 1000;
    app.queue = vec![track(0)];
    app.current = Some(0);

    app.handle_msg(key('i'));

    assert!(resolve_seqs(&mut plex).is_empty(), "play-next never queues a whole playlist");
    assert_eq!(app.queue.len(), 2);
    assert_eq!(app.queue[1].rating_key, "t1");
}

/// A playlist start that has left the playlist is reported as an error that says so.
#[tokio::test]
async fn vanished_playlist_start_reported() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.playlists.playlists = vec![playlist(0)];
    app.playlists.state.select(Some(0));
    app.playlists.focus = PlaylistsCol::List;
    app.tab = Tab::Playlists;
    app.handle_msg(enter());
    let seq = resolve_seqs(&mut plex)[0];
    app.handle_msg(reply(seq, QueueOutcome::StartMissing));

    assert!(
        app.status.contains("no longer in the playlist"),
        "the refusal must say what happened: {:?}",
        app.status
    );
    assert!(app.status_error, "it is shown as an error");
}

#[tokio::test]
async fn search_track_enter_replaces_with_album_remainder() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.search.hits = vec![SearchHit {
        kind: HitKind::Track,
        rating_key: "t9".into(),
        title: "Track".into(),
        artist: "Artist".into(),
        subtitle: "Album".into(),
        duration_ms: Some(1000),
        album_key: None,
        artist_key: None,
    }];
    app.search.state.select(Some(0));
    app.tab = Tab::Search;

    app.handle_msg(enter());

    let request = last_sent(&mut plex, |cmd| match cmd {
        PlexCmd::ResolveTrack { key, action, rest_of_album, .. } => {
            Some((key, action, rest_of_album))
        }
        _ => None,
    });
    assert_eq!(request, Some(("t9".to_string(), QueueAction::Replace, true)));
}

#[tokio::test]
async fn browser_album_right_drills_into_tracks() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.browser.albums = vec![Album {
        rating_key: "al1".into(),
        title: "Album".into(),
        artist: "Artist".into(),
        year: Some(2024),
        thumb: None,
    }];
    app.browser.albums_state.select(Some(0));
    app.browser.focus = BrowserCol::Albums;
    app.tab = Tab::Browser;

    app.handle_msg(key_code(KeyCode::Right));

    assert_eq!(app.browser.focus, BrowserCol::Tracks);
    let request = last_sent(&mut plex, |cmd| match cmd {
        PlexCmd::LoadTracks(key) => Some(key),
        _ => None,
    });
    assert_eq!(request.as_deref(), Some("al1"));
}

/// Missing years sort last in both year orders; ties go by title; an unknown mode is `year`.
#[test]
fn albums_sort_by_mode_with_missing_years() {
    use shmc::ui::app::sort_albums;
    let set = || {
        vec![
            album_by("1", "beta", "A", Some(2010)),
            album_by("2", "Zed", "A", None),
            album_by("3", "Alpha", "A", Some(2010)),
            album_by("4", "gamma", "A", Some(1999)),
            album_by("5", "alpha", "A", None),
            album_by("6", "Alpha", "A", Some(1990)),
        ]
    };
    let keys = |mode: &str| {
        let mut v = set();
        sort_albums(&mut v, mode);
        v.iter().map(|a| a.rating_key.clone()).collect::<Vec<_>>().join("")
    };
    assert_eq!(keys("year"), "643152");
    assert_eq!(keys("year_desc"), "314652");
    assert_eq!(keys("title"), "635142");
    assert_eq!(keys("Year_Desc"), "314652");
    assert_eq!(keys("sideways"), keys("year"));
}

/// Albums are ordered by `ui.album_sort`, not the server's order.
#[tokio::test]
async fn albums_loaded_applies_the_configured_order() {
    let mut app = new_app().await;
    app.album_sort = "year_desc".into();
    app.browser.albums_for = Some("a1".into());
    app.handle_msg(Msg::AlbumsLoaded {
        artist_key: "a1".into(),
        albums: vec![
            album_by("old", "Old", "A", Some(1990)),
            album_by("none", "None", "A", None),
            album_by("new", "New", "A", Some(2020)),
        ],
    });
    let keys: Vec<&str> = app.browser.albums.iter().map(|a| a.rating_key.as_str()).collect();
    assert_eq!(keys, ["new", "old", "none"]);
}

async fn browser_with_albums(albums: Vec<Album>) -> TestApp {
    let mut app = new_app().await;
    app.tab = Tab::Browser;
    app.browser.artists =
        vec![Artist { rating_key: "a0".into(), title: "Browsed Artist".into(), thumb: None }];
    app.browser.artists_state.select(Some(0));
    app.browser.albums_for = Some("a0".into());
    app.browser.albums = albums;
    app.browser.albums_state.select(Some(0));
    app.browser.focus = BrowserCol::Albums;
    app
}

fn album_by(key: &str, title: &str, artist: &str, year: Option<u32>) -> Album {
    Album { rating_key: key.into(), title: title.into(), artist: artist.into(), year, thumb: None }
}

/// The rendered line holding `needle`.
fn row_with(screen: &str, needle: &str) -> String {
    screen
        .lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no row with {needle:?}\n{screen}"))
        .to_string()
}

/// A row is `Title (year)`; the artist follows only when it differs from the
/// browsed one; no year, no parentheses.
#[tokio::test]
async fn album_rows_show_year_and_differing_artist() {
    let mut app = browser_with_albums(vec![
        album_by("1", "Mine", "Browsed Artist", Some(2019)),
        album_by("2", "Compilation", "Other Artist", Some(2020)),
        album_by("3", "Undated", "Browsed Artist", None),
    ])
    .await;
    assert!(app.browser.tracks_for.is_none());
    for w in [100u16, 140, 200] {
        let s = render(&mut app, w, 30);
        let mine = row_with(&s, "Mine");
        assert!(mine.contains("Mine (2019)"), "{mine}");
        assert!(!mine.contains("Mine (2019) -"), "{mine}");
        assert!(row_with(&s, "Compilation").contains("Compilation (2020) - Other Artist"));
        let undated = row_with(&s, "Undated");
        assert!(!undated.contains("Undated ("), "{undated}");
        assert!(!undated.contains("Undated -"), "{undated}");
    }
}

/// In a narrow pane the artist suffix goes first and the year stays, even with wide characters.
#[tokio::test]
async fn narrow_album_rows_drop_artist_first() {
    let mut app = browser_with_albums(vec![
        album_by("1", "Compilation Of Things", "Other Artist", Some(2020)),
        album_by("2", "日本語日本語日本語日本語日本語日本語", "Browsed Artist", Some(2018)),
    ])
    .await;
    for w in 50u16..=120 {
        let s = render(&mut app, w, 30);
        let comp = s.lines().find(|l| l.contains("Compilation") || l.contains("(2020)"));
        let comp = comp.unwrap_or_else(|| panic!("w={w}\n{s}"));
        assert!(comp.contains("(2020)"), "w={w}: year lost\n{comp}");
        if !comp.contains("Other") {
            assert!(!comp.contains(" - "), "w={w}: {comp}");
        }
        let cjk = s.lines().find(|l| l.contains("(2018)"));
        assert!(cjk.is_some(), "w={w}: wide title pushed the year out\n{s}");
    }
    // the suffix really is dropped somewhere in the range
    let s = render(&mut app, 60, 30);
    let comp = row_with(&s, "(2020)");
    assert!(!comp.contains("Other"), "{comp}");
}

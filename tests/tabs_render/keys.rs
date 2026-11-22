use super::*;
use shmc::plex::models::Artist;
use shmc::plex::models::{HitKind, SearchHit};
use shmc::ui::app::{BrowserCol, Pane, SearchDetail};

fn key_table(text: &str) -> KeyTable {
    toml::from_str::<shmc::config::Config>(text).unwrap().keys
}

/// Up on the first row jumps to the last, and down on the last to the first.
#[tokio::test]
async fn arrow_keys_wrap_at_list_ends() {
    let mut app = new_app().await;
    app.queue.items = (0..6).map(track).collect();
    app.queue.state.select(Some(0));
    app.tab = Tab::Queue;

    app.handle_msg(key_code(KeyCode::Up));
    assert_eq!(app.queue.state.selected(), Some(5), "up from the top wraps to the bottom");

    app.handle_msg(key_code(KeyCode::Down));
    assert_eq!(app.queue.state.selected(), Some(0), "down from the bottom wraps to the top");

    app.queue.state.select(Some(3));
    app.handle_msg(key('j'));
    assert_eq!(app.queue.state.selected(), Some(4), "j moves down as usual");
    app.queue.state.select(Some(0));
    app.handle_msg(key('k'));
    assert_eq!(app.queue.state.selected(), Some(5), "k wraps too");
}

/// The next key press drops the status, but not one that press sets itself.
#[tokio::test]
async fn status_clears_on_next_key() {
    let mut app = new_app().await;
    app.queue.items = (0..10).map(track).collect();
    app.queue.state.select(Some(0));

    // disarm queue follow first; that has its own status
    app.handle_msg(key('j'));
    app.status = "Added: Starfall! 4".into();
    app.handle_msg(key('j')); // plain navigation
    assert!(app.status.is_empty(), "navigation must dismiss the status");

    app.status = "Added: Starfall! 4".into();
    app.handle_msg(key('D')); // an action that posts a message of its own
    assert!(app.status.contains("clear the queue"), "the new message must survive: {}", app.status);
}

fn ctrl(c: char) -> Msg {
    Msg::Key(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)))
}

/// Ctrl-C quits even while typing, and a Ctrl-letter is never text.
#[tokio::test]
async fn ctrl_c_quits_while_typing() {
    for mode in ["search", "filter", "command"] {
        let mut app = new_app().await;
        app.queue.items = (0..3).map(track).collect();
        match mode {
            "search" => {
                app.tab = Tab::Search;
                app.search.focused = true;
            }
            "filter" => app.handle_msg(key('/')),
            _ => app.handle_msg(key(':')),
        }
        app.handle_msg(key('a'));
        app.handle_msg(ctrl('x'));
        let typed = match mode {
            "search" => app.search.input.clone(),
            "filter" => app.queue.filter.text.clone(),
            _ => app.command.text.clone(),
        };
        assert_eq!(typed, "a", "{mode}: Ctrl-X must not insert text");
        assert!(!app.should_quit, "{mode}");
        app.handle_msg(ctrl('c'));
        assert!(app.should_quit, "{mode}: Ctrl-C must quit");
    }
}

/// `nan` and `inf` parse as floats; volume must never become NaN.
#[tokio::test]
async fn vol_and_seek_reject_non_finite_numbers() {
    let mut app = new_app().await;
    for cmd in ["vol nan", "vol inf", "vol +inf", "seek nan", "seek -inf"] {
        run_cmd(&mut app, cmd);
        assert!(app.volume.is_finite(), "{cmd}");
        assert_eq!(app.volume, 63.0, "{cmd}");
        assert!(app.status_error && app.status.contains("usage"), "{cmd}: {}", app.status);
    }
}

/// Error text can carry a stream URL; the status must not show its token.
#[tokio::test]
async fn status_never_shows_a_token() {
    let mut app = new_app().await;
    app.handle_msg(Msg::Error("GET http://h:32400/p?X-Plex-Token=SECRETTOKEN&x=1 failed".into()));
    assert!(!app.status.contains("SECRETTOKEN"), "{}", app.status);
    assert!(app.status.contains("failed"));
}

/// The prompt draws its own colon, so `:pause` must run without it in the buffer.
#[tokio::test]
async fn colon_commands_accept_the_prompt_prefix() {
    let mut app = new_app().await;
    app.stopped = false;
    app.volume = 10.0;

    run_cmd(&mut app, "pause");
    assert!(!app.command.typing);
    assert!(
        !app.status.contains("unknown command"),
        "the prompt colon is not part of the buffer: {}",
        app.status
    );

    run_cmd(&mut app, "vol 42");
    assert_eq!(app.volume, 42.0);
    assert!(app.status.contains("volume: 42%"));

    run_cmd(&mut app, "q");
    assert!(app.should_quit);
}

#[tokio::test]
async fn colon_commands_cover_playback_modes_and_tabs() {
    let mut app = new_app().await;
    app.queue.items = vec![track(0), track(1)];
    app.current = Some(0);
    app.stopped = false;

    run_cmd(&mut app, "stop");
    assert!(app.stopped, ":stop must keep the queue but stop playback");
    assert_eq!(app.queue.items.len(), 2);

    run_cmd(&mut app, "play");
    assert!(!app.stopped, ":play must restart the current track");

    app.volume = 40.0;
    run_cmd(&mut app, "vol +5");
    assert_eq!(app.volume, 45.0, ":vol +N is relative");

    run_cmd(&mut app, "seek -10");
    assert!(app.status.contains("seek -10s"));

    run_cmd(&mut app, "shuffle");
    assert!(app.random);
    run_cmd(&mut app, "repeat");
    assert!(app.loop_playlist);
    run_cmd(&mut app, "single");
    assert!(app.loop_file);
    run_cmd(&mut app, "consume");
    assert!(app.consume);

    run_cmd(&mut app, "reshuffle");
    assert!(app.random, "reshuffle leaves shuffle on");
    assert!(app.status.contains("reshuffled"));

    app.tab = Tab::Queue;
    app.current = Some(1);
    app.queue.state.select(Some(0));
    run_cmd(&mut app, "playing");
    assert_eq!(app.queue.state.selected(), Some(1));

    run_cmd(&mut app, "browser");
    assert_eq!(app.tab, Tab::Browser);
    run_cmd(&mut app, "playlists");
    assert_eq!(app.tab, Tab::Playlists);
    run_cmd(&mut app, "queue");
    assert_eq!(app.tab, Tab::Queue);
    run_cmd(&mut app, "search");
    assert_eq!(app.tab, Tab::Search);
    assert!(app.search.focused);
    app.search.focused = false;
    run_cmd(&mut app, "help");
    assert_eq!(app.tab, Tab::Help);
}

/// A key press after the prompt timed out belongs to the list, not the old question.
#[tokio::test]
async fn key_after_expired_prompt_not_swallowed() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let mut large = playlist(0);
    large.item_count = Some(5000);
    app.playlists.list.items = vec![large];
    app.playlists.tracks_for = Some("p0".into());
    app.playlists.tracks.items = (0..4).map(track).collect();
    app.playlists.tracks.state.select(Some(0));
    app.playlists.focus = PlaylistsCol::Items;
    app.tab = Tab::Playlists;
    app.confirm_playlist_over = 1000;
    // times out at once, so the next key press is late
    app.confirm_timeout_secs = 0;

    app.handle_msg(enter());
    assert!(app.status.contains("tracks?"), "the prompt must be showing: {:?}", app.status);

    app.handle_msg(key('j'));

    assert_eq!(
        app.playlists.tracks.state.selected(),
        Some(1),
        "the key press must move the cursor, not cancel the question"
    );
    assert!(
        !app.status.contains("cancelled"),
        "the question must not swallow the key: {:?}",
        app.status
    );
    assert!(resolve_seqs(&mut plex).is_empty(), "an expired question must not queue anything");
}

/// `g <letter>` in Help has nothing to jump to and reports no miss.
#[tokio::test]
async fn g_letter_in_help_says_nothing() {
    let mut app = new_app().await;
    app.tab = Tab::Help;

    app.handle_msg(key('g'));
    app.handle_msg(key('a'));

    assert_eq!(app.tab, Tab::Help);
    assert!(app.status.is_empty(), "Help has no titles to jump to: {:?}", app.status);
}

#[tokio::test]
async fn huge_sleep_timer_is_clamped() {
    let mut app = new_app().await;
    run_cmd(&mut app, "sleep 4294967295");

    assert_eq!(
        app.sleep_timer_label().as_deref(),
        Some("sleep 1440m"),
        "an absurd duration is clamped to a day"
    );
    assert!(app.status.contains("clamped"), "the clamp must be visible: {:?}", app.status);
}

#[tokio::test]
async fn hints_name_rebound_keys() {
    let keys = key_table("[keys.queue]\nremove = [\"X\"]\n[keys.playlists]\nactivate = [\"Z\"]\n");
    let mut app = new_app_with_keys(&keys).await;
    app.tab = Tab::Queue;
    app.queue.items = (0..3).map(track).collect();
    app.queue.state.select(Some(0));
    app.handle_msg(key('v'));
    assert!(app.status.contains("X remove"), "{:?}", app.status);
    assert!(!app.status.contains("d remove"), "{:?}", app.status);

    app.visual = None;
    app.tab = Tab::Playlists;
    app.playlists.focus = PlaylistsCol::List;
    app.playlists.list.items =
        vec![listed_playlist("p0", "A", 5000), listed_playlist("p1", "B", 5000)];
    app.playlists.list.state.select(Some(0));
    let frame = render(&mut app, 120, 30);
    assert!(frame.contains("Z adds it"), "{frame}");
    app.playlists.preview_cache.insert("p1".into(), (0..3).map(track).collect());
    app.handle_msg(key('j'));
    assert!(app.status.contains("preview:"), "{:?}", app.status);
    assert!(app.status.contains("Z adds all"), "{:?}", app.status);
}

/// Newlines in a title or error are escaped; the status is one line.
#[tokio::test]
async fn status_message_stays_on_one_line() {
    let mut app = new_app().await;

    app.transient("Night\nMix".into());
    assert_eq!(app.status, "Night\\nMix");

    app.transient_err("bad\r\nthing".into());
    assert_eq!(app.status, "bad\\r\\nthing");
    assert!(app.status_error);
}

#[tokio::test]
async fn rebound_key_replaces_old_binding() {
    let keys = key_table("[keys.queue]\nremove = [\"Delete\"]\n");
    let mut app = new_app_with_keys(&keys).await;
    app.tab = Tab::Queue;
    app.queue.items = (0..4).map(track).collect();
    app.queue.state.select(Some(1));

    app.handle_msg(key('d'));
    assert_eq!(app.queue.items.len(), 4, "d is not remove any more");

    app.handle_msg(key_code(KeyCode::Delete));
    assert_eq!(
        app.queue.items.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
        vec!["t0", "t2", "t3"]
    );
}

#[tokio::test]
async fn ctrl_c_quits_when_quit_unbound() {
    let keys = key_table("[keys.global]\nquit = []\n");
    let mut app = new_app_with_keys(&keys).await;

    app.handle_msg(Msg::Key(Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))));

    assert!(app.should_quit, "Ctrl-C is reserved for quitting");
}

#[tokio::test]
async fn clear_confirm_accepts_rebound_key() {
    let keys = key_table("[keys.queue]\nclear = [\"X\"]\n");
    let mut app = new_app_with_keys(&keys).await;
    app.queue.items = (0..3).map(track).collect();
    app.current = Some(0);
    app.stopped = false;

    app.handle_msg(key('D'));
    assert_eq!(app.queue.items.len(), 3, "D is not clear any more");

    app.handle_msg(key('X'));
    assert_eq!(app.queue.items.len(), 3, "the first press only asks");
    assert!(app.status.contains('X'), "the question names the key: {}", app.status);

    app.handle_msg(key('X'));
    assert!(app.queue.items.is_empty(), "the rebound key confirms too");
}

#[tokio::test]
async fn gg_runs_what_it_is_bound_to() {
    let keys = key_table("[keys.browser]\ntop = [\"Home\"]\nbottom = [\"gg\"]\n");
    let mut app = new_app_with_keys(&keys).await;
    app.tab = Tab::Browser;
    app.browser.artists.items = (0..10)
        .map(|i| Artist {
            rating_key: format!("a{i}"),
            title: format!("Artist {i:02}"),
            thumb: None,
        })
        .collect();
    app.browser.artists.state.select(Some(0));

    app.handle_msg(key('g'));
    app.handle_msg(key('g'));

    assert_eq!(app.browser.artists.state.selected(), Some(9));
}

#[tokio::test]
async fn paste_into_filter_inserts_and_refilters() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..12).map(track).collect();
    app.queue.state.select(Some(0));
    app.queue.filter.typing = true;

    app.handle_msg(Msg::Paste("Starfall! 9".into()));

    assert_eq!(app.queue.filter.text, "Starfall! 9");
    let (text, _, shown, total) = app.filter_typing().expect("still typing");
    assert_eq!(text, "Starfall! 9");
    assert_eq!((shown, total), (1, 12), "the paste must re-apply the filter");
}

#[tokio::test]
async fn paste_with_no_focus_changes_nothing() {
    for tab in [Tab::Browser, Tab::Queue] {
        let mut app = new_app().await;
        app.tab = tab;
        app.queue.items = (0..3).map(track).collect();
        app.browser.artists.filter.text = "Northwind".into();
        app.search.input = "kept".into();

        // `/` and `\r` in the paste must not start a filter or a search
        app.handle_msg(Msg::Paste("/night\r".into()));

        assert_eq!(app.browser.artists.filter.text, "Northwind", "{tab:?}");
        assert!(!app.browser.artists.filter.typing, "{tab:?}");
        assert!(app.queue.filter.text.is_empty(), "{tab:?}: no filter was started");
        assert!(!app.queue.filter.typing, "{tab:?}");
        assert_eq!(app.search.input, "kept", "{tab:?}");
        assert!(app.search.last_query.is_empty(), "{tab:?}");
        assert!(!app.search.focused, "{tab:?}: a paste must not steal focus");
        assert!(!app.command.typing, "{tab:?}");
        assert!(!app.status.contains("searching"), "{tab:?}: {}", app.status);
        assert_eq!(app.tab, tab);
    }
}

#[tokio::test]
async fn pasted_control_characters_are_text_not_actions() {
    let mut app = new_app().await;
    app.tab = Tab::Search;
    app.search.focused = true;

    app.handle_msg(Msg::Paste("a\rb\u{1b}[A".into()));

    // `\r` must not run the search, Esc must not defocus
    assert_eq!(app.search.input, "a\rb\u{1b}[A");
    assert!(app.search.last_query.is_empty());
    assert!(app.search.focused);
    assert!(app.status.is_empty(), "{}", app.status);
}

#[tokio::test]
async fn search_history_recalled_with_up_down() {
    let mut app = new_app().await;
    app.tab = Tab::Search;
    app.search.focused = true;
    app.search.input = "first".into();
    app.handle_msg(enter());
    app.search.focused = true;
    app.search.input = "second".into();
    app.handle_msg(enter());

    app.search.focused = true;
    app.search.input.clear();
    app.handle_msg(key_code(KeyCode::Up));
    assert_eq!(app.search.input, "second");
    app.handle_msg(key_code(KeyCode::Up));
    assert_eq!(app.search.input, "first");
    app.handle_msg(key_code(KeyCode::Down));
    assert_eq!(app.search.input, "second");
    app.handle_msg(key_code(KeyCode::Down));
    assert!(app.search.input.is_empty());
}

#[tokio::test]
async fn g_letter_jumps_to_first_matching_title() {
    let (mut app, mut plex_rx) = new_app_with_plex().await;
    app.tab = Tab::Browser;
    app.browser.focus = BrowserCol::Artists;
    app.browser.artists.items = ["Alpha", "Beta", "Bravo", "Charlie"]
        .iter()
        .enumerate()
        .map(|(i, title)| Artist {
            rating_key: format!("a{i}"),
            title: (*title).into(),
            thumb: None,
        })
        .collect();
    app.browser.artists.state.select(Some(0));

    app.handle_msg(key('g'));
    app.handle_msg(key('b'));
    assert_eq!(app.browser.artists.state.selected(), Some(1));

    // 'a' is append here, but after `g` it jumps
    app.handle_msg(key('g'));
    app.handle_msg(key('a'));
    assert_eq!(app.browser.artists.state.selected(), Some(0));
    assert_eq!(app.status, "jump: A");
    assert!(plex_rx.try_recv().is_err(), "'g a' must not append the selection");

    // likewise 'c' (loop-file)
    app.handle_msg(key('g'));
    app.handle_msg(key('c'));
    assert_eq!(app.browser.artists.state.selected(), Some(3));
    assert!(!app.loop_file, "'g c' must jump, not toggle loop-file");

    app.handle_msg(key('g'));
    app.handle_msg(key('d'));
    assert_eq!(app.browser.artists.state.selected(), Some(3));
    assert!(app.status.contains("no row starts with D"));
}

#[tokio::test]
async fn sleep_timer_cycles_and_pauses_at_deadline() {
    let mut app = new_app().await;
    app.handle_msg(key('t'));
    assert!(
        app.sleep_timer_label().is_some_and(|s| s.starts_with("sleep 15m")),
        "first S sets 15 minutes"
    );
    app.handle_msg(key('t'));
    assert!(app.sleep_timer_label().is_some_and(|s| s.starts_with("sleep 30m")));
    app.handle_msg(key('t'));
    assert!(app.sleep_timer_label().is_some_and(|s| s.starts_with("sleep 60m")));
    app.handle_msg(key('t'));
    assert!(app.sleep_timer_label().is_none());

    // firing the posted timer pauses playback and clears it
    app.queue.items = (0..3).map(track).collect();
    app.current = Some(1);
    app.stopped = false;
    let (tx, mut rx) = unbounded_channel();
    app.mpv.tx = tx;
    app.handle_msg(key('t'));
    app.handle_msg(Msg::SleepTimer { epoch: 5 });
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::SetPause { paused: true })));
    assert!(rx.try_recv().is_err(), "nothing but the pause is sent");
    assert!(app.paused && !app.stopped);
    assert_eq!(app.current, Some(1));
    assert!(app.sleep_timer_label().is_none());
}

/// `g` only steals letters: `g 2` still switches tabs and `gg` is still Top.
#[tokio::test]
async fn g_keeps_non_letter_bindings() {
    let mut app = new_app().await;
    app.tab = Tab::Playlists;
    app.playlists.list.items = ["Bravo", "Charlie", "Delta"]
        .iter()
        .enumerate()
        .map(|(i, title)| Playlist { title: (*title).into(), ..playlist(i) })
        .collect();
    app.playlists.list.state.select(Some(2));

    app.handle_msg(key('g'));
    app.handle_msg(key('b'));
    assert_eq!(app.playlists.list.state.selected(), Some(0), "'g b' jumps");
    assert_eq!(app.tab, Tab::Playlists);

    app.handle_msg(key('g'));
    app.handle_msg(key('3'));
    assert_eq!(app.tab, Tab::Browser, "'g 2' must still switch tabs");

    app.tab = Tab::Playlists;
    app.playlists.list.state.select(Some(2));
    app.handle_msg(key('g'));
    app.handle_msg(key('g'));
    assert_eq!(app.playlists.list.state.selected(), Some(0), "'gg' goes to the top");
}

/// A timer that fires while paused leaves the track paused.
#[tokio::test]
async fn sleep_timer_keeps_a_paused_track_paused() {
    let mut app = new_app().await;
    app.queue.items = (0..3).map(track).collect();
    app.current = Some(0);
    app.stopped = false;
    app.paused = true;
    let (tx, mut rx) = unbounded_channel();
    app.mpv.tx = tx;

    app.handle_msg(key('t'));
    app.handle_msg(Msg::SleepTimer { epoch: 1 });

    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::SetPause { paused: true })));
    assert!(app.paused && !app.stopped);
}

/// The `:` line, the filter and the search input keep the caret and the tail in view.
#[tokio::test]
async fn long_prompt_keeps_caret_and_tail_visible() {
    for (open, tab) in [(':', Tab::Queue), ('/', Tab::Queue), ('/', Tab::Search)] {
        let mut app = new_app().await;
        app.tab = tab;
        app.handle_msg(key(open));
        for c in "abcdefghijklmnopqrstuvwxyz0123456789TAIL".chars() {
            app.handle_msg(key(c));
        }
        let text = render(&mut app, 40, 20);
        let bottom = text.lines().nth(18).unwrap();
        assert!(bottom.contains('█') && bottom.contains("TAIL"), "{open} {tab:?}: {bottom}");
    }
}

/// `o` finds the playing track in Discover's tracks, a Search list and the results.
#[tokio::test]
async fn o_finds_the_playing_track_in_discover_and_search() {
    let mut app = new_app().await;
    app.queue.items = vec![track(5)];
    app.current = Some(0);
    app.allow_writes = true;
    app.handle_msg(key('2'));
    app.handle_msg(super::discover::loaded());
    super::discover::previewed(&mut app, 1, 8);
    app.handle_msg(key('l'));
    app.handle_msg(key('o'));
    let Pane::Tracks(pane) = &app.discover.pane else { panic!("the station's tracks") };
    assert_eq!(pane.state.selected(), Some(5));

    let mut state = ratatui::widgets::TableState::default();
    state.select(Some(0));
    let tracks = (0..8).map(track).collect();
    app.search.detail = Some(SearchDetail {
        key: "al1".into(),
        label: "Album".into(),
        tracks,
        state,
        matches: None,
        radio: None,
    });
    app.tab = Tab::Search;
    app.handle_msg(key('o'));
    assert_eq!(app.search.detail.as_ref().and_then(|d| d.state.selected()), Some(5));

    app.search.detail = None;
    app.search.hits = (0..8)
        .map(|i| SearchHit {
            kind: HitKind::Track,
            rating_key: format!("t{i}"),
            title: format!("Hit {i}"),
            artist: "Artist".into(),
            subtitle: String::new(),
            duration_ms: None,
            album_key: None,
            artist_key: None,
            user_rating: None,
        })
        .collect();
    app.search.state.select(Some(0));
    app.handle_msg(key('o'));
    assert_eq!(app.search.state.selected(), Some(5));
}

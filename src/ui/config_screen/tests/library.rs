use super::*;

type Gate = std::sync::mpsc::Sender<Result<Vec<Section>, String>>;
type Asked = std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>;

/// A lookup that waits for the test to answer and notes the server and token.
fn screen_with_a_lookup() -> (Screen, Gate, Asked) {
    let (tx, rx) = std::sync::mpsc::channel();
    let rx = std::sync::Mutex::new(rx);
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = asked.clone();
    let mut screen = ctx();
    screen.library_lookup = std::sync::Arc::new(move |server, token| {
        seen.lock().unwrap().push((server, token));
        rx.lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|_| Err("test never answered".into()))
    });
    select(&mut screen, "plex.music_section");
    (screen, tx, asked)
}

fn section(key: &str, title: &str) -> Section {
    Section { key: key.into(), title: title.into() }
}

/// Wait (bounded) for the lookup's answer to be taken.
fn settle(screen: &mut Screen) {
    for _ in 0..500 {
        screen.poll_library();
        if screen.library_rx.is_none() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the lookup never landed");
}

#[test]
fn library_picker_stores_id() {
    let _palette = theme::palette_test_lock();
    let (mut screen, gate, asked) = screen_with_a_lookup();
    // Not saved yet: the lookup must use what the screen holds.
    screen.cfg.plex.server = "http://unsaved.example:32400".into();
    screen.cfg.plex.token = "fake-token-for-the-test".into();
    enter(&mut screen);
    let text = frame(&mut screen, 100, 30);
    assert!(text.contains("loading"), "{text}");
    assert!(text.contains("first one found"), "the default is always there: {text}");

    gate.send(Ok(vec![section("3", "Music"), section("7", "Live")])).unwrap();
    settle(&mut screen);
    let text = frame(&mut screen, 100, 30);
    assert!(!text.contains("loading") && text.contains("Music") && text.contains("Live"), "{text}");
    assert_eq!(
        asked.lock().unwrap().as_slice(),
        [("http://unsaved.example:32400".to_string(), "fake-token-for-the-test".to_string())]
    );

    // default, Music, Live: the id goes in, not the name.
    press(&mut screen, 'j');
    press(&mut screen, 'j');
    enter(&mut screen);
    assert_eq!(screen.cfg.plex.music_section, "7");
    assert!(screen.editor.is_none());
}

#[test]
fn failed_lookup_leaves_picker_usable() {
    let _palette = theme::palette_test_lock();
    let (mut screen, gate, _) = screen_with_a_lookup();
    screen.cfg.plex.music_section = "3".into();
    screen.cfg.plex.token = "fake-token-for-the-test".into();
    enter(&mut screen);
    gate.send(Err("could not ask the server".into())).unwrap();
    settle(&mut screen);
    let text = frame(&mut screen, 100, 30);
    assert!(text.contains("could not ask the server"), "{text}");
    assert!(!text.contains("fake-token"), "{text}");
    // The current id is still there to keep, and the default to go back to.
    assert!(text.contains("3 (current)"), "{text}");
    // Cursor starts on the current id; up to the default and choose it.
    press(&mut screen, 'k');
    enter(&mut screen);
    assert_eq!(screen.cfg.plex.music_section, "");
}

#[test]
fn unlisted_library_id_stays_selectable() {
    let _palette = theme::palette_test_lock();
    let (mut screen, gate, _) = screen_with_a_lookup();
    screen.cfg.plex.music_section = "5".into();
    screen.saved = screen.cfg.clone();
    enter(&mut screen);
    gate.send(Ok(vec![section("3", "Music")])).unwrap();
    settle(&mut screen);
    let text = frame(&mut screen, 100, 30);
    assert!(text.contains("5 (not found)"), "{text}");
    // The cursor is on it, so Enter keeps it.
    screen.handle(key(KeyCode::Esc)).unwrap();
    assert!(screen.editor.is_none());
    assert_eq!(screen.cfg.plex.music_section, "5");
    assert!(!screen.dirty());
}

#[test]
fn id_can_still_be_typed() {
    let _palette = theme::palette_test_lock();
    for via_row in [false, true] {
        let (mut screen, _gate, _) = screen_with_a_lookup();
        enter(&mut screen);
        if via_row {
            // default, then `type an id…` (nothing has loaded).
            press(&mut screen, 'j');
            enter(&mut screen);
        } else {
            press(&mut screen, 't');
        }
        assert!(matches!(screen.editor, Some(Editor::Field { .. })));
        press(&mut screen, '9');
        enter(&mut screen);
        assert_eq!(screen.cfg.plex.music_section, "9");
    }
}

#[test]
fn silent_server_is_given_up_on() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let server = format!("http://{}", listener.local_addr().expect("addr"));
    let started = std::time::Instant::now();
    let err = fetch_libraries(&server, "fake-token-for-the-test", Duration::from_millis(300))
        .expect_err("nobody answers");
    assert!(started.elapsed() < Duration::from_secs(5), "it hung");
    assert!(!err.contains("fake-token") && !err.contains("127.0.0.1"), "{err}");
}

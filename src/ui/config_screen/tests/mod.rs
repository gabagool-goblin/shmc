use super::draw::LIST_GAP;
use super::edit::{parse_colour, symbol_value, value_for};
use super::*;
use crate::inventory::Kind;
use ratatui::backend::TestBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::Terminal;

mod args;
mod fields;
mod library;
mod list;
mod review;
mod theme_tab;

fn ctx() -> Screen {
    ctx_for(std::path::PathBuf::from("/tmp/shmc.toml"), Config::default())
}

fn ctx_for(path: std::path::PathBuf, cfg: Config) -> Screen {
    Screen::new(path, cfg)
}

fn frame(screen: &mut Screen, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
    terminal.draw(|f| screen.draw(f)).expect("draw");
    let buf = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            if let Some(cell) = buf.cell((x, y)) {
                text.push_str(cell.symbol());
            }
        }
        text.push('\n');
    }
    text
}

#[test]
fn theme_tab_shows_swatch_and_hex() {
    let mut screen = ctx();
    screen.tab = Tab::Theme;
    let text = frame(&mut screen, 100, 40);
    assert!(text.contains("Accent"), "{text}");
    assert!(text.contains("███"), "a colour is drawn as itself: {text}");
    assert!(text.contains("#7aa2f7"), "and as its hex: {text}");
    assert!(!text.contains("Plex server"), "{text}");
}

#[test]
fn changed_value_marked_against_default() {
    let mut screen = ctx();
    screen.cfg.mpv.volume = 50.0;
    let text = frame(&mut screen, 100, 30);
    assert!(text.contains("50 *"), "a changed value is marked: {text}");
    assert!(!text.contains("(missing) *"), "an untouched value has no marker: {text}");
    screen.cfg.plex.server = "https://plex.example".into();
    let text = frame(&mut screen, 100, 30);
    assert!(text.contains("https://plex.example"), "{text}");
    assert!(!text.contains("https://plex.example *"), "{text}");
}

#[test]
fn long_list_gets_scrollbar() {
    let mut screen = ctx();
    let text = frame(&mut screen, 100, 18);
    assert!(text.contains('▲'), "the scrollbar is drawn: {text}");
    assert!(text.contains('▼'), "{text}");
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn screen_fits_narrow_and_short_terminal() {
    for (w, h) in [(100, 30), (80, 24), (60, 20), (100, 12), (44, 10)] {
        for tab in [Tab::Config, Tab::Theme, Tab::Diagnostics] {
            let mut screen = ctx();
            screen.tab = tab;
            let text = frame(&mut screen, w, h);
            assert!(text.contains("keys"), "{tab:?} at {w}x{h} lost the footer:\n{text}");
        }
        let mut screen = ctx();
        let text = frame(&mut screen, w, h);
        assert!(text.contains("Settings"), "{w}x{h} lost the pane title:\n{text}");
    }
}

/// A screen on a file holding `plex.server` and `plex.token`, with `ui.art_pct` set to 55.
fn screen_with_an_edit() -> (tempfile::TempDir, PathBuf, Screen) {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[plex]\nserver = \"http://x:32400\"\ntoken = \"OLD\"\n").expect("write");
    let cfg = Config::read(&path).expect("read");
    let mut screen = ctx_for(path.clone(), cfg);
    let setting = crate::inventory::find("ui.art_pct").expect("ui.art_pct");
    screen.set(setting, "55");
    (dir, path, screen)
}

fn append(path: &PathBuf, text: &str) {
    let mut body = std::fs::read_to_string(path).expect("read");
    body.push_str(text);
    std::fs::write(path, body).expect("write");
}

#[test]
fn outside_edit_survives_save() {
    let (_dir, path, mut screen) = screen_with_an_edit();
    append(&path, "[ui]\nscrolloff = 9\n");
    screen.handle(key(KeyCode::Char('s'))).expect("review");
    screen.handle(key(KeyCode::Char('s'))).expect("save");

    let on_disk = Config::read(&path).expect("read");
    assert_eq!(on_disk.ui.art_pct, 55);
    assert_eq!(on_disk.ui.scrolloff, 9);
    assert_eq!(screen.cfg.ui.scrolloff, 9, "the screen shows the file");
    assert!(!screen.dirty());
}

#[test]
fn real_conflict_asks_and_answers_apply() {
    for answer in [KeyCode::Char('o'), KeyCode::Char('t'), KeyCode::Esc] {
        let (_dir, path, mut screen) = screen_with_an_edit();
        append(&path, "[ui]\nart_pct = 60\n");
        let asked = std::fs::read_to_string(&path).expect("read");

        screen.handle(key(KeyCode::Char('s'))).expect("review");
        screen.handle(key(KeyCode::Char('s'))).expect("save");
        assert!(screen.conflict_ask.is_some(), "{answer:?}: it asks");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), asked, "nothing written yet");
        // Another key does nothing while it is up.
        screen.handle(key(KeyCode::Char('x'))).expect("key");
        assert!(screen.conflict_ask.is_some());

        screen.handle(key(answer)).expect("answer");
        assert!(screen.conflict_ask.is_none(), "{answer:?}: the prompt is gone");
        let on_disk = Config::read(&path).expect("read").ui.art_pct;
        match answer {
            KeyCode::Char('o') => {
                assert_eq!((on_disk, screen.cfg.ui.art_pct), (55, 55));
                assert!(!screen.dirty());
            }
            KeyCode::Char('t') => {
                assert_eq!((on_disk, screen.cfg.ui.art_pct), (60, 60));
                assert!(!screen.dirty());
            }
            _ => {
                assert_eq!((on_disk, screen.cfg.ui.art_pct), (60, 55));
                assert!(screen.dirty());
            }
        }
    }

    // The quit prompt's `s` with a conflict pending does not close the screen.
    let (_dir, path, mut screen) = screen_with_an_edit();
    append(&path, "[ui]\nart_pct = 60\n");
    screen.ask_to_quit();
    assert!(screen.handle_quit_ask(QuitAsk::Dirty, key(KeyCode::Char('s'))));
    assert!(screen.conflict_ask.is_some());
}

#[test]
fn quitting_asks_first() {
    let mut screen = ctx();
    screen.ask_to_quit();
    assert_eq!(screen.quit_ask, Some(QuitAsk::Clean));
    let text = frame(&mut screen, 100, 30);
    assert!(text.contains("quit? (y/n)"), "{text}");
    assert!(text.contains("y quit"), "the footer offers the answer: {text}");

    assert!(screen.handle_quit_ask(QuitAsk::Clean, key(KeyCode::Char('n'))));
    assert_eq!(screen.quit_ask, None);

    screen.ask_to_quit();
    assert!(!screen.handle_quit_ask(QuitAsk::Clean, key(KeyCode::Char('y'))));
}

#[test]
fn ctrl_c_asks_instead_of_quitting() {
    let mut screen = ctx();
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(screen.handle(ctrl_c).expect("handle"), "ctrl-c asks; it must not close the screen");
    assert_eq!(screen.quit_ask, Some(QuitAsk::Clean));

    // A second Ctrl-C is the way out of a wedged screen.
    assert!(!screen.handle_quit_ask(QuitAsk::Clean, ctrl_c));
}

#[test]
fn unsaved_changes_offer_save_on_quit() {
    let (_dir, path, mut screen) = screen_with_an_edit();
    screen.ask_to_quit();
    assert_eq!(screen.quit_ask, Some(QuitAsk::Dirty));
    let text = frame(&mut screen, 100, 30);
    assert!(text.contains("unsaved changes"), "{text}");
    assert!(text.contains("save and quit"), "{text}");

    assert!(!screen.handle_quit_ask(QuitAsk::Dirty, key(KeyCode::Char('s'))));
    let written = std::fs::read_to_string(&path).expect("read back");
    assert!(written.contains("art_pct = 55"), "{written}");
}

#[test]
fn save_writes_file_and_keeps_backup() {
    let (dir, path, mut screen) = screen_with_an_edit();
    let original = std::fs::read_to_string(&path).expect("read");
    screen.save();

    let written = std::fs::read_to_string(&path).expect("read back");
    assert!(written.contains("art_pct = 55"), "the file has the edit: {written}");
    let backup = std::fs::read_to_string(dir.path().join("config.toml.bak")).expect("backup");
    assert_eq!(backup, original, "the backup is the file exactly as it was before the save");
    assert!(screen.status.starts_with("saved"), "the status says saved: {}", screen.status);
    assert!(!screen.dirty(), "a saved screen has nothing pending");
}

#[test]
fn rejected_value_keeps_field_open() {
    let mut screen = ctx();
    select(&mut screen, "ui.art_pct");
    enter(&mut screen);
    type_text(&mut screen, "x");
    enter(&mut screen);
    assert!(screen.status_error, "{}", screen.status);
    let Some(Editor::Field { text, .. }) = &screen.editor else {
        panic!("the field closed on a rejected value");
    };
    assert!(text.text().ends_with('x'), "{:?}", text.text());
    assert_eq!(screen.cfg.ui.art_pct, Config::default().ui.art_pct);
}

/// The preview follows `ui.volume_display` and `ui.mode_flags`.
#[test]
fn preview_follows_header_options() {
    let mut screen = ctx();
    screen.switch(Tab::Theme);
    let plain = frame(&mut screen, 120, 40);
    assert!(plain.contains("Volume: ▁▂▃▄▅▆▇█  60%"), "{plain}");
    assert!(!plain.contains("Repeat"), "{plain}");

    screen.cfg.ui.volume_display = "percent".into();
    screen.cfg.ui.mode_flags = "words".into();
    let text = frame(&mut screen, 120, 40);
    assert!(text.contains("Volume: 60%"), "{text}");
    assert!(text.contains("Repeat Shuffle Single Consume"), "{text}");
    screen.cfg.ui.volume_display = "meter".into();
    let meter = frame(&mut screen, 120, 40);
    assert!(meter.contains("Volume: ▁▂▃▄▅▆▇█") && !meter.contains("60%"), "{meter}");

    let narrow = frame(&mut screen, 80, 40);
    assert!(narrow.contains("Rep Shu Sin Con"), "the short words take over:\n{narrow}");
    assert!(!narrow.contains("Repeat"), "{narrow}");
}

#[test]
fn preview_follows_icon_set() {
    let mut screen = ctx();
    screen.switch(Tab::Theme);
    screen.cfg.ui.icons = "ascii".into();
    let text = frame(&mut screen, 120, 40);
    assert!(text.contains("rep"), "{text}");
    assert!(!text.contains('\u{f0456}'), "{text}");
}

/// The preview follows the unsaved header mode and border style.
#[test]
fn preview_follows_header_mode_and_borders() {
    let _palette = theme::palette_test_lock();
    let mut screen = ctx();
    screen.switch(Tab::Theme);
    let plain = frame(&mut screen, 120, 40);
    assert!(!plain.contains("Playlists") && !plain.contains('╭'), "{plain}");

    screen.cfg.ui.header = "compact".into();
    let mut look = theme::Look::DEFAULT;
    assert!(theme::set_spacing_text(&mut look, "borders", "rounded"));
    theme::set_look(look);
    let text = frame(&mut screen, 120, 40);
    theme::set_look(theme::Look::DEFAULT);
    assert!(text.contains(" Playlists ") && text.contains("Volume: "), "the tab row: {text}");
    assert!(text.contains('╭') && !text.contains("[Playing]"), "{text}");
}

#[test]
fn checks_mark_setting_warnings() {
    let mut screen = ctx();
    screen.tab = Tab::Diagnostics;
    screen.report = Some(Report {
        settings: vec![
            doctor::Entry::ok("path:   /tmp/x"),
            doctor::Entry::warn("server is not https://"),
        ],
        ..Report::default()
    });
    let text = frame(&mut screen, 100, 30);
    assert!(text.contains("  ! server is not https://"), "{text}");
    assert!(text.contains("  path:   /tmp/x"), "{text}");
}

#[test]
fn keystroke_clears_last_message() {
    let mut screen = ctx();
    let setting = crate::inventory::find("ui.art_pct").expect("ui.art_pct");
    screen.set(setting, "55");
    assert!(screen.status.starts_with("ui.art_pct"), "{}", screen.status);

    screen.handle(key(KeyCode::Down)).expect("handle");
    assert!(screen.status.is_empty(), "{}", screen.status);
}

#[test]
fn token_row_never_writes_placeholder_back() {
    let (_dir, path, mut screen) = screen_with_token("MY-SECRET-TOKEN");
    select(&mut screen, "plex.token");

    // The list shows the redacted placeholder, which must not reach the field.
    let text = frame(&mut screen, 100, 30);
    assert!(text.contains("(set)"), "the list is redacted: {text}");

    screen.begin_edit(crate::inventory::find("plex.token").expect("token"));
    let text = frame(&mut screen, 100, 30);
    assert!(!text.contains(" (set)_"), "the field does not carry the placeholder: {text}");
    assert!(
        text.contains("(empty; Enter keeps the current value)"),
        "the field says what empty means: {text}"
    );

    screen.handle(key(KeyCode::Enter)).expect("commit");
    screen.save();
    let written = std::fs::read_to_string(&path).expect("read back");
    assert!(
        written.contains("token = \"MY-SECRET-TOKEN\""),
        "the token survived an untouched edit: {written}"
    );
    assert!(!written.contains("(set)"), "the file has no placeholder: {written}");
}

#[test]
fn typed_token_replaces_old_one() {
    let (_dir, path, mut screen) = screen_with_token("OLD-TOKEN");
    open_token_field(&mut screen);
    type_text(&mut screen, "NEW-TOKEN");
    screen.handle(key(KeyCode::Enter)).expect("commit");
    assert!(screen.dirty(), "the redacted list must not hide a new token");
    screen.save();

    let written = std::fs::read_to_string(&path).expect("read back");
    assert!(
        written.contains("token = \"NEW-TOKEN\""),
        "a typed token replaces the old one: {written}"
    );
    assert!(!written.contains("OLD-TOKEN"), "{written}");
    assert!(
        !screen.status.contains("NEW-TOKEN"),
        "the new token is not echoed in the status: {}",
        screen.status
    );
}

fn open_token_field(screen: &mut Screen) {
    select(screen, "plex.token");
    screen.begin_edit(crate::inventory::find("plex.token").expect("token"));
}

fn screen_with_token(token: &str) -> (tempfile::TempDir, std::path::PathBuf, Screen) {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, format!("[plex]\nserver = \"http://x:32400\"\ntoken = \"{token}\"\n"))
        .expect("write");
    let cfg = Config::read(&path).expect("read");
    let screen = ctx_for(path.clone(), cfg);
    (dir, path, screen)
}

#[test]
fn paste_with_no_field_open_changes_nothing() {
    let (_dir, _path, mut screen) = screen_with_token("OLD");
    screen.handle_paste("NEW-TOKEN".into());
    assert!(screen.editor.is_none());
    assert!(!screen.dirty());

    screen.begin_edit(crate::inventory::find("ui.icons").expect("ui.icons"));
    screen.handle_paste("emoji".into());
    assert!(matches!(screen.editor, Some(Editor::Picker { .. })));
    assert!(!screen.dirty());
}

#[test]
fn ctrl_c_cancels_field_not_types_c() {
    let _palette = theme::palette_test_lock();
    let (_dir, _path, mut screen) = screen_with_token("OLD");
    open_token_field(&mut screen);
    screen.handle(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)).expect("ctrl-c");
    assert!(screen.editor.is_none(), "Ctrl-C closes the field");
    screen.handle(key(KeyCode::Enter)).expect("reopen");
    screen.handle(key(KeyCode::Enter)).expect("commit untouched");
    assert_eq!(screen.cfg.plex.token, "OLD");
    assert!(!screen.dirty());
}

#[test]
fn enter_and_q_keep_unsaved_changes() {
    for code in [KeyCode::Enter, KeyCode::Char('q')] {
        let mut screen = ctx();
        screen.cfg.ui.icons = "emoji".into();
        screen.ask_to_quit();
        assert_eq!(screen.quit_ask, Some(QuitAsk::Dirty));
        assert!(screen.handle(key(code)).expect("handle"), "{code:?} stays open");
        assert_eq!(screen.quit_ask, Some(QuitAsk::Dirty));
    }
    let mut screen = ctx();
    screen.cfg.ui.icons = "emoji".into();
    screen.ask_to_quit();
    assert!(!screen.handle(key(KeyCode::Char('y'))).expect("handle"));
    let mut screen = ctx();
    screen.ask_to_quit();
    assert!(!screen.handle(key(KeyCode::Enter)).expect("handle"));
}

#[test]
fn failed_check_shows_error() {
    let mut screen = ctx();
    screen.tab = Tab::Diagnostics;
    screen.checking = true;
    *screen.check_slot.lock().unwrap() = Some(Err("could not start the checks".into()));
    let text = frame(&mut screen, 100, 30);
    assert!(!screen.checking, "{text}");
    assert!(text.contains("could not start the checks"), "{text}");
    assert!(!text.contains("checking …"), "{text}");
}

#[test]
fn check_thread_ending_silently_is_noticed() {
    let mut screen = ctx();
    screen.tab = Tab::Diagnostics;
    screen.checking = true;
    let thread = std::thread::spawn(|| {});
    while !thread.is_finished() {
        std::thread::yield_now();
    }
    screen.check_thread = Some(thread);
    let text = frame(&mut screen, 100, 30);
    assert!(!screen.checking, "{text}");
    assert!(text.contains("the checks stopped unexpectedly"), "{text}");
}

#[test]
fn picker_opens_on_current_value() {
    let mut screen = ctx();
    screen.cfg.ui.icons = "emoji".into(); // not one of the offered options
    let icons = crate::inventory::find("ui.icons").expect("ui.icons");
    screen.begin_edit(icons);
    let Some(Editor::Picker { options, index, .. }) = &screen.editor else {
        panic!("a picker is open");
    };
    assert_eq!(options[*index], "emoji", "the highlighted option is the one in effect");
    let text = frame(&mut screen, 100, 30);
    assert!(text.contains("nerd") && text.contains("ascii"), "{text}");
}

#[test]
fn diagnostics_end_shows_last_screen() {
    let mut screen = ctx();
    screen.tab = Tab::Diagnostics;
    screen.report = Some(Report::default());
    frame(&mut screen, 100, 30);
    assert!(screen.diag_visible > 1, "the pane recorded its height: {}", screen.diag_visible);

    screen.handle(key(KeyCode::PageDown)).expect("page");
    assert_eq!(
        screen.diag_scroll, screen.diag_visible,
        "a page is the pane's height, not one line"
    );

    screen.handle(key(KeyCode::End)).expect("end");
    assert_eq!(screen.diag_scroll, screen.diag_max, "End is the last page the pane can show");
}

#[test]
fn help_pane_shows_what_value_changes_from() {
    let mut screen = ctx();
    screen.cfg.ui.art_pct = 55;
    select(&mut screen, "ui.art_pct");
    let text = frame(&mut screen, 100, 40);
    assert!(
        text.contains("default 40") && text.contains("value   55"),
        "the pane says what it changes from: {text}"
    );
}

#[test]
fn colour_field_lists_names() {
    let mut screen = ctx();
    screen.tab = Tab::Theme;
    select(&mut screen, "theme.accent");
    let accent = crate::inventory::find("theme.accent").expect("theme.accent");
    screen.begin_edit(accent);
    let text = frame(&mut screen, 100, 40);
    assert!(text.contains("#7aa2f7_"), "the field shows a cursor: {text}");
    assert!(text.contains("black"), "the ANSI names are listed: {text}");
    assert!(text.contains("yellow"), "{text}");
}

#[test]
fn colour_accepts_hex_or_ansi_name() {
    assert_eq!(parse_colour("#7aa2f7").as_deref(), Some("#7aa2f7"));
    assert_eq!(parse_colour("7aa2f7").as_deref(), Some("#7aa2f7"));
    assert_eq!(parse_colour("#7AA2F7").as_deref(), Some("#7aa2f7"));
    assert_eq!(parse_colour("red").as_deref(), Some("#800000"));
    assert_eq!(parse_colour("bright blue").as_deref(), Some("#0000ff"));
    assert_eq!(parse_colour("mauve"), None);
}

#[test]
fn kind_turns_text_into_field_value() {
    assert_eq!(
        value_for(Kind::Int { min: Some(0), max: Some(100) }, "55").expect("an int"),
        toml::Value::Integer(55)
    );
    assert!(value_for(Kind::Int { min: Some(0), max: Some(100) }, "700").is_err());
    assert!(value_for(Kind::Int { min: Some(0), max: None }, "seven").is_err());
    assert_eq!(value_for(Kind::Bool, "true").expect("a bool"), toml::Value::Boolean(true));
    assert_eq!(
        value_for(Kind::Pair, "512, 256").expect("a pair"),
        toml::Value::Array(vec![toml::Value::Integer(512), toml::Value::Integer(256)])
    );
    assert!(value_for(Kind::Pair, "512").is_err());
    assert_eq!(
        value_for(Kind::Triple, "40, 0, 60").expect("weights"),
        toml::Value::Array(vec![40, 0, 60].into_iter().map(toml::Value::Integer).collect())
    );
    for bad in ["40,60", "1,2,3,4", "0,0,0", "-1,5,5", "a,b,c", "1,2,20000"] {
        assert!(value_for(Kind::Triple, bad).is_err(), "{bad}");
    }
    assert_eq!(
        value_for(Kind::List, "--a, --b").expect("a list"),
        toml::Value::Array(vec![
            toml::Value::String("--a".into()),
            toml::Value::String("--b".into())
        ])
    );
}

fn select(screen: &mut Screen, key: &str) {
    screen.cursor = screen
        .visible()
        .iter()
        .position(|s| s.key == key)
        .unwrap_or_else(|| panic!("{key} is not on this tab"));
}

fn snap(cfg: &Config) -> String {
    toml::to_string(cfg).expect("serialise")
}

fn press(screen: &mut Screen, c: char) {
    screen.handle(key(KeyCode::Char(c))).expect("handle");
}

#[test]
fn r_resets_only_the_highlighted_setting() {
    let mut screen = ctx();
    screen.cfg.ui.art_pct = 55;
    screen.cfg.ui.scrolloff = 9;
    select(&mut screen, "ui.art_pct");
    press(&mut screen, 'r');
    assert_eq!(screen.cfg.ui.art_pct, Config::default().ui.art_pct);
    assert_eq!(screen.cfg.ui.scrolloff, 9, "the other change stays");
    assert!(screen.status.contains("ui.art_pct"), "{}", screen.status);

    // Already at its default: it says so and changes nothing.
    let before = snap(&screen.cfg);
    press(&mut screen, 'r');
    assert_eq!(snap(&screen.cfg), before);
}

#[test]
fn r_refuses_settings_without_default() {
    let cfg = Config {
        plex: crate::config::PlexConfig {
            server: "http://x:32400".into(),
            token: "FAKE".into(),
            music_section: "7".into(),
            ..Default::default()
        },
        ..Config::default()
    };
    let mut screen = ctx_for("/tmp/shmc.toml".into(), cfg.clone());
    for key in ["plex.server", "plex.token", "plex.music_section"] {
        select(&mut screen, key);
        press(&mut screen, 'r');
        assert_eq!(snap(&screen.cfg), snap(&cfg), "{key} is untouched");
        assert!(screen.status.contains(key), "{}", screen.status);
        assert!(!screen.status.contains("FAKE"));
    }
}

#[test]
fn capital_r_asks_then_restores_tab() {
    let cfg = Config {
        plex: crate::config::PlexConfig {
            server: "http://x:32400".into(),
            token: "FAKE".into(),
            music_section: "7".into(),
            ..Default::default()
        },
        ..Config::default()
    };
    let mut screen = ctx_for("/tmp/shmc.toml".into(), cfg.clone());
    press(&mut screen, 'R');
    assert_eq!(screen.restore_ask, None, "nothing to restore, nothing asked");
    screen.cfg.ui.art_pct = 55;
    screen.cfg.ui.scrolloff = 9;
    let changed = snap(&screen.cfg);

    press(&mut screen, 'R');
    assert_eq!(screen.restore_ask, Some(2));
    assert_eq!(snap(&screen.cfg), changed, "nothing changes until y");

    press(&mut screen, 'n');
    assert_eq!(screen.restore_ask, None);
    assert_eq!(snap(&screen.cfg), changed, "another key cancels");
    assert!(screen.status.is_empty());

    press(&mut screen, 'R');
    press(&mut screen, 'y');
    assert_eq!(screen.restore_ask, None);
    assert_eq!(snap(&screen.cfg), snap(&cfg), "defaults back; server, token, section kept");
}

fn type_text(screen: &mut Screen, text: &str) {
    for c in text.chars() {
        press(screen, c);
    }
}

fn open_args(screen: &mut Screen) {
    select(screen, "mpv.extra_args");
    enter(screen);
    assert!(matches!(screen.editor, Some(Editor::List(_))));
}

fn with_ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn type_filter(screen: &mut Screen, text: &str) {
    press(screen, '/');
    for c in text.chars() {
        press(screen, c);
    }
}

fn keys_in_view(screen: &Screen) -> Vec<&'static str> {
    screen.view().iter().map(|s| s.key).collect()
}

/// A screen whose themes directory is `<tmp>/themes`, holding `files`.
fn themed(tmp: &std::path::Path, files: &[(&str, &str)]) -> Screen {
    let dir = tmp.join("themes");
    std::fs::create_dir_all(&dir).unwrap();
    for (name, text) in files {
        std::fs::write(dir.join(name), text).unwrap();
    }
    let mut cfg = Config::default();
    cfg.theme.dir = Some(dir);
    let mut screen = ctx_for(tmp.join("config.toml"), cfg);
    screen.switch(Tab::Theme);
    screen
}

fn enter(screen: &mut Screen) {
    screen.handle(key(KeyCode::Enter)).expect("handle");
}

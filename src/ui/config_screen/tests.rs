use super::draw::LIST_GAP;
use super::edit::{parse_colour, symbol_value, value_for};
use super::*;
use crate::inventory::Kind;
use ratatui::backend::TestBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::Terminal;

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

/// The preview is a copy of the header, so it follows the two header
/// options: the number alone, and the flags as words where they fit.
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

/// The preview draws through the player's header code, so it follows the
/// unsaved header mode and border style.
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
    frame(&mut screen, 100, 30); // the pane records its height
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

/// `*`, `r` and `R` judge a colour against the chosen theme, not against
/// what shmc ships.
#[test]
fn colours_marked_and_restored_against_chosen_theme() {
    let _palette = theme::palette_test_lock();
    let mut screen = ctx();
    screen.switch(Tab::Theme);
    screen.set(inventory::find("theme.name").unwrap(), "catppuccin-mocha");
    let marked = |screen: &Screen, key: &str| {
        let values = screen.values();
        screen.is_changed(inventory::find(key).unwrap(), &values)
    };
    assert_eq!(screen.value(inventory::find("theme.accent").unwrap()), "#89b4fa");
    assert!(marked(&screen, "theme.name"));
    assert!(!marked(&screen, "theme.accent"), "a preset's colour is not a change");

    screen.set(inventory::find("theme.accent").unwrap(), "#112233");
    assert!(marked(&screen, "theme.accent"));
    select(&mut screen, "theme.accent");
    press(&mut screen, 'r');
    assert_eq!(screen.cfg.theme.accent, None);
    assert!(!marked(&screen, "theme.accent"));
    assert_eq!(screen.cfg.theme.name, "catppuccin-mocha", "r on a colour keeps the theme");

    screen.set(inventory::find("theme.secondary").unwrap(), "#445566");
    press(&mut screen, 'R');
    press(&mut screen, 'y');
    assert_eq!(screen.cfg.theme.name, "default");
    assert_eq!(screen.cfg.theme.secondary, None);
    assert_eq!(theme::theme(), theme::DEFAULT);
}

/// `none` is a real value of the background: it beats a theme's, shows as
/// text with no swatch, and `r` drops it back to the theme's.
#[test]
fn background_row_shows_swatch() {
    let _palette = theme::palette_test_lock();
    let mut screen = ctx();
    screen.switch(Tab::Theme);
    let bg = inventory::find("theme.background").unwrap();
    let row = |screen: &mut Screen| {
        frame(screen, 100, 70).lines().find(|l| l.contains("Background")).unwrap().to_string()
    };
    let marked = |screen: &Screen| screen.is_changed(bg, &screen.values());

    let line = row(&mut screen);
    assert!(line.contains("none") && !line.contains('█'), "{line}");
    assert!(!marked(&screen));

    screen.set(inventory::find("theme.name").unwrap(), "nord");
    let line = row(&mut screen);
    assert!(line.contains("#2e3440") && line.contains('█'), "{line}");
    assert!(!marked(&screen), "a preset's background is not a change");
    assert_eq!(theme::theme().background, themes::preset("nord").unwrap().theme.background);

    screen.set(bg, "none");
    assert_eq!(screen.cfg.theme.background.as_deref(), Some("none"));
    assert_eq!(theme::theme().background, None, "the palette follows");
    assert!(marked(&screen));
    select(&mut screen, "theme.background");
    press(&mut screen, 'r');
    assert_eq!(screen.cfg.theme.background, None);
    assert!(theme::theme().background.is_some());

    screen.set(bg, "#2e3440");
    assert!(marked(&screen), "the same colour is still an override");
    press(&mut screen, 'R');
    press(&mut screen, 'y');
    assert_eq!(screen.cfg.theme.background, None);
    assert_eq!(theme::theme(), theme::DEFAULT);

    screen.set(bg, "mauve");
    assert_eq!(screen.cfg.theme.background, None, "not a colour: refused");
    assert!(screen.status_error);
}

/// A background paints the whole screen, preview included, and follows the
/// picker; without one nothing is painted.
#[test]
fn background_paints_screen_and_follows_preview() {
    let _palette = theme::palette_test_lock();
    let mut screen = ctx();
    screen.switch(Tab::Theme);
    let bgs = |screen: &mut Screen, w: u16, h: u16| {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        terminal.draw(|f| screen.draw(f)).expect("draw");
        let buf = terminal.backend().buffer().clone();
        // Text in the terminal's own colour would vanish on a light
        // background: every visible cell is coloured by the theme.
        let plain: Vec<&str> = buf
            .content()
            .iter()
            .filter(|c| c.fg == ratatui::style::Color::Reset && !c.symbol().trim().is_empty())
            .map(|c| c.symbol())
            .collect();
        assert!(plain.is_empty(), "drawn in the terminal's colour: {plain:?}");
        buf.content().iter().map(|c| c.bg).collect::<Vec<_>>()
    };
    assert_eq!(bgs(&mut screen, 100, 40)[0], ratatui::style::Color::Reset);

    for name in ["catppuccin-latte", "nord"] {
        let preset = themes::preset(name).unwrap();
        let entries = themes::available(None).entries;
        screen.preview_theme(name, &entries);
        for (w, h) in [(100, 40), (60, 12), (30, 40)] {
            let cells = bgs(&mut screen, w, h);
            let holes = cells.iter().filter(|c| **c == ratatui::style::Color::Reset).count();
            assert_eq!(holes, 0, "{name} at {w}x{h} left {holes} cells unpainted");
            assert!(cells.contains(&preset.theme.background.unwrap()));
        }
        screen.switch(Tab::Config);
        let holes = bgs(&mut screen, 100, 30)
            .iter()
            .filter(|c| **c == ratatui::style::Color::Reset)
            .count();
        assert_eq!(holes, 0, "{name}: the Config tab left {holes} cells unpainted");
        screen.switch(Tab::Theme);
    }
    screen.apply_theme();
}

#[test]
fn theme_sample_yields_on_small_screens() {
    let _palette = theme::palette_test_lock();
    let mut screen = ctx();
    screen.switch(Tab::Theme);
    let text = frame(&mut screen, 100, 40);
    for needle in ["Song name", "Another song", "ing song", "A plain row", "█"] {
        assert!(text.contains(needle), "{needle}\n{text}");
    }
    // The header is still there, and the colour list keeps all its rows.
    assert!(text.contains("[Playing]"), "{text}");
    assert!(text.contains("Visualizer top"), "{text}");
    // Shorter or narrower screens drop the sample first and never panic.
    let short = frame(&mut screen, 100, 20);
    assert!(!short.contains("Another song"), "{short}");
    assert!(short.contains("[Playing]"), "{short}");
    for (w, h) in [(100, 24), (30, 40), (16, 10), (1, 1), (60, 12)] {
        frame(&mut screen, w, h);
    }
}

/// A queue cursor fills its row; the sample's must too, or the colours it
/// previews do not look like the player's.
#[test]
fn sample_cursors_fill_rows() {
    let _palette = theme::palette_test_lock();
    let mut screen = ctx();
    screen.switch(Tab::Theme);
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).expect("terminal");
    terminal.draw(|f| screen.draw(f)).expect("draw");
    let buf = terminal.backend().buffer();
    let row_bg = |needle: &str| {
        (0..buf.area.height).find_map(|y| {
            let line: String =
                (0..buf.area.width).map(|x| buf.cell((x, y)).map_or(" ", |c| c.symbol())).collect();
            let x = line.find(needle)? as u16;
            // Well past the text, still left of the visualizer strip.
            buf.cell((x + needle.len() as u16 + 20, y)).map(|c| c.bg)
        })
    };
    assert_eq!(row_bg("Song name  Artist"), Some(theme::theme().accent));
    assert_eq!(row_bg("Another song"), Some(theme::theme().highlight));
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

#[test]
fn esc_restores_palette_after_preview() {
    let _palette = theme::palette_test_lock();
    let tmp = tempfile::tempdir().unwrap();
    let mut screen = themed(
        tmp.path(),
        &[(
            "mine.toml",
            "name = \"My Theme\"\nauthor = \"Me\"\nvariant = \"light\"\naccent = \"#0a0b0c\"\n",
        )],
    );
    screen.cfg.theme.secondary = Some("#445566".into());
    screen.apply_theme();
    let before = theme::theme();
    let open_on_mine = |screen: &mut Screen| {
        select(screen, "theme.name");
        enter(screen);
        let Some(Editor::Picker { options, .. }) = &screen.editor else {
            panic!("the chooser is not open");
        };
        let at = options.iter().position(|o| o == "mine").expect("the file is listed");
        let first = options.iter().position(|o| o == "default").unwrap();
        assert!(first < at, "presets come first");
        for _ in first..at {
            press(screen, 'j');
        }
    };

    open_on_mine(&mut screen);
    assert_eq!(theme::theme().accent, ratatui::style::Color::Rgb(0x0a, 0x0b, 0x0c));
    assert_ne!(theme::theme().secondary, before.secondary, "the preview is the plain theme");
    let text = frame(&mut screen, 100, 40);
    for needle in ["mine", "My Theme", "author  Me", "variant light", "themes/mine.toml"] {
        assert!(text.contains(needle), "{needle}\n{text}");
    }
    assert_eq!(screen.cfg.theme.name, "default", "previewing writes nothing");

    screen.handle(key(KeyCode::Esc)).expect("handle");
    assert!(screen.editor.is_none());
    assert_eq!(theme::theme(), before, "Esc puts the old palette back");
    assert_eq!(screen.cfg.theme.name, "default");

    open_on_mine(&mut screen);
    enter(&mut screen);
    assert_eq!(screen.cfg.theme.name, "mine");
    assert_eq!(theme::theme().accent, ratatui::style::Color::Rgb(0x0a, 0x0b, 0x0c));
    assert!(screen.cfg.theme.secondary.is_none(), "choosing it cleared the override");
    screen.cfg.theme = crate::config::ThemeConfig::default();
    screen.apply_theme();
}

/// Styles preview while the chooser moves: Esc restores, choosing keeps,
/// and a theme without styles removes them.
#[test]
fn esc_restores_previewed_styles() {
    let _palette = theme::palette_test_lock();
    let tmp = tempfile::tempdir().unwrap();
    let mut screen = themed(
        tmp.path(),
        &[(
            "styled.toml",
            "[styles]\ncurrent = { italic = true }\ntab_active = { bold = false }\n",
        )],
    );
    assert_eq!(theme::look().styles.count(), 0);
    pick_theme(&mut screen, "styled");
    assert_eq!(theme::look().styles.count(), 2, "the preview applies them");
    assert!(theme::current().add_modifier.contains(ratatui::style::Modifier::ITALIC));
    let text = frame(&mut screen, 100, 40);
    assert!(text.contains("styles  2 set"), "{text}");
    move_picker(&mut screen, "nord");
    assert_eq!(theme::look().styles.count(), 0, "a theme without styles removes them");
    assert!(!frame(&mut screen, 100, 40).contains("styles  "));

    screen.handle(key(KeyCode::Esc)).unwrap();
    assert_eq!(theme::look().styles.count(), 0, "Esc puts the old styles back");

    pick_theme(&mut screen, "styled");
    enter(&mut screen);
    assert_eq!(screen.cfg.theme.name, "styled");
    assert_eq!(theme::look().styles.count(), 2, "choosing keeps them");
    pick_theme(&mut screen, "default");
    screen.handle(key(KeyCode::Esc)).unwrap();
    assert_eq!(theme::look().styles.count(), 2, "Esc restores the chosen theme's styles");
    reset_palette(&mut screen);
    assert_eq!(theme::look().styles.count(), 0);
}

#[test]
fn x_exports_active_palette() {
    let _palette = theme::palette_test_lock();
    let tmp = tempfile::tempdir().unwrap();
    let mut screen = themed(tmp.path(), &[]);
    assert!(frame(&mut screen, 100, 40).contains("x export"));

    screen.cfg.theme.accent = Some("#0a0b0c".into());
    press(&mut screen, 'x');
    assert!(matches!(&screen.editor, Some(Editor::Export { text }) if text.text() == "my-theme"));
    // An id that is not one is refused and the prompt stays to fix it.
    for _ in 0..8 {
        screen.handle(key(KeyCode::Backspace)).unwrap();
    }
    for c in "Bad Id".chars() {
        press(&mut screen, c);
    }
    enter(&mut screen);
    assert!(screen.editor.is_some() && screen.status_error, "{}", screen.status);
    for _ in 0..6 {
        screen.handle(key(KeyCode::Backspace)).unwrap();
    }
    for c in "mine".chars() {
        press(&mut screen, c);
    }
    enter(&mut screen);
    assert!(screen.editor.is_none() && !screen.status_error, "{}", screen.status);
    assert!(screen.status.contains("mine"), "{}", screen.status);
    let found = themes::available(screen.cfg.theme.dir.as_deref());
    let mine = found.entries.iter().find(|e| e.id == "mine").expect("exported");
    assert_eq!(mine.theme, theme::from_config(&screen.cfg.theme).0);
    assert_eq!(mine.theme.accent, ratatui::style::Color::Rgb(0x0a, 0x0b, 0x0c));
    screen.cfg.theme = crate::config::ThemeConfig::default();
    screen.apply_theme();
}

/// A colour pinned to what the theme already has is invisible; `*`, `r`
/// and `R` must still see it, or it stays when the theme changes.
#[test]
fn override_equal_to_theme_colour_is_dropped() {
    let _palette = theme::palette_test_lock();
    let tmp = tempfile::tempdir().unwrap();
    let mut screen = themed(tmp.path(), &[]);
    let pinned = theme::hex(theme::DEFAULT.accent);
    let marked = |screen: &Screen| {
        let values = screen.values();
        screen.is_changed(inventory::find("theme.accent").unwrap(), &values)
    };

    screen.cfg.theme.accent = Some(pinned.clone());
    assert!(marked(&screen));
    select(&mut screen, "theme.accent");
    press(&mut screen, 'r');
    assert_eq!(screen.cfg.theme.accent, None);
    assert!(screen.status.contains("removed"), "{}", screen.status);
    press(&mut screen, 'r');
    assert!(screen.status.contains("already"), "{}", screen.status);

    screen.cfg.theme.accent = Some(pinned);
    press(&mut screen, 'R');
    assert_eq!(screen.restore_ask, Some(1));
    press(&mut screen, 'y');
    assert_eq!(screen.cfg.theme.accent, None);
}

/// The settings pane's rows, trimmed: the left 52% of the frame without
/// its border and scrollbar column, exactly what the list is drawn into.
fn list_lines(text: &str, w: u16) -> Vec<String> {
    let pane = Layout::horizontal([Constraint::Percentage(52), Constraint::Min(20)])
        .split(Rect::new(0, 0, w, 1))[0]
        .width as usize;
    text.lines()
        .map(|l| {
            let cols: Vec<char> = l.chars().collect();
            let end = pane.saturating_sub(2).min(cols.len());
            cols[1.min(end)..end].iter().collect::<String>().trim_end().to_string()
        })
        .collect()
}

/// Where the value starts on a settings row: after the label and the first
/// run of two spaces. None for a heading or an empty row.
fn value_col(line: &str) -> Option<usize> {
    let cols: Vec<char> = line.chars().collect();
    let gap = (1..cols.len()).find(|&i| cols[i] == ' ' && cols[i - 1] == ' ' && i > 2)?;
    (gap..cols.len()).find(|&i| cols[i] != ' ')
}

#[test]
fn long_label_is_cut_not_pushed() {
    let longest = inventory::ALL
        .iter()
        .filter(|s| !inventory::is_theme_group(s.group))
        .map(|s| theme::str_width(s.label))
        .max()
        .unwrap();
    for w in [50u16, 100, 160] {
        for tab in [Tab::Config, Tab::Theme] {
            let mut screen = ctx();
            screen.tab = tab;
            // Tall enough that every setting is on screen.
            let text = frame(&mut screen, w, 90);
            let lines = list_lines(&text, w);
            let title = lines
                .iter()
                .position(|l| l.contains("Settings") || l.starts_with(" Theme ─"))
                .unwrap();
            let cols: std::collections::BTreeSet<usize> = lines
                .iter()
                .skip(title + 1)
                .take_while(|l| !l.contains("changed from default"))
                .filter_map(|l| value_col(l))
                .collect();
            assert_eq!(cols.len(), 1, "{tab:?} at {w}: values start in {cols:?}\n{text}");
            if tab == Tab::Config && w == 100 {
                // Sized from the labels, not a fixed 22.
                let col = *cols.iter().next().unwrap();
                assert!(col <= 1 + longest + LIST_GAP, "col {col}\n{text}");
            }
            if tab == Tab::Config && w == 50 {
                assert!(lines.iter().any(|l| l.contains('…')), "a long label is cut:\n{text}");
            }
        }
    }
}

/// Select by key and step through the whole list, checking the cursor is
/// always on a setting and that its heading shows when it opens a group.
#[test]
fn cursor_skips_group_headings() {
    let mut screen = ctx();
    let text = frame(&mut screen, 100, 90);
    let lines = list_lines(&text, 100);
    let at: Vec<usize> = inventory::GROUPS
        .iter()
        .filter(|g| !inventory::is_theme_group(g))
        .map(|g| {
            lines
                .iter()
                .position(|l| l.trim() == *g)
                .unwrap_or_else(|| panic!("no heading {g}:\n{text}"))
        })
        .collect();
    assert!(at.windows(2).all(|p| p[0] < p[1]), "in GROUPS order: {at:?}");
    let theme_tab = {
        let mut s = ctx();
        s.tab = Tab::Theme;
        frame(&mut s, 100, 70)
    };
    let theme_lines = list_lines(&theme_tab, 100);
    let at: Vec<usize> = inventory::THEME_GROUPS
        .iter()
        .map(|g| {
            theme_lines
                .iter()
                .position(|l| l.trim() == *g)
                .unwrap_or_else(|| panic!("no heading {g}:\n{theme_tab}"))
        })
        .collect();
    assert!(at.windows(2).all(|p| p[0] < p[1]), "in order: {at:?}");

    // Walk the list in a short pane, where it has to scroll.
    let cases = [
        (Tab::Config, 14u16),
        (Tab::Config, 20),
        (Tab::Config, 40),
        (Tab::Theme, 24),
        (Tab::Theme, 30),
        (Tab::Theme, 70),
    ];
    for (tab, h) in cases {
        let all: Vec<&Setting> = {
            let mut probe = ctx();
            probe.tab = tab;
            probe.visible()
        };
        let mut screen = ctx();
        screen.tab = tab;
        for (i, setting) in all.iter().enumerate() {
            assert_eq!(screen.selected().map(|s| s.key), Some(setting.key));
            let text = frame(&mut screen, 100, h);
            let lines = list_lines(&text, 100);
            let row = lines
                .iter()
                .position(|l| value_col(l).is_some() && l.trim_start().starts_with(setting.label))
                .unwrap_or_else(|| panic!("{} is off screen at h={h}:\n{text}", setting.key));
            if i == 0 || all[i - 1].group != setting.group {
                assert!(
                    row > 0 && lines[row - 1].trim() == setting.group,
                    "{} opens {} without its heading at h={h}:\n{text}",
                    setting.key,
                    setting.group
                );
            }
            press(&mut screen, 'j');
        }
        assert_eq!(screen.cursor, all.len() - 1, "j stops on the last setting");
        screen.handle(key(KeyCode::Home)).unwrap();
        assert_eq!(screen.cursor, 0);
        screen.handle(key(KeyCode::End)).unwrap();
        assert_eq!(screen.selected().map(|s| s.key), Some(all[all.len() - 1].key));
        let text = frame(&mut screen, 100, h);
        assert!(text.contains(all[all.len() - 1].label), "{text}");
    }
}

#[test]
fn brackets_jump_between_groups() {
    let mut screen = ctx();
    let first_of = |g: &str| screen.visible().iter().position(|s| s.group == g).unwrap();
    let (playback, art) = (first_of("Playback"), first_of("Album art"));
    press(&mut screen, ']');
    assert_eq!(screen.cursor, playback);
    screen.cursor += 1;
    press(&mut screen, ']');
    assert_eq!(screen.cursor, art);
    press(&mut screen, '[');
    assert_eq!(screen.cursor, playback);
    press(&mut screen, '[');
    assert_eq!(screen.cursor, 0);
    press(&mut screen, '[');
    assert_eq!(screen.cursor, 0, "the first group has nothing before it");
    screen.handle(key(KeyCode::End)).unwrap();
    press(&mut screen, ']');
    assert_eq!(screen.cursor, screen.visible().len() - 1, "nor the last anything after");
    screen.handle(key(KeyCode::Tab)).unwrap();
    assert_eq!(screen.tab, Tab::Theme);
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

#[test]
fn filter_hides_empty_groups() {
    let mut screen = ctx();
    type_filter(&mut screen, "plex SER");
    assert_eq!(keys_in_view(&screen), ["plex.server"], "label, any case");
    screen.handle(key(KeyCode::Esc)).unwrap();
    type_filter(&mut screen, "art_pct");
    assert_eq!(keys_in_view(&screen), ["ui.art_pct"], "key");
    screen.handle(key(KeyCode::Esc)).unwrap();
    type_filter(&mut screen, "cleartext");
    assert_eq!(keys_in_view(&screen), ["plex.server"], "help text");

    // Only the Plex group survives, so only its heading is drawn.
    screen.handle(key(KeyCode::Enter)).unwrap();
    let text = frame(&mut screen, 100, 40);
    let lines = list_lines(&text, 100);
    assert!(lines.iter().any(|l| l.trim() == "Plex"), "{text}");
    assert!(!lines.iter().any(|l| l.trim() == "Playback"), "{text}");
    assert!(text.contains("filter: cleartext (1/"), "{text}");
}

#[test]
fn filtered_edit_changes_shown_setting() {
    let mut screen = ctx();
    let before = screen.cfg.ui.center_cursor;
    let (art_square, auto_follow) = (screen.cfg.ui.art_square, screen.cfg.ui.auto_follow);
    type_filter(&mut screen, "center_cursor");
    screen.handle(key(KeyCode::Enter)).unwrap();
    assert_eq!(screen.cursor, 0, "the first match, not the setting's place in the tab");
    screen.handle(key(KeyCode::Enter)).unwrap();
    assert_eq!(screen.cfg.ui.center_cursor, !before);
    assert_eq!(screen.cfg.ui.art_square, art_square);
    assert_eq!(screen.cfg.ui.auto_follow, auto_follow);
    // r on the same row restores that setting.
    press(&mut screen, 'r');
    assert_eq!(screen.cfg.ui.center_cursor, before);
    screen.handle(key(KeyCode::Right)).unwrap();
    assert_eq!(screen.cfg.ui.center_cursor, !before);
}

#[test]
fn esc_clears_filter_keeps_cursor() {
    let mut screen = ctx();
    select(&mut screen, "ui.scrolloff");
    type_filter(&mut screen, "ui.");
    assert!(keys_in_view(&screen).iter().all(|k| k.contains("ui.")));
    assert_eq!(screen.selected().map(|s| s.key), Some("ui.scrolloff"));
    // Narrow to something that no longer holds it: the nearest match.
    for c in "art".chars() {
        press(&mut screen, c);
    }
    assert!(screen.selected().is_some_and(|s| s.key.starts_with("ui.art")));
    screen.handle(key(KeyCode::Enter)).unwrap();
    assert!(!screen.filter.typing && screen.filter.is_active());
    // Esc with the filter applied drops it and nothing else; q is still q.
    screen.handle(key(KeyCode::Esc)).unwrap();
    assert!(!screen.filter.is_active() && screen.quit_ask.is_none());
    assert!(screen.cursor < screen.visible().len());
    // Esc while typing clears too, and a tab change drops it.
    type_filter(&mut screen, "zzz");
    screen.handle(key(KeyCode::Esc)).unwrap();
    assert!(!screen.filter.is_active() && !screen.filter.typing);
    type_filter(&mut screen, "ui");
    screen.handle(key(KeyCode::Enter)).unwrap();
    screen.handle(key(KeyCode::Tab)).unwrap();
    assert!(!screen.filter.is_active());
}

#[test]
fn no_match_disables_row_keys() {
    let mut screen = ctx();
    let before = snap(&screen.cfg);
    type_filter(&mut screen, "no such setting");
    screen.handle(key(KeyCode::Enter)).unwrap();
    assert!(screen.selected().is_none());
    for code in [
        KeyCode::Enter,
        KeyCode::Right,
        KeyCode::Left,
        KeyCode::Char('r'),
        KeyCode::Char('R'),
        KeyCode::Char('j'),
        KeyCode::Char('['),
        KeyCode::Char(']'),
        KeyCode::PageDown,
        KeyCode::End,
        KeyCode::Home,
    ] {
        screen.handle(key(code)).unwrap();
    }
    assert_eq!(snap(&screen.cfg), before);
    assert!(screen.editor.is_none());
    for (w, h) in [(100, 30), (44, 10)] {
        let text = frame(&mut screen, w, h);
        assert!(text.contains("no settings match"), "{w}x{h}\n{text}");
    }
}

#[test]
fn keys_bar_drops_hints_before_clipping() {
    let mut screen = ctx();
    let wide = frame(&mut screen, 120, 24);
    assert!(
        wide.contains("groups") && wide.contains("filter") && wide.contains("←→ change"),
        "{wide}"
    );
    for w in [60u16, 44, 30] {
        let text = frame(&mut screen, w, 24);
        let bar = text
            .lines()
            .find(|l| l.contains("move"))
            .unwrap_or_else(|| panic!("no keys bar at {w}:\n{text}"));
        // `q quit` is the last hint to go, so a bar that still ends in it
        // lost whole hints rather than the end of the line.
        assert!(bar.trim_end_matches(['│', ' ']).ends_with("q quit"), "{w}: {bar}");
    }
    let narrow = frame(&mut screen, 44, 24);
    assert!(!narrow.contains("groups") && !narrow.contains("←→"), "{narrow}");
}

fn with_ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn field_text(screen: &Screen) -> String {
    match &screen.editor {
        Some(Editor::Field { text, .. } | Editor::Export { text }) => text.text().to_string(),
        _ => panic!("no text input is open"),
    }
}

#[test]
fn field_edits_at_cursor() {
    let mut screen = ctx();
    screen.cfg.plex.server = "http://plex.example:32400".into();
    select(&mut screen, "plex.server");
    enter(&mut screen);
    screen.handle(key(KeyCode::Home)).unwrap();
    for _ in 0..7 {
        screen.handle(key(KeyCode::Right)).unwrap();
    }
    screen.handle(key(KeyCode::Delete)).unwrap();
    press(&mut screen, 'P');
    assert_eq!(field_text(&screen), "http://Plex.example:32400");
    enter(&mut screen);
    assert_eq!(screen.cfg.plex.server, "http://Plex.example:32400");
}

#[test]
fn paste_lands_at_cursor_without_newline() {
    let mut screen = ctx();
    screen.cfg.plex.server = "http://example:32400".into();
    select(&mut screen, "plex.server");
    enter(&mut screen);
    for _ in 0..13 {
        screen.handle(key(KeyCode::Left)).unwrap();
    }
    screen.handle_paste("plex.\n".into());
    assert_eq!(field_text(&screen), "http://plex.example:32400");
}

#[test]
fn secret_keeps_mask_with_cursor() {
    let mut screen = ctx();
    open_token_field(&mut screen);
    for c in "abc".chars() {
        press(&mut screen, c);
    }
    screen.handle(key(KeyCode::Left)).unwrap();
    screen.handle(key(KeyCode::Left)).unwrap();
    press(&mut screen, 'X');
    let text = frame(&mut screen, 100, 30);
    assert!(!text.contains("abc") && !text.contains("aXb"), "{text}");
    assert!(text.contains("••••"), "{text}");
    enter(&mut screen);
    assert_eq!(screen.cfg.plex.token, "aXbc");
}

#[test]
fn long_value_scrolls_to_keep_cursor_visible() {
    let mut screen = ctx();
    screen.cfg.plex.server = format!("http://START{}END", "x".repeat(70));
    select(&mut screen, "plex.server");
    enter(&mut screen);
    let end = frame(&mut screen, 60, 30);
    assert!(end.contains("xEND") && !end.contains("START"), "{end}");
    screen.handle(key(KeyCode::Home)).unwrap();
    let start = frame(&mut screen, 60, 30);
    assert!(start.contains("START") && !start.contains("END"), "{start}");
}

#[test]
fn filter_and_export_prompt_take_cursor_keys() {
    let mut screen = ctx();
    type_filter(&mut screen, "art_pct");
    screen.handle(key(KeyCode::Home)).unwrap();
    screen.handle_paste("ui.".into());
    assert_eq!(screen.filter.text, "ui.art_pct");
    assert_eq!(keys_in_view(&screen), ["ui.art_pct"]);
    screen.handle(with_ctrl('u')).unwrap();
    assert_eq!(screen.filter.text, "art_pct");
    screen.handle(key(KeyCode::End)).unwrap();
    screen.handle(key(KeyCode::Left)).unwrap();
    screen.handle(key(KeyCode::Backspace)).unwrap();
    assert_eq!(screen.filter.text, "art_pt");
    // The filter is the one input that is already applied while typed.
    assert!(screen.filter.typing);

    let _palette = theme::palette_test_lock();
    let tmp = tempfile::tempdir().unwrap();
    let mut screen = themed(tmp.path(), &[]);
    press(&mut screen, 'x');
    screen.handle(key(KeyCode::Home)).unwrap();
    press(&mut screen, 'a');
    screen.handle_paste("b".into());
    screen.handle(key(KeyCode::Delete)).unwrap();
    assert_eq!(field_text(&screen), "aby-theme");
}

#[test]
fn arrows_step_number_within_bounds() {
    let mut screen = ctx();
    // 0-100: by 5.
    screen.cfg.ui.art_pct = 40;
    select(&mut screen, "ui.art_pct");
    screen.handle(key(KeyCode::Right)).unwrap();
    assert_eq!(screen.cfg.ui.art_pct, 45);
    assert!(screen.status.contains("ui.art_pct = 45"), "{}", screen.status);
    screen.cfg.ui.art_pct = 98;
    screen.handle(key(KeyCode::Right)).unwrap();
    assert_eq!(screen.cfg.ui.art_pct, 100);
    screen.handle(key(KeyCode::Right)).unwrap();
    assert_eq!(screen.cfg.ui.art_pct, 100, "past the end is not an error");
    assert!(screen.status.contains("limit") && !screen.status_error, "{}", screen.status);
    screen.cfg.ui.art_pct = 3;
    screen.handle(key(KeyCode::Left)).unwrap();
    assert_eq!(screen.cfg.ui.art_pct, 0);
    // 1-10: by 1.
    select(&mut screen, "ui.visualizer_bar_width");
    let before = screen.cfg.ui.visualizer_bar_width;
    press(&mut screen, 'l');
    assert_eq!(screen.cfg.ui.visualizer_bar_width, before + 1);
    screen.cfg.ui.visualizer_bar_width = 1;
    press(&mut screen, 'h');
    assert_eq!(screen.cfg.ui.visualizer_bar_width, 1);
    // No upper bound: ones; a float with a wide range: ones, no drift.
    select(&mut screen, "mpv.volume");
    screen.cfg.mpv.volume = 30.5;
    screen.handle(key(KeyCode::Right)).unwrap();
    assert_eq!(screen.cfg.mpv.volume, 31.5);
}

#[test]
fn arrows_toggle_bool_and_cycle_enum() {
    let mut screen = ctx();
    select(&mut screen, "ui.art_square");
    let before = screen.cfg.ui.art_square;
    screen.handle(key(KeyCode::Left)).unwrap();
    assert_eq!(screen.cfg.ui.art_square, !before);
    screen.handle(key(KeyCode::Right)).unwrap();
    assert_eq!(screen.cfg.ui.art_square, before);

    select(&mut screen, "ui.album_sort");
    screen.cfg.ui.album_sort = "title".into();
    screen.handle(key(KeyCode::Right)).unwrap();
    assert_eq!(screen.cfg.ui.album_sort, "year", "past the last wraps to the first");
    screen.handle(key(KeyCode::Left)).unwrap();
    assert_eq!(screen.cfg.ui.album_sort, "title");
    screen.handle(key(KeyCode::Left)).unwrap();
    assert_eq!(screen.cfg.ui.album_sort, "year_desc");
}

#[test]
fn right_changes_theme_like_chooser() {
    let _palette = theme::palette_test_lock();
    let tmp = tempfile::tempdir().unwrap();
    let mut screen = themed(tmp.path(), &[("gone.toml", "name = \"Gone\"\n")]);
    select(&mut screen, "theme.name");
    let before = screen.cfg.theme.name.clone();
    screen.handle(key(KeyCode::Right)).unwrap();
    assert_ne!(screen.cfg.theme.name, before);
    assert_eq!(theme::accent().fg, Some(theme::from_config(&screen.cfg.theme).0.accent));
    assert!(screen.status.contains("theme.name"), "{}", screen.status);
    // A name whose theme file is gone does not panic: it steps on.
    screen.cfg.theme.name = "deleted-since".into();
    screen.handle(key(KeyCode::Left)).unwrap();
    screen.cfg.theme = crate::config::ThemeConfig::default();
    screen.apply_theme();
}

#[test]
fn right_ignored_on_text_list_colour_secret() {
    let _palette = theme::palette_test_lock();
    let mut screen = ctx();
    let before = snap(&screen.cfg);
    for key_name in ["plex.server", "plex.token", "mpv.extra_args", "album_art.max_size_px"] {
        select(&mut screen, key_name);
        screen.handle(key(KeyCode::Right)).unwrap();
        screen.handle(key(KeyCode::Left)).unwrap();
        assert!(screen.status.is_empty(), "{key_name}: {}", screen.status);
    }
    screen.switch(Tab::Theme);
    select(&mut screen, "theme.accent");
    screen.handle(key(KeyCode::Right)).unwrap();
    screen.switch(Tab::Diagnostics);
    screen.handle(key(KeyCode::Right)).unwrap();
    assert_eq!(snap(&screen.cfg), before);
    assert!(screen.editor.is_none());
}

/// The colour the list draws a row's label in.
fn label_fg(screen: &mut Screen, label: &str) -> ratatui::style::Color {
    let mut terminal = Terminal::new(TestBackend::new(100, 70)).expect("terminal");
    terminal.draw(|f| screen.draw(f)).expect("draw");
    let buf = terminal.backend().buffer();
    for y in 0..buf.area.height {
        let row: String =
            (0..buf.area.width).map(|x| buf.cell((x, y)).map_or(" ", |c| c.symbol())).collect();
        // The list is the left pane; the details pane repeats the label.
        if let Some(at) = row.find(label).filter(|&at| row[..at].chars().count() < 40) {
            let x = row[..at].chars().count() as u16;
            return buf.cell((x, y)).expect("cell").fg;
        }
    }
    panic!("{label} is not drawn");
}

#[test]
fn unused_setting_dimmed_with_reason() {
    let _palette = theme::palette_test_lock();
    let mut screen = ctx();
    let (text, dim) = (theme::text().fg.unwrap(), theme::dim().fg.unwrap());
    assert_ne!(text, dim);
    assert_eq!(screen.cfg.ui.visualizer_mode, "adaptive");
    assert_eq!(label_fg(&mut screen, "Bar count"), dim);
    assert_eq!(label_fg(&mut screen, "Spread anchor"), dim);
    assert_eq!(label_fg(&mut screen, "Bar width"), text, "that one always applies");

    // On the cursor the row keeps the cursor's colours, and the details
    // pane gives the reason; its value is still shown and Enter opens it.
    select(&mut screen, "ui.visualizer_bars");
    assert_eq!(label_fg(&mut screen, "Bar count"), theme::current().fg.unwrap());
    let shown = frame(&mut screen, 100, 70);
    assert!(shown.contains("only used in spread mode"), "{shown}");
    assert!(shown.contains("Bar count"), "{shown}");
    enter(&mut screen);
    assert!(matches!(&screen.editor, Some(Editor::Field { .. })), "still editable");
    screen.handle(key(KeyCode::Esc)).unwrap();

    select(&mut screen, "ui.visualizer_mode");
    screen.handle(key(KeyCode::Right)).unwrap();
    assert_eq!(screen.cfg.ui.visualizer_mode, "spread");
    assert_eq!(label_fg(&mut screen, "Spread anchor"), text);
    select(&mut screen, "ui.visualizer_bars");
    assert!(!frame(&mut screen, 100, 70).contains("only used in spread mode"));

    // The cava tunables follow `cava.enabled`, and the art ones the backend.
    select(&mut screen, "plex.server");
    screen.cfg.cava.enabled = false;
    screen.cfg.album_art.backend = "none".into();
    assert_eq!(label_fg(&mut screen, "Visualizer fps"), dim);
    let switch = crate::inventory::find("cava.enabled").unwrap();
    assert!(screen.unused_because(switch, &screen.values()).is_none(), "the switch itself applies");
    assert_eq!(label_fg(&mut screen, "Art pane width"), dim);
    assert_eq!(label_fg(&mut screen, "Art backend"), text);
    screen.cfg.cava.enabled = true;
    screen.cfg.album_art.backend = "auto".into();
    assert_eq!(label_fg(&mut screen, "Visualizer fps"), text);
    assert_eq!(label_fg(&mut screen, "Art pane width"), text);
}

// Library picker

type Gate = std::sync::mpsc::Sender<Result<Vec<Section>, String>>;
type Asked = std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>;

/// A screen whose library lookup waits for the test to answer, and notes
/// the server and token it was asked with.
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

// The argument list

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

#[test]
fn comma_argument_survives_editor_and_file() {
    let _palette = theme::palette_test_lock();
    let (_dir, path, mut screen) = screen_with_an_edit();
    let af = "--af=lavfi=[loudnorm,volume=2]";
    open_args(&mut screen);
    press(&mut screen, 'a');
    type_text(&mut screen, af);
    enter(&mut screen);
    screen.handle(key(KeyCode::Esc)).unwrap();
    assert_eq!(screen.cfg.mpv.extra_args, vec![af.to_string()]);
    // Through the review and the writer, and back.
    press(&mut screen, 's');
    press(&mut screen, 's');
    assert_eq!(Config::read(&path).unwrap().mpv.extra_args, vec![af.to_string()]);
    // Opened and finished untouched, the argument is not split either.
    open_args(&mut screen);
    screen.handle(key(KeyCode::Esc)).unwrap();
    assert_eq!(screen.cfg.mpv.extra_args, vec![af.to_string()]);
    assert!(!screen.dirty());
}

#[test]
fn list_rows_add_move_and_delete() {
    let _palette = theme::palette_test_lock();
    let mut screen = ctx();
    screen.cfg.mpv.extra_args = vec!["--a".into(), "--b".into(), "--c".into()];
    screen.saved = screen.cfg.clone();
    let args = |screen: &Screen| screen.cfg.mpv.extra_args.clone();
    let done = |screen: &mut Screen| screen.handle(key(KeyCode::Esc)).unwrap();

    open_args(&mut screen);
    press(&mut screen, 'J'); // --a down
    press(&mut screen, 'J');
    press(&mut screen, 'J'); // already last: stays
    press(&mut screen, 'K'); // up one
    press(&mut screen, 'a'); // after the cursor, empty: dropped again
    enter(&mut screen);
    done(&mut screen);
    assert_eq!(args(&screen), ["--b", "--a", "--c"]);

    open_args(&mut screen);
    press(&mut screen, 'j');
    press(&mut screen, 'd'); // --a
    done(&mut screen);
    assert_eq!(args(&screen), ["--b", "--c"]);

    open_args(&mut screen);
    press(&mut screen, 'd');
    press(&mut screen, 'd');
    press(&mut screen, 'd'); // nothing left: no panic
    done(&mut screen);
    assert!(args(&screen).is_empty());
    assert!(screen.dirty());
}

#[test]
fn invalid_queue_columns_list_refused() {
    let _palette = theme::palette_test_lock();
    let mut screen = ctx();
    screen.saved = screen.cfg.clone();
    select(&mut screen, "ui.queue_columns");
    enter(&mut screen);
    press(&mut screen, 'a');
    type_text(&mut screen, "bogus");
    enter(&mut screen);
    screen.handle(key(KeyCode::Esc)).unwrap();
    assert!(screen.status_error, "{}", screen.status);
    assert_eq!(screen.cfg.ui.queue_columns, crate::config::default_queue_columns());
    assert!(!screen.dirty());
}

#[test]
fn cancelled_row_edit_changes_nothing() {
    let _palette = theme::palette_test_lock();
    let mut screen = ctx();
    screen.cfg.mpv.extra_args = vec!["--a".into()];
    screen.saved = screen.cfg.clone();
    open_args(&mut screen);
    enter(&mut screen);
    type_text(&mut screen, "xyz");
    screen.handle(key(KeyCode::Esc)).unwrap(); // the row edit
    press(&mut screen, 'a');
    type_text(&mut screen, "--new");
    screen.handle(key(KeyCode::Esc)).unwrap(); // a new row, cancelled: gone
    assert!(matches!(screen.editor, Some(Editor::List(_))), "still in the list");
    screen.handle(key(KeyCode::Esc)).unwrap();
    assert_eq!(screen.cfg.mpv.extra_args, ["--a"]);
    assert!(!screen.dirty());
}

// The save review and the counts

#[test]
fn review_lists_changes_never_secret() {
    let _palette = theme::palette_test_lock();
    let (_dir, _path, mut screen) = screen_with_token("ZZ-OLD-SECRET");
    for (key, value) in [
        ("ui.art_pct", "55"),
        ("cava.enabled", "false"),
        ("mpv.extra_args", "--a, --b"),
        ("plex.token", "NEW-SECRET-VALUE"),
    ] {
        screen.set(crate::inventory::find(key).unwrap(), value);
    }
    press(&mut screen, 's');
    assert!(screen.review.is_some());
    let text = frame(&mut screen, 100, 30);
    assert!(text.contains("ui.art_pct") && text.contains("55"), "{text}");
    assert!(text.contains("cava.enabled") && text.contains("false"), "{text}");
    assert!(text.contains("mpv.extra_args") && text.contains("--a"), "{text}");
    assert!(text.contains('→'), "{text}");
    let token_row = text.lines().find(|l| l.contains(" plex.token ")).expect("token row");
    assert!(token_row.contains("(changed)"), "{token_row}");
    for leaked in ["NEW-SECRET-VALUE", "ZZ-OLD-SECRET"] {
        assert!(!text.contains(leaked) && !screen.status.contains(leaked), "{leaked} leaked");
    }
}

#[test]
fn long_review_scrolls() {
    let _palette = theme::palette_test_lock();
    let mut screen = ctx();
    let bools: Vec<_> = crate::inventory::ALL
        .iter()
        .filter(|s| s.kind == Kind::Bool && s.group != "Theme")
        .collect();
    for setting in &bools {
        let on = screen.value(setting).trim() != "true";
        screen.set(setting, &on.to_string());
    }
    screen.set(crate::inventory::find("mpv.path").unwrap(), &"/very/long/path/".repeat(12));
    press(&mut screen, 's');
    let last = screen.review.as_ref().unwrap().rows.last().unwrap().0.clone();
    let text = frame(&mut screen, 80, 14);
    assert!(!text.contains(&last), "the last row starts out of view: {text}");
    assert!(!text.contains(&"/very/long/path/".repeat(12)), "cut: {text}");
    let before = text.clone();
    for _ in 0..40 {
        press(&mut screen, 'j');
    }
    let after = frame(&mut screen, 80, 14);
    assert_ne!(before, after, "j scrolls");
    assert!(after.contains(&last), "the last change is reachable: {after}");
}

#[test]
fn s_with_nothing_to_save_says_so() {
    let (_dir, path, mut screen) = screen_with_token("ZZ-OLD-SECRET");
    let before = std::fs::read_to_string(&path).unwrap();
    press(&mut screen, 's');
    assert!(screen.review.is_none());
    assert!(screen.status.contains("nothing to save"), "{}", screen.status);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn review_writes_only_on_s_or_enter() {
    let _palette = theme::palette_test_lock();
    let (_dir, path, mut screen) = screen_with_token("ZZ-OLD-SECRET");
    let before = std::fs::read_to_string(&path).unwrap();
    screen.set(crate::inventory::find("ui.art_pct").unwrap(), "55");
    press(&mut screen, 's');
    screen.handle(key(KeyCode::Esc)).unwrap();
    assert!(screen.review.is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "nothing written");
    assert!(screen.dirty(), "the change is still pending");

    for confirm in [KeyCode::Enter, KeyCode::Char('s')] {
        let (_dir, path, mut screen) = screen_with_token("ZZ-OLD-SECRET");
        screen.set(crate::inventory::find("ui.art_pct").unwrap(), "55");
        press(&mut screen, 's');
        screen.handle(key(confirm)).unwrap();
        assert_eq!(Config::read(&path).unwrap().ui.art_pct, 55, "{confirm:?}");
        assert!(screen.review.is_none() && !screen.dirty());
        assert!(screen.status.contains("next starts"), "{}", screen.status);
    }
}

#[test]
fn tab_bar_counts_unsaved_changes() {
    let _palette = theme::palette_test_lock();
    let (_dir, _path, mut screen) = screen_with_token("ZZ-OLD-SECRET");
    let text = frame(&mut screen, 100, 30);
    assert!(!text.contains('•'), "{text}");
    screen.set(crate::inventory::find("ui.art_pct").unwrap(), "55");
    screen.set(crate::inventory::find("cava.enabled").unwrap(), "false");
    screen.set(crate::inventory::find("theme.accent").unwrap(), "#112233");
    let text = frame(&mut screen, 100, 30);
    assert!(text.contains("Config •2") && text.contains("Theme •1"), "{text}");
    assert!(!text.contains("Diagnostics •"), "{text}");
    press(&mut screen, 's');
    press(&mut screen, 's');
    let text = frame(&mut screen, 100, 30);
    assert!(!text.contains('•'), "{text}");
}

#[test]
fn modes_keep_keys_bar_whole_when_narrow() {
    let _palette = theme::palette_test_lock();
    for w in [100u16, 60, 44, 30] {
        let mut lib = ctx();
        select(&mut lib, "plex.music_section");
        enter(&mut lib);
        let mut list = ctx();
        open_args(&mut list);
        let mut review = ctx();
        review.set(crate::inventory::find("ui.art_pct").unwrap(), "55");
        press(&mut review, 's');
        for (screen, last) in
            [(&mut lib, "Esc cancel"), (&mut list, "Esc done"), (&mut review, "Esc back")]
        {
            let text = frame(screen, w, 24);
            let bar = text
                .lines()
                .find(|l| l.contains(last))
                .unwrap_or_else(|| panic!("{last} lost at {w}:\n{text}"));
            assert!(bar.trim_end_matches(['│', ' ']).ends_with(last), "{w}: {bar}");
        }
    }
}

#[test]
fn dimmed_setting_says_why_unused() {
    let _palette = theme::palette_test_lock();
    let mut screen = ctx();
    assert_ne!(screen.cfg.cava.style, "custom");
    select(&mut screen, "cava.noise_reduction");
    enter(&mut screen);
    screen.handle(key(KeyCode::Backspace)).unwrap();
    screen.handle(key(KeyCode::Backspace)).unwrap();
    screen.handle(key(KeyCode::Backspace)).unwrap();
    type_text(&mut screen, "60");
    enter(&mut screen);
    assert_eq!(screen.cfg.cava.noise_reduction, 60, "still saved");
    assert!(
        screen.status.starts_with("cava.noise_reduction = 60; ")
            && screen.status.contains("custom style"),
        "{}",
        screen.status
    );
    // The key name is not clipped off a narrow status line.
    let text = frame(&mut screen, 50, 24);
    assert!(text.contains("cava.noise_reduction"), "{text}");

    // ←/→ go through the same road.
    screen.status.clear();
    screen.handle(key(KeyCode::Right)).unwrap();
    assert_eq!(screen.cfg.cava.noise_reduction, 65);
    assert!(screen.status.contains("custom style"), "{}", screen.status);

    // A setting that applies gets no note, and neither does this one once
    // the style that uses it is chosen.
    select(&mut screen, "ui.art_pct");
    screen.handle(key(KeyCode::Right)).unwrap();
    assert_eq!(screen.status.matches(';').count(), 0, "{}", screen.status);
    screen.set(crate::inventory::find("cava.style").unwrap(), "custom");
    select(&mut screen, "cava.noise_reduction");
    screen.handle(key(KeyCode::Right)).unwrap();
    assert!(!screen.status.contains(';'), "{}", screen.status);
}

/// A theme screen on a file with two colour overrides on the default theme.
fn screen_with_overrides() -> (tempfile::TempDir, PathBuf, Screen) {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("config.toml");
    std::fs::write(
            &path,
            "[plex]\nserver = \"http://x:32400\"\n[theme]\nname = \"default\"\naccent = \"#112233\"\nsecondary = \"#445566\"\n",
        )
        .unwrap();
    let cfg = Config::read(&path).expect("read");
    let mut screen = ctx_for(path.clone(), cfg);
    screen.apply_theme();
    screen.switch(Tab::Theme);
    select(&mut screen, "theme.name");
    (dir, path, screen)
}

/// Open the chooser and move to `name`.
fn pick_theme(screen: &mut Screen, name: &str) {
    enter(screen);
    move_picker(screen, name);
}

/// Move the open chooser to `name`.
fn move_picker(screen: &mut Screen, name: &str) {
    let Some(Editor::Picker { options, index, .. }) = &screen.editor else {
        panic!("the chooser is not open");
    };
    let (from, to) = (*index, options.iter().position(|o| o == name).expect("listed"));
    for _ in 0..to.abs_diff(from) {
        press(screen, if to > from { 'j' } else { 'k' });
    }
}

fn reset_palette(screen: &mut Screen) {
    screen.cfg.theme = crate::config::ThemeConfig::default();
    screen.apply_theme();
}

#[test]
fn other_theme_clears_overrides_until_saved() {
    let _palette = theme::palette_test_lock();
    let (_dir, path, mut screen) = screen_with_overrides();
    pick_theme(&mut screen, "nord");
    // While moving, the plain theme is shown, not the overrides on it.
    assert_eq!(theme::theme().accent, themes::preset("nord").unwrap().theme.accent);
    enter(&mut screen);
    assert_eq!(screen.cfg.theme.name, "nord");
    assert!(screen.cfg.theme.accent.is_none() && screen.cfg.theme.secondary.is_none());
    assert!(screen.status.contains("2 overrides cleared"), "{}", screen.status);
    assert_eq!(theme::theme().accent, themes::preset("nord").unwrap().theme.accent);

    // Nothing is gone from the file yet; the review and the count say so.
    assert!(std::fs::read_to_string(&path).unwrap().contains("#112233"));
    let text = frame(&mut screen, 100, 30);
    assert!(text.contains("Theme •3"), "{text}");
    press(&mut screen, 's');
    let text = frame(&mut screen, 100, 30);
    let row =
        text.lines().find(|l| l.contains("theme.accent")).expect("the removed override is listed");
    assert!(row.contains("#112233") && row.contains("(theme)"), "{row}");
    press(&mut screen, 's');
    let file = std::fs::read_to_string(&path).unwrap();
    assert!(!file.contains("#112233") && !file.contains("#445566"), "{file}");
    let back = Config::read(&path).unwrap();
    assert_eq!(back.theme.name, "nord");
    assert!(back.theme.accent.is_none() && back.theme.secondary.is_none());
    reset_palette(&mut screen);
}

#[test]
fn same_theme_and_esc_keep_overrides() {
    let _palette = theme::palette_test_lock();
    let (_dir, _path, mut screen) = screen_with_overrides();
    let before = theme::theme();
    pick_theme(&mut screen, "nord");
    screen.handle(key(KeyCode::Esc)).unwrap();
    assert_eq!(screen.cfg.theme.name, "default");
    assert_eq!(screen.cfg.theme.accent.as_deref(), Some("#112233"));
    assert_eq!(theme::theme(), before);
    enter(&mut screen);
    enter(&mut screen);
    assert_eq!(screen.cfg.theme.accent.as_deref(), Some("#112233"));
    assert_eq!(screen.cfg.theme.secondary.as_deref(), Some("#445566"));
    assert!(!screen.status.contains("cleared"), "{}", screen.status);
    assert!(!screen.dirty());
    reset_palette(&mut screen);
}

#[test]
fn stepping_the_theme_clears_the_overrides_too() {
    let _palette = theme::palette_test_lock();
    let (_dir, _path, mut screen) = screen_with_overrides();
    screen.cfg.theme.secondary = None;
    screen.handle(key(KeyCode::Right)).unwrap();
    assert_ne!(screen.cfg.theme.name, "default");
    assert!(screen.cfg.theme.accent.is_none());
    assert!(screen.status.contains("1 override cleared"), "{}", screen.status);
    reset_palette(&mut screen);
}

/// Choosing another theme drops the spacing and symbol overrides with the
/// colours, and the status counts all of them.
#[test]
fn other_theme_clears_spacing_symbol_overrides() {
    let _palette = theme::palette_test_lock();
    let (_dir, _path, mut screen) = screen_with_overrides();
    screen.cfg.theme.inset = Some(2);
    screen.cfg.theme.scroll_up = Some("^".into());
    pick_theme(&mut screen, "nord");
    enter(&mut screen);
    assert_eq!(screen.cfg.theme.name, "nord");
    assert!(screen.cfg.theme.inset.is_none() && screen.cfg.theme.scroll_up.is_none());
    assert!(screen.status.contains("4 overrides cleared"), "{}", screen.status);
    assert_eq!(theme::spacing(), theme::Spacing::DEFAULT);
    reset_palette(&mut screen);
}

/// A bad symbol is refused with its key named; `r` drops an override, or
/// says the row already matches.
#[test]
fn bad_symbol_is_refused() {
    let _palette = theme::palette_test_lock();
    let mut screen = ctx();
    screen.switch(Tab::Theme);
    let ellipsis = inventory::find("theme.ellipsis").unwrap();
    let before = snap(&screen.cfg);
    for bad in ["界", "", "ab", "\u{1b}"] {
        screen.set(ellipsis, bad);
        assert!(
            screen.status.starts_with("rejected") && screen.status_error,
            "{bad:?}: {}",
            screen.status
        );
        assert!(screen.status.contains("theme.ellipsis"), "{}", screen.status);
        assert_eq!(snap(&screen.cfg), before, "{bad:?} changed the config");
    }
    screen.set(inventory::find("theme.volume_meter").unwrap(), "■");
    assert!(screen.status.starts_with("rejected"), "one glyph is neither a ramp nor a pair");

    screen.set(ellipsis, "~");
    assert_eq!(screen.cfg.theme.ellipsis.as_deref(), Some("~"));
    select(&mut screen, "theme.ellipsis");
    press(&mut screen, 'r');
    assert_eq!(screen.cfg.theme.ellipsis, None);
    assert!(screen.status.contains("removed the theme.ellipsis override"), "{}", screen.status);
    press(&mut screen, 'r');
    assert!(screen.status.contains("already matches"), "{}", screen.status);
    reset_palette(&mut screen);
}

/// The preview copies the player's header and visualizer strip, so it
/// draws the symbols in force and sits its blocks apart by the gap.
#[test]
fn preview_follows_spacing_and_symbols() {
    let mut screen = ctx();
    screen.switch(Tab::Theme);
    let tops = |text: &str| -> Vec<usize> {
        text.lines().enumerate().filter(|(_, l)| l.starts_with('┌')).map(|(i, _)| i).collect()
    };
    let plain = frame(&mut screen, 100, 45);
    assert!(plain.contains("Volume: ▁▂▃▄▅▆▇█  60%"), "{plain}");
    assert_eq!(&tops(&plain)[..3], [0, 3, 7], "tabs, header, sample");

    let mut look = theme::Look::DEFAULT;
    assert!(theme::set_spacing(&mut look, "block_gap", 2));
    assert!(theme::set_spacing(&mut look, "inset", 2));
    assert!(theme::set_symbol(&mut look, "volume_meter", "■·"));
    assert!(theme::set_symbol(&mut look, "visualizer", "⠁⠃⠇⠏⠟⠿⡿⣿"));
    theme::set_look(look);
    let text = frame(&mut screen, 100, 45);
    // 60%: five lit cells and three unlit, so both glyphs of the pair show.
    assert!(text.contains("Volume: ■■■■■···  60%"), "{text}");
    assert_eq!(&tops(&text)[..3], [0, 3, 9], "the sample is two rows lower");
    assert!(text.contains('⣿') && text.contains('⠿'), "the strip uses the ramp:\n{text}");
    let header = text.lines().nth(4).unwrap();
    assert_eq!(header.find('[').map(|i| header[..i].chars().count()), Some(3), "inset 2: {header}");
    for glyph in "▁▂▃▄▅▆▇".chars() {
        assert!(!text.contains(glyph), "{glyph} is still drawn:\n{text}");
    }
}

/// `solid` is accepted on the progress glyph rows (the screen's rule is the
/// loader's) and stored as typed; `r` removes it like any override.
#[test]
fn progress_rows_accept_solid() {
    let _palette = theme::palette_test_lock();
    let mut screen = ctx();
    screen.switch(Tab::Theme);
    for key in ["theme.progress_filled", "theme.progress_empty", "theme.progress_head"] {
        assert_eq!(symbol_value(key, "solid").unwrap(), toml::Value::String("solid".into()));
    }
    assert!(symbol_value("theme.ellipsis", "solid").is_err());
    let head = inventory::find("theme.progress_head").unwrap();
    screen.set(head, "solid");
    assert_eq!(screen.cfg.theme.progress_head.as_deref(), Some("solid"));
    select(&mut screen, "theme.progress_head");
    press(&mut screen, 'r');
    assert_eq!(screen.cfg.theme.progress_head, None);
}

/// The progress bar row is a picker of the preset names; the choice is
/// stored as the name, and a name is all it takes.
#[test]
fn progress_bar_row_stores_preset_name() {
    let _palette = theme::palette_test_lock();
    let mut screen = ctx();
    screen.switch(Tab::Theme);
    let bar = inventory::find("theme.progress_bar").unwrap();
    assert_eq!(screen.value(bar), "solid");
    select(&mut screen, "theme.progress_bar");
    enter(&mut screen);
    let Some(Editor::Picker { options, .. }) = &screen.editor else {
        panic!("no picker");
    };
    assert_eq!(options, &["solid", "line", "dot", "plain", "angled", "rounded", "custom…"]);
    press(&mut screen, 'j');
    press(&mut screen, 'j');
    assert_eq!(theme::symbols().progress_head, Some('●'), "the preview follows the highlight");
    enter(&mut screen);
    assert_eq!(screen.cfg.theme.progress_bar.as_deref(), Some("dot"));
    assert!(symbol_value("theme.progress_bar", "=").is_err(), "a name only");
}

/// The meter and the visualizer rows pick a preset (then `custom…` for
/// glyphs) and store the name, not the glyphs; `r` removes it.
#[test]
fn meter_visualizer_rows_store_preset_name() {
    let _palette = theme::palette_test_lock();
    let mut screen = ctx();
    screen.switch(Tab::Theme);
    let meter = inventory::find("theme.volume_meter").unwrap();
    assert_eq!(screen.value(meter), "blocks", "unset is the default preset");

    select(&mut screen, "theme.volume_meter");
    enter(&mut screen);
    let Some(Editor::Picker { options, index, .. }) = &screen.editor else {
        panic!("no picker");
    };
    assert_eq!(options, &["blocks", "squares", "outline", "plain", "custom…"]);
    assert_eq!(*index, 0);
    // The sample follows the highlight, and Esc puts the old look back.
    for _ in 0..3 {
        press(&mut screen, 'j');
    }
    assert_eq!(theme::symbols().volume_meter, theme::Meter::Pair('#', '-'));
    assert!(frame(&mut screen, 100, 45).contains("Volume: #####---  60%"));
    screen.handle(key(KeyCode::Esc)).unwrap();
    assert_eq!(theme::symbols().volume_meter, theme::Symbols::DEFAULT.volume_meter);

    enter(&mut screen);
    press(&mut screen, 'j');
    enter(&mut screen);
    assert_eq!(screen.cfg.theme.volume_meter.as_deref(), Some("squares"), "the name, not ■·");
    assert_eq!(screen.value(meter), "squares");
    // ←/→ cycle the names and wrap.
    for want in ["outline", "plain", "blocks", "squares"] {
        screen.handle(key(KeyCode::Right)).unwrap();
        assert_eq!(screen.cfg.theme.volume_meter.as_deref(), Some(want));
    }
    press(&mut screen, 'r');
    assert_eq!(screen.cfg.theme.volume_meter, None);

    // Glyphs typed show as themselves and are left alone by ←/→; the
    // picker opens on `custom…`, which leads to the text field.
    screen.set(meter, "<>");
    assert_eq!(screen.value(meter), "<>");
    screen.handle(key(KeyCode::Right)).unwrap();
    assert_eq!(screen.cfg.theme.volume_meter.as_deref(), Some("<>"));
    enter(&mut screen);
    let Some(Editor::Picker { index, .. }) = &screen.editor else {
        panic!("no picker");
    };
    assert_eq!(*index, 4);
    enter(&mut screen);
    let Some(Editor::Field { initial, .. }) = &screen.editor else {
        panic!("custom… did not open the field");
    };
    assert_eq!(initial, "<>");
    reset_palette(&mut screen);
}

#[test]
fn paste_during_question_does_not_edit_filter() {
    let mut screen = ctx();
    type_filter(&mut screen, "ar");
    screen.handle(with_ctrl('c')).unwrap();
    assert!(screen.quit_ask.is_some());
    screen.handle_paste("zzz".into());
    assert_eq!(screen.filter.text, "ar");
}

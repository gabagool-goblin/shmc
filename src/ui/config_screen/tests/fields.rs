use super::*;

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

    // On the cursor the row keeps the cursor's colours; the details pane says why.
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

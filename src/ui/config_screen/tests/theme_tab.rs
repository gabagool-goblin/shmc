use super::*;

/// `*`, `r` and `R` judge a colour against the chosen theme.
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

/// `none` beats a theme's background, shows with no swatch, and `r` drops it.
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

/// A background paints the whole screen and follows the picker.
#[test]
fn background_paints_screen_and_follows_preview() {
    let _palette = theme::palette_test_lock();
    let mut screen = ctx();
    screen.switch(Tab::Theme);
    let bgs = |screen: &mut Screen, w: u16, h: u16| {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        terminal.draw(|f| screen.draw(f)).expect("draw");
        let buf = terminal.backend().buffer().clone();
        // Text in the terminal's own colour would vanish on a light background.
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

/// The sample's cursors fill their rows, as the queue's do.
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

/// Styles preview in the chooser: Esc restores, choosing keeps, none removes them.
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

/// `*`, `r` and `R` see an override equal to the theme's colour.
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

/// Choosing another theme drops spacing and symbol overrides too, and counts them.
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

/// A bad symbol is refused by key; `r` drops an override or says it matches.
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

/// The preview draws the symbols in force and spaces its blocks by the gap.
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

/// `solid` is accepted on the progress glyph rows; `r` removes it like any override.
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

/// The progress bar row picks a preset and stores its name.
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

/// The meter and visualizer rows store a preset's name, not its glyphs.
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

    // Typed glyphs are left alone by ←/→, and the picker opens on `custom…`.
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

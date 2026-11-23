use super::*;

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

    // No note for a setting that applies, nor for this one once its style is chosen.
    select(&mut screen, "ui.art_pct");
    screen.handle(key(KeyCode::Right)).unwrap();
    assert_eq!(screen.status.matches(';').count(), 0, "{}", screen.status);
    screen.set(crate::inventory::find("cava.style").unwrap(), "custom");
    select(&mut screen, "cava.noise_reduction");
    screen.handle(key(KeyCode::Right)).unwrap();
    assert!(!screen.status.contains(';'), "{}", screen.status);
}

use super::*;

/// The settings pane's rows: the left 52% without its border and scrollbar column.
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

/// Where a row's value starts, after the first two spaces; None for a heading.
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

/// The cursor stays on settings, and a group's heading shows when it opens one.
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
        // `q quit` goes last, so whole hints were dropped, not the end of the line.
        assert!(bar.trim_end_matches(['│', ' ']).ends_with("q quit"), "{w}: {bar}");
    }
    let narrow = frame(&mut screen, 44, 24);
    assert!(!narrow.contains("groups") && !narrow.contains("←→"), "{narrow}");
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

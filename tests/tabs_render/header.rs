use super::*;
use shmc::ui::theme;

#[tokio::test]
async fn transport_state_shows_badge_and_glyph() {
    let mut app = new_app().await;
    app.queue.items = (0..5).map(track).collect();
    app.queue.state.select(Some(1));
    app.current = Some(1);
    app.stopped = false;
    app.duration = 257.0;
    app.position = 42.0;
    app.tab = Tab::Queue;

    let play = render(&mut app, 120, 40);
    assert!(play.contains('>'), "progress bar must show the play glyph\nframe:\n{play}");
    app.paused = true;
    let pause = render(&mut app, 120, 40);
    assert!(pause.contains("[Paused]"), "frame:\n{pause}");
    assert!(pause.contains("||"), "pause glyph\n{pause}");
    app.paused = false;
    app.stopped = true;
    let stop = render(&mut app, 120, 40);
    assert!(stop.contains("[Stopped]"), "frame:\n{stop}");
    assert!(stop.contains('■'), "stop glyph\n{stop}");
    for frame in [&play, &pause, &stop] {
        assert!(!frame.contains('\u{23f8}'), "no U+23F8 pause glyph");
        assert!(!frame.contains('\u{fe0f}'), "no variation selector");
    }
}

#[tokio::test]
async fn header_renders_a_cjk_title() {
    let mut app = new_app().await;
    app.queue.items = (0..30).map(track).collect();
    app.queue.state.select(Some(3));
    app.current = Some(3);
    app.stopped = false;
    app.tab = Tab::Queue;
    app.queue.items[3].title = "日本語の曲".into();
    let cjk = render(&mut app, 120, 40);
    let header_nospace: String = cjk.lines().take(4).collect::<String>().replace(' ', "");
    assert!(
        header_nospace.contains("日本語の曲"),
        "CJK title must render in the center zone\nframe:\n{cjk}"
    );
    let row1 = cjk.lines().nth(1).unwrap_or("").replace(' ', "");
    let row2 = cjk.lines().nth(2).unwrap_or("");
    assert!(row1.contains("日本語の曲"), "row1 title\n{row1}");
    assert!(row2.contains("Northwind Sounds"), "row2 credit\n{row2}");
}

#[tokio::test]
async fn volume_digits_do_not_shift_the_meter() {
    let mut app = new_app().await;
    app.volume = 100.0;
    let full = render(&mut app, 120, 40);
    app.volume = 95.0;
    let less = render(&mut app, 120, 40);

    let x = |frame: &str| {
        frame.lines().find(|l| l.contains("Volume:")).and_then(|l| char_x(l, "Volume:"))
    };
    assert!(x(&full).is_some(), "volume meter missing\n{full}");
    assert_eq!(x(&full), x(&less), "volume digits moved the meter");
}

/// The header shows the file's audio details, not the live bitrate.
#[tokio::test]
async fn header_shows_file_audio_details() {
    let mut app = new_app().await;
    app.queue.items = vec![track(0)];
    app.current = Some(0);
    app.stopped = false;
    app.duration = 257.0;
    app.position = 32.0;
    app.audio = AudioInfo {
        codec: Some("flac".into()),
        sample_rate_hz: Some(44100),
        bit_depth: Some(16),
        bitrate_kbps: Some(857),
        channels: Some(2),
        ..AudioInfo::default()
    };

    let frame = render(&mut app, 120, 40);
    assert!(
        frame.contains("(FLAC 44.1k/16)"),
        "header missing parenthesized file audio details\n{frame}"
    );
    assert!(
        !frame.contains("(857 kbps)"),
        "the live bitrate should not be shown when file details exist\n{frame}"
    );
}

/// The title and credit line are centred on the frame at any width.
#[tokio::test]
async fn header_title_is_centred_on_the_frame() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    app.queue.items = vec![track(0)];
    app.current = Some(0);
    for w in 60u16..=250 {
        let frame = render(&mut app, w, 30);
        let lines: Vec<&str> = frame.lines().collect();
        for (row, needle) in [(1, "Starfall! 0"), (2, "Northwind Sounds - Evening Satellite")] {
            let Some(x) = char_x(lines[row], needle) else {
                // the credit is ellipsized in a narrow centre, the title never
                assert!(row == 2 && w < 100, "{needle:?} missing at width {w}\n{frame}");
                continue;
            };
            // twice the distance between the text's centre and the frame's
            let off = (2 * x + needle.len()) as isize - w as isize;
            assert!(off.abs() <= 1, "{needle:?} is {off} half-cells off centre at width {w}");
        }
    }
}

#[tokio::test]
async fn progress_bar_stable_when_position_gains_digit() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    app.duration = 7200.0;
    let mut ends = Vec::new();
    for pos in [0.0, 599.0, 600.0, 3599.0, 3600.0] {
        app.position = pos;
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| views::draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer();
        let text = flatten(&terminal);
        let lines: Vec<&str> = text.lines().collect();
        // the last such line (the header shows the times too)
        let y = lines.iter().rposition(|l| l.contains("/ 2:00:00")).expect("time label") as u16;
        let last =
            (0..100).rev().find(|&x| buf.cell((x, y)).unwrap().bg != Color::Reset).expect("bar");
        ends.push(last);
    }
    assert!(ends.windows(2).all(|p| p[0] == p[1]), "bar end moved: {ends:?}");
}

/// A header at `w` columns; the app is dropped on return, as it holds the lock.
async fn header_with(volume_display: &str, mode_flags: &str, w: u16) -> Terminal<TestBackend> {
    let ui = UiConfig {
        volume_display: volume_display.into(),
        mode_flags: mode_flags.into(),
        ..ascii_ui()
    };
    let (mut app, _plex) = new_app_with_ui(ui).await;
    app.volume = 40.0;
    app.loop_playlist = true;
    let mut terminal = Terminal::new(TestBackend::new(w, 30)).unwrap();
    terminal.draw(|f| views::draw(f, &mut app)).unwrap();
    terminal
}

/// Columns of the header's two zone dividers.
fn dividers(terminal: &Terminal<TestBackend>) -> Vec<u16> {
    let buf = terminal.backend().buffer();
    (1..buf.area.width - 1)
        .filter(|x| buf.cell((*x, 1)).is_some_and(|c| c.symbol() == "│"))
        .collect()
}

/// Mode words shorten with the width down to R Z S C, never icons; the title stays centred.
#[tokio::test]
async fn mode_words_step_down_by_width() {
    let s = shmc::ui::icons::status(true);
    // (header width, an on label, an off label, the spelling, text that must not show)
    let cases: [(u16, &str, &str, &str, &str); 4] = [
        (110, "Repeat", "Shuffle", "Repeat Shuffle Single Consume", ""),
        (140, "Repeat", "Shuffle", "Repeat Shuffle Single Consume", ""),
        (100, "Rep", "Shu", "Rep Shu Sin Con", "Repeat"),
        (80, "Rep", "Shu", "Rep Shu Sin Con", "Repeat"),
    ];
    for (w, on, off, spelled, absent) in cases {
        let term = header_with("both", "words", w).await;
        let text = flatten(&term);
        let row2 = text.lines().nth(2).unwrap();
        assert!(row2.contains(spelled), "{w}: {row2}");
        assert!(absent.is_empty() || !text.contains(absent), "{w}: {row2}");
        assert_eq!(fg_at(&term, on), Some(theme::accent().fg.unwrap()), "{w}: on");
        assert_eq!(fg_at(&term, off), Some(theme::dim().fg.unwrap()), "{w}: off");
        assert!(!row2.contains(s.repeat) && !row2.contains(s.consume), "{w}: no icons: {row2}");
        let divs = dividers(&term);
        assert_eq!(divs.len(), 2, "{w}: {divs:?}");
        assert_eq!(divs[0] + divs[1], w - 1, "{w}: the title column is centred: {divs:?}");
        let title = char_x(text.lines().nth(1).unwrap(), "No Song").unwrap() as u16 + 3;
        assert!(title.abs_diff(w / 2) <= 1, "{w}: the title is at {title}");
    }
    for w in [44u16, 48] {
        let term = header_with("both", "words", w).await;
        let text = flatten(&term);
        let row2 = text.lines().nth(2).unwrap();
        assert!(row2.contains("R Z S C") && !text.contains("Rep"), "{w}: {row2}");
        assert_eq!(fg_at(&term, "R Z S C"), Some(theme::accent().fg.unwrap()), "{w}: on");
        assert_eq!(fg_at(&term, "Z S C"), Some(theme::dim().fg.unwrap()), "{w}: off");
        assert!(!row2.contains(s.repeat), "{w}: no icons: {row2}");
        let divs = dividers(&term);
        assert_eq!(divs[0] + divs[1], w - 1, "{w}: {divs:?}");
    }
}

/// With the sleep label sharing the row the words step down, not turn to icons.
#[tokio::test]
async fn sleep_label_steps_words_not_icons() {
    let ui = UiConfig { mode_flags: "words".into(), ..ascii_ui() };
    let (mut app, _plex) = new_app_with_ui(ui).await;
    run_cmd(&mut app, "sleep 15");
    let mut term = Terminal::new(TestBackend::new(140, 30)).unwrap();
    term.draw(|f| views::draw(f, &mut app)).unwrap();
    let text = flatten(&term);
    let row2 = text.lines().nth(2).unwrap();
    assert!(row2.contains("sleep 15m Rep Shu Sin Con"), "{row2}");
    assert!(
        !row2.contains("Repeat") && !row2.contains(shmc::ui::icons::status(true).repeat),
        "{row2}"
    );
}

/// Each `ui.volume_display` draws what it names (unknown is both), the columns fixed.
#[tokio::test]
async fn volume_display_meter_number_or_both() {
    for w in [80u16, 120] {
        let both = header_with("both", "icons", w).await;
        for (value, meter, number) in [
            ("meter", true, false),
            ("percent", false, true),
            ("both", true, true),
            ("bogus", true, true),
        ] {
            let term = header_with(value, "icons", w).await;
            let text = flatten(&term);
            let row = text.lines().nth(1).unwrap();
            assert_eq!(row.contains('▁'), meter, "{value} {w}: {row}");
            assert_eq!(row.contains("40%"), number, "{value} {w}: {row}");
            assert_eq!(dividers(&term), dividers(&both), "{value} {w}: the columns do not move");
        }
    }
}

/// Narrow compact rows drop the right side, then the sleep timer, before a tab is cut.
#[tokio::test]
async fn compact_row_steps_down_before_tab_clips() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    app.ui.header_compact = true;
    run_cmd(&mut app, "sleep 15");
    let row = |app: &mut App, w: u16| row_text(&grid(app, w, 24), 1);
    for w in [120u16, 80, 60, 50, 48] {
        let text = row(&mut app, w);
        for name in ["Queue", "Discover", "Browser", "Playlists", "Search", "Help"] {
            assert!(text.contains(&format!(" {name} ")), "{name} at {w}: {text:?}");
        }
        assert!(!text.contains('>'), "no state glyph at {w}: {text:?}");
    }
    let wide = row(&mut app, 120);
    let left_of_tabs = wide.find("sleep 15m").zip(wide.find("Queue")).is_some_and(|(s, q)| s < q);
    assert!(left_of_tabs, "the timer on the left: {wide:?}");
    assert!(wide.contains("Volume: ") && wide.contains('▁'), "the meter at 120: {wide:?}");
    let mid = row(&mut app, 90);
    assert!(
        mid.contains("sleep 15m") && mid.contains('%') && !mid.contains('▁'),
        "a shorter volume at 90: {mid:?}"
    );
    let narrow = row(&mut app, 66);
    assert!(
        narrow.contains("sleep 15m") && !narrow.contains('%'),
        "only the timer at 66: {narrow:?}"
    );
    let tight = row(&mut app, 48);
    assert!(!tight.contains("sleep"), "{tight:?}");
}

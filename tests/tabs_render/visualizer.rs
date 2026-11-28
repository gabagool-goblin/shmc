use super::*;

fn lines_with(text: &str, needle: &str) -> usize {
    text.lines().filter(|l| l.contains(needle)).count()
}

/// The visualizer keeps drawing while paused, in about half the column.
#[tokio::test]
async fn visualizer_keeps_drawing_while_paused() {
    let mut app = new_app().await;
    app.art_enabled = true;
    app.queue.items = (0..30).map(track).collect();
    app.queue.state.select(Some(3));
    app.current = Some(3);
    app.stopped = false;
    app.duration = 3725.0;
    app.position = 42.0;
    app.paused = true;
    app.tab = Tab::Queue;
    let bar_glyphs = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let bar_rows = |frame: &str| {
        frame.lines().skip(7).filter(|l| l.chars().any(|c| bar_glyphs.contains(&c))).count()
    };

    app.cava_alive = true;
    app.cava_frame = vec![220u8; 20];
    let h = render(&mut app, 120, 40);
    assert!(
        !h.contains("visualizer off") && !h.contains("cava is not running"),
        "a live cava must not be reported as off\n{h}"
    );
    app.cava_alive = false;
    let off = render(&mut app, 120, 40);
    let viz_rows = bar_rows(&h).saturating_sub(bar_rows(&off));
    assert!(
        viz_rows >= 10,
        "visualizer should be tall (about half the column), got {viz_rows} rows\n{h}"
    );
    let qrows = lines_with(&h, "Evening Satellite");
    assert!(qrows >= 8, "queue must keep rows, got {qrows}\n{h}");
}

/// Bars are centred, an odd leftover cell on the right, in both modes, art on or off.
#[tokio::test]
async fn visualizer_is_centred_in_its_pane() {
    let mut app = new_app().await;
    app.cava_alive = true;
    app.cava_frame = vec![255u8; 96];
    for art in [false, true] {
        for mode in ["adaptive", "spread"] {
            app.ui.visualizer_mode = mode.into();
            queue_app_with_art(&mut app, art);
            for w in 40u16..=250 {
                let g = grid(&mut app, w, 30);
                let row = g.iter().rev().find(|r| r.iter().any(|c| c == "█")).expect("bars");
                let first = row.iter().position(|c| c == "█").unwrap();
                let edge_l = row[..first].iter().rposition(|c| c == "│").unwrap();
                let edge_r = row.iter().rposition(|c| c == "│").unwrap();
                let last = row.iter().rposition(|c| c == "█").unwrap();
                let (l, r) = (first - edge_l - 1, edge_r - last - 1);
                let case = format!("art={art} {mode} w={w}: blank {l} left, {r} right");
                assert!(l.abs_diff(r) <= 1, "{case}");
                assert!((l + r) % 2 == 1 || l == r, "{case}");
                assert!(l <= r, "{case}");
            }
        }
    }
}

/// Spread mode: the set number of bars once they fit, even bars and gaps, the anchor fixed.
#[tokio::test]
async fn spread_visualizer_even_bars_and_gaps() {
    let mut app = new_app().await;
    app.cava_alive = true;
    app.cava_frame = vec![255u8; 96];
    queue_app_with_art(&mut app, false);
    app.ui.visualizer_mode = "spread".into();
    app.ui.visualizer_bars = 12;
    app.ui.visualizer_bar_width = 2;
    app.ui.visualizer_bar_gap = 1;
    for anchor in ["gap", "bar"] {
        app.ui.visualizer_anchor = anchor.into();
        let g = grid(&mut app, 32, 30);
        let row = g.iter().rev().find(|r| r.iter().any(|c| c == "█")).expect("bars");
        let runs =
            (0..row.len()).filter(|&x| row[x] == "█" && (x == 0 || row[x - 1] != "█")).count();
        let cells = row.iter().filter(|c| *c == "█").count();
        assert!(runs < 12 && cells >= 2 * runs, "{anchor}: {runs} bars, {cells} cells");
        let mut last = (0, 0);
        for w in [60u16, 100, 160, 250] {
            let g = grid(&mut app, w, 30);
            let row = g.iter().rev().find(|r| r.iter().any(|c| c == "█")).expect("bars");
            let (mut bars, mut gaps): (Vec<usize>, Vec<usize>) = (vec![], vec![]);
            let mut x = row.iter().position(|c| c == "█").unwrap();
            let end = row.iter().rposition(|c| c == "█").unwrap();
            while x <= end {
                let on = row[x] == "█";
                let len = row[x..=end].iter().take_while(|c| (*c == "█") == on).count();
                if on {
                    bars.push(len);
                } else {
                    gaps.push(len);
                }
                x += len;
            }
            let case = format!("{anchor} w={w}: bars {bars:?} gaps {gaps:?}");
            assert_eq!(bars.len(), 12, "{case}");
            assert!(
                bars.iter().all(|&b| b == bars[0]) && gaps.iter().all(|&x| x == gaps[0]),
                "{case}"
            );
            let (bar, gap) = (bars[0], gaps[0]);
            match anchor {
                "gap" => assert_eq!(gap, 1, "{case}"),
                _ => assert_eq!(bar, 2, "{case}"),
            }
            assert!(bar >= last.0 && gap >= last.1, "{case}: grows with the pane");
            last = (bar, gap);
        }
        assert!(if anchor == "gap" { last.0 > 2 } else { last.1 > 1 });
    }
}

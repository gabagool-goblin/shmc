use super::*;
use shmc::ui::scroll::{compute_offset, ScrollId};

#[tokio::test]
async fn queue_follow_pins_playing_until_browse() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..300).map(track).collect();
    app.current = Some(0);
    app.queue.state.select(Some(0));
    app.stopped = false;

    let _ = render(&mut app, 120, 40);
    let visible = app.scroll.visible(ScrollId::Queue);
    assert!(visible > 0 && visible < 300, "unexpected queue height {visible}");

    app.handle_msg(playlist_pos(80));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue.state.selected(), Some(80));
    assert_eq!(
        app.scroll.offset(ScrollId::Queue),
        80,
        "the playing row must be the first visible queue row"
    );
    let _ = render(&mut app, 120, 40);
    assert_eq!(
        app.scroll.offset(ScrollId::Queue),
        80,
        "top alignment must persist across repaints"
    );

    app.handle_msg(key('j'));
    assert_eq!(app.queue.state.selected(), Some(81));
    let _ = render(&mut app, 120, 40);
    assert_eq!(
        app.scroll.offset(ScrollId::Queue),
        80,
        "the first manual step must not scroll the pane"
    );
    app.handle_msg(playlist_pos(100));
    let _ = render(&mut app, 120, 40);
    assert_eq!(
        app.queue.state.selected(),
        Some(81),
        "a track change must not yank a browsing cursor"
    );
    assert_ne!(app.scroll.offset(ScrollId::Queue), 100);

    app.handle_msg(key('o'));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue.state.selected(), Some(100));
    assert_eq!(app.scroll.offset(ScrollId::Queue), 100);

    app.handle_msg(playlist_pos(110));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue.state.selected(), Some(110));
    assert_eq!(app.scroll.offset(ScrollId::Queue), 110);
}

/// The first step after `o` moves one row, not back by the scrolloff margin.
#[tokio::test]
async fn first_step_after_o_does_not_scroll() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..300).map(track).collect();
    app.current = Some(80);
    app.queue.state.select(Some(80));
    app.stopped = false;
    let _ = render(&mut app, 120, 40);
    let visible = app.scroll.visible(ScrollId::Queue);
    let margin = app.ui.scrolloff.min((visible.saturating_sub(1)) / 2);
    assert!(visible > margin + 1 && visible < 300, "unexpected queue height {visible}");

    app.handle_msg(key('o'));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue.state.selected(), Some(80));
    assert_eq!(app.scroll.offset(ScrollId::Queue), 80);

    let first_scroll = 80 + visible - margin;
    for row in 81..first_scroll {
        app.handle_msg(key('j'));
        let _ = render(&mut app, 120, 40);
        assert_eq!(app.queue.state.selected(), Some(row));
        assert_eq!(
            app.scroll.offset(ScrollId::Queue),
            80,
            "stepping to row {row} must not scroll the pane"
        );
    }

    // the margin scrolls one row, then normal scrolling resumes
    app.handle_msg(key('j'));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue.state.selected(), Some(first_scroll));
    assert_eq!(app.scroll.offset(ScrollId::Queue), 81);
}

/// A move before the `o` pin is drawn scrolls normally, auto-follow on or off.
#[tokio::test]
async fn move_after_o_does_not_repin_row() {
    for follow in [true, false] {
        let mut app = new_app().await;
        app.tab = Tab::Queue;
        app.queue.items = (0..300).map(track).collect();
        app.current = Some(80);
        app.queue.state.select(Some(80));
        app.stopped = false;
        app.ui.auto_follow = follow;
        let _ = render(&mut app, 120, 40);
        let visible = app.scroll.visible(ScrollId::Queue);
        let before = app.scroll.offset(ScrollId::Queue);

        app.handle_msg(key('o'));
        app.handle_msg(key('j'));
        let _ = render(&mut app, 120, 40);
        assert_eq!(app.queue.state.selected(), Some(81), "auto_follow {follow}");
        assert_eq!(
            app.scroll.offset(ScrollId::Queue),
            compute_offset(81, before, visible, 300, app.ui.scrolloff),
            "auto_follow {follow}: a move before the pin is drawn must not pin row 81 to the top"
        );
    }
}

#[tokio::test]
async fn queue_end_wrap_jumps_to_top() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..300).map(track).collect();
    app.current = Some(299);
    app.queue.state.select(Some(299));
    app.stopped = false;
    app.handle_msg(key('o'));
    let _ = render(&mut app, 120, 40);
    let visible = app.scroll.visible(ScrollId::Queue);
    assert_eq!(
        app.scroll.offset(ScrollId::Queue),
        300 - visible,
        "the pin clamps flush to the bottom"
    );

    app.handle_msg(key('j'));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue.state.selected(), Some(0));
    assert_eq!(app.scroll.offset(ScrollId::Queue), 0);
}

#[tokio::test]
async fn queue_follow_does_not_fight_the_filter() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..120).map(track).collect();
    app.current = Some(0);
    app.queue.state.select(Some(0));
    app.stopped = false;

    filter_queue(&mut app, "Starfall! 9");
    assert!(app.queue.filter.cursor_of(0).is_none(), "test filter must hide the playing track");

    app.handle_msg(playlist_pos(90));
    let _ = render(&mut app, 120, 40);
    assert_eq!(
        app.queue.state.selected(),
        Some(0),
        "auto-follow must leave a filtered queue alone"
    );

    filter_queue(&mut app, "Starfall! 90");
    app.handle_msg(key('o'));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue.state.selected(), Some(0));
    assert_eq!(app.scroll.offset(ScrollId::Queue), 0);
}

#[tokio::test]
async fn queue_gg_and_shift_g_reach_the_ends() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..10).map(track).collect();
    app.queue.state.select(Some(4));

    keys(&mut app, "G");
    assert_eq!(app.queue.state.selected(), Some(9));
    keys(&mut app, "gg");
    assert_eq!(app.queue.state.selected(), Some(0));
    assert_eq!(app.tab, Tab::Queue, "g no longer opens the Browser");
}

#[tokio::test]
async fn auto_follow_can_be_disabled() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..300).map(track).collect();
    app.current = Some(0);
    app.queue.state.select(Some(0));
    app.stopped = false;

    app.handle_msg(key('j'));
    assert!(app.status.contains("queue follow: paused"));
    app.handle_msg(key('j'));
    assert!(app.status.is_empty(), "an already-paused follow must not spam the status line");

    app.handle_msg(key('o'));
    assert_eq!(app.status, "queue follow: on");

    // with the option off, `o` jumps once but does not stay armed
    app.current = Some(80);
    app.ui.auto_follow = false;
    app.handle_msg(key('o'));
    assert_eq!(app.status, "queue follow: off");
    let _ = render(&mut app, 120, 40);
    let jump_offset = app.scroll.offset(ScrollId::Queue);
    assert_eq!(jump_offset, 80, "the one-shot jump still pins to the top");

    let _ = render(&mut app, 120, 40);
    assert_eq!(
        app.scroll.offset(ScrollId::Queue),
        jump_offset,
        "a repaint must not move the one-shot jump"
    );
    app.handle_msg(key('j'));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue.state.selected(), Some(81));
    assert_eq!(
        app.scroll.offset(ScrollId::Queue),
        jump_offset,
        "browsing from the one-shot jump must not scroll the pane"
    );

    let selected = app.queue.state.selected();
    app.handle_msg(playlist_pos(120));
    let _ = render(&mut app, 120, 40);
    assert_eq!(
        app.queue.state.selected(),
        selected,
        "disabled auto-follow must not move the cursor"
    );
}

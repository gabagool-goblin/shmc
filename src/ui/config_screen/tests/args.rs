use super::*;

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

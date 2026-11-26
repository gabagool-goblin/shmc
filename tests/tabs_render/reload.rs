//! A saved config applied to the running player.

use super::*;
use ratatui::style::Color;
use shmc::ui::app::reload::Reload;
use shmc::ui::theme;

/// A save sets the process-wide theme and look; this puts them back.
pub(super) struct AppearanceGuard(theme::Theme, theme::Look);

impl AppearanceGuard {
    pub(super) fn new() -> Self {
        AppearanceGuard(theme::theme(), theme::look())
    }
}

impl Drop for AppearanceGuard {
    fn drop(&mut self) {
        theme::set_theme(self.0);
        theme::set_look(self.1);
    }
}

pub(super) fn saved(text: &str, later: &[&'static str]) -> Msg {
    let cfg = toml::from_str(text).unwrap();
    Msg::ConfigRead(Box::new(Ok(Reload { cfg, later: later.to_vec() })))
}

#[tokio::test]
async fn a_save_changes_columns_colours_and_keys_at_once() {
    let mut app = new_app().await;
    let _appearance = AppearanceGuard::new();
    app.queue.items = (0..3).map(track).collect();
    app.queue.state.select(Some(0));
    assert!(!render(&mut app, 120, 30).contains("Year"));

    app.handle_msg(saved(
        "[ui]\nqueue_columns = [\"title\", \"year\", \"len\"]\n\
         [theme]\naccent = \"#123456\"\n\
         [keys.queue]\nremove = [\"X\"]\n",
        &[],
    ));
    assert_eq!(app.status, "settings applied");
    let screen = render(&mut app, 120, 30);
    assert!(screen.contains("Year"), "{screen}");
    assert_eq!(theme::accent_color(), Color::Rgb(0x12, 0x34, 0x56));
    app.handle_msg(key('v'));
    assert!(app.status.contains("X remove"), "{:?}", app.status);
}

#[tokio::test]
async fn settings_for_the_next_start_are_named_by_key() {
    let mut app = new_app().await;
    let _appearance = AppearanceGuard::new();
    app.handle_msg(saved(
        "[plex]\nserver = \"http://elsewhere:32400\"\ntoken = \"FAKE-RELOAD-TOKEN\"\n",
        &["plex.server", "plex.token"],
    ));
    assert_eq!(app.status, "settings applied; plex.server and plex.token on the next start");
}

#[tokio::test]
async fn a_file_that_does_not_parse_changes_nothing() {
    let mut app = new_app().await;
    let before = app.ui.scrolloff;
    let error = "config.toml does not parse (line 2): invalid type: string, expected usize";
    app.handle_msg(Msg::ConfigRead(Box::new(Err(error.into()))));
    assert!(app.status.ends_with("; settings unchanged"), "{}", app.status);
    assert_eq!(app.ui.scrolloff, before);
}

#[tokio::test]
async fn the_art_size_cap_stays_as_started() {
    let mut app = new_app().await;
    let _appearance = AppearanceGuard::new();
    let started = app.art_max_px;
    app.handle_msg(saved("[album_art]\nmax_size_px = [1234, 1234]\n", &["album_art.max_size_px"]));
    assert_eq!(app.art_max_px, started);
}

#[tokio::test]
async fn only_an_art_setting_redraws_the_cover() {
    let mut app = new_app().await;
    let _appearance = AppearanceGuard::new();
    app.handle_msg(saved("", &[]));
    let shown = Some(ratatui::layout::Rect::new(1, 1, 30, 15));
    app.art.shown_rect = shown;
    app.handle_msg(saved("[ui]\nscrolloff = 2\n", &[]));
    assert_eq!(app.art.shown_rect, shown, "an unrelated save resent the cover");
    app.handle_msg(saved("[ui]\nart_pct = 55\n", &[]));
    assert_eq!(app.art.shown_rect, None);
}

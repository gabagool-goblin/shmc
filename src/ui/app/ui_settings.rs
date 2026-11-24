use crate::config::{Config, QueueColumn, RatingStyle, UiConfig, VolumeDisplay};

/// With cava off the pane would be an empty box, so it gets no height.
pub fn player_ui(cfg: &Config) -> UiConfig {
    let mut ui = cfg.ui.clone();
    if !cfg.cava.enabled {
        ui.visualizer_pct = 0;
    }
    ui
}

/// The `[ui]` settings as the player uses them.
pub struct UiSettings {
    pub rating_style: RatingStyle,
    /// 0 = previews off.
    pub preview_tracks: usize,
    pub preview_debounce_ms: u64,
    pub confirm_playlist_over: usize,
    pub scrolloff: usize,
    pub center_cursor: bool,
    pub auto_follow: bool,
    pub icons_ascii: bool,
    pub volume_display: VolumeDisplay,
    pub mode_words: bool,
    pub header_compact: bool,
    pub art_pct: u16,
    pub art_square: bool,
    pub art_metadata: bool,
    pub art_pad_px: u16,
    /// % of the queue column, min 8 rows; 0 hides.
    pub visualizer_pct: u16,
    pub album_sort: String,
    /// 0 = `<` always goes back.
    pub prev_restart_secs: u64,
    pub visualizer_mode: String,
    pub visualizer_bars: u16,
    pub visualizer_anchor: String,
    pub visualizer_bar_width: u16,
    pub visualizer_bar_gap: u16,
    pub numbers_playlists: bool,
    /// Title, Artist, Album; 0 hides one.
    pub column_weights: [u32; 3],
    pub queue_columns: Vec<QueueColumn>,
}

impl UiSettings {
    pub fn new(ui: &UiConfig) -> Self {
        Self {
            rating_style: ui.rating_mode(),
            preview_tracks: ui.preview_tracks,
            preview_debounce_ms: ui.preview_debounce_ms,
            confirm_playlist_over: ui.confirm_playlist_over,
            scrolloff: ui.scrolloff,
            center_cursor: ui.center_cursor,
            auto_follow: ui.auto_follow,
            icons_ascii: ui.ascii_icons(),
            volume_display: ui.volume_mode(),
            mode_words: ui.mode_words(),
            header_compact: ui.compact_header(),
            art_pct: ui.art_pct,
            art_square: ui.art_square,
            art_metadata: ui.art_metadata,
            art_pad_px: ui.art_pad_px,
            visualizer_pct: ui.visualizer_pct,
            album_sort: ui.album_sort.clone(),
            prev_restart_secs: ui.prev_restart_secs,
            visualizer_mode: ui.visualizer_mode.clone(),
            visualizer_bars: ui.visualizer_bars,
            visualizer_anchor: ui.visualizer_anchor.clone(),
            visualizer_bar_width: ui.visualizer_bar_width,
            visualizer_bar_gap: ui.visualizer_bar_gap,
            numbers_playlists: ui.numbers_playlists(),
            column_weights: ui.effective_column_weights(),
            queue_columns: ui.resolved_queue_columns(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cava_off_gives_visualizer_no_height() {
        let mut cfg = Config::default();
        assert!(cfg.cava.enabled && cfg.ui.visualizer_pct > 0);
        assert_eq!(player_ui(&cfg).visualizer_pct, cfg.ui.visualizer_pct);
        cfg.cava.enabled = false;
        assert_eq!(player_ui(&cfg).visualizer_pct, 0);
    }
}

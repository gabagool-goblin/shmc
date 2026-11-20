use super::widgets::{cell, title_spans};
use crate::config::QueueColumn;
use crate::ui::filter::ListFilter;
use crate::ui::scrollbar::list_split;
use crate::ui::theme;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row};
use ratatui::Frame;

fn digits(n: usize) -> u16 {
    theme::digits(n)
}

/// The `#` column is wide enough for every count under a thousand, so an
/// ordinary album never moves the columns beside it.
const TRACK_NUMBER_MIN_WIDTH: u16 = 3;

/// Width of the `#` column: what the largest possible number needs, with a
/// floor so a 9-track and a 100-track album start Title at the same offset.
pub(super) fn track_number_width(bound: usize) -> u16 {
    digits(bound).max(TRACK_NUMBER_MIN_WIDTH)
}

/// The narrowest a weighted column is drawn.
const WEIGHTED_COL_MIN: u16 = 6;

/// At the default weights each table keeps its own `defaults`, which plain
/// weights cannot reproduce.
const DEFAULT_WEIGHTS: [u32; 3] = [30, 24, 46];

/// Which of a track table's text columns (Title, Artist, Album) exist and how
/// wide; constraints, headers and cells all read it so they agree.
#[derive(Clone, Copy)]
pub(super) struct TrackCols {
    /// `ui.column_weights`: Title, Artist, Album.
    weights: [u32; 3],
    /// Each table column's index into `weights` (Search puts Artist first).
    order: [usize; 3],
    defaults: [Constraint; 3],
}

impl TrackCols {
    pub(super) fn new(weights: [u32; 3], order: [usize; 3], defaults: [Constraint; 3]) -> Self {
        Self { weights, order, defaults }
    }

    /// Title, Artist, Album order: Queue, Playlists and the album drill-in.
    pub(super) fn plain(weights: [u32; 3], defaults: [Constraint; 3]) -> Self {
        Self::new(weights, [0, 1, 2], defaults)
    }

    fn weight(&self, col: usize) -> u32 {
        self.weights[self.order[col]]
    }

    /// Table column `col`'s position among the shown ones, None when hidden.
    pub(super) fn slot(&self, col: usize) -> Option<usize> {
        (self.weight(col) > 0).then(|| (0..col).filter(|c| self.weight(*c) > 0).count())
    }

    pub(super) fn constraints(&self) -> Vec<Constraint> {
        if self.weights == DEFAULT_WEIGHTS {
            return self.defaults.to_vec();
        }
        (0..3)
            .filter(|c| self.weight(*c) > 0)
            .map(|c| Constraint::Fill(self.weight(c).min(u16::MAX as u32) as u16))
            .collect()
    }

    pub(super) fn headers(&self, labels: [&'static str; 3]) -> Vec<HeaderSpec> {
        (0..3).filter(|c| self.weight(*c) > 0).map(|c| HeaderSpec::left(labels[c])).collect()
    }
}

pub(super) fn track_cols(num_w: Option<u16>, len_w: u16, tc: &TrackCols) -> Vec<Constraint> {
    let mut v = Vec::new();
    if let Some(w) = num_w {
        v.push(Constraint::Length(w));
    }
    v.extend(tc.constraints());
    v.push(Constraint::Length(len_w));
    v
}

/// Album's floor at the default weights; a higher one squeezes Title instead.
const ALBUM_COL_MIN: u16 = 6;

pub(super) const TRACK_LEGACY: [Constraint; 3] =
    [Constraint::Percentage(30), Constraint::Percentage(24), Constraint::Min(ALBUM_COL_MIN)];

pub(super) const SEARCH_HIT_COLS: [Constraint; 3] =
    [Constraint::Percentage(24), Constraint::Percentage(36), Constraint::Min(ALBUM_COL_MIN)];

pub(super) const SEARCH_TRACK_COLS: [Constraint; 3] =
    [Constraint::Percentage(34), Constraint::Percentage(24), Constraint::Min(ALBUM_COL_MIN)];

/// Width of the Len column: the longest track's time, at least `LEN_COL_MIN`.
pub(super) fn len_col_width(longest_ms: Option<u64>) -> u16 {
    let w = longest_ms.map_or(0, |ms| theme::format_time(ms as f64 / 1000.0).len());
    (w as u16).max(theme::LEN_COL_MIN)
}

pub(super) fn track_headers(num_w: Option<u16>, tc: &TrackCols) -> Vec<HeaderSpec> {
    let mut v = Vec::new();
    if num_w.is_some() {
        v.push(HeaderSpec::right("#"));
    }
    v.extend(tc.headers(["Title", "Artist", "Album"]));
    v.push(HeaderSpec::right("Len"));
    v
}

/// The Queue's columns in display order, with the widths of the fixed ones;
/// constraints, headers and cells all read it so they agree.
pub(super) struct QueueCols {
    cols: Vec<QueueColumn>,
    weights: [u32; 3],
    len_w: u16,
    track_w: u16,
    year_w: u16,
    codec_w: u16,
}

/// Codec values longer than this are ellipsized.
const CODEC_COL_MAX: u16 = 8;

impl QueueCols {
    /// `listed` is `App::queue_columns`; on a table narrower than the rest
    /// can fit, codec, year and track go first.
    pub(super) fn new(
        listed: &[QueueColumn],
        weights: [u32; 3],
        tracks: &[crate::plex::Track],
        width: u16,
    ) -> Self {
        let digits = |n: Option<u32>| n.map_or(0, |n| n.to_string().len() as u16);
        let codec = |t: &crate::plex::Track| {
            t.audio.codec.as_ref().map_or(0, |c| theme::str_width(c) as u16)
        };
        let mut qc = Self {
            cols: listed
                .iter()
                .copied()
                .filter(|c| c.text_index().is_none_or(|i| weights[i] > 0))
                .collect(),
            weights,
            len_w: len_col_width(tracks.iter().filter_map(|t| t.duration_ms).max()),
            track_w: tracks.iter().map(|t| digits(t.track_no)).max().unwrap_or(0).max(1),
            year_w: tracks.iter().map(|t| digits(t.year)).max().unwrap_or(0).max(4),
            codec_w: tracks.iter().map(codec).max().unwrap_or(0).clamp(5, CODEC_COL_MAX),
        };
        for extra in [QueueColumn::Codec, QueueColumn::Year, QueueColumn::Track] {
            if qc.min_width() <= width as u32 {
                break;
            }
            qc.cols.retain(|c| *c != extra);
        }
        qc
    }

    fn fixed_width(&self, col: QueueColumn) -> Option<u16> {
        match col {
            QueueColumn::Len => Some(self.len_w),
            QueueColumn::Track => Some(self.track_w),
            QueueColumn::Year => Some(self.year_w),
            QueueColumn::Codec => Some(self.codec_w),
            _ => None,
        }
    }

    fn weight(&self, col: QueueColumn) -> u32 {
        col.text_index().map_or(0, |i| self.weights[i])
    }

    fn min_width(&self) -> u32 {
        let width: u32 =
            self.cols.iter().map(|c| self.fixed_width(*c).unwrap_or(WEIGHTED_COL_MIN) as u32).sum();
        width + self.cols.len().saturating_sub(1) as u32
    }

    pub(super) fn constraints(&self) -> Vec<Constraint> {
        if self.cols == QueueColumn::DEFAULT && self.weights == DEFAULT_WEIGHTS {
            let mut v = TRACK_LEGACY.to_vec();
            v.push(Constraint::Length(self.len_w));
            return v;
        }
        self.cols
            .iter()
            .map(|c| match self.fixed_width(*c) {
                Some(w) => Constraint::Length(w),
                None => Constraint::Fill(self.weight(*c).min(u16::MAX as u32) as u16),
            })
            .collect()
    }

    pub(super) fn headers(&self) -> Vec<HeaderSpec> {
        self.cols
            .iter()
            .map(|c| match c {
                QueueColumn::Title => HeaderSpec::left("Title"),
                QueueColumn::Artist => HeaderSpec::left("Artist"),
                QueueColumn::Album => HeaderSpec::left("Album"),
                QueueColumn::Len => HeaderSpec::right("Len"),
                QueueColumn::Track => HeaderSpec::right("#"),
                QueueColumn::Year => HeaderSpec::right("Year"),
                QueueColumn::Codec => HeaderSpec::left("Codec"),
            })
            .collect()
    }

    /// `col_w` is the laid-out width of each column of `cols`.
    pub(super) fn cells(
        &self,
        t: &crate::plex::Track,
        base: Style,
        playing: bool,
        col_w: &[u16],
        needle: &str,
        rating: Option<&str>,
    ) -> Vec<Cell<'static>> {
        let dim = if playing { base } else { theme::dim() };
        let right = |n: Option<u32>| {
            Cell::from(Line::from(n.map(|n| n.to_string()).unwrap_or_default()).right_aligned())
                .style(dim)
        };
        self.cols
            .iter()
            .zip(col_w)
            .map(|(c, w)| match c {
                QueueColumn::Title => {
                    Cell::from(Line::from(title_spans(&t.title, rating, *w, needle, base, dim)))
                }
                QueueColumn::Artist => cell(&t.artist, *w, needle, base),
                QueueColumn::Album => cell(&t.album, *w, needle, base),
                QueueColumn::Len => len_cell(t, dim),
                QueueColumn::Track => right(t.track_no),
                QueueColumn::Year => right(t.year),
                QueueColumn::Codec => {
                    let codec = t.audio.codec.as_deref().unwrap_or("").to_uppercase();
                    Cell::from(theme::ellipsize(&codec, *w)).style(dim)
                }
            })
            .collect()
    }
}

/// Rows of a track table; `cells` makes one row's cells from the track, its
/// index in `tracks` and the row's base style.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_track_rows(
    tracks: &[crate::plex::Track],
    filter: &ListFilter,
    offset: usize,
    visible: usize,
    total: usize,
    current_key: Option<&str>,
    selected: Option<(usize, usize)>,
    cells: impl Fn(&crate::plex::Track, usize, Style, bool) -> Vec<Cell<'static>>,
) -> Vec<Row<'static>> {
    let raw = tracks.len();
    let end = (offset + visible).min(total);
    let mut rows: Vec<Row> = Vec::with_capacity(end.saturating_sub(offset));
    for fi in offset..end {
        let real = filter.real(fi, raw);
        let Some(t) = tracks.get(real) else { continue };
        let playing = current_key == Some(t.rating_key.as_str());
        let base =
            if playing { theme::accent().add_modifier(Modifier::BOLD) } else { theme::text() };
        let row = Row::new(cells(t, real, base, playing));
        rows.push(match selected {
            Some((lo, hi)) if (lo..=hi).contains(&fi) => row.style(theme::current_unfocused()),
            _ => row,
        });
    }
    rows
}

/// The cells of a Playlists or Search track row: optional number, the shown
/// text columns, Len.
#[allow(clippy::too_many_arguments)]
pub(super) fn plain_track_cells(
    t: &crate::plex::Track,
    real: usize,
    base: Style,
    playing: bool,
    col_w: &[u16],
    num_w: Option<u16>,
    tc: &TrackCols,
    needle: &str,
    rating: Option<&str>,
) -> Vec<Cell<'static>> {
    let base_idx = if num_w.is_some() { 1 } else { 0 };
    let dim = if playing { base } else { theme::dim() };
    let mut cells: Vec<Cell> = Vec::new();
    if num_w.is_some() {
        cells.push(Cell::from(Line::from(Span::styled(
            format!("{:>w$}", real + 1, w = col_w[0] as usize),
            dim,
        ))));
    }
    for (col, text) in [&t.title, &t.artist, &t.album].into_iter().enumerate() {
        if let Some(slot) = tc.slot(col) {
            let w = col_w[base_idx + slot];
            cells.push(if col == 0 {
                Cell::from(Line::from(title_spans(text, rating, w, needle, base, dim)))
            } else {
                cell(text, w, needle, base)
            });
        }
    }
    cells.push(len_cell(t, dim));
    cells
}

fn len_cell(t: &crate::plex::Track, style: Style) -> Cell<'static> {
    let dur = t.duration_ms.map(|ms| theme::format_time(ms as f64 / 1000.0)).unwrap_or_default();
    Cell::from(Line::from(dur).right_aligned()).style(style)
}

pub(super) struct HeaderSpec {
    label: &'static str,
    right: bool,
}

impl HeaderSpec {
    pub(super) fn left(label: &'static str) -> Self {
        Self { label, right: false }
    }
    pub(super) fn right(label: &'static str) -> Self {
        Self { label, right: true }
    }
}

/// A table's header, rule and rows area. Column widths are computed once here
/// so header text and row ellipsizing match the Table's own layout.
pub(super) struct TableFrame {
    pub(super) rows: Rect,
    pub(super) col_w: Vec<u16>,
    pub(super) bar: Option<Rect>,
}

pub(super) fn table_frame(
    f: &mut Frame,
    pane: Rect,
    total: usize,
    cols: &[Constraint],
    headers: Option<&[HeaderSpec]>,
    indicator: Option<String>,
) -> TableFrame {
    let mut body = pane;
    if let Some(txt) = indicator {
        if body.height > 1 {
            let r = Rect { x: body.x, y: body.y, width: body.width, height: 1 };
            f.render_widget(Paragraph::new(Line::from(Span::styled(txt, theme::dim()))), r);
            body.y += 1;
            body.height -= 1;
        }
    }
    let header_rows = if headers.is_some() { 2 } else { 0 };
    let visible = body.height.saturating_sub(header_rows) as usize;
    let (content, bar) = list_split(body, total, visible);

    let col_w = layout_widths(cols, content.width);

    let mut y = content.y;
    if let Some(hs) = headers {
        if content.height >= 1 {
            let mut spans: Vec<Span> = Vec::new();
            for (i, h) in hs.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::raw(" "));
                }
                let w = col_w.get(i).copied().unwrap_or(0);
                // not dim: too faint for a header
                spans.push(Span::styled(header_cell(h.label, w, h.right), theme::text()));
            }
            let r = Rect { x: content.x, y, width: content.width, height: 1 };
            f.render_widget(Paragraph::new(Line::from(spans)), r);
            y += 1;
        }
        if content.height >= 2 {
            let r = Rect { x: content.x, y, width: content.width, height: 1 };
            if theme::spacing().borders != theme::BorderStyle::None {
                f.render_widget(
                    Paragraph::new("─".repeat(content.width as usize)).style(theme::dim()),
                    r,
                );
            }
            y += 1;
        }
    }

    let rows = Rect {
        x: content.x,
        y,
        width: content.width,
        height: content.height.saturating_sub(header_rows),
    };
    TableFrame { rows, col_w, bar }
}

/// Column widths as the Table lays them out (1-cell spacing).
fn layout_widths(cols: &[Constraint], width: u16) -> Vec<u16> {
    if width == 0 {
        return vec![0; cols.len()];
    }
    if cols.iter().any(|c| matches!(c, Constraint::Fill(_))) {
        return weighted_widths(cols, width);
    }
    let probe = Rect { x: 0, y: 0, width, height: 1 };
    Layout::horizontal(cols.to_vec()).spacing(1).split(probe).iter().map(|r| r.width).collect()
}

/// Widths for `Length`s and `Fill` weights: the fills share what is left, each
/// keeping `WEIGHTED_COL_MIN` when there is room.
fn weighted_widths(cols: &[Constraint], width: u16) -> Vec<u16> {
    let fixed: u32 = cols
        .iter()
        .map(|c| match c {
            Constraint::Length(l) => *l as u32,
            _ => 0,
        })
        .sum::<u32>()
        + cols.len().saturating_sub(1) as u32;
    let room = (width as u32).saturating_sub(fixed);
    let weights: Vec<u64> =
        cols.iter().map(|c| if let Constraint::Fill(w) = c { *w as u64 } else { 0 }).collect();
    let fills = weights.iter().filter(|w| **w > 0).count() as u32;
    let min = if room >= WEIGHTED_COL_MIN as u32 * fills { WEIGHTED_COL_MIN as u64 } else { 0 };

    let mut pinned = vec![false; cols.len()];
    loop {
        let free: u64 =
            (0..cols.len()).filter(|i| weights[*i] > 0 && !pinned[*i]).map(|i| weights[i]).sum();
        let pins = pinned.iter().filter(|p| **p).count() as u64;
        let space = (room as u64).saturating_sub(pins * min);
        let low = (0..cols.len())
            .find(|&i| weights[i] > 0 && !pinned[i] && space * weights[i] / free.max(1) < min);
        match low {
            Some(i) => pinned[i] = true,
            None => break,
        }
    }
    let free: u64 =
        (0..cols.len()).filter(|i| weights[*i] > 0 && !pinned[*i]).map(|i| weights[i]).sum();
    let pins = pinned.iter().filter(|p| **p).count() as u64;
    let space = (room as u64).saturating_sub(pins * min);
    let mut out: Vec<u16> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| match c {
            Constraint::Length(l) => (*l).min(width),
            _ if pinned[i] => min as u16,
            _ => (space * weights[i] / free.max(1)) as u16,
        })
        .collect();
    // Cells lost to rounding go to the widest shares first.
    let left = space.saturating_sub(
        (0..cols.len())
            .filter(|i| weights[*i] > 0 && !pinned[*i])
            .map(|i| out[i] as u64)
            .sum::<u64>(),
    );
    let mut by_share: Vec<usize> =
        (0..cols.len()).filter(|i| weights[*i] > 0 && !pinned[*i]).collect();
    by_share.sort_by_key(|i| std::cmp::Reverse(weights[*i]));
    for i in by_share.into_iter().cycle().take(left as usize) {
        out[i] += 1;
    }
    out
}

fn header_cell(s: &str, w: u16, right: bool) -> String {
    let t = theme::ellipsize(s, w);
    let tw = theme::str_width(&t) as u16;
    let pad = w.saturating_sub(tw) as usize;
    if right {
        format!("{}{}", " ".repeat(pad), t)
    } else {
        format!("{}{}", t, " ".repeat(pad))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_weight_keeps_column_readable() {
        let cols = |w: [u16; 3]| {
            let mut v: Vec<Constraint> = w.iter().map(|w| Constraint::Fill(*w)).collect();
            v.push(Constraint::Length(5));
            v
        };
        for width in [30u16, 61, 100, 157] {
            let w = weighted_widths(&cols([1, 1, 98]), width);
            assert!(w[0] >= WEIGHTED_COL_MIN && w[1] >= WEIGHTED_COL_MIN, "{width}: {w:?}");
            // the gaps and the Len column are not shared out
            assert_eq!(w.iter().sum::<u16>() + 3, width, "{width}: {w:?}");
        }
        // too narrow for the minimum
        let w = weighted_widths(&cols([1, 1, 98]), 14);
        assert_eq!(w.iter().sum::<u16>() + 3, 14, "{w:?}");
    }

    /// Narrow default tables squeeze Album before Title and Artist.
    #[test]
    fn narrow_default_tables_keep_title_before_album() {
        for (defaults, title) in [(TRACK_LEGACY, 0), (SEARCH_HIT_COLS, 1), (SEARCH_TRACK_COLS, 0)] {
            let mut cols = defaults.to_vec();
            cols.push(Constraint::Length(theme::LEN_COL_MIN));
            for width in 20u16..=120 {
                let w = layout_widths(&cols, width);
                let squeezed = (0..2).any(|c| w[c] < ALBUM_COL_MIN);
                assert!(!squeezed || w[2] <= ALBUM_COL_MIN, "{width}: {w:?}");
                assert!(w[title] > 1, "{width}: {w:?}");
            }
        }
    }

    /// `m:ss` and `mm:ss` share a width; an hour-long track widens it just enough.
    #[test]
    fn len_column_as_wide_as_its_times() {
        assert_eq!(len_col_width(None), theme::LEN_COL_MIN);
        assert_eq!(len_col_width(Some(257_293)), 5); // 4:17
        assert_eq!(len_col_width(Some(2_520_000)), 5); // 42:00
        assert_eq!(len_col_width(Some(3_725_000)), 7); // 1:02:05
        assert_eq!(len_col_width(Some(36_000_000)), 8); // 10:00:00
    }
}

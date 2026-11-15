use super::widgets::clean;
use super::{draw_block_as, draw_vdivider};
use crate::config::VolumeDisplay;
use crate::ui::app::{App, Tab};
use crate::ui::icons;
use crate::ui::theme;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// The volume meter's cells: always eight, whatever its glyphs.
pub(crate) const METER_CELLS: usize = 8;

/// Meter cells to light for a volume percentage.
pub(crate) fn volume_fill(volume: f64) -> usize {
    let cells = METER_CELLS as f64;
    ((volume / 100.0) * cells).round().clamp(0.0, cells) as usize
}

/// The meter as spans, `filled` cells lit. A ramp theme has cell `i` as its
/// `i`th glyph lit or not; a pair has one glyph for lit and one for unlit.
pub(crate) fn volume_meter(filled: usize) -> Vec<Span<'static>> {
    let meter = theme::symbols().volume_meter;
    (0..METER_CELLS)
        .map(|i| {
            let lit = i < filled;
            let glyph = match meter {
                theme::Meter::Ramp(ramp) => ramp[i],
                theme::Meter::Pair(on, off) => {
                    if lit {
                        on
                    } else {
                        off
                    }
                }
            };
            Span::styled(glyph.to_string(), if lit { theme::accent() } else { theme::dim() })
        })
        .collect()
}

/// The header's volume: the meter, the percentage or both, as `display` says.
/// The config screen's preview draws through here too.
pub(crate) fn volume_spans(volume: f64, display: VolumeDisplay) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled("Volume: ", theme::accent())];
    let pct = volume.round() as i32;
    if display == VolumeDisplay::Percent {
        spans.push(Span::styled(format!("{pct}%"), theme::accent().add_modifier(Modifier::BOLD)));
        return spans;
    }
    spans.extend(volume_meter(volume_fill(volume)));
    if display == VolumeDisplay::Meter {
        return spans;
    }
    // Fixed width, or "100%" shifts the right-aligned line by a cell.
    spans.push(Span::styled(format!(" {pct:>3}%"), theme::accent().add_modifier(Modifier::BOLD)));
    spans
}

/// The mode flags as words, widest spelling first. Shuffle and Single collide
/// on their first letter, so the letters are R Z S C (Z is the shuffle key).
pub(crate) const MODE_SPELLINGS: [[&str; 4]; 3] = [
    ["Repeat", "Shuffle", "Single", "Consume"],
    ["Rep", "Shu", "Sin", "Con"],
    ["R", "Z", "S", "C"],
];

/// The four words, separators and a gap before the state on the left.
pub(crate) const MODE_WORDS_W: u16 = 35;

/// Width of both side columns, shared so the title stays centred. Fits
/// "0:42 / 4:17 (FLAC 44.1k/16)"; the flags shorten before it passes a third.
pub(crate) fn header_side_width(inner_w: u16, words: bool) -> u16 {
    let third = inner_w / 3;
    let pct = (inner_w as u32 * 15 / 100) as u16;
    let side = pct.max(27);
    if words && side.max(MODE_WORDS_W) <= third {
        return side.max(MODE_WORDS_W);
    }
    side.min(third)
}

/// The icons, or in words mode the widest spelling that fits `room`.
pub(crate) fn mode_flag_labels(words: bool, ascii: bool, room: usize) -> [&'static str; 4] {
    if !words {
        let s = icons::status(ascii);
        return [s.repeat, s.random, s.single, s.consume];
    }
    let width = |l: &[&str; 4]| l.iter().map(|w| w.chars().count()).sum::<usize>() + 3;
    MODE_SPELLINGS.into_iter().find(|l| width(l) <= room).unwrap_or(MODE_SPELLINGS[2])
}

/// The four flags as one row: on is accent and bold, off is dim.
pub(crate) fn mode_flag_spans(labels: [&'static str; 4], on: [bool; 4]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            *label,
            if on[i] { theme::accent().add_modifier(Modifier::BOLD) } else { theme::dim() },
        ));
    }
    spans
}

/// Audio info as (long, compact). Lossy tracks show the bitrate and their
/// compact form drops the rate: `44.1k/320k` would read as rate/depth.
fn format_audio(audio: &crate::plex::AudioInfo) -> (String, String) {
    let codec = audio.codec.as_deref().map(|c| c.to_ascii_uppercase());
    let rate = audio.sample_rate_hz.map(format_rate);
    let same = |s: String| (s.clone(), s);
    let with = |c: &Option<String>, rest: String| match c {
        Some(c) => format!("{c} {rest}"),
        None => rest,
    };
    match (&codec, rate, audio.bit_depth, audio.bitrate_kbps) {
        (_, Some(r), Some(d), _) => {
            (with(&codec, format!("{r}Hz/{d}bit")), with(&codec, format!("{r}/{d}")))
        }
        (Some(_), None, Some(d), _) => same(with(&codec, format!("{d}bit"))),
        (_, Some(r), None, Some(b)) => {
            (with(&codec, format!("{b}kbps/{r}Hz")), with(&codec, format!("{b}k")))
        }
        (_, None, None, Some(b)) => {
            (with(&codec, format!("{b}kbps")), with(&codec, format!("{b}k")))
        }
        (_, Some(r), None, None) => (with(&codec, format!("{r}Hz")), with(&codec, r)),
        (Some(c), None, None, None) => same(c.clone()),
        _ => (String::new(), String::new()),
    }
}

/// `0:42 / 4:17` plus the longest audio form that fits `width`; the time alone
/// is ellipsized if it does not fit.
fn header_times(times: &str, long: &str, compact: &str, width: u16) -> String {
    let forms = [long, compact].map(|info| {
        if info.is_empty() {
            String::new()
        } else {
            format!("{times} ({info})")
        }
    });
    for f in forms.iter().filter(|f| !f.is_empty()) {
        if theme::str_width(f) <= width as usize {
            return f.clone();
        }
    }
    theme::ellipsize(times, width)
}

fn format_rate(hz: u32) -> String {
    let khz = hz as f64 / 1000.0;
    let s = format!("{khz:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    format!("{s}k")
}

/// What the header shows, as plain data: the player fills it from the app and
/// the config screen's preview from placeholders, and both draw through here.
pub(crate) struct HeaderInfo {
    pub stopped: bool,
    pub paused: bool,
    /// Title, artist, album.
    pub track: Option<(String, String, String)>,
    pub times: String,
    /// Long and compact audio forms.
    pub audio: (String, String),
    pub volume: f64,
    pub volume_display: VolumeDisplay,
    pub mode_words: bool,
    pub ascii: bool,
    /// Repeat, shuffle, single, consume.
    pub flags: [bool; 4],
    pub sleep: Option<String>,
}

pub(super) fn header_info(app: &App) -> HeaderInfo {
    let (mut long, mut compact) = format_audio(&app.audio);
    if long.is_empty() {
        // No Plex stream details: fall back to mpv's live bitrate.
        let kbps = (app.bitrate_bps / 1000.0).round();
        if kbps >= 1.0 {
            long = format!("{} kbps", kbps as i64);
            compact = long.clone();
        }
    }
    HeaderInfo {
        stopped: app.stopped,
        paused: app.paused,
        track: app.current_track().map(|t| (clean(&t.title), clean(&t.artist), clean(&t.album))),
        times: format!(
            "{} / {}",
            theme::format_time(app.position),
            theme::format_time(app.duration)
        ),
        audio: (long, compact),
        volume: app.volume,
        volume_display: app.volume_display,
        mode_words: app.mode_words,
        ascii: app.icons_ascii,
        flags: [app.loop_playlist, app.random, app.loop_file, app.consume],
        sleep: app.sleep_timer_label(),
    }
}

/// The header block: two rows, boxed or not by `style`.
pub(crate) fn draw_header_block(
    f: &mut Frame,
    area: Rect,
    style: theme::BorderStyle,
    info: &HeaderInfo,
) {
    let inner = draw_block_as(f, area, style, None);
    if inner.width < 16 || inner.height == 0 {
        return;
    }
    let has_row2 = inner.height >= 2;

    let side_w = header_side_width(inner.width, info.mode_words);
    let (left_w, right_w) = (side_w, side_w);
    let center_w = inner.width.saturating_sub(left_w + right_w + 2);

    let left = Rect { x: inner.x, y: inner.y, width: left_w, height: 1 };
    let center = Rect { x: inner.x + left_w + 1, y: inner.y, width: center_w, height: 1 };
    let right =
        Rect { x: inner.x + left_w + 1 + center_w + 1, y: inner.y, width: right_w, height: 1 };

    let (state_str, state_style) = if info.stopped {
        ("[Stopped]", theme::dim())
    } else if info.paused {
        ("[Paused]", theme::secondary())
    } else {
        ("[Playing]", theme::secondary())
    };
    f.render_widget(Paragraph::new(state_str).style(state_style), left);

    let title_line = match &info.track {
        Some((title, _, _)) => Line::from(Span::styled(
            theme::ellipsize(title, center.width),
            theme::text().add_modifier(Modifier::BOLD),
        )),
        None => Line::from(Span::styled("No Song", theme::dim())),
    };
    f.render_widget(Paragraph::new(title_line).alignment(Alignment::Center), center);

    let vol = volume_spans(info.volume, info.volume_display);
    f.render_widget(Paragraph::new(Line::from(vol)).alignment(Alignment::Right), right);

    if has_row2 {
        let y = inner.y + 1;
        let left2 = Rect { y, ..left };
        let center2 = Rect { y, ..center };
        let right2 = Rect { y, ..right };

        f.render_widget(
            Paragraph::new(header_times(&info.times, &info.audio.0, &info.audio.1, left2.width))
                .style(theme::text()),
            left2,
        );

        let credit = match &info.track {
            Some((_, artist, album)) => {
                let mut spans =
                    vec![Span::styled(theme::ellipsize(artist, center2.width), theme::secondary())];
                if !album.is_empty() {
                    spans.push(Span::styled(" - ", theme::text()));
                    spans.push(Span::styled(theme::ellipsize(album, center2.width), theme::text()));
                }
                Line::from(spans)
            }
            None => Line::from(""),
        };
        f.render_widget(Paragraph::new(credit).alignment(Alignment::Center), center2);

        let mut icon_spans: Vec<Span> = Vec::new();
        if let Some(label) = &info.sleep {
            icon_spans.push(Span::styled(label.clone(), theme::secondary()));
            icon_spans.push(Span::raw(" "));
        }
        // The words step down to what the sleep label leaves.
        let lead: usize = icon_spans.iter().map(|s| s.content.chars().count()).sum();
        let room = (right2.width as usize).saturating_sub(lead);
        let labels = mode_flag_labels(info.mode_words, info.ascii, room);
        icon_spans.extend(mode_flag_spans(labels, info.flags));
        f.render_widget(Paragraph::new(Line::from(icon_spans)).alignment(Alignment::Right), right2);
    }

    draw_vdivider(f, inner.x + left_w, inner);
    draw_vdivider(f, inner.x + left_w + 1 + center_w, inner);
}

pub(super) fn tab_name(t: Tab) -> &'static str {
    match t {
        Tab::Queue => "Queue",
        Tab::Browser => "Browser",
        Tab::Playlists => "Playlists",
        Tab::Search => "Search",
        Tab::Help => "Help",
    }
}

fn tab_spans(names: &[&str], active: usize) -> Vec<Span<'static>> {
    names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let style = if i == active { theme::tab_active() } else { theme::tab_inactive() };
            Span::styled(format!(" {name} "), style)
        })
        .collect()
}

/// The config screen's tab bar: always boxed, whatever `theme.borders` says.
pub fn draw_tabs(f: &mut Frame, area: Rect, names: &[&str], active: usize) {
    draw_tab_strip(f, area, theme::BorderStyle::Box, names, active);
}

pub(super) fn draw_tab_strip(
    f: &mut Frame,
    area: Rect,
    style: theme::BorderStyle,
    names: &[&str],
    active: usize,
) {
    let inner = draw_block_as(f, area, style, None);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    f.render_widget(
        Paragraph::new(Line::from(tab_spans(names, active))).alignment(Alignment::Center),
        inner,
    );
}

/// Every spelling of the flags, widest first.
fn flag_label_forms(words: bool, ascii: bool) -> Vec<[&'static str; 4]> {
    let mut forms: Vec<[&'static str; 4]> = Vec::new();
    let mut room = usize::MAX;
    loop {
        let labels = mode_flag_labels(words, ascii, room);
        if forms.last() == Some(&labels) {
            return forms;
        }
        let width = labels.iter().map(|l| l.chars().count()).sum::<usize>() + 3;
        forms.push(labels);
        room = width - 1;
    }
}

/// The compact header's right side, widest first: the flags shorten before the
/// volume drops its meter, then its label.
fn compact_right_forms(info: &HeaderInfo) -> Vec<(Vec<Span<'static>>, u16)> {
    let mut volumes = vec![volume_spans(info.volume, info.volume_display)];
    if info.volume_display != VolumeDisplay::Percent {
        volumes.push(volume_spans(info.volume, VolumeDisplay::Percent));
    }
    volumes.push(vec![Span::styled(
        format!("{}%", info.volume.round() as i32),
        theme::accent().add_modifier(Modifier::BOLD),
    )]);
    let labels = flag_label_forms(info.mode_words, info.ascii);
    let mut forms = Vec::new();
    for volume in volumes {
        for l in &labels {
            let mut spans = volume.clone();
            spans.push(Span::raw("  "));
            spans.extend(mode_flag_spans(*l, info.flags));
            let width = spans.iter().map(|s| theme::str_width(&s.content)).sum::<usize>() as u16;
            forms.push((spans, width));
        }
    }
    forms
}

/// The compact header: sleep timer, tabs and right side on one row. The tabs
/// are never cut; the right side goes first, the sleep timer last.
pub(crate) fn draw_tab_row(
    f: &mut Frame,
    area: Rect,
    style: theme::BorderStyle,
    names: &[&str],
    active: usize,
    info: &HeaderInfo,
) {
    let inner = draw_block_as(f, area, style, None);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let tabs = tab_spans(names, active);
    let tabs_w = tabs.iter().map(|s| theme::str_width(&s.content)).sum::<usize>() as u16;
    let sleep_w = info.sleep.as_deref().map_or(0, |s| theme::str_width(s) as u16 + 1);

    let forms = compact_right_forms(info);
    let mut tries: Vec<(bool, Option<usize>)> = (0..forms.len()).map(|i| (true, Some(i))).collect();
    tries.extend([(true, None), (false, None)]);
    let used = |(with_sleep, right): (bool, Option<usize>)| {
        let left = if with_sleep { sleep_w } else { 0 };
        (left, right.map_or(0, |i| forms[i].1 + 1))
    };
    let pick = tries
        .into_iter()
        .find(|t| {
            let (l, r) = used(*t);
            l + tabs_w + r <= inner.width
        })
        .unwrap_or((false, None));
    let (left_used, right_used) = used(pick);

    let centred = inner.width.saturating_sub(tabs_w) / 2;
    let x =
        centred.clamp(left_used, (inner.width.saturating_sub(right_used + tabs_w)).max(left_used));
    let row = Rect { height: 1, ..inner };
    f.render_widget(
        Paragraph::new(Line::from(tabs)),
        Rect { x: row.x + x, width: tabs_w, ..row }.intersection(row),
    );
    if let (true, Some(sleep)) = (pick.0, &info.sleep) {
        f.render_widget(
            Paragraph::new(sleep.as_str()).style(theme::secondary()),
            Rect { width: sleep_w.saturating_sub(1), ..row },
        );
    }
    if let Some(i) = pick.1 {
        let (spans, width) = &forms[i];
        f.render_widget(
            Paragraph::new(Line::from(spans.clone())),
            Rect { x: row.right() - width, width: *width, ..row },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_meter_fills_every_cell_at_full() {
        assert_eq!(volume_fill(0.0), 0);
        assert_eq!(volume_fill(50.0), 4);
        assert_eq!(volume_fill(100.0), METER_CELLS);
    }

    #[test]
    fn audio_info_formats_long_and_compact() {
        let info = |codec: &str, rate: Option<u32>, depth: Option<u32>, kbps: Option<u32>| {
            crate::plex::AudioInfo {
                codec: Some(codec.into()),
                sample_rate_hz: rate,
                bit_depth: depth,
                bitrate_kbps: kbps,
                ..Default::default()
            }
        };
        let fmt = |a| format_audio(&a);
        let both = |l: &str, c: &str| (l.to_string(), c.to_string());
        assert_eq!(
            fmt(info("flac", Some(44100), Some(16), None)),
            both("FLAC 44.1kHz/16bit", "FLAC 44.1k/16")
        );
        assert_eq!(
            fmt(info("flac", Some(96000), Some(24), None)),
            both("FLAC 96kHz/24bit", "FLAC 96k/24")
        );
        // the compact lossy form drops the rate
        assert_eq!(
            fmt(info("mp3", Some(44100), None, Some(320))),
            both("MP3 320kbps/44.1kHz", "MP3 320k")
        );
        assert_eq!(fmt(info("mp3", None, None, Some(128))), both("MP3 128kbps", "MP3 128k"));
        assert_eq!(fmt(info("flac", None, Some(16), None)), both("FLAC 16bit", "FLAC 16bit"));
        assert_eq!(fmt(info("ogg", Some(48000), None, None)), both("OGG 48kHz", "OGG 48k"));
        assert_eq!(fmt(info("wav", None, None, None)), both("WAV", "WAV"));
        assert_eq!(fmt(crate::plex::AudioInfo::default()), both("", ""));
        assert_eq!(format_rate(44100), "44.1k");
        assert_eq!(format_rate(48000), "48k");
        assert_eq!(format_rate(22050), "22.05k");
    }

    #[test]
    fn header_times_steps_down_to_what_fits() {
        let (long, compact) = ("FLAC 44.1kHz/16bit", "FLAC 44.1k/16");
        let t = "0:42 / 4:17";
        let long_full = "0:42 / 4:17 (FLAC 44.1kHz/16bit)"; // 32 cells
        let compact_full = "0:42 / 4:17 (FLAC 44.1k/16)"; // 27 cells
        assert_eq!(header_times(t, long, compact, 32), long_full);
        assert_eq!(header_times(t, long, compact, 31), compact_full);
        assert_eq!(header_times(t, long, compact, 27), compact_full);
        assert_eq!(header_times(t, long, compact, 26), t);
        assert_eq!(header_times(t, long, compact, 11), t);
        assert_eq!(header_times(t, long, compact, 6), "0:42 …");
        assert_eq!(header_times(t, "", "", 40), t);
        // wide characters count in cells (15 chars, 16 cells)
        assert_eq!(header_times(t, "音声", "音", 15), t);
        assert_eq!(header_times(t, "音声", "音", 16), format!("{t} (音)"));
    }
}

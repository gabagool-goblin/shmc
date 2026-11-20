use super::widgets::placeholder;
use crate::ui::app::App;
use crate::ui::theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Left padding for the bar group; an odd leftover cell goes right.
fn viz_pad(width: usize, group_w: usize) -> usize {
    width.saturating_sub(group_w) / 2
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VisualizerMode {
    Adaptive,
    Spread,
}

/// Spread mode: which of bar width and gap keeps its set size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpreadAnchor {
    Bar,
    Gap,
}

/// A width per bar and per gap, and the left padding.
#[derive(Debug, PartialEq, Eq)]
struct VizLayout {
    bars: Vec<usize>,
    gaps: Vec<usize>,
    pad: usize,
}

impl VizLayout {
    fn group_w(&self) -> usize {
        self.bars.iter().sum::<usize>() + self.gaps.iter().sum::<usize>()
    }
}

/// Adaptive adds bars, then widens them; spread widens what the anchor does not name.
fn visualizer_layout(
    width: usize,
    mode: VisualizerMode,
    anchor: SpreadAnchor,
    min_bar_w: u16,
    min_gap: u16,
    fixed_bars: u16,
    max_bars: usize,
) -> VizLayout {
    let width = width.max(1);
    let max_bars = max_bars.max(1);
    let min_bar_w = (min_bar_w.max(1) as usize).min(width);
    match mode {
        VisualizerMode::Adaptive => {
            let min_gap = min_gap as usize;
            let divisor = (min_bar_w + min_gap).max(1);
            let bars = ((width + min_gap) / divisor).clamp(1, max_bars);
            let bar_w = if bars == max_bars {
                (width.saturating_sub(bars.saturating_sub(1) * min_gap) / bars)
                    .max(min_bar_w)
                    .min(width)
            } else {
                min_bar_w
            };
            let group_w = bars * bar_w + bars.saturating_sub(1) * min_gap;
            VizLayout {
                bars: vec![bar_w; bars],
                gaps: vec![min_gap; bars - 1],
                pad: viz_pad(width, group_w),
            }
        }
        VisualizerMode::Spread => {
            // Both set sizes are minimums; a pane narrower than one bar gets one cut bar.
            let n0 = (fixed_bars as usize).clamp(1, max_bars);
            let set_gap = min_gap as usize;
            let n = ((width + set_gap) / (min_bar_w + set_gap)).clamp(1, n0);
            let (bar_w, gap_w) = match anchor {
                SpreadAnchor::Gap => ((width - (n - 1) * set_gap) / n, set_gap),
                SpreadAnchor::Bar if n >= 2 => (min_bar_w, (width - n * min_bar_w) / (n - 1)),
                SpreadAnchor::Bar => (min_bar_w, 0),
            };
            let gap_count = n - 1;
            let mut layout =
                VizLayout { bars: vec![bar_w; n], gaps: vec![gap_w; gap_count], pad: 0 };
            layout.pad = viz_pad(width, layout.group_w());
            layout
        }
    }
}

/// Downsample cava's bands to the bars by bucket maximum, keeping peaks.
fn sample_visualizer_bars(frame: &[u8], bars: usize) -> Vec<u8> {
    if bars == 0 {
        return Vec::new();
    }
    if frame.is_empty() {
        return vec![0; bars];
    }
    let n = frame.len();
    (0..bars)
        .map(|b| {
            let start = b * n / bars;
            let end = ((b + 1) * n / bars).max(start + 1).min(n);
            frame[start..end].iter().copied().max().unwrap_or(0)
        })
        .collect()
}

pub(super) fn draw_visualizer(f: &mut Frame, app: &App, area: Rect) {
    if area.width < 4 || area.height < 1 {
        return;
    }
    if !app.cava_alive {
        placeholder(f, area, &app.cava_note);
        return;
    }
    let w = area.width as usize;
    let h = area.height as usize;
    let mode = if app.visualizer_mode.eq_ignore_ascii_case("spread") {
        VisualizerMode::Spread
    } else {
        VisualizerMode::Adaptive
    };
    let anchor = if app.visualizer_anchor.eq_ignore_ascii_case("bar") {
        SpreadAnchor::Bar
    } else {
        SpreadAnchor::Gap
    };
    let layout = visualizer_layout(
        w,
        mode,
        anchor,
        app.visualizer_bar_width,
        app.visualizer_bar_gap,
        app.visualizer_bars,
        crate::playback::cava::BARS,
    );
    let bars = layout.bars.len();
    let pad = layout.pad;
    let frame = &app.cava_frame;
    let values = sample_visualizer_bars(frame, bars);
    let denom = (h.saturating_sub(1)).max(1) as f32;
    let ramp = theme::symbols().visualizer;

    let mut lines: Vec<Line> = Vec::with_capacity(h);
    for row in 0..h {
        let from_bottom = h - 1 - row;
        let color = theme::viz_color(from_bottom as f32 / denom);
        let mut s = " ".repeat(pad);
        for (b, v) in values.iter().enumerate() {
            let level = (*v as f32 / 255.0) * (h * 8) as f32;
            let fill = (level - from_bottom as f32 * 8.0).clamp(0.0, 8.0);
            let ch = if fill <= 0.0 { ' ' } else { ramp[(fill.ceil() as usize).min(8) - 1] };
            for _ in 0..layout.bars[b] {
                s.push(ch);
            }
            if let Some(&gap) = layout.gaps.get(b) {
                for _ in 0..gap {
                    s.push(' ');
                }
            }
        }
        lines.push(Line::from(Span::styled(s, Style::default().fg(color))));
    }
    f.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spread mode, both anchors, every width.
    #[test]
    fn spread_layout_adaptive_with_cap() {
        let group = |n: usize, bar: usize, gap: usize| n * bar + (n - 1) * gap;
        for anchor in [SpreadAnchor::Bar, SpreadAnchor::Gap] {
            for cap in [1u16, 2, 3, 24, 96] {
                for w0 in 1..=6u16 {
                    for g0 in 0..=4u16 {
                        for w in 1..=300usize {
                            let l = visualizer_layout(
                                w,
                                VisualizerMode::Spread,
                                anchor,
                                w0,
                                g0,
                                cap,
                                96,
                            );
                            let case =
                                format!("{anchor:?} w={w} cap={cap} bar={w0} gap={g0} -> {l:?}");
                            let n = l.bars.len();
                            let (bar, gap) = (l.bars[0], l.gaps.first().copied().unwrap_or(0));
                            let (w0, g0, cap) = (w0 as usize, g0 as usize, cap as usize);
                            assert!(n >= 1 && l.gaps.len() == n - 1, "{case}");
                            assert!(l.bars.iter().all(|&b| b == bar), "{case}");
                            assert!(l.gaps.iter().all(|&x| x == gap), "{case}");
                            let used = l.group_w();
                            assert!(used <= w, "{case}");
                            assert!(l.pad.abs_diff(w - used - l.pad) <= 1, "{case}");
                            assert!(l.pad <= w - used - l.pad, "{case}");

                            if w < w0 {
                                // one cut bar
                                assert_eq!((n, bar), (1, w), "{case}");
                                continue;
                            }
                            let n_fit = (w + g0) / (w0 + g0);
                            assert_eq!(n, cap.min(n_fit), "{case}");
                            assert!(bar >= w0 && (n == 1 || gap >= g0), "{case}");
                            match anchor {
                                SpreadAnchor::Gap => {
                                    assert!(n == 1 || gap == g0, "{case}");
                                    assert!(group(n, bar + 1, gap) > w, "{case}");
                                }
                                SpreadAnchor::Bar => {
                                    assert_eq!(bar, w0, "{case}");
                                    if n > 1 {
                                        assert!(group(n, bar, gap + 1) > w, "{case}");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// (bars, bar width, gap, left margin) by hand.
    #[test]
    fn spread_layout_by_example() {
        let cells = |w, anchor, bw, gap, count| {
            let l = visualizer_layout(w, VisualizerMode::Spread, anchor, bw, gap, count, 96);
            (l.bars.len(), l.bars[0], l.gaps.first().copied().unwrap_or(0), l.pad)
        };
        // 24 bars of 3 with gaps of 2 need 118: 115 shows 23, not thinner ones
        assert_eq!(cells(115, SpreadAnchor::Gap, 3, 2, 24), (23, 3, 2, 1));
        assert_eq!(cells(115, SpreadAnchor::Bar, 3, 2, 24), (23, 3, 2, 1));
        assert_eq!(cells(118, SpreadAnchor::Gap, 3, 2, 24), (24, 3, 2, 0));
        assert_eq!(cells(160, SpreadAnchor::Gap, 3, 2, 24), (24, 4, 2, 9));
        assert_eq!(cells(160, SpreadAnchor::Bar, 3, 2, 24), (24, 3, 3, 9));
        assert_eq!(cells(300, SpreadAnchor::Gap, 2, 1, 20), (20, 14, 1, 0));
        assert_eq!(cells(300, SpreadAnchor::Bar, 2, 1, 20), (20, 2, 13, 6));
        // a lone bar
        assert_eq!(cells(30, SpreadAnchor::Gap, 2, 1, 1), (1, 30, 0, 0));
        assert_eq!(cells(30, SpreadAnchor::Bar, 2, 1, 1), (1, 2, 0, 14));
        assert_eq!(cells(2, SpreadAnchor::Bar, 5, 1, 8), (1, 2, 0, 0));
    }

    /// (bar count, bar width, gap, margin) at several widths.
    #[test]
    fn adaptive_layout_at_several_widths() {
        let cells = |w, bw, gap| {
            let l =
                visualizer_layout(w, VisualizerMode::Adaptive, SpreadAnchor::Gap, bw, gap, 20, 96);
            let n = l.bars.len();
            assert!(
                l.bars.iter().all(|&b| b == l.bars[0]) && l.gaps.iter().all(|&g| g == l.gaps[0])
            );
            (n, l.bars[0], l.gaps.first().copied().unwrap_or(0), l.pad)
        };
        assert_eq!(cells(40, 2, 1), (13, 2, 1, 1));
        assert_eq!(cells(58, 2, 1), (19, 2, 1, 1));
        assert_eq!(cells(100, 2, 1), (33, 2, 1, 1));
        assert_eq!(cells(250, 2, 1), (83, 2, 1, 1));
        assert_eq!(cells(297, 2, 1), (96, 2, 1, 5));
        assert_eq!(cells(400, 2, 1), (96, 3, 1, 8));
        assert_eq!(cells(100, 3, 2), (20, 3, 2, 1));
        assert_eq!(cells(4, 3, 2), (1, 3, 0, 0));
        assert_eq!(cells(10, 1, 0), (10, 1, 0, 0));
    }

    #[test]
    fn visualizer_sampling_preserves_peaks() {
        let frame: Vec<u8> = (0..96).collect();
        assert_eq!(sample_visualizer_bars(&frame, 4), vec![23, 47, 71, 95]);
        assert_eq!(sample_visualizer_bars(&[], 3), vec![0, 0, 0]);
    }
}

pub mod app;
pub mod config_screen;
pub mod filter;
pub mod icons;
pub mod keymap;
pub mod period;
pub mod playlist_load;
pub mod queue_seq;
pub mod scroll;
pub mod scrollbar;
pub mod term;
pub mod text_input;
pub mod theme;
pub mod themes;
pub mod views;
pub mod when;

pub use app::{App, Msg};

use ratatui_image::picker::{Picker, ProtocolType};

/// The cell size comes from the tty: `CSI 16 t` can answer in logical points.
pub fn image_picker() -> Option<Picker> {
    match (Picker::from_query_stdio().ok(), physical_cell_size()) {
        (Some(queried), Some(cell)) => {
            let picker = picker_with(cell, queried.protocol_type());
            if picker_cell(&queried) != cell {
                tracing::info!(
                    queried = ?queried.font_size(),
                    physical = ?cell,
                    "cell-size query disagrees with tty pixels; using tty pixels"
                );
            }
            Some(picker)
        }
        (Some(queried), None) => Some(queried),
        // No query answer, but the tty knows its pixels.
        (None, Some(cell)) => Some(picker_from_cell(cell)),
        (None, None) => None,
    }
}

fn picker_with(cell: (u16, u16), protocol: ProtocolType) -> Picker {
    let mut picker = picker_from_cell(cell);
    picker.set_protocol_type(protocol);
    picker
}

// Deprecated without a replacement: no other constructor takes a cell size.
#[allow(deprecated)]
fn picker_from_cell(cell: (u16, u16)) -> Picker {
    Picker::from_fontsize(cell.into())
}

pub fn picker_cell(picker: &Picker) -> (u16, u16) {
    let size = picker.font_size();
    (size.width, size.height)
}

/// For a cell size measured after startup; `None` when nothing changed.
pub(crate) fn picker_for_cell(old: &Picker, measured: Option<(u16, u16)>) -> Option<Picker> {
    let cell = measured.filter(|&cell| cell != picker_cell(old))?;
    Some(picker_with(cell, old.protocol_type()))
}

/// Cell size in physical pixels from `TIOCGWINSZ`.
pub(crate) fn physical_cell_size() -> Option<(u16, u16)> {
    let ws = crossterm::terminal::window_size().ok()?;
    cell_size(ws.columns, ws.rows, ws.width, ws.height)
}

fn cell_size(cols: u16, rows: u16, width: u16, height: u16) -> Option<(u16, u16)> {
    if cols == 0 || rows == 0 || width == 0 || height == 0 {
        return None;
    }
    let cell = (width / cols, height / rows);
    (cell.0 > 0 && cell.1 > 0).then_some(cell)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_size_uses_tty_physical_pixels() {
        // a 2x display where CSI 16 t says 11×23 points
        assert_eq!(cell_size(80, 25, 1760, 1150), Some((22, 46)));
    }

    #[test]
    fn new_cell_size_rebuilds_picker_same_protocol() {
        let mut old = picker_from_cell((10, 20));
        old.set_protocol_type(ProtocolType::Iterm2);

        assert!(picker_for_cell(&old, Some((10, 20))).is_none(), "unchanged");
        assert!(picker_for_cell(&old, None).is_none(), "nothing measured");
        let new = picker_for_cell(&old, Some((22, 46))).expect("changed");
        assert_eq!(picker_cell(&new), (22, 46));
        assert_eq!(new.protocol_type(), ProtocolType::Iterm2);
    }

    #[test]
    fn cell_size_needs_real_geometry() {
        assert_eq!(cell_size(0, 25, 1760, 1150), None);
        assert_eq!(cell_size(80, 0, 1760, 1150), None);
        assert_eq!(cell_size(80, 25, 0, 1150), None);
        assert_eq!(cell_size(80, 25, 1760, 0), None);
        assert_eq!(cell_size(100, 25, 80, 1150), None);
    }
}

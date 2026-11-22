//! Loudness leveling from Plex's analysis, applied by a bundled mpv script.

use crate::plex::Loudness;
use std::path::PathBuf;

/// ReplayGain 2.0's reference level, in LUFS.
const REFERENCE_LUFS: f64 = -18.0;

const SCRIPT: &str = include_str!("leveling.lua");

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Leveling {
    #[default]
    Off,
    Track,
    Album,
}

impl Leveling {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "track" => Some(Self::Track),
            "album" => Some(Self::Album),
            _ => None,
        }
    }

    /// In dB, capped at the peak; 0 without analysis.
    pub fn gain_db(self, loudness: &Loudness, preamp_db: f64) -> f64 {
        let track = loudness.track_lufs.map(|lufs| REFERENCE_LUFS - lufs);
        let (gain, peak) = match self {
            Self::Off => return 0.0,
            Self::Track => (track, loudness.track_peak),
            Self::Album => {
                (loudness.album_gain.or(track), loudness.album_peak.or(loudness.track_peak))
            }
        };
        let Some(gain) = gain else { return 0.0 };
        let gain = gain + preamp_db;
        match peak.filter(|&p| p > 0.0) {
            Some(peak) => gain.min(-20.0 * peak.log10()),
            None => gain,
        }
    }
}

pub fn part_id(part_key: &str) -> Option<&str> {
    let id = part_key.strip_prefix("/library/parts/")?.split('/').next()?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then_some(id)
}

/// 0600 in the private run dir, so no other user can swap it.
pub fn write_script() -> std::io::Result<PathBuf> {
    let path = crate::cache::private_run_dir()?.join("leveling.lua");
    crate::cache::write_private_atomic(&path, SCRIPT.as_bytes())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analysed(lufs: f64, peak: f64, album_gain: f64, album_peak: f64) -> Loudness {
        Loudness {
            track_lufs: Some(lufs),
            track_peak: Some(peak),
            album_gain: Some(album_gain),
            album_peak: Some(album_peak),
        }
    }

    #[test]
    fn track_and_album_gains_come_from_their_own_fields() {
        let l = analysed(-6.0, 0.5, -11.0, 0.5);
        assert_eq!(Leveling::Track.gain_db(&l, 0.0), -12.0);
        assert_eq!(Leveling::Album.gain_db(&l, 0.0), -11.0);
        assert_eq!(Leveling::Album.gain_db(&l, 2.5), -8.5);
        assert_eq!(Leveling::Off.gain_db(&l, 2.5), 0.0);
    }

    #[test]
    fn gain_never_lifts_the_peak_past_full_scale() {
        let l = analysed(-30.0, 0.5, 12.0, 0.5);
        let cap = -20.0 * 0.5f64.log10();
        assert!((Leveling::Track.gain_db(&l, 0.0) - cap).abs() < 1e-9);
        assert!((Leveling::Album.gain_db(&l, 0.0) - cap).abs() < 1e-9);
    }

    #[test]
    fn unanalysed_track_plays_as_it_is() {
        assert_eq!(Leveling::Track.gain_db(&Loudness::default(), 3.0), 0.0);
        let track_only = Loudness { track_lufs: Some(-8.0), ..Loudness::default() };
        assert_eq!(Leveling::Album.gain_db(&track_only, 0.0), -10.0, "album falls back to track");
    }

    #[test]
    fn part_id_is_the_number_after_parts() {
        assert_eq!(part_id("/library/parts/25415/1779387072/file.flac"), Some("25415"));
        assert_eq!(part_id("/library/parts/x/1/file.flac"), None);
        assert_eq!(part_id("/library/metadata/1"), None);
    }
}

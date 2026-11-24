use crate::config::{KeyTable, UiConfig};
use crate::playback::{MpvCmd, MpvEvent, MpvHandle};
use crate::plex::{Album, Artist, AudioInfo, SearchHit, Track};
use crate::ui::keymap::Keymap;
use crate::ui::playlist_load::PlaylistLoad;
use crate::ui::queue_seq::{QueueOutcome, QueueSeq};
use crate::ui::scroll::ScrollStore;
use image::DynamicImage;
use ratatui::widgets::{ListState, TableState};
use ratatui_image::picker::Picker;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;
use ui_settings::UiSettings;

mod activate;
mod adventure;
mod discover;
mod events;
mod input;
mod leveling;
mod list;
mod list_request;
mod nav;
mod plex_worker;
mod queue;
mod radio;
mod rating;
pub mod reload;
mod report;
mod resume;
pub mod saved_queue;
mod search_back;
mod shuffle;
mod similar;
pub mod ui_settings;

pub use adventure::{Adventure, AdventureOutcome, Stop};
pub use discover::{
    trail, DiscoverCmd, DiscoverCol, DiscoverLoad, DiscoverMsg, DiscoverView, ItemsPane,
    LibraryOutcome, OpenKind, Opened, Pane, Preview, Row, TracksPane, TracksSource,
};
pub use list::FilteredList;
pub use list_request::{ListKind, ListRequest};
pub use plex_worker::{spawn_plex_worker, PlexCmd, PlexHandle};
use queue::PosGuard;
pub use radio::{ActiveRadio, RadioEnv, RadioOutcome, RadioSeed};
pub use search_back::{SavedResults, SearchBack};
pub use similar::{SimilarHits, SimilarKind, SimilarOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Queue,
    Discover,
    Browser,
    Playlists,
    Search,
    Help,
}

impl Tab {
    pub fn next(self) -> Self {
        match self {
            Tab::Queue => Tab::Discover,
            Tab::Discover => Tab::Browser,
            Tab::Browser => Tab::Playlists,
            Tab::Playlists => Tab::Search,
            Tab::Search => Tab::Help,
            Tab::Help => Tab::Queue,
        }
    }
    pub fn prev(self) -> Self {
        match self {
            Tab::Queue => Tab::Help,
            Tab::Discover => Tab::Queue,
            Tab::Browser => Tab::Discover,
            Tab::Playlists => Tab::Browser,
            Tab::Search => Tab::Playlists,
            Tab::Help => Tab::Search,
        }
    }
    pub fn index(self) -> usize {
        match self {
            Tab::Queue => 0,
            Tab::Discover => 1,
            Tab::Browser => 2,
            Tab::Playlists => 3,
            Tab::Search => 4,
            Tab::Help => 5,
        }
    }
    pub const ALL: [Tab; 6] =
        [Tab::Queue, Tab::Discover, Tab::Browser, Tab::Playlists, Tab::Search, Tab::Help];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueAction {
    Append,
    Replace,
    PlayNext,
}

#[derive(Debug, Clone)]
pub enum Confirm {
    Track {
        title: String,
    },
    /// A visual-mode range.
    Tracks,
    Album {
        name: String,
    },
    Artist {
        name: String,
    },
    Playlist {
        name: String,
    },
    /// Enter on a track inside a playlist: the rest of the list from there.
    FromList {
        name: String,
    },
}

impl Confirm {
    pub fn added(&self, n: usize) -> String {
        match self {
            Confirm::Track { title } => format!("Added: {title}"),
            Confirm::Tracks => format!("Added {n} tracks"),
            Confirm::Album { name } => format!("Added album: {name} ({n} tracks)"),
            Confirm::Artist { name } => format!("Added artist: {name} ({n} tracks)"),
            Confirm::Playlist { name } => format!("Added playlist: {name} ({n} tracks)"),
            Confirm::FromList { name } => format!("Added {n} tracks from {name}"),
        }
    }
    pub fn playing(&self, n: usize) -> String {
        match self {
            Confirm::Track { title } => format!("Playing: {title}"),
            Confirm::Tracks => format!("Playing {n} tracks"),
            Confirm::Album { name } => format!("Playing album: {name} ({n} tracks)"),
            Confirm::Artist { name } => format!("Playing artist: {name} ({n} tracks)"),
            Confirm::Playlist { name } => format!("Playing playlist: {name} ({n} tracks)"),
            Confirm::FromList { name } => format!("Playing {n} tracks from {name}"),
        }
    }
}

pub enum Msg {
    Key(crossterm::event::Event),
    /// Bracketed paste: inserted as text, never run as key bindings.
    Paste(String),
    Resize,
    ArtistsLoaded(Vec<Artist>),
    AlbumsLoaded {
        artist_key: String,
        albums: Vec<Album>,
    },
    TracksLoaded {
        album_key: String,
        tracks: Vec<Track>,
    },
    SearchResults {
        query: String,
        hits: Vec<SearchHit>,
    },
    SearchAlbumLoaded {
        key: String,
        label: String,
        tracks: Vec<Track>,
    },
    SimilarLoaded {
        key: String,
        label: String,
        outcome: SimilarOutcome,
    },
    RadioLoaded {
        key: String,
        label: String,
        outcome: RadioOutcome,
    },
    AdventureProgress {
        key: String,
        leg: usize,
        legs: usize,
    },
    AdventureBuilt {
        key: String,
        outcome: AdventureOutcome,
    },
    Discover(DiscoverMsg),
    PlaylistsLoaded(Vec<crate::plex::Playlist>),
    PlaylistSaved(crate::plex::SaveOutcome),
    PlaylistItemsLoaded {
        rating_key: String,
        tracks: Vec<Track>,
    },
    /// Names what failed, so only the load serving a Browser jump ends it.
    LoadFailed {
        key: String,
        error: String,
    },
    PreviewFailed {
        key: String,
    },
    PreviewPlaylist {
        key: String,
        epoch: u64,
    },
    /// Never applied directly: `QueueSeq` releases it in request order.
    QueueReply {
        seq: u64,
        outcome: QueueOutcome,
    },
    /// Not a `QueueReply`: applies at once if that load is still current.
    PlaylistPage {
        load_id: u64,
        tracks: Vec<Track>,
        last: bool,
    },
    PlaylistLoadStopped {
        load_id: u64,
        error: String,
    },
    GotoArtist(Artist),
    ArtReady {
        thumb_path: String,
        img: Arc<DynamicImage>,
        file: Option<PathBuf>,
    },
    /// The cover's dwell is over; draw again.
    ArtDue,
    ConnectionChecked {
        epoch: u64,
        ok: bool,
    },
    ArtFailed {
        thumb_path: String,
    },
    TrackInfo {
        rating_key: String,
        audio: AudioInfo,
        user_rating: Option<u8>,
    },
    TrackInfoFailed {
        rating_key: String,
    },
    Rated {
        rating_key: String,
        rating: Option<u8>,
        title: String,
    },
    Mpv(MpvEvent),
    Cava(Vec<u8>),
    /// cava is alive, even while silent.
    CavaReady,
    CavaDead(String),
    /// The epoch tells a stale timer from the current one.
    ClearStatus(u64),
    SleepTimer {
        epoch: u64,
    },
    Error(String),
    /// A signal: the same as `q`.
    Quit,
    ConfigRead(Box<Result<reload::Reload, String>>),
}

pub struct Browser {
    pub artists: FilteredList<Artist>,
    pub albums: FilteredList<Album>,
    pub tracks: FilteredList<Track>,
    pub focus: BrowserCol,
    pub albums_for: Option<String>,
    pub tracks_for: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserCol {
    Artists,
    Albums,
    Tracks,
}

impl Default for Browser {
    fn default() -> Self {
        Self {
            artists: FilteredList::default(),
            albums: FilteredList::default(),
            tracks: FilteredList::default(),
            focus: BrowserCol::Artists,
            albums_for: None,
            tracks_for: None,
        }
    }
}

pub struct PlaylistsView {
    pub list: FilteredList<crate::plex::Playlist>,
    pub focus: PlaylistsCol,
    pub tracks: FilteredList<Track>,
    /// The playlist whose items are loaded.
    pub tracks_for: Option<String>,
    pub tracks_total: Option<usize>,
    pub preview_cache: HashMap<String, Vec<Track>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistsCol {
    List,
    Items,
}

impl Default for PlaylistsView {
    fn default() -> Self {
        Self {
            list: FilteredList::default(),
            focus: PlaylistsCol::List,
            tracks: FilteredList::default(),
            tracks_for: None,
            tracks_total: None,
            preview_cache: HashMap::new(),
        }
    }
}

/// The Search tab's album view or similar tracks list; never touches the queue.
pub struct SearchDetail {
    pub key: String,
    pub label: String,
    pub tracks: Vec<Track>,
    pub state: TableState,
    /// Set on a similar-tracks list: the match percentage of each track.
    pub matches: Option<Vec<Option<u8>>>,
    /// Set on a radio list: Enter plays it as a radio.
    pub radio: Option<crate::plex::radio::RadioQueue>,
}

#[derive(Default)]
pub struct SearchState {
    pub input: String,
    pub focused: bool,
    pub hits: Vec<SearchHit>,
    pub state: TableState,
    pub last_query: String,
    /// Oldest first.
    pub history: Vec<String>,
    history_pos: Option<usize>,
    pub list_request: Option<ListRequest>,
    /// Set while `hits` are a similar albums or artists list, not a search result.
    pub similar: Option<SimilarHits>,
    /// What Esc steps back through, oldest first.
    pub back: Vec<SearchBack>,
    pub detail: Option<SearchDetail>,
}

#[derive(Debug, Default)]
pub struct CommandMode {
    pub typing: bool,
    pub text: String,
}

/// How long the cover stays out after its cells were cleared.
pub const COVER_DWELL: std::time::Duration = std::time::Duration::from_millis(120);

#[derive(Default)]
pub struct ArtState {
    pub img: Option<Arc<DynamicImage>>,
    pub protocol: Option<ratatui_image::protocol::StatefulProtocol>,
    /// Where the cover was painted; later frames skip those cells, not re-send it.
    pub shown_rect: Option<ratatui::layout::Rect>,
    /// The rect `protocol` was pre-centred and encoded for.
    pub prepared_rect: Option<ratatui::layout::Rect>,
    /// Re-sending the image in a draw holds the screen, so it stays out until then.
    pub hold_until: Option<Instant>,
    /// The Queue tab was left since the cover was last on screen.
    pub away: bool,
    /// The protocol was rendered this frame; reported when a draw is slow.
    pub emitted: bool,
    /// The path `img` came from; the same path is never loaded again.
    pub thumb: Option<String>,
    /// The file `img` is cached in, if any.
    pub file: Option<PathBuf>,
    /// A path with a `LoadThumb` in flight.
    pub requested: Option<String>,
}

impl ArtState {
    fn clear(&mut self) {
        self.img = None;
        self.protocol = None;
        self.shown_rect = None;
        self.prepared_rect = None;
        self.thumb = None;
        self.file = None;
        self.requested = None;
        self.hold_until = None;
    }
}

#[derive(Default)]
struct ShuffleSnapshot {
    order: Vec<Track>,
}

/// A Browser reveal waiting on the artist; a missing album or track stops a level up.
#[derive(Debug, Clone)]
struct PendingGoto {
    artist_key: String,
    album_key: Option<String>,
    track_key: Option<String>,
    from: Tab,
}

struct PendingPlaylist {
    key: String,
    confirm: Confirm,
    action: QueueAction,
    /// An `entry_key`; None = the whole list.
    start: Option<String>,
    count: Option<u32>,
    armed_at: Instant,
}

/// Visual mode: `anchor` is a real index into `list`'s underlying vec.
#[derive(Debug, Clone, Copy)]
pub struct Visual {
    pub list: ListRef,
    pub anchor: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListRef {
    Queue,
    BArtists,
    BAlbums,
    BTracks,
    PlList,
    PlItems,
    Search,
    SearchDetail,
    DSections,
    DTracks,
    DItems,
    Help,
}

pub struct App {
    pub should_quit: bool,
    /// mpv went away on its own, not because shmc quit.
    pub mpv_exited: bool,
    pub tab: Tab,
    /// Where Esc/q leaves Help to.
    pub help_prev: Tab,

    pub server: String,
    pub token: String,

    pub plex: PlexHandle,
    pub mpv: MpvHandle,

    pub queue: FilteredList<Track, TableState>,
    pub current: Option<usize>,
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub stopped: bool,
    pub volume: f64,
    /// bits/s; 0 = unknown, not shown.
    pub bitrate_bps: f64,
    pub audio: AudioInfo,
    pub loop_playlist: bool,
    pub loop_file: bool,
    pub random: bool,
    shuffle_snapshot: Option<ShuffleSnapshot>,
    /// `d` removals for `u`; other queue changes clear it, as the indices no longer hold.
    removed: Vec<Vec<(usize, Track)>>,
    pub visual: Option<Visual>,
    /// Cleared by the matching event and by every app-initiated playback change.
    expect_pos: Option<PosGuard>,
    sleep_epoch: u64,
    sleep_until: Option<Instant>,
    sleep_mins: Option<u32>,
    saving: bool,
    /// Dropping it wakes the timer thread so it exits.
    sleep_cancel: Option<Sender<()>>,
    /// App-side: each track leaves the queue once it finishes.
    pub consume: bool,
    pending_playlist: Option<PendingPlaylist>,
    /// When "clear the queue?" was asked.
    pending_clear: Option<Instant>,
    /// The "rate the track?" question, with the track it named.
    pending_rating: Option<rating::PendingRating>,
    /// Ratings known per rating key, from a track's metadata or a confirmed write.
    pub ratings: HashMap<String, Option<u8>>,
    pub allow_writes: bool,
    /// Where playback reports go; None when `plex.report_playback` is off.
    pub reporter: Option<UnboundedSender<crate::plex::report::Report>>,
    report: Option<report::TrackReport>,
    /// The playlist still arriving after its first page.
    playlist_load: Option<PlaylistLoad>,
    queue_seq: QueueSeq,
    pub radio: Option<ActiveRadio>,
    pub adventure: Adventure,
    /// After this long a y/n question lapses and the key goes through.
    pub confirm_timeout_secs: u64,
    preview_epoch: u64,
    /// Not asked again while it stays highlighted.
    preview_blocked: Option<String>,
    pub audio_cache: HashMap<String, AudioInfo>,
    /// The track mpv failed on, for `>` while stopped; inserts and removals clear it.
    pub failed_at: Option<usize>,
    /// Failures since a file last loaded.
    err_streak: u32,
    restore: saved_queue::Restore,
    autosave: saved_queue::Autosave,
    resume: resume::Resume,
    leveling: leveling::LevelingState,

    pub discover: DiscoverView,
    pub browser: Browser,
    pub playlists: PlaylistsView,
    pub search: SearchState,
    pub command: CommandMode,
    pub help_state: ListState,
    pub keymap: Keymap,
    pub art: ArtState,
    pub cava_frame: Vec<u8>,
    pub cava_alive: bool,
    pub cava_note: String,

    pub picker: Option<Picker>,
    pub art_enabled: bool,
    /// Hand the cover to mpv for Now Playing.
    pub now_playing_cover: bool,
    /// mpv's playing file has no cover yet.
    cover_due: bool,
    /// [0, 0] = unlimited.
    pub art_max_px: [u32; 2],

    pub scroll: ScrollStore,
    /// Follows the playing row until the user browses; `o` re-arms it.
    queue_follow: bool,
    pub ui: UiSettings,
    /// The config just applied, for the parts `main` owns.
    pub reloaded: Option<Box<crate::config::Config>>,

    pub status: String,
    pub status_error: bool,
    status_epoch: u64,
    pending_goto: Option<PendingGoto>,
    pending_g: bool,

    msg_tx: UnboundedSender<Msg>,
}

/// Stops a dead server from running through the queue in seconds.
const FAILED_STREAK_LIMIT: u32 = 3;

const MAX_UNDO: usize = 20;

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        server: String,
        token: String,
        plex: PlexHandle,
        mpv: MpvHandle,
        volume: f64,
        picker: Option<Picker>,
        art_enabled: bool,
        art_max_px: [u32; 2],
        msg_tx: UnboundedSender<Msg>,
        ui: UiConfig,
        keys: &KeyTable,
    ) -> Self {
        let (keymap, key_problems) = Keymap::from_table(keys);
        let mut app = Self {
            should_quit: false,
            mpv_exited: false,
            tab: Tab::Queue,
            help_prev: Tab::Queue,
            server,
            token,
            plex,
            mpv,
            queue: FilteredList::default(),
            current: None,
            position: 0.0,
            duration: 0.0,
            paused: false,
            stopped: true,
            volume,
            bitrate_bps: 0.0,
            audio: AudioInfo::default(),
            loop_playlist: false,
            loop_file: false,
            random: false,
            shuffle_snapshot: None,
            removed: Vec::new(),
            visual: None,
            expect_pos: None,
            sleep_epoch: 0,
            sleep_until: None,
            sleep_mins: None,
            saving: false,
            sleep_cancel: None,
            consume: false,
            pending_playlist: None,
            pending_clear: None,
            pending_rating: None,
            ratings: HashMap::new(),
            allow_writes: false,
            reporter: None,
            report: None,
            playlist_load: None,
            queue_seq: QueueSeq::default(),
            confirm_timeout_secs: 10,
            preview_epoch: 0,
            preview_blocked: None,
            audio_cache: HashMap::new(),
            failed_at: None,
            err_streak: 0,
            restore: saved_queue::Restore::default(),
            autosave: saved_queue::Autosave::default(),
            resume: resume::Resume::default(),
            leveling: leveling::LevelingState::default(),
            discover: DiscoverView::default(),
            radio: None,
            adventure: Adventure::default(),
            browser: Browser::default(),
            playlists: PlaylistsView::default(),
            search: SearchState::default(),
            command: CommandMode::default(),
            help_state: ListState::default(),
            keymap,
            art: ArtState::default(),
            cava_frame: vec![0; crate::playback::cava::BARS],
            cava_alive: false,
            cava_note: "visualizer off".into(),
            picker,
            art_enabled,
            now_playing_cover: cfg!(target_os = "macos"),
            cover_due: false,
            art_max_px,
            scroll: ScrollStore::default(),
            queue_follow: ui.auto_follow,
            ui: UiSettings::new(&ui),
            reloaded: None,
            status: String::new(),
            status_error: false,
            status_epoch: 0,
            pending_goto: None,
            pending_g: false,
            msg_tx,
        };
        if let Some(problem) = key_problems.first() {
            app.transient(format!("{problem}; using the default keys"));
        }
        app
    }

    pub fn current_track(&self) -> Option<&Track> {
        self.current.and_then(|i| self.queue.items.get(i))
    }

    /// Centered in the progress row for ~2s.
    pub fn transient(&mut self, msg: String) {
        self.transient_secs(msg, 2, false);
    }

    pub fn transient_err(&mut self, msg: String) {
        self.transient_secs(msg, 4, true);
    }

    pub(crate) fn hold_art(&mut self) {
        self.art.hold_until = Some(Instant::now() + COVER_DWELL);
        let tx = self.msg_tx.clone();
        std::thread::spawn(move || {
            std::thread::sleep(COVER_DWELL);
            // The receiver is gone at shutdown (and in tests).
            let _ = tx.send(Msg::ArtDue);
        });
    }

    fn transient_secs(&mut self, msg: String, secs: u64, is_err: bool) {
        // one line always: a newline in a title must not push the layout
        self.status = crate::redact::escape_field(&crate::redact::redact_secrets(&msg));
        self.status_error = is_err;
        self.status_epoch += 1;
        let epoch = self.status_epoch;
        let tx = self.msg_tx.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(secs));
            let _ = tx.send(Msg::ClearStatus(epoch));
        });
    }

    /// The epoch bump voids the pending `ClearStatus`, so a message from this key keeps its time.
    fn dismiss_status(&mut self) {
        if self.status.is_empty() {
            return;
        }
        self.status.clear();
        self.status_error = false;
        self.status_epoch += 1;
    }

    fn set_volume(&mut self, v: f64) {
        self.volume = v.clamp(0.0, 100.0);
        let _ = self.mpv.tx.send(MpvCmd::SetVolume(self.volume));
    }

    /// `:sleep 4294967295` would otherwise arm a timer for ~8000 years.
    const MAX_SLEEP_MINUTES: u32 = 1440;

    /// The epoch makes a cancelled or replaced timer harmless.
    fn set_sleep_timer(&mut self, minutes: Option<u32>) {
        let requested = minutes.filter(|m| *m > 0);
        self.sleep_epoch = self.sleep_epoch.wrapping_add(1);
        self.sleep_until = None;
        self.sleep_mins = requested.map(|m| m.min(Self::MAX_SLEEP_MINUTES));
        self.sleep_cancel = None;
        let Some(m) = self.sleep_mins else {
            self.transient("sleep timer: off".into());
            return;
        };
        let secs = u64::from(m) * 60;
        self.sleep_until = Some(Instant::now() + Duration::from_secs(secs));
        let epoch = self.sleep_epoch;
        let tx = self.msg_tx.clone();
        let (cancel_tx, cancel_rx) = mpsc::channel();
        self.sleep_cancel = Some(cancel_tx);
        std::thread::spawn(move || {
            if let Err(RecvTimeoutError::Timeout) =
                cancel_rx.recv_timeout(Duration::from_secs(secs))
            {
                let _ = tx.send(Msg::SleepTimer { epoch });
            }
        });
        if requested != Some(m) {
            self.transient(format!("sleep timer: {m}m (clamped)"));
        } else {
            self.transient(format!("sleep timer: {m}m"));
        }
    }

    pub fn sleep_timer_label(&self) -> Option<String> {
        self.sleep_mins?;
        let left = self
            .sleep_until
            .map(|t| t.saturating_duration_since(Instant::now()))
            .unwrap_or_default();
        let secs = left.as_secs();
        let label = if secs >= 60 { format!("{}m", secs.div_ceil(60)) } else { format!("{secs}s") };
        Some(format!("sleep {label}"))
    }

    /// `S`: 15 → 30 → 60 → off.
    fn cycle_sleep_timer(&mut self) {
        let next = match self.sleep_mins {
            None => Some(15),
            Some(m) if m < 30 => Some(30),
            Some(m) if m < 60 => Some(60),
            Some(_) => None,
        };
        self.set_sleep_timer(next);
    }
}

pub trait SelState {
    fn selected(&self) -> Option<usize>;
    fn select(&mut self, index: Option<usize>);
}

impl SelState for ListState {
    fn selected(&self) -> Option<usize> {
        ListState::selected(self)
    }
    fn select(&mut self, index: Option<usize>) {
        ListState::select(self, index);
    }
}

impl SelState for TableState {
    fn selected(&self) -> Option<usize> {
        TableState::selected(self)
    }
    fn select(&mut self, index: Option<usize>) {
        TableState::select(self, index);
    }
}

/// No year sorts last, and ties are broken so the server's order never shows.
pub fn sort_albums(albums: &mut [Album], mode: &str) {
    let title = |a: &Album| a.title.to_lowercase();
    let oldest = |a: &Album| a.year.unwrap_or(u32::MAX);
    if mode.eq_ignore_ascii_case("title") {
        albums.sort_by(|a, b| title(a).cmp(&title(b)).then_with(|| oldest(a).cmp(&oldest(b))));
    } else if mode.eq_ignore_ascii_case("year_desc") {
        albums.sort_by(|a, b| {
            a.year
                .is_none()
                .cmp(&b.year.is_none())
                .then_with(|| b.year.cmp(&a.year))
                .then_with(|| title(a).cmp(&title(b)))
        });
    } else {
        albums.sort_by(|a, b| oldest(a).cmp(&oldest(b)).then_with(|| title(a).cmp(&title(b))));
    }
}

pub fn move_sel<S: SelState + ?Sized>(state: &mut S, len: usize, delta: i32) {
    if len == 0 {
        state.select(None);
        return;
    }
    let Some(cur) = state.selected() else {
        state.select(Some(0));
        return;
    };
    // wraps at both ends; a cursor left outside by a shrink is clamped first
    let bound = len as i32;
    let cur = (cur as i32).clamp(0, bound - 1);
    state.select(Some((cur + delta).rem_euclid(bound) as usize));
}

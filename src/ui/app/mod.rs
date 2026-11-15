use crate::config::{KeyTable, UiConfig, VolumeDisplay};
use crate::playback::{LoadItem, MpvCmd, MpvEvent, MpvHandle};
use crate::plex::{Album, Artist, AudioInfo, PlexClient, SearchHit, Track};
use crate::ui::filter::{self, ListFilter};
use crate::ui::keymap::{self, Act, KeyCtx, Keymap};
use crate::ui::playlist_load::{PlaylistLoad, PlaylistRequest};
use crate::ui::queue_seq::{QueueOutcome, QueueReplyGuard, QueueSeq};
use crate::ui::scroll::{self, ScrollId, ScrollStore};
use image::DynamicImage;
use ratatui::widgets::{ListState, TableState};
use ratatui_image::picker::Picker;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Queue,
    Browser,
    Playlists,
    Search,
    Help,
}

impl Tab {
    pub fn next(self) -> Self {
        match self {
            Tab::Queue => Tab::Browser,
            Tab::Browser => Tab::Playlists,
            Tab::Playlists => Tab::Search,
            Tab::Search => Tab::Help,
            Tab::Help => Tab::Queue,
        }
    }
    pub fn prev(self) -> Self {
        match self {
            Tab::Queue => Tab::Help,
            Tab::Browser => Tab::Queue,
            Tab::Playlists => Tab::Browser,
            Tab::Search => Tab::Playlists,
            Tab::Help => Tab::Search,
        }
    }
    pub fn index(self) -> usize {
        match self {
            Tab::Queue => 0,
            Tab::Browser => 1,
            Tab::Playlists => 2,
            Tab::Search => 3,
            Tab::Help => 4,
        }
    }
    pub const ALL: [Tab; 5] = [Tab::Queue, Tab::Browser, Tab::Playlists, Tab::Search, Tab::Help];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueAction {
    Append,
    Replace,
    PlayNext,
}

/// What an add/play confirmation message is about.
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
    PlaylistsLoaded(Vec<crate::plex::Playlist>),
    PlaylistSaved(crate::plex::SaveOutcome),
    PlaylistItemsLoaded {
        rating_key: String,
        tracks: Vec<Track>,
    },
    /// Unlike `Error` it names what failed, so a pending Browser jump is ended only
    /// by the load that serves it.
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
    },
    /// The cover's dwell is over; draw again.
    ArtDue,
    ArtFailed {
        thumb_path: String,
    },
    TrackInfo {
        rating_key: String,
        audio: AudioInfo,
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
}

#[derive(Debug)]
pub enum PlexCmd {
    LoadArtists,
    LoadAlbums(String),
    LoadTracks(String),
    Search(String),
    LoadPlaylists,
    /// Creates a new playlist; never edits or deletes one.
    SavePlaylist {
        name: String,
        keys: Vec<String>,
    },
    LoadPlaylistPreview {
        key: String,
        size: usize,
    },
    ResolvePlaylist {
        seq: u64,
        key: String,
        confirm: Confirm,
        action: QueueAction,
        /// An `entry_key`: start from that entry.
        start: Option<String>,
        stream: bool,
        /// The total to show when the server gives none.
        expected: usize,
        cancel: Arc<AtomicBool>,
    },
    ResolveTrack {
        seq: u64,
        key: String,
        confirm: Confirm,
        action: QueueAction,
        rest_of_album: bool,
    },
    ResolveAlbum {
        seq: u64,
        key: String,
        confirm: Confirm,
        action: QueueAction,
    },
    ResolveArtistTracks {
        seq: u64,
        key: String,
        confirm: Confirm,
        action: QueueAction,
    },
    ResolveArtist(String),
    LoadAlbumDetail {
        key: String,
        label: String,
    },
    LoadThumb {
        path: String,
    },
    /// Sample rate and bit depth come only from the full metadata.
    LoadTrackInfo(String),
}

pub struct PlexHandle {
    pub tx: UnboundedSender<PlexCmd>,
}

pub struct Browser {
    pub artists: Vec<Artist>,
    pub artists_state: ListState,
    pub albums: Vec<Album>,
    pub albums_state: ListState,
    pub tracks: Vec<Track>,
    pub tracks_state: ListState,
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
            artists: Vec::new(),
            artists_state: ListState::default(),
            albums: Vec::new(),
            albums_state: ListState::default(),
            tracks: Vec::new(),
            tracks_state: ListState::default(),
            focus: BrowserCol::Artists,
            albums_for: None,
            tracks_for: None,
        }
    }
}

pub struct PlaylistsView {
    pub playlists: Vec<crate::plex::Playlist>,
    pub state: ListState,
    pub focus: PlaylistsCol,
    pub items: Vec<Track>,
    pub items_state: ListState,
    /// The playlist whose items are (being) loaded.
    pub items_for: Option<String>,
    pub items_total: Option<usize>,
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
            playlists: Vec::new(),
            state: ListState::default(),
            focus: PlaylistsCol::List,
            items: Vec::new(),
            items_state: ListState::default(),
            items_for: None,
            items_total: None,
            preview_cache: HashMap::new(),
        }
    }
}

/// Enter on an album hit; never touches the queue.
pub struct SearchDetail {
    pub key: String,
    pub label: String,
    pub tracks: Vec<Track>,
    pub state: TableState,
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
    /// The album being loaded for the drill-in.
    pub detail_request: Option<String>,
    pub detail: Option<SearchDetail>,
}

#[derive(Debug, Default)]
pub struct CommandMode {
    pub typing: bool,
    pub text: String,
}

/// One parsed `:` command. A leading colon is accepted but the buffer never
/// holds one.
#[derive(Debug, PartialEq, Eq)]
enum Command<'a> {
    Empty,
    Quit,
    Next,
    Prev,
    Pause,
    Play,
    Stop,
    Seek(&'a str),
    Volume(&'a str),
    Clear,
    Save(&'a str),
    BadSave,
    Shuffle,
    Repeat,
    Single,
    Consume,
    Reshuffle,
    Playing,
    Sleep(&'a str),
    BadSleep,
    Tab(Tab),
    BadSeek,
    BadVolume,
    Unknown(&'a str),
}

fn parse_command(raw: &str) -> Command<'_> {
    let raw = raw.trim().trim_start_matches(':').trim();
    if let Some(rest) = raw.strip_prefix("save") {
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            let name = rest.trim();
            return if name.is_empty() { Command::BadSave } else { Command::Save(name) };
        }
    }
    let parts: Vec<&str> = raw.split_whitespace().collect();
    match parts.as_slice() {
        [] => Command::Empty,
        ["q"] | ["quit"] => Command::Quit,
        ["n"] | ["next"] => Command::Next,
        ["p"] | ["prev"] => Command::Prev,
        ["pause"] => Command::Pause,
        ["play"] | ["resume"] => Command::Play,
        ["stop"] => Command::Stop,
        ["seek", n] => Command::Seek(n),
        ["seek", ..] => Command::BadSeek,
        ["vol", n] => Command::Volume(n),
        ["vol", ..] => Command::BadVolume,
        ["clear"] => Command::Clear,
        ["shuffle"] => Command::Shuffle,
        ["repeat"] => Command::Repeat,
        ["single"] => Command::Single,
        ["consume"] => Command::Consume,
        ["reshuffle"] => Command::Reshuffle,
        ["playing"] => Command::Playing,
        ["sleep"] => Command::Sleep(""),
        ["sleep", arg] => Command::Sleep(arg),
        ["sleep", ..] => Command::BadSleep,
        ["queue"] => Command::Tab(Tab::Queue),
        ["browser"] => Command::Tab(Tab::Browser),
        ["playlists"] => Command::Tab(Tab::Playlists),
        ["search"] => Command::Tab(Tab::Search),
        ["help"] => Command::Tab(Tab::Help),
        _ => Command::Unknown(raw),
    }
}

/// How long the cover stays out after its cells were cleared.
pub const COVER_DWELL: std::time::Duration = std::time::Duration::from_millis(120);

#[derive(Default)]
pub struct ArtState {
    pub img: Option<Arc<DynamicImage>>,
    pub protocol: Option<ratatui_image::protocol::StatefulProtocol>,
    /// Where the cover was painted. While it matches the layout, later frames
    /// only skip those cells instead of copying the protocol payload again.
    pub shown_rect: Option<ratatui::layout::Rect>,
    /// The rect `protocol` was pre-centred and encoded for.
    pub prepared_rect: Option<ratatui::layout::Rect>,
    /// The cover stays out until then: re-sending the image in a draw holds the
    /// screen. Set after its cells were cleared.
    pub hold_until: Option<Instant>,
    /// The Queue tab was left since the cover was last on screen.
    pub away: bool,
    /// The protocol was rendered in the frame being drawn; the UI loop resets
    /// it per frame and reports it when a draw is slow.
    pub emitted: bool,
    /// The path `img` came from; the same path is never loaded again.
    pub thumb: Option<String>,
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
        self.requested = None;
        self.hold_until = None;
    }
}

#[derive(Default)]
struct ShuffleSnapshot {
    order: Vec<Track>,
}

/// A reveal in the Browser while it resolves the artist. A missing album ends
/// the jump on the artist, a missing track on the album.
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
    Help,
}

fn scroll_id(l: ListRef) -> ScrollId {
    match l {
        ListRef::Queue => ScrollId::Queue,
        ListRef::BArtists => ScrollId::BrowserArtists,
        ListRef::BAlbums => ScrollId::BrowserAlbums,
        ListRef::BTracks => ScrollId::BrowserTracks,
        ListRef::PlList => ScrollId::PlaylistList,
        ListRef::PlItems => ScrollId::PlaylistItems,
        ListRef::Search => ScrollId::SearchResults,
        ListRef::SearchDetail => ScrollId::SearchDetail,
        ListRef::Help => ScrollId::Help,
    }
}

/// The filter counter's label.
fn scope_label(l: ListRef) -> &'static str {
    match l {
        ListRef::Queue => "Queue",
        ListRef::BArtists => "Artists",
        ListRef::BAlbums => "Albums",
        ListRef::BTracks => "Tracks",
        ListRef::PlList => "Playlists",
        ListRef::PlItems => "Playlist tracks",
        ListRef::Search => "Search",
        ListRef::SearchDetail => "Album",
        ListRef::Help => "Help",
    }
}

/// Fisher–Yates shuffle of `[0, n)`, seeded from the clock.
fn shuffled_indices(n: usize) -> Vec<usize> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut indices: Vec<usize> = (0..n).collect();
    if n < 2 {
        return indices;
    }
    let mut seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() ^ d.subsec_nanos() as u64)
        .unwrap_or(0)
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    for i in (1..n).rev() {
        seed ^= seed >> 30;
        seed = seed.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        seed ^= seed >> 27;
        seed = seed.wrapping_mul(0x94D0_49BB_1331_11EB);
        seed ^= seed >> 31;
        let j = (seed as usize) % (i + 1);
        indices.swap(i, j);
    }
    indices
}

/// The `playlist-pos` mpv should report after a change shmc made. A consume
/// or a reorder makes mpv report stale positions first; those must be ignored.
struct PosGuard {
    expected: usize,
    ignores: u32,
    /// Consume leaves at most two stale events; a reorder can leave thousands,
    /// so it is bounded by `until`.
    max_ignores: Option<u32>,
    /// No guard may outlive its burst or it would swallow unrelated events.
    until: Instant,
}

/// Both bursts are over in milliseconds; this is a safety net.
const POS_GUARD_TTL: Duration = Duration::from_secs(2);

impl PosGuard {
    fn consume(expected: usize) -> Self {
        Self { expected, ignores: 0, max_ignores: Some(2), until: Instant::now() + POS_GUARD_TTL }
    }

    fn reorder(expected: usize) -> Self {
        Self { expected, ignores: 0, max_ignores: None, until: Instant::now() + POS_GUARD_TTL }
    }
}

/// The `Move`s that turn the queue into `order` (`order[new] = old`).
struct QueuePlan {
    /// `(from, to)` in send order.
    moves: Vec<(usize, usize)>,
    /// `position[old] = new`
    position: Vec<usize>,
    /// Where the playing entry ends up.
    play: Option<usize>,
    /// A move shifted the playing entry on the way, so mpv will report
    /// intermediate positions and a guard is needed.
    play_moved: bool,
}

/// Moves each track into place, slot 0 first. Tracks not yet placed keep their
/// order, so a track sits at `t` plus how many of them are below it.
fn queue_move_plan(order: &[usize], play: Option<usize>) -> Option<QueuePlan> {
    let n = order.len();
    let mut seen = vec![false; n];
    let mut position = vec![0usize; n];
    for (new, &old) in order.iter().enumerate() {
        // refuse before any move is sent: it would corrupt both queues
        if old >= n || seen[old] {
            return None;
        }
        seen[old] = true;
        position[old] = new;
    }

    let mut live = Fenwick::full(n);
    let mut moves = Vec::new();
    let mut play = play.filter(|&p| p < n);
    let mut play_moved = false;
    for (target, &item) in order.iter().enumerate() {
        let from = target + live.below(item);
        // `item` is still live, so every move is a backward one
        debug_assert!(from >= target);
        if from != target {
            moves.push((from, target));
            if let Some(at) = play {
                if at == from {
                    play = Some(target);
                    play_moved = true;
                } else if at < from && at >= target {
                    play = Some(at + 1);
                    play_moved = true;
                }
            }
        }
        live.remove(item);
    }

    Some(QueuePlan { moves, position, play, play_moved })
}

/// Spelled out so the crate does not need the very new `isolate_lowest_one`.
fn low_bit(i: usize) -> usize {
    1 << i.trailing_zeros()
}

/// Fenwick tree over "still to be placed": `below(i)` counts the live items
/// with a smaller original index.
struct Fenwick {
    tree: Vec<usize>,
    len: usize,
}

impl Fenwick {
    fn full(n: usize) -> Self {
        // every item live, built in O(n)
        let mut tree = vec![0usize; n + 1];
        for i in 1..=n {
            tree[i] += 1;
            let parent = i + low_bit(i);
            if parent <= n {
                tree[parent] += tree[i];
            }
        }
        Self { tree, len: n }
    }

    fn below(&self, i: usize) -> usize {
        let mut idx = i;
        let mut sum = 0;
        while idx > 0 {
            sum += self.tree[idx];
            idx -= low_bit(idx);
        }
        sum
    }

    fn remove(&mut self, i: usize) {
        let mut idx = i + 1;
        while idx <= self.len {
            self.tree[idx] -= 1;
            idx += low_bit(idx);
        }
    }
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

    // playback mirror: the queue is mpv's playlist
    pub queue: Vec<Track>,
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
    /// `d` removals for `u`, newest last. Any other queue change clears it,
    /// since the indices only hold for the queue the removals left.
    removed: Vec<Vec<(usize, Track)>>,
    pub visual: Option<Visual>,
    /// Cleared by the matching event and by every app-initiated playback
    /// change.
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
    /// When "clear the queue?" was asked; `y` or `D` confirms, any other key
    /// cancels, and it times out like `pending_playlist`.
    pending_clear: Option<Instant>,
    /// The playlist still arriving after its first page.
    playlist_load: Option<PlaylistLoad>,
    queue_seq: QueueSeq,
    /// 0 = previews off.
    pub preview_tracks: usize,
    pub preview_debounce_ms: u64,
    /// Ask before queueing more tracks than this.
    pub confirm_playlist_over: usize,
    /// After this long a y/n question lapses and the key goes through.
    pub confirm_timeout_secs: u64,
    preview_epoch: u64,
    /// Not asked again while it stays highlighted.
    preview_blocked: Option<String>,
    pub audio_cache: HashMap<String, AudioInfo>,
    /// The track mpv failed on; `>` while stopped resumes after it. A reorder
    /// remaps it, an insert or removal clears it.
    pub failed_at: Option<usize>,
    /// Failures since a file last loaded; a burst means the queue or the
    /// server is broken, not one file.
    err_streak: u32,

    pub browser: Browser,
    pub queue_state: TableState,
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
    /// [0, 0] = unlimited.
    pub art_max_px: [u32; 2],

    pub scroll: ScrollStore,
    pub scrolloff: usize,
    pub center_cursor: bool,
    pub auto_follow: bool,
    /// Follows the playing row until the user browses; `o` re-arms it.
    queue_follow: bool,
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
    pub queue_columns: Vec<crate::config::QueueColumn>,
    pub queue_filter: ListFilter,
    pub browser_filters: [ListFilter; 3], // artists, albums, tracks
    pub pl_list_filter: ListFilter,
    pub pl_items_filter: ListFilter,

    pub status: String,
    pub status_error: bool,
    status_epoch: u64,
    pending_goto: Option<PendingGoto>,
    pending_g: bool,

    msg_tx: UnboundedSender<Msg>,
}

/// Failures in a row that mean the queue is unplayable, not one bad file; a
/// dead server would otherwise run through the queue in seconds.
const FAILED_STREAK_LIMIT: u32 = 3;

const MAX_UNDO: usize = 20;

/// Ctrl-/Alt-letters are shortcuts, never text for an input box.
fn has_ctrl_or_alt(key: &crossterm::event::KeyEvent) -> bool {
    use crossterm::event::KeyModifiers;
    key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

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
            queue: Vec::new(),
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
            playlist_load: None,
            queue_seq: QueueSeq::default(),
            preview_tracks: ui.preview_tracks,
            preview_debounce_ms: ui.preview_debounce_ms,
            confirm_playlist_over: ui.confirm_playlist_over,
            confirm_timeout_secs: 10,
            preview_epoch: 0,
            preview_blocked: None,
            audio_cache: HashMap::new(),
            failed_at: None,
            err_streak: 0,
            browser: Browser::default(),
            queue_state: TableState::default(),
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
            art_max_px,
            scroll: ScrollStore::default(),
            scrolloff: ui.scrolloff,
            center_cursor: ui.center_cursor,
            auto_follow: ui.auto_follow,
            queue_follow: ui.auto_follow,
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
            queue_filter: ListFilter::default(),
            browser_filters: Default::default(),
            pl_list_filter: ListFilter::default(),
            pl_items_filter: ListFilter::default(),
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
        self.current.and_then(|i| self.queue.get(i))
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

    /// The epoch bump makes the pending timer's `ClearStatus` a no-op, so a
    /// message set by this same key press keeps its full lifetime.
    fn dismiss_status(&mut self) {
        if self.status.is_empty() {
            return;
        }
        self.status.clear();
        self.status_error = false;
        self.status_epoch += 1;
    }

    pub fn handle_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Key(ev) => self.handle_key(ev),
            Msg::Paste(text) => self.handle_paste(text),
            Msg::Quit => self.should_quit = true,
            Msg::Resize => {
                // ratatui clears the screen on a size change; the cover's cells
                // would stay skipped as "already painted".
                self.art.shown_rect = None;
                // a drag-resize sends the image once, after it settles
                self.hold_art();
                // a font zoom or another display changes the cell size
                let new = self.picker.as_ref().and_then(|old| {
                    crate::ui::picker_for_cell(old, crate::ui::physical_cell_size())
                });
                if let Some(picker) = new {
                    self.picker = Some(picker);
                    self.art.protocol = None;
                    self.art.prepared_rect = None;
                }
            }
            Msg::ArtistsLoaded(artists) => {
                self.browser.artists = artists;
                self.refilter(ListRef::BArtists);
                self.browser.artists_state.select(if self.browser.artists.is_empty() {
                    None
                } else {
                    Some(0)
                });
            }
            Msg::AlbumsLoaded { artist_key, albums } => {
                if self.browser.albums_for.as_deref() == Some(artist_key.as_str()) {
                    self.browser.albums = albums;
                    sort_albums(&mut self.browser.albums, &self.album_sort);
                    self.browser.albums_state.select(if self.browser.albums.is_empty() {
                        None
                    } else {
                        Some(0)
                    });
                    self.browser.tracks.clear();
                    self.drop_visual(ListRef::BTracks);
                    self.browser.tracks_state.select(None);
                    self.browser.tracks_for = None;
                    self.refilter(ListRef::BAlbums);
                    self.continue_goto_artist(&artist_key);
                }
            }
            Msg::TracksLoaded { album_key, tracks } => {
                if self.browser.tracks_for.as_deref() == Some(album_key.as_str()) {
                    self.browser.tracks = tracks;
                    self.drop_visual(ListRef::BTracks);
                    self.browser.tracks_state.select(if self.browser.tracks.is_empty() {
                        None
                    } else {
                        Some(0)
                    });
                    self.refilter(ListRef::BTracks);
                    self.continue_goto_album(&album_key);
                }
            }
            Msg::SearchResults { query, hits } => {
                if query == self.search.last_query {
                    self.search.hits = hits;
                    self.search.state.select(if self.search.hits.is_empty() {
                        None
                    } else {
                        Some(0)
                    });
                    self.search.detail = None;
                    self.search.detail_request = None;
                    self.transient(format!("{} results for {:?}", self.search.hits.len(), query));
                }
            }
            Msg::SearchAlbumLoaded { key, label, tracks } => {
                if self.search.detail_request.as_deref() == Some(key.as_str()) {
                    self.search.detail_request = None;
                    let mut state = TableState::default();
                    if !tracks.is_empty() {
                        state.select(Some(0));
                    }
                    self.search.detail = Some(SearchDetail { key, label, tracks, state });
                    self.search.focused = false;
                }
            }
            Msg::PlaylistsLoaded(list) => {
                self.playlists.playlists = list;
                self.playlists.preview_cache.clear();
                self.playlists.items_for = None;
                self.playlists.items_total = None;
                self.playlists.items.clear();
                self.drop_visual(ListRef::PlItems);
                self.refilter(ListRef::PlList);
                self.playlists.state.select(if self.playlists.playlists.is_empty() {
                    None
                } else {
                    Some(0)
                });
            }
            Msg::PlaylistSaved(outcome) => {
                use crate::plex::SaveOutcome;
                self.saving = false;
                let reload = match outcome {
                    SaveOutcome::Saved { name, count, repeats_skipped } => {
                        tracing::info!(count, repeats_skipped, "saved the queue as a playlist");
                        let mut msg = format!(
                            "saved {count} track{} to \"{name}\"",
                            if count == 1 { "" } else { "s" }
                        );
                        if repeats_skipped > 0 {
                            msg.push_str(&format!(
                                " ({repeats_skipped} repeats skipped: a Plex playlist holds each track once)"
                            ));
                        }
                        self.transient(msg);
                        true
                    }
                    SaveOutcome::Partial { name, saved, total, reason } => {
                        tracing::warn!(saved, total, "saved the queue only in part");
                        self.transient_err(format!(
                            "saved {saved} of {total} tracks to \"{name}\"; the rest were not added: {reason}"
                        ));
                        true
                    }
                    SaveOutcome::NameTaken { name } => {
                        self.transient_err(format!(
                            "a playlist named \"{name}\" already exists; choose another name"
                        ));
                        false
                    }
                    SaveOutcome::Failed { reason } => {
                        tracing::warn!("playlist save failed");
                        self.transient_err(format!("could not save the playlist: {reason}"));
                        false
                    }
                };
                if reload {
                    let _ = self.plex.tx.send(PlexCmd::LoadPlaylists);
                }
            }
            Msg::PlaylistItemsLoaded { rating_key, tracks } => {
                if self.playlists.items_for.as_deref() == Some(rating_key.as_str()) {
                    self.playlists.preview_cache.insert(rating_key, tracks.clone());
                    self.playlists.items = tracks;
                    self.drop_visual(ListRef::PlItems);
                    self.refilter(ListRef::PlItems);
                    self.playlists.items_state.select(if self.playlists.items.is_empty() {
                        None
                    } else {
                        Some(0)
                    });
                    self.announce_preview_cap();
                }
            }
            Msg::PreviewPlaylist { key, epoch } => {
                let still_selected = self.selected_playlist_key().as_deref() == Some(key.as_str());
                if epoch == self.preview_epoch
                    && still_selected
                    && self.tab == Tab::Playlists
                    && self.playlists.focus == PlaylistsCol::List
                    && self.playlists.items_for.as_deref() == Some(key.as_str())
                {
                    self.load_playlist_preview(key);
                }
            }
            Msg::QueueReply { seq, outcome } => {
                for due in self.queue_seq.deliver(seq, outcome) {
                    self.apply_queue_outcome(due);
                }
            }
            Msg::PlaylistPage { load_id, tracks, last } => {
                self.apply_playlist_page(load_id, tracks, last)
            }
            Msg::PlaylistLoadStopped { load_id, error } => {
                let Some(l) = self.playlist_load.take_if(|l| l.id == load_id) else { return };
                if l.landed {
                    self.transient_err(format!(
                        "playlist load stopped at {}: {error}",
                        l.progress()
                    ));
                }
            }
            Msg::GotoArtist(artist) => {
                // a late reply must not drag the user back after they left
                if !matches!(&self.pending_goto, Some(p) if p.artist_key == artist.rating_key) {
                    return;
                }
                self.browser.focus = BrowserCol::Artists;
                // the jump selects a real index, which a filter would misplace
                self.browser_filters[0].clear();
                let key = artist.rating_key.clone();
                if !self.browser.artists.iter().any(|a| a.rating_key == key) {
                    self.browser.artists.push(artist);
                    self.browser.artists.sort_by_key(|a| a.title.to_lowercase());
                }
                let pos = self.browser.artists.iter().position(|a| a.rating_key == key);
                self.browser.artists_state.select(pos);
                self.load_albums(key);
            }
            Msg::ArtReady { thumb_path, img } => {
                let wanted = self.current_thumb_path();
                if wanted.as_deref() != Some(thumb_path.as_str()) {
                    // Playback moved on. Without this the path stays "in
                    // flight" and coming back to that track never asks again.
                    if self.art.requested.as_deref() == Some(thumb_path.as_str()) {
                        self.art.requested = None;
                    }
                } else {
                    if self.art.thumb.as_deref() == Some(thumb_path.as_str())
                        && self.art.img.is_some()
                    {
                        // Already showing this cover: leave it as it is.
                        self.art.requested = None;
                        return;
                    }
                    self.art.thumb = Some(thumb_path);
                    self.art.requested = None;
                    self.art.img = Some(img);
                    self.art.protocol = None; // force re-encode for current size
                    self.art.shown_rect = None;
                    self.art.prepared_rect = None;
                    // A new cover is drawn at once, not after a dwell.
                    self.art.hold_until = None;
                }
            }
            Msg::ArtDue => {}
            Msg::ArtFailed { thumb_path } => {
                if self.art.requested.as_deref() == Some(thumb_path.as_str()) {
                    self.art.requested = None;
                }
            }
            Msg::TrackInfo { rating_key, audio } => {
                let wanted = self.current_track().map(|t| t.rating_key.as_str())
                    == Some(rating_key.as_str());
                if wanted {
                    if let Some(t) = self.current.and_then(|i| self.queue.get_mut(i)) {
                        t.audio = audio.clone();
                    }
                    self.audio_cache.insert(rating_key, audio.clone());
                    self.audio = audio;
                }
            }
            Msg::Mpv(ev) => self.handle_mpv(ev),
            Msg::Cava(frame) => {
                // cava emits frames before playback starts: don't gate on
                // `stopped`
                self.cava_frame = frame;
                self.cava_alive = true;
                self.cava_note.clear();
            }
            Msg::CavaReady => {
                self.cava_alive = true;
                self.cava_note.clear();
            }
            Msg::CavaDead(hint) => {
                self.cava_alive = false;
                self.cava_note = hint.clone();
                self.transient_err(hint);
            }
            Msg::ClearStatus(epoch) => {
                if epoch == self.status_epoch {
                    self.status.clear();
                    self.status_error = false;
                }
            }
            Msg::SleepTimer { epoch } => {
                if epoch == self.sleep_epoch && self.sleep_until.is_some() {
                    self.sleep_until = None;
                    self.sleep_mins = None;
                    self.sleep_cancel = None;
                    // Stop and stay stopped: set `pause` rather than toggle it,
                    // as mpv's state is what went stale.
                    self.paused = false;
                    let _ = self.mpv.tx.send(MpvCmd::SetPause { paused: false });
                    if !self.stopped {
                        let _ = self.mpv.tx.send(MpvCmd::Stop);
                        self.expect_pos = None;
                        self.stopped = true;
                        self.position = 0.0;
                        self.duration = 0.0;
                    }
                    self.transient("sleep timer: playback stopped".into());
                }
            }
            Msg::Error(e) => {
                tracing::error!(e);
                self.transient_err(e);
            }
            Msg::LoadFailed { key, error } => {
                tracing::error!(error);
                let serves_jump = matches!(
                    &self.pending_goto,
                    Some(p) if p.artist_key == key || p.album_key.as_deref() == Some(key.as_str())
                );
                if serves_jump {
                    let pg = self.pending_goto.take().expect("serves_jump checked above");
                    self.goto_tab(pg.from);
                    self.transient_err("Album not found in library".into());
                } else {
                    self.transient_err(error);
                }
            }
            Msg::PreviewFailed { key } => {
                // Retry on the next highlight, not on every message: a dead
                // server would be asked nonstop.
                if self.playlists.items_for.as_deref() == Some(key.as_str()) {
                    self.playlists.items_for = None;
                    self.preview_blocked = Some(key);
                }
            }
        }
        self.preview_playlist();
    }

    /// Whether `pos` is a stale position to ignore. The guard ends on its
    /// expected value, count or deadline.
    fn stale_position(&mut self, pos: usize) -> bool {
        let Some(guard) = self.expect_pos.as_mut() else {
            return false;
        };
        if pos == guard.expected {
            self.expect_pos = None;
            return false;
        }
        guard.ignores = guard.ignores.saturating_add(1);
        let spent = guard.max_ignores.is_some_and(|max| guard.ignores > max)
            || Instant::now() >= guard.until;
        if spent {
            self.expect_pos = None;
        }
        true
    }

    fn handle_mpv(&mut self, ev: MpvEvent) {
        match ev {
            MpvEvent::Property { name, value } => match name.as_str() {
                "time-pos" => {
                    if let Some(v) = value.as_ref().and_then(|v| v.as_f64()) {
                        self.position = v;
                        // a failing file never gets a playback time, so this is
                        // the one signal that a track really plays
                        if v > 0.0 {
                            self.err_streak = 0;
                            self.failed_at = None;
                        }
                    }
                }

                "pause" => {
                    if let Some(v) = value.as_ref().and_then(|v| v.as_bool()) {
                        self.paused = v;
                        if v {
                            self.cava_frame = vec![0; crate::playback::cava::BARS];
                        }
                    }
                }
                "volume" => {
                    if let Some(v) = value.as_ref().and_then(|v| v.as_f64()) {
                        self.volume = v;
                    }
                }
                "loop-playlist" => self.loop_playlist = prop_on(&value),
                "loop-file" => self.loop_file = prop_on(&value),
                // shmc owns the shuffled order
                "shuffle" => {}
                "audio-bitrate" => {
                    self.bitrate_bps = value.as_ref().and_then(|v| v.as_f64()).unwrap_or(0.0);
                }
                "playlist-pos" => {
                    let pos = value.as_ref().and_then(|v| v.as_i64());
                    match pos {
                        Some(p) if p >= 0 => {
                            let p = p as usize;
                            // stale: the queue shrank (consume) before mpv's
                            // correction; `current` must not point past the end
                            if p >= self.queue.len() {
                                return;
                            }
                            if self.stale_position(p) {
                                return;
                            }
                            let changed = self.current != Some(p);
                            self.current = Some(p);
                            self.stopped = false;
                            self.fetch_art_for_current();
                            self.fetch_audio_for_current();
                            if changed {
                                self.follow_queue();
                            }
                        }
                        _ => {
                            self.expect_pos = None;
                            self.current = None;
                            self.stopped = true;
                            self.position = 0.0;
                            self.audio = AudioInfo::default();
                            self.cava_frame = vec![0; crate::playback::cava::BARS];
                        }
                    }
                }
                _ => {}
            },
            MpvEvent::EndFile { reason } => {
                if reason == "error" {
                    self.err_streak += 1;
                    // mpv may fail a few entries before the Stop lands, so keep
                    // the first failure of the burst
                    let marker = self.failed_at.or(self.current);
                    self.failed_at = marker;
                    let title = marker
                        .and_then(|i| self.queue.get(i))
                        .map(|t| t.title.clone())
                        .unwrap_or_else(|| "track".into());
                    // with repeat on mpv wraps; the streak limit ends a queue
                    // where everything fails
                    let more = (self.loop_playlist && self.queue.len() > 1)
                        || marker.map_or(!self.queue.is_empty(), |f| f + 1 < self.queue.len());
                    tracing::info!(
                        track = %crate::redact::escape_field(&title),
                        streak = self.err_streak,
                        "mpv failed to play a track"
                    );
                    if !more || self.err_streak >= FAILED_STREAK_LIMIT {
                        let _ = self.mpv.tx.send(MpvCmd::Stop);
                        self.stopped = true;
                        self.position = 0.0;
                        self.duration = 0.0;
                        let hint = if more {
                            let key = self
                                .keymap
                                .first_key(self.key_ctx(), &Act::NextTrack)
                                .unwrap_or_else(|| ":n".into());
                            format!("press {key} for the next track")
                        } else {
                            "pick another song".into()
                        };
                        self.transient_err(format!("{title} failed to play; {hint}"));
                    } else {
                        self.transient_err(format!("{title} failed to play; skipping"));
                    }
                }
                if reason == "eof" && self.consume {
                    if let Some(c) = self.current {
                        self.consume_finished(c);
                    }
                }
            }
            MpvEvent::Shutdown => {
                self.transient_err("mpv exited".into());
                self.stopped = true;
                self.mpv_exited = true;
            }
            MpvEvent::Ready => {}
        }
    }

    fn current_thumb_path(&self) -> Option<String> {
        self.current_track().and_then(|t| t.thumb.clone()).or_else(|| {
            self.current_track()
                .and_then(|t| t.parent_rating_key.clone())
                .map(|k| format!("/library/metadata/{}/thumb", k))
        })
    }

    fn fetch_art_for_current(&mut self) {
        if !self.art_enabled {
            return;
        }
        let Some(path) = self.current_thumb_path() else {
            self.art.clear();
            return;
        };
        if self.art.img.is_some() && self.art.thumb.as_deref() == Some(path.as_str()) {
            return;
        }
        if self.art.requested.as_deref() == Some(path.as_str()) {
            return;
        }
        self.art.requested = Some(path.clone());
        let _ = self.plex.tx.send(PlexCmd::LoadThumb { path });
    }

    /// Shows the bulk audio info at once, then asks for the full metadata.
    fn fetch_audio_for_current(&mut self) {
        let Some(t) = self.current_track() else {
            self.audio = AudioInfo::default();
            return;
        };
        let key = t.rating_key.clone();
        let bulk = t.audio.clone();
        let full = t.audio.sample_rate_hz.is_some() || t.audio.bit_depth.is_some();
        let duration = t.duration_ms.map(|ms| ms as f64 / 1000.0).unwrap_or(0.0);
        self.duration = duration;
        if full {
            self.audio = bulk;
            return;
        }
        if let Some(cached) = self.audio_cache.get(&key).cloned() {
            self.audio = cached;
            return;
        }
        self.audio = bulk;
        let _ = self.plex.tx.send(PlexCmd::LoadTrackInfo(key));
    }

    /// Failures are shown here, not sent as `Msg::Error`, so they stay in
    /// request order.
    fn apply_queue_outcome(&mut self, outcome: QueueOutcome) {
        match outcome {
            QueueOutcome::Tracks { tracks, confirm, action } => match action {
                QueueAction::Append => self.add_tracks(tracks, confirm),
                QueueAction::Replace => {
                    // a playlist that fits in its first page gets plain `Tracks`
                    // and no pages, so its load ends here
                    self.playlist_load = None;
                    self.replace_queue_and_play(tracks, confirm);
                }
                QueueAction::PlayNext => self.play_next_tracks(tracks, confirm),
            },
            QueueOutcome::Streaming { tracks, confirm, load_id, total } => {
                let n = tracks.len();
                if self.replace_queue_and_play(tracks, confirm) {
                    if let Some(l) = self.playlist_load.as_mut().filter(|l| l.id == load_id) {
                        l.landed = true;
                        l.loaded = n;
                        l.total = total.max(n);
                    }
                } else {
                    self.cancel_playlist_load();
                }
            }
            QueueOutcome::Failed(e) => {
                tracing::error!(e);
                self.transient_err(e);
            }
            QueueOutcome::StartMissing => {
                let mut msg = String::from("that track is no longer in the playlist");
                if let Some(key) = self.keymap.first_key(KeyCtx::Playlists, &Act::Activate) {
                    msg.push_str(&format!(
                        "; press {key} on the playlist to play it from the start"
                    ));
                }
                self.transient_err(msg);
            }
        }
    }

    /// Tracks already in hand still take a request number, so they land after
    /// earlier requests that are loading.
    fn queue_local(&mut self, tracks: Vec<Track>, confirm: Confirm, action: QueueAction) {
        let id = self.issue_seq(action == QueueAction::Replace);
        let due = self.queue_seq.deliver(id, QueueOutcome::Tracks { tracks, confirm, action });
        if due.is_empty() {
            self.transient("queued after the request that is still loading".into());
            return;
        }
        for o in due {
            self.apply_queue_outcome(o);
        }
    }

    /// Appends without playing.
    pub fn add_tracks(&mut self, tracks: Vec<Track>, confirm: Confirm) {
        match self.append_playable(tracks) {
            0 => self.transient_err("no playable media for selection".into()),
            n => self.transient(confirm.added(n)),
        }
    }

    /// The tracks that can stream, with their URLs. Built together so the
    /// queue and mpv's playlist cannot drift apart by one.
    fn playable(&self, tracks: Vec<Track>) -> (Vec<Track>, Vec<LoadItem>) {
        tracks
            .into_iter()
            .filter_map(|t| {
                let item = self.load_item(&t)?;
                Some((t, item))
            })
            .unzip()
    }

    fn load_item(&self, t: &Track) -> Option<LoadItem> {
        Some(LoadItem { url: t.stream_url(&self.server, &self.token)?, title: t.title.clone() })
    }

    /// New tracks join the end of the pre-shuffle order, so turning shuffle
    /// off keeps them.
    fn extend_shuffle_snapshot(&mut self, tracks: &[Track]) {
        if !self.random {
            return;
        }
        match self.shuffle_snapshot.as_mut() {
            Some(snapshot) => snapshot.order.extend_from_slice(tracks),
            None => self.shuffle_snapshot = Some(ShuffleSnapshot { order: tracks.to_vec() }),
        }
    }

    /// Put the playable `tracks` at the end of the app queue and of mpv's
    /// playlist, in step. Returns how many there were.
    fn append_playable(&mut self, tracks: Vec<Track>) -> usize {
        let (playable, items) = self.playable(tracks);
        let n = playable.len();
        if n == 0 {
            return 0;
        }
        self.extend_shuffle_snapshot(&playable);
        self.queue.extend(playable);
        self.removed.clear();
        let _ = self.mpv.tx.send(MpvCmd::Append(items));
        self.refilter(ListRef::Queue);
        if self.queue_state.selected().is_none() {
            self.queue_state.select(Some(0));
        }
        n
    }

    /// Dropped when its load was superseded.
    fn apply_playlist_page(&mut self, load_id: u64, tracks: Vec<Track>, last: bool) {
        let Some(l) = self.playlist_load.as_mut().filter(|l| l.id == load_id && l.landed) else {
            tracing::debug!(load_id, "dropping a page of a superseded playlist load");
            return;
        };
        l.loaded += tracks.len();
        l.total = l.total.max(l.loaded);
        let progress = l.progress();
        self.append_playable(tracks);
        if last {
            self.playlist_load = None;
            self.dismiss_status();
        } else {
            self.transient(format!("loading playlist {progress}"));
        }
    }

    /// Forget the load (late pages are ignored) and stop the worker.
    fn cancel_playlist_load(&mut self) {
        if let Some(l) = self.playlist_load.take() {
            l.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// A replace also supersedes a playlist that is still loading.
    fn issue_seq(&mut self, replaces: bool) -> u64 {
        if replaces {
            self.cancel_playlist_load();
        }
        self.queue_seq.issue(replaces)
    }

    fn supersede_queue(&mut self) {
        self.cancel_playlist_load();
        self.queue_seq.supersede();
    }

    /// Anything that adds, reorders or reshuffles would land among tracks that
    /// have not arrived yet. Says so and returns true.
    fn refuse_while_loading(&mut self) -> bool {
        let Some(l) = &self.playlist_load else { return false };
        let msg = if l.landed {
            format!("playlist still loading ({})", l.progress())
        } else {
            "playlist still loading".into()
        };
        self.transient(msg);
        true
    }

    pub fn replace_queue_and_play(&mut self, tracks: Vec<Track>, confirm: Confirm) -> bool {
        let (mut playable, _) = self.playable(tracks);
        let n = playable.len();
        if n == 0 {
            self.transient_err("no playable media for selection".into());
            return false;
        }
        self.shuffle_new_queue(&mut playable);
        let (playable, items) = self.playable(playable);
        self.queue = playable;
        self.removed.clear();
        self.drop_visual(ListRef::Queue);
        let _ = self.mpv.tx.send(MpvCmd::Play(items));
        self.current = Some(0);
        self.expect_pos = None;
        self.stopped = false;
        self.position = 0.0;
        self.failed_at = None;
        self.err_streak = 0;
        self.queue_filter.clear();
        self.refilter(ListRef::Queue);
        self.queue_state.select(Some(0));
        self.fetch_art_for_current();
        self.fetch_audio_for_current();
        self.transient(confirm.playing(n));
        self.follow_queue();
        true
    }

    /// Inserts after the current track without interrupting it.
    pub fn play_next_tracks(&mut self, tracks: Vec<Track>, confirm: Confirm) {
        let (playable, items) = self.playable(tracks);
        let n = playable.len();
        if n == 0 {
            self.transient_err("no playable media for selection".into());
            return;
        }
        if self.queue.is_empty() && self.current.is_none() {
            self.replace_queue_and_play(playable, confirm);
            return;
        }
        // `current` can be one past the end after a consume, before mpv corrects it
        let index = self.current.map_or(0, |c| (c + 1).min(self.queue.len()));
        let append_at = self.queue.len();
        self.expect_pos = None;
        self.extend_shuffle_snapshot(&playable);
        let _ = self.mpv.tx.send(MpvCmd::Insert { index, append_at, items });
        self.queue.splice(index..index, playable);
        self.removed.clear();
        self.drop_visual(ListRef::Queue);
        // inserting shifts later indices (see `failed_at`)
        self.failed_at = None;
        self.refilter(ListRef::Queue);
        self.select_queue_row(index);
        self.transient(format!("queued next: {n} track{}", if n == 1 { "" } else { "s" }));
    }

    /// Put the queue cursor on real index `real`. The cursor counts filtered
    /// rows, so a row the filter hides keeps the old cursor, clamped.
    fn select_queue_row(&mut self, real: usize) {
        let vis = self.vis_len(ListRef::Queue);
        if vis == 0 {
            self.queue_state.select(None);
            return;
        }
        let sel = self
            .queue_filter
            .cursor_of(real)
            .unwrap_or_else(|| self.queue_state.selected().unwrap_or(0));
        self.queue_state.select(Some(sel.min(vis - 1)));
    }

    pub fn play_queue_index(&mut self, idx: usize) {
        if idx >= self.queue.len() {
            return;
        }
        let _ = self.mpv.tx.send(MpvCmd::PlayIndex(idx));
        self.current = Some(idx);
        self.expect_pos = None;
        self.stopped = false;
        self.failed_at = None;
        self.err_streak = 0;
        self.select_queue_row(idx);
        self.fetch_art_for_current();
        self.fetch_audio_for_current();
        self.follow_queue();
    }

    /// Where index `i` lands when the entry at `from` moves to `to`.
    fn shifted(i: usize, from: usize, to: usize) -> usize {
        if i == from {
            to
        } else if from < i && to >= i {
            i - 1
        } else if from > i && to <= i {
            i + 1
        } else {
            i
        }
    }

    /// J/K. Refused while filtered: the visible row would silently name a
    /// different slot.
    fn move_queue(&mut self, dir: i32) {
        if self.random {
            self.transient("turn shuffle off to reorder".into());
            return;
        }
        if self.queue_filter.is_active() {
            self.transient("clear the queue filter to reorder".into());
            return;
        }
        let Some(from) = self.real_at(ListRef::Queue) else { return };
        let to = if dir < 0 {
            let Some(t) = from.checked_sub(1) else { return };
            t
        } else if from + 1 < self.queue.len() {
            from + 1
        } else {
            return;
        };
        let item = self.queue.remove(from);
        self.queue.insert(to, item);
        self.removed.clear();
        self.drop_visual(ListRef::Queue);
        self.expect_pos = None;
        let _ = self.mpv.tx.send(MpvCmd::Move { from, to });
        if let Some(c) = self.current {
            self.current = Some(Self::shifted(c, from, to));
        }
        if let Some(f) = self.failed_at {
            self.failed_at = Some(Self::shifted(f, from, to));
        }
        self.refilter(ListRef::Queue);
        self.queue_state.select(Some(to.min(self.vis_len(ListRef::Queue).saturating_sub(1))));
    }

    fn start_visual(&mut self) {
        let l = self.active_list();
        if !matches!(l, ListRef::Queue | ListRef::BTracks | ListRef::PlItems) {
            self.transient("visual mode works in the tracks column".into());
            return;
        }
        if self.cursor_of(l).is_none() {
            return;
        }
        let Some(anchor) = self.real_at(l) else { return };
        self.visual = Some(Visual { list: l, anchor });
        let ctx = self.key_ctx();
        let acts: &[(&Act, &str)] = if l == ListRef::Queue {
            &[(&Act::RemoveFromQueue, "remove")]
        } else {
            &[(&Act::AddToQueue, "add"), (&Act::PlayNext, "insert"), (&Act::Activate, "play")]
        };
        let mut hint = String::from("visual: move to select");
        for (act, label) in acts {
            if let Some(key) = self.keymap.first_key(ctx, act) {
                hint.push_str(&format!(" · {key} {label}"));
            }
        }
        hint.push_str(" · Esc cancel");
        self.transient(hint);
    }

    fn drop_visual(&mut self, l: ListRef) {
        if self.visual.is_some_and(|v| v.list == l) {
            self.visual = None;
        }
    }

    /// The selection as inclusive visible-row positions; None while the anchor
    /// is filtered out or the cursor is in another list.
    pub fn visual_range(&self) -> Option<(usize, usize)> {
        let v = self.visual?;
        if v.list != self.active_list() {
            return None;
        }
        let a = self.filter(v.list)?.cursor_of(v.anchor)?;
        let c = self.cursor_of(v.list)?;
        Some((a.min(c), a.max(c)))
    }

    fn remove_visual_range(&mut self) {
        let Some((lo, hi)) = self.visual_range() else {
            self.visual = None;
            return;
        };
        let len = self.queue.len();
        let indices: Vec<usize> = (lo..=hi).map(|fi| self.queue_filter.real(fi, len)).collect();
        self.remove_indices(&indices);
    }

    /// Queues the Browser or Playlists selection and leaves visual mode; a
    /// refused add keeps it.
    fn queue_visual_range(&mut self, action: QueueAction) {
        if action != QueueAction::Replace && self.refuse_while_loading() {
            return;
        }
        let Some((lo, hi)) = self.visual_range() else {
            self.visual = None;
            return;
        };
        let Some(l) = self.visual.map(|v| v.list) else { return };
        let (items, filter) = match l {
            ListRef::BTracks => (&self.browser.tracks, &self.browser_filters[2]),
            ListRef::PlItems => (&self.playlists.items, &self.pl_items_filter),
            _ => return,
        };
        let tracks: Vec<Track> =
            (lo..=hi).map(|fi| items[filter.real(fi, items.len())].clone()).collect();
        self.visual = None;
        let confirm = match tracks.as_slice() {
            [t] => Confirm::Track { title: t.title.clone() },
            _ => Confirm::Tracks,
        };
        self.queue_local(tracks, confirm, action);
    }

    pub fn remove_from_queue(&mut self, idx: usize) {
        self.remove_indices(&[idx]);
    }

    /// `indices`: ascending, unique, real queue indices.
    fn remove_indices(&mut self, indices: &[usize]) {
        if indices.is_empty() || indices.last().is_some_and(|&i| i >= self.queue.len()) {
            return;
        }
        self.drop_visual(ListRef::Queue);
        if self.removed.len() == MAX_UNDO {
            self.removed.remove(0);
        }
        self.removed.push(indices.iter().map(|&i| (i, self.queue[i].clone())).collect());
        // descending: each index is still valid when mpv gets to it
        for &i in indices.iter().rev() {
            self.queue.remove(i);
            let _ = self.mpv.tx.send(MpvCmd::Remove(i));
        }
        self.failed_at = None;
        self.expect_pos = None;
        let first_row = indices
            .first()
            .and_then(|&i| self.queue_filter.cursor_of(i))
            .or(self.queue_state.selected())
            .unwrap_or(0);
        // the playing track was pulled out and nothing has replaced it yet
        let mut resume = None;
        if let Some(c) = self.current {
            let below = indices.iter().filter(|&&i| i < c).count();
            if indices.binary_search(&c).is_ok() {
                // mpv stops when its playing entry is removed; hand over to the
                // track that slid into the slot
                self.current = None;
                resume = Some(c - below);
            } else {
                self.current = Some(c - below);
                if below >= 2 {
                    self.expect_pos = Some(PosGuard::reorder(c - below));
                }
            }
        }
        self.refilter(ListRef::Queue);
        let vis = self.vis_len(ListRef::Queue);
        if vis == 0 {
            self.queue_state.select(None);
        } else {
            self.queue_state.select(Some(first_row.min(vis - 1)));
        }
        if let Some(idx) = resume {
            if idx < self.queue.len() {
                // the cursor stays put; `o` jumps to the playing track
                let _ = self.mpv.tx.send(MpvCmd::PlayIndex(idx));
                self.current = Some(idx);
                self.stopped = false;
                self.failed_at = None;
                self.err_streak = 0;
                self.fetch_art_for_current();
                self.fetch_audio_for_current();
                self.follow_queue();
            } else {
                self.position = 0.0;
                self.duration = 0.0;
                self.stopped = true;
                self.audio = AudioInfo::default();
            }
        }
    }

    pub fn undo_remove(&mut self) {
        let Some(batch) = self.removed.last() else {
            self.transient("nothing to undo".into());
            return;
        };
        let Some(items) = batch.iter().map(|(_, t)| self.load_item(t)).collect::<Option<Vec<_>>>()
        else {
            self.transient_err("cannot restore: no playable media".into());
            return;
        };
        let Some(batch) = self.removed.pop() else { return };
        self.drop_visual(ListRef::Queue);
        self.expect_pos = None;
        self.failed_at = None;
        let old_current = self.current;
        let first_index = batch[0].0.min(self.queue.len());
        let mut start = 0;
        while start < batch.len() {
            let mut end = start + 1;
            while end < batch.len() && batch[end].0 == batch[end - 1].0 + 1 {
                end += 1;
            }
            let len = self.queue.len();
            let index = batch[start].0.min(len);
            let _ = self.mpv.tx.send(MpvCmd::Insert {
                index,
                append_at: len,
                items: items[start..end].to_vec(),
            });
            self.queue.splice(index..index, batch[start..end].iter().map(|(_, t)| t.clone()));
            if let Some(c) = self.current {
                if index <= c {
                    self.current = Some(c + (end - start));
                }
            }
            start = end;
        }
        // Insert appends and then moves each track into place
        if let (Some(old), Some(new)) = (old_current, self.current) {
            if new - old >= 2 {
                self.expect_pos = Some(PosGuard::reorder(new));
            }
        }
        self.refilter(ListRef::Queue);
        self.select_queue_row(first_index);
        if batch.len() == 1 {
            self.transient(format!("restored: {}", batch[0].1.title));
        } else {
            self.transient(format!("restored {} tracks", batch.len()));
        }
    }

    /// The track at `idx` finished and mpv has already advanced. Never send
    /// `playlist-play-index` here: it would restart the next track at 0:00.
    fn consume_finished(&mut self, idx: usize) {
        if idx >= self.queue.len() {
            return;
        }
        self.removed.clear();
        // the anchor stays on its track, or on the next one if that was it
        self.visual = self.visual.and_then(|v| {
            if v.list != ListRef::Queue {
                return Some(v);
            }
            let a = if idx < v.anchor { v.anchor - 1 } else { v.anchor };
            (self.queue.len() > 1).then(|| Visual { anchor: a.min(self.queue.len() - 2), ..v })
        });
        // The cursor counts filtered rows: remember which item it is on.
        let sel_real = self.real_at(ListRef::Queue);
        self.queue.remove(idx);
        self.failed_at = None;
        let _ = self.mpv.tx.send(MpvCmd::Remove(idx));

        self.refilter(ListRef::Queue);
        if let Some(r) = sel_real.filter(|&r| r != idx) {
            let moved = if r > idx { r - 1 } else { r };
            if let Some(c) = self.queue_filter.cursor_of(moved) {
                self.queue_state.select(Some(c));
            }
        }

        let next = if idx < self.queue.len() {
            Some(idx)
        } else if self.loop_playlist && !self.queue.is_empty() {
            // repeat: mpv wraps to the first remaining entry
            Some(0)
        } else {
            None
        };
        // mpv may report the pre-removal position of the next track
        self.expect_pos = next.map(PosGuard::consume);
        match next {
            Some(i) => {
                self.current = Some(i);
                self.stopped = false;
                self.position = 0.0;
                self.duration = 0.0;
                self.fetch_art_for_current();
                self.fetch_audio_for_current();
                self.follow_queue();
            }
            None => {
                self.current = None;
                self.stopped = true;
                self.position = 0.0;
                self.duration = 0.0;
                self.audio = AudioInfo::default();
            }
        }

        let vis = self.vis_len(ListRef::Queue);
        if vis == 0 {
            self.queue_state.select(None);
        } else {
            let sel = self.queue_state.selected().unwrap_or(0).min(vis - 1);
            self.queue_state.select(Some(sel));
        }
    }

    fn ask_clear_queue(&mut self) {
        if self.queue.is_empty() {
            // nothing to lose, but a replace still loading must go
            self.supersede_queue();
            self.clear_queue();
            return;
        }
        self.pending_playlist = None;
        self.pending_clear = Some(Instant::now());
        let key = self
            .keymap
            .first_key(KeyCtx::Queue, &Act::ClearQueue)
            .unwrap_or_else(|| ":clear".into());
        self.transient_secs(
            format!(
                "clear the queue ({} tracks)? press {key} again or y to confirm · any other key cancels",
                self.queue.len()
            ),
            self.confirm_timeout_secs,
            false,
        );
    }

    pub fn clear_queue(&mut self) {
        self.queue.clear();
        self.shuffle_snapshot = None;
        self.removed.clear();
        self.drop_visual(ListRef::Queue);
        self.expect_pos = None;
        self.scroll.clear_top(ScrollId::Queue);
        self.current = None;
        self.stopped = true;
        self.position = 0.0;
        self.duration = 0.0;
        self.queue_state.select(None);
        self.refilter(ListRef::Queue);
        // a bare `playlist-clear` leaves mpv playing the loaded entry
        let _ = self.mpv.tx.send(MpvCmd::Stop);
        let _ = self.mpv.tx.send(MpvCmd::Clear);
        self.art.clear();
        self.audio = AudioInfo::default();
        self.cava_frame = vec![0; crate::playback::cava::BARS];
        self.failed_at = None;
        self.err_streak = 0;
        self.transient("queue cleared".into());
    }

    pub fn load_albums(&mut self, artist_key: String) {
        self.browser.albums_for = Some(artist_key.clone());
        self.browser.albums.clear();
        self.browser.albums_state.select(None);
        self.browser_filters[1].clear(); // new artist = new album list
        self.browser.tracks.clear();
        self.drop_visual(ListRef::BTracks);
        self.browser.tracks_state.select(None);
        self.browser.tracks_for = None;
        let _ = self.plex.tx.send(PlexCmd::LoadAlbums(artist_key));
    }

    pub fn load_tracks(&mut self, album_key: String) {
        self.browser.tracks_for = Some(album_key.clone());
        self.browser.tracks.clear();
        self.drop_visual(ListRef::BTracks);
        self.browser.tracks_state.select(None);
        self.browser_filters[2].clear();
        let _ = self.plex.tx.send(PlexCmd::LoadTracks(album_key));
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

    fn goto_album(&mut self) {
        let t = self.queue_state.selected().and_then(|i| {
            let real = self.queue_filter.real(i, self.queue.len());
            self.queue.get(real).cloned()
        });
        let Some(t) = t else { return };
        self.reveal(t.grandparent_rating_key, t.parent_rating_key, Some(t.rating_key));
    }

    /// Search results and the album view's tracks.
    fn reveal_search(&mut self) {
        use crate::plex::HitKind;
        if let Some(d) = self.search.detail.as_ref() {
            let Some(t) = d.state.selected().and_then(|i| d.tracks.get(i)) else { return };
            let (artist, album, track) =
                (t.grandparent_rating_key.clone(), Some(d.key.clone()), Some(t.rating_key.clone()));
            self.reveal(artist, album, track);
            return;
        }
        let Some(hit) = self.search.state.selected().and_then(|i| self.search.hits.get(i)).cloned()
        else {
            return;
        };
        match hit.kind {
            HitKind::Artist => self.reveal(Some(hit.rating_key), None, None),
            HitKind::Album => self.reveal(hit.artist_key, Some(hit.rating_key), None),
            HitKind::Track => self.reveal(hit.artist_key, hit.album_key, Some(hit.rating_key)),
        }
    }

    fn reveal(
        &mut self,
        artist_key: Option<String>,
        album_key: Option<String>,
        track_key: Option<String>,
    ) {
        let Some(artist_key) = artist_key else {
            self.transient_err("Album not found in library".into());
            return;
        };
        if track_key.is_some() && album_key.is_none() {
            self.transient_err("Album not found in library".into());
            return;
        }
        let from = self.tab;
        self.goto_tab(Tab::Browser);
        self.pending_goto =
            Some(PendingGoto { artist_key: artist_key.clone(), album_key, track_key, from });
        self.browser.focus = BrowserCol::Artists;
        let _ = self.plex.tx.send(PlexCmd::ResolveArtist(artist_key));
    }

    fn continue_goto_artist(&mut self, artist_key: &str) {
        let wanted = matches!(&self.pending_goto, Some(p) if p.artist_key == artist_key);
        if !wanted {
            return;
        }
        let pg = self.pending_goto.take().expect("wanted checked above");
        let Some(album_key) = pg.album_key.clone() else { return };
        if let Some(pos) = self.browser.albums.iter().position(|a| a.rating_key == album_key) {
            self.browser.albums_state.select(Some(pos));
            self.browser.focus = BrowserCol::Tracks;
            self.load_tracks(album_key);
            self.pending_goto = Some(pg); // the tracks select the row
        } else {
            self.goto_tab(pg.from);
            self.transient_err("Album not found in library".into());
        }
    }

    fn continue_goto_album(&mut self, album_key: &str) {
        let wanted =
            matches!(&self.pending_goto, Some(p) if p.album_key.as_deref() == Some(album_key));
        if !wanted {
            return;
        }
        let pg = self.pending_goto.take().expect("wanted checked above");
        let Some(track_key) = pg.track_key else { return };
        if let Some(pos) = self.browser.tracks.iter().position(|t| t.rating_key == track_key) {
            self.browser.tracks_state.select(Some(pos));
            self.scroll.center_once = Some(ScrollId::BrowserTracks);
        }
    }

    fn handle_key(&mut self, ev: crossterm::event::Event) {
        use crossterm::event::{Event, KeyEvent, KeyEventKind};

        let Event::Key(key @ KeyEvent { kind: KeyEventKind::Press | KeyEventKind::Repeat, .. }) =
            ev
        else {
            return;
        };

        // a held `D` must not answer its own "clear the queue?"
        if key.kind == KeyEventKind::Repeat && self.pending_clear.is_some() {
            return;
        }

        self.dismiss_status();

        // Ctrl-C quits from anywhere, even mid-input
        if keymap::key_id(&key) == "C-c" {
            self.exec(Act::Quit);
            return;
        }

        // an expired question must not swallow a later key as a cancel
        if self
            .pending_playlist
            .as_ref()
            .is_some_and(|p| p.armed_at.elapsed() >= Duration::from_secs(self.confirm_timeout_secs))
        {
            self.pending_playlist = None;
        }

        if let Some(pending) = self.pending_playlist.take() {
            match key.code {
                crossterm::event::KeyCode::Char('y') | crossterm::event::KeyCode::Char('Y') => {
                    self.send_resolve_playlist(
                        pending.key,
                        pending.confirm,
                        pending.action,
                        pending.start,
                        pending.count,
                    );
                }
                _ => self.transient("cancelled".into()),
            }
            return;
        }

        if self
            .pending_clear
            .is_some_and(|t| t.elapsed() >= Duration::from_secs(self.confirm_timeout_secs))
        {
            self.pending_clear = None;
        }
        if self.pending_clear.take().is_some() {
            let id = keymap::key_id(&key);
            let clear_key = self.keymap.is_context_key(KeyCtx::Queue, &id)
                && self.keymap.lookup(KeyCtx::Queue, &id) == Some(&Act::ClearQueue);
            match key.code {
                // `y` matches the playlist question; the clear key itself is
                // already under the finger
                crossterm::event::KeyCode::Char('y') | crossterm::event::KeyCode::Char('Y') => {
                    self.supersede_queue();
                    self.clear_queue();
                }
                _ if clear_key => {
                    self.supersede_queue();
                    self.clear_queue();
                }
                _ => self.transient("cancelled".into()),
            }
            return;
        }

        if self.tab == Tab::Search && self.search.focused {
            self.input_search(key);
            return;
        }
        if self.command.typing {
            self.input_command(key);
            return;
        }
        let filter_typing = {
            let l = self.active_list();
            self.filter(l).map(|f| f.typing).unwrap_or(false)
        };
        if filter_typing {
            self.input_filter(key);
            return;
        }

        let id = keymap::key_id(&key);
        let ctx = self.key_ctx();

        if let Some(v) = self.visual.filter(|v| v.list != ListRef::Queue && !self.pending_g) {
            if v.list != self.active_list() {
                self.visual = None;
            } else if id == "Esc" {
                self.visual = None;
                return;
            } else {
                match self.keymap.lookup(ctx, &id) {
                    Some(Act::VisualMode | Act::Back) => {
                        self.visual = None;
                        return;
                    }
                    Some(Act::AddToQueue) => {
                        self.queue_visual_range(QueueAction::Append);
                        return;
                    }
                    Some(Act::PlayNext) => {
                        self.queue_visual_range(QueueAction::PlayNext);
                        return;
                    }
                    Some(Act::Activate) => {
                        self.queue_visual_range(QueueAction::Replace);
                        return;
                    }
                    Some(
                        Act::StartFilter | Act::ColumnLeft | Act::ColumnRight | Act::JumpToPlaying,
                    ) => self.visual = None,
                    _ => {}
                }
            }
        }

        if self.visual.is_some() && self.tab == Tab::Queue {
            if id == "Esc" {
                self.visual = None;
                return;
            }
            match self.keymap.lookup(ctx, &id) {
                Some(Act::VisualMode) => {
                    self.visual = None;
                    return;
                }
                Some(Act::RemoveFromQueue) => {
                    self.remove_visual_range();
                    return;
                }
                Some(Act::MoveQueueUp | Act::MoveQueueDown) => {
                    self.transient("leave visual mode (Esc) to reorder".into());
                    return;
                }
                Some(
                    Act::Play
                    | Act::UndoRemove
                    | Act::ClearQueue
                    | Act::GotoAlbum
                    | Act::StartFilter,
                ) => {
                    self.visual = None;
                }
                _ => {}
            }
        }

        // `gg` only where a lone `g` is unbound
        if self.pending_g {
            self.pending_g = false;
            if ctx == KeyCtx::Browser {
                self.pending_goto = None;
            }
            if id == "g" {
                if let Some(act) = self.keymap.lookup(ctx, "gg").cloned() {
                    self.exec(act);
                }
                return;
            }
            // letters always jump, even when bound (`g a` must not append);
            // everything else keeps its binding (`g 2` still picks a tab)
            if let Some(c) = id.chars().next().filter(|_| id.chars().count() == 1) {
                if c.is_alphabetic() {
                    self.jump_to_letter(c);
                    return;
                }
            }
        }
        if id == "g" && self.keymap.lookup(ctx, "g").is_none() {
            self.pending_g = true;
            return;
        }

        if let Some(act) = self.keymap.lookup(ctx, &id) {
            if ctx == KeyCtx::Browser && self.keymap.is_context_key(ctx, &id) {
                self.pending_goto = None;
            }
            self.exec(*act);
        }
    }

    fn input_search(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Esc => {
                self.search.focused = false;
                self.search.history_pos = None;
            }
            KeyCode::Enter => {
                let q = self.search.input.trim().to_string();
                self.search.focused = false;
                self.search.history_pos = None;
                if !q.is_empty() {
                    if self.search.history.last() != Some(&q) {
                        self.search.history.push(q.clone());
                        if self.search.history.len() > 20 {
                            self.search.history.remove(0);
                        }
                    }
                    self.search.last_query = q.clone();
                    self.search.detail = None;
                    self.search.detail_request = None;
                    let _ = self.plex.tx.send(PlexCmd::Search(q));
                    self.transient("searching…".into());
                }
            }
            KeyCode::Up => self.search_history(-1),
            KeyCode::Down => self.search_history(1),
            KeyCode::Backspace => {
                self.search.history_pos = None;
                self.search.input.pop();
            }
            KeyCode::Char(c) if !has_ctrl_or_alt(&key) => self.search_push(&c.to_string()),
            _ => {}
        }
    }

    fn search_push(&mut self, text: &str) {
        self.search.history_pos = None;
        self.search.input.push_str(text);
    }

    /// Stepping past the newest entry returns to an empty input.
    fn search_history(&mut self, dir: i32) {
        let len = self.search.history.len();
        if len == 0 {
            return;
        }
        let pos = match (self.search.history_pos, dir < 0) {
            (None, true) => len - 1,
            (Some(p), true) => p.saturating_sub(1),
            (Some(p), false) if p + 1 < len => p + 1,
            (Some(_), false) => {
                self.search.history_pos = None;
                self.search.input.clear();
                return;
            }
            (None, false) => return,
        };
        self.search.history_pos = Some(pos);
        self.search.input = self.search.history[pos].clone();
    }

    fn input_filter(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Esc => self.close_filter(),
            KeyCode::Enter => {
                let l = self.active_list();
                if let Some(f) = self.filter_mut(l) {
                    f.typing = false;
                }
            }
            KeyCode::Backspace => {
                let l = self.active_list();
                let became_empty = match self.filter_mut(l) {
                    Some(f) => {
                        f.text.pop();
                        f.text.is_empty()
                    }
                    None => false,
                };
                if became_empty {
                    self.close_filter();
                } else {
                    self.refilter_active_and_reset();
                }
            }
            KeyCode::Char(c) if !has_ctrl_or_alt(&key) => self.filter_push(&c.to_string()),
            KeyCode::Up => self.with_sel(|st, vis| move_sel(st, vis, -1)),
            KeyCode::Down => self.with_sel(|st, vis| move_sel(st, vis, 1)),
            _ => {}
        }
    }

    fn filter_push(&mut self, text: &str) {
        let l = self.active_list();
        if let Some(f) = self.filter_mut(l) {
            f.text.push_str(text);
        }
        self.refilter_active_and_reset();
    }

    /// A dropped string containing `/` must not start a filter or a search.
    fn handle_paste(&mut self, text: String) {
        if self.filter_typing().is_some() {
            self.filter_push(&text);
        } else if self.tab == Tab::Search && self.search.focused {
            self.search_push(&text);
        }
    }

    /// Clears the filter but keeps the cursor on the same underlying item.
    fn close_filter(&mut self) {
        let l = self.active_list();
        let total = self.total_len(l);
        let cursor = self.cursor_of(l).unwrap_or(0);
        if let Some(f) = self.filter_mut(l) {
            let real = f.real(cursor, total);
            f.clear();
            self.with_sel(|st, vis| {
                st.select(if vis == 0 { None } else { Some(real.min(vis - 1)) });
            });
        }
    }

    fn input_command(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Esc => {
                self.command.typing = false;
                self.command.text.clear();
            }
            KeyCode::Enter => {
                let cmd = std::mem::take(&mut self.command.text);
                self.command.typing = false;
                self.run_command(&cmd);
            }
            KeyCode::Backspace => {
                if self.command.text.pop().is_none() {
                    self.command.typing = false;
                }
            }
            KeyCode::Char(c) if !has_ctrl_or_alt(&key) => self.command.text.push(c),
            _ => {}
        }
    }

    fn save_queue_as_playlist(&mut self, name: &str) {
        if self.queue.is_empty() {
            self.transient_err("the queue is empty".into());
            return;
        }
        if self.refuse_while_loading() {
            return;
        }
        if self.saving {
            self.transient_err("a save is already running".into());
            return;
        }
        let keys = self.queue.iter().map(|t| t.rating_key.clone()).collect();
        self.saving = true;
        let _ = self.plex.tx.send(PlexCmd::SavePlaylist { name: name.to_string(), keys });
        self.transient("saving playlist…".into());
    }

    fn run_command(&mut self, raw: &str) {
        match parse_command(raw) {
            Command::Empty => {}
            Command::Quit => self.should_quit = true,
            Command::Next => self.exec(Act::NextTrack),
            Command::Prev => self.exec(Act::PrevTrack),
            Command::Pause => self.exec(Act::TogglePause),
            Command::Play => {
                if self.stopped {
                    if let Some(i) = self.current {
                        self.play_queue_index(i);
                    } else {
                        self.play_from_stopped();
                    }
                } else if self.paused {
                    self.exec(Act::TogglePause);
                } else {
                    self.transient("already playing".into());
                }
            }
            Command::Stop => {
                // A replace still in flight would start playing over the stop.
                let cancelled = self.queue_seq.pending() || self.playlist_load.is_some();
                self.supersede_queue();
                let was_stopped = self.stopped;
                if !was_stopped {
                    let _ = self.mpv.tx.send(MpvCmd::Stop);
                    self.expect_pos = None;
                    self.stopped = true;
                    self.position = 0.0;
                    self.duration = 0.0;
                }
                self.transient(match (was_stopped, cancelled) {
                    (_, true) => "stopped; cancelled the pending queue change".into(),
                    (true, false) => "already stopped".into(),
                    (false, false) => "stopped".into(),
                });
            }
            Command::Seek(n) => match n.parse::<f64>() {
                Ok(secs) if secs.is_finite() => {
                    let _ = self.mpv.tx.send(MpvCmd::SeekRel(secs));
                    self.transient(format!("seek {secs:+.0}s"));
                }
                _ => {
                    self.transient_err(format!("usage: :seek ±seconds (got {n:?})"));
                }
            },
            Command::Volume(n) => {
                let relative = n.starts_with('+') || n.starts_with('-');
                match n.parse::<f64>() {
                    Ok(v) if v.is_finite() => {
                        self.set_volume(if relative { self.volume + v } else { v });
                        self.transient(format!("volume: {}%", self.volume.round() as i32));
                    }
                    _ => {
                        self.transient_err(format!("usage: :vol 0-100 (got {n:?})"));
                    }
                }
            }
            Command::BadSeek => self.transient_err("usage: :seek ±seconds".into()),
            Command::BadVolume => self.transient_err("usage: :vol 0-100".into()),
            Command::Clear => self.ask_clear_queue(),
            Command::Save(name) => self.save_queue_as_playlist(name),
            Command::BadSave => self.transient_err("usage: :save <name>".into()),
            Command::Shuffle => self.exec(Act::ToggleShuffle),
            Command::Repeat => self.exec(Act::ToggleRepeat),
            Command::Single => self.exec(Act::ToggleSingle),
            Command::Consume => self.exec(Act::ToggleConsume),
            Command::Reshuffle => self.exec(Act::Reshuffle),
            Command::Sleep(arg) => {
                if arg.is_empty() {
                    match self.sleep_mins {
                        Some(m) => self.transient(format!("sleep timer: {m}m")),
                        None => self.transient("sleep timer: off".into()),
                    }
                } else if arg.eq_ignore_ascii_case("off") || arg == "0" {
                    self.set_sleep_timer(None);
                } else {
                    match arg.parse::<u32>() {
                        Ok(m) => self.set_sleep_timer(Some(m)),
                        Err(_) => {
                            self.transient_err(format!("usage: :sleep minutes|off (got {arg:?})"))
                        }
                    }
                }
            }
            Command::BadSleep => {
                self.transient_err("usage: :sleep minutes|off".into());
            }
            Command::Playing => {
                if self.current.is_some() {
                    self.jump_to_playing();
                } else {
                    self.transient("nothing playing".into());
                }
            }
            Command::Tab(Tab::Search) => self.exec(Act::StartSearch),
            Command::Tab(t) => self.goto_tab(t),
            Command::Unknown(raw) => {
                self.transient_err(format!("unknown command: :{raw}"));
            }
        }
    }

    fn key_ctx(&self) -> KeyCtx {
        match self.tab {
            Tab::Queue => KeyCtx::Queue,
            Tab::Browser => KeyCtx::Browser,
            Tab::Playlists => KeyCtx::Playlists,
            Tab::Search => KeyCtx::Search,
            Tab::Help => KeyCtx::Help,
        }
    }

    fn active_list(&self) -> ListRef {
        match self.tab {
            Tab::Queue => ListRef::Queue,
            Tab::Browser => match self.browser.focus {
                BrowserCol::Artists => ListRef::BArtists,
                BrowserCol::Albums => ListRef::BAlbums,
                BrowserCol::Tracks => ListRef::BTracks,
            },
            Tab::Playlists => match self.playlists.focus {
                PlaylistsCol::List => ListRef::PlList,
                PlaylistsCol::Items => ListRef::PlItems,
            },
            Tab::Search => {
                if self.search.detail.is_some() {
                    ListRef::SearchDetail
                } else {
                    ListRef::Search
                }
            }
            Tab::Help => ListRef::Help,
        }
    }

    fn filter(&self, l: ListRef) -> Option<&ListFilter> {
        match l {
            ListRef::Queue => Some(&self.queue_filter),
            ListRef::BArtists => Some(&self.browser_filters[0]),
            ListRef::BAlbums => Some(&self.browser_filters[1]),
            ListRef::BTracks => Some(&self.browser_filters[2]),
            ListRef::PlList => Some(&self.pl_list_filter),
            ListRef::PlItems => Some(&self.pl_items_filter),
            ListRef::Search | ListRef::SearchDetail | ListRef::Help => None,
        }
    }

    fn filter_mut(&mut self, l: ListRef) -> Option<&mut ListFilter> {
        match l {
            ListRef::Queue => Some(&mut self.queue_filter),
            ListRef::BArtists => Some(&mut self.browser_filters[0]),
            ListRef::BAlbums => Some(&mut self.browser_filters[1]),
            ListRef::BTracks => Some(&mut self.browser_filters[2]),
            ListRef::PlList => Some(&mut self.pl_list_filter),
            ListRef::PlItems => Some(&mut self.pl_items_filter),
            ListRef::Search | ListRef::SearchDetail | ListRef::Help => None,
        }
    }

    fn total_len(&self, l: ListRef) -> usize {
        match l {
            ListRef::Queue => self.queue.len(),
            ListRef::BArtists => self.browser.artists.len(),
            ListRef::BAlbums => self.browser.albums.len(),
            ListRef::BTracks => self.browser.tracks.len(),
            ListRef::PlList => self.playlists.playlists.len(),
            ListRef::PlItems => self.playlists.items.len(),
            ListRef::Search => self.search.hits.len(),
            ListRef::SearchDetail => {
                self.search.detail.as_ref().map(|d| d.tracks.len()).unwrap_or(0)
            }
            ListRef::Help => self.keymap.help_lines().len(),
        }
    }

    fn vis_len(&self, l: ListRef) -> usize {
        match self.filter(l) {
            Some(f) => f.len(self.total_len(l)),
            None => self.total_len(l),
        }
    }

    /// An index into the filtered view.
    fn cursor_of(&self, l: ListRef) -> Option<usize> {
        match l {
            ListRef::Queue => self.queue_state.selected(),
            ListRef::BArtists => self.browser.artists_state.selected(),
            ListRef::BAlbums => self.browser.albums_state.selected(),
            ListRef::BTracks => self.browser.tracks_state.selected(),
            ListRef::PlList => self.playlists.state.selected(),
            ListRef::PlItems => self.playlists.items_state.selected(),
            ListRef::Search => self.search.state.selected(),
            ListRef::SearchDetail => self.search.detail.as_ref().and_then(|d| d.state.selected()),
            ListRef::Help => self.help_state.selected(),
        }
    }

    /// The cursor as an index into the underlying list; None when nothing is
    /// visible, so actions no-op under a filter with no matches.
    fn real_at(&self, l: ListRef) -> Option<usize> {
        if self.vis_len(l) == 0 {
            return None;
        }
        let cursor = self.cursor_of(l)?;
        let total = self.total_len(l);
        if total == 0 {
            return None;
        }
        Some(match self.filter(l) {
            Some(f) => f.real(cursor, total),
            None => cursor.min(total - 1),
        })
    }

    fn with_sel(&mut self, f: impl FnOnce(&mut dyn SelState, usize)) {
        let l = self.active_list();
        let vis = self.vis_len(l);
        match l {
            ListRef::Queue => f(&mut self.queue_state, vis),
            ListRef::BArtists => f(&mut self.browser.artists_state, vis),
            ListRef::BAlbums => f(&mut self.browser.albums_state, vis),
            ListRef::BTracks => f(&mut self.browser.tracks_state, vis),
            ListRef::PlList => f(&mut self.playlists.state, vis),
            ListRef::PlItems => f(&mut self.playlists.items_state, vis),
            ListRef::Search => f(&mut self.search.state, vis),
            ListRef::SearchDetail => {
                if let Some(d) = self.search.detail.as_mut() {
                    f(&mut d.state, vis);
                }
            }
            ListRef::Help => f(&mut self.help_state, vis),
        }
    }

    fn goto_tab(&mut self, t: Tab) {
        if self.tab == Tab::Search && t != Tab::Search {
            self.search.focused = false;
        }
        if t == Tab::Help && self.tab != Tab::Help {
            self.help_prev = self.tab;
        }
        if self.visual.is_some_and(|v| {
            !matches!(
                (v.list, t),
                (ListRef::Queue, Tab::Queue)
                    | (ListRef::BTracks, Tab::Browser)
                    | (ListRef::PlItems, Tab::Playlists)
            )
        }) {
            self.visual = None;
        }
        self.tab = t;
        self.pending_goto = None;
        if t == Tab::Queue {
            self.follow_queue();
        }
    }

    fn exec(&mut self, act: Act) {
        if matches!(
            act,
            Act::AddToQueue
                | Act::PlayNext
                | Act::UndoRemove
                | Act::ToggleShuffle
                | Act::Reshuffle
                | Act::MoveQueueUp
                | Act::MoveQueueDown
        ) && self.refuse_while_loading()
        {
            return;
        }
        match act {
            Act::Quit => self.should_quit = true,
            Act::NextTab => {
                let t = self.tab.next();
                self.goto_tab(t);
            }
            Act::PrevTab => {
                let t = self.tab.prev();
                self.goto_tab(t);
            }
            Act::GotoTab(t) => self.goto_tab(t),

            Act::MoveUp => {
                self.pause_queue_follow();
                self.with_sel(|st, vis| move_sel(st, vis, -1));
            }
            Act::MoveDown => {
                self.pause_queue_follow();
                self.with_sel(|st, vis| move_sel(st, vis, 1));
            }
            Act::PageUp => {
                self.pause_queue_follow();
                self.page(-1);
            }
            Act::PageDown => {
                self.pause_queue_follow();
                self.page(1);
            }
            Act::Top => {
                self.pause_queue_follow();
                self.with_sel(|st, vis| {
                    st.select(if vis == 0 { None } else { Some(0) });
                });
            }
            Act::Bottom => {
                self.pause_queue_follow();
                self.with_sel(|st, vis| {
                    st.select(if vis == 0 { None } else { Some(vis - 1) });
                });
            }

            Act::ColumnLeft => self.column_left(),
            Act::ColumnRight => self.column_right(),

            Act::Activate => self.activate(),
            Act::AddToQueue => self.add_selected(QueueAction::Append),
            Act::PlayNext => self.add_selected(QueueAction::PlayNext),
            Act::MoveQueueUp => {
                self.pause_queue_follow();
                self.move_queue(-1);
            }
            Act::MoveQueueDown => {
                self.pause_queue_follow();
                self.move_queue(1);
            }
            Act::Play => {
                if let Some(real) = self.real_at(ListRef::Queue) {
                    self.play_queue_index(real);
                }
            }
            Act::RemoveFromQueue => {
                if let Some(real) = self.real_at(ListRef::Queue) {
                    self.remove_from_queue(real);
                }
            }
            Act::VisualMode => self.start_visual(),
            Act::UndoRemove => self.undo_remove(),
            Act::ClearQueue => self.ask_clear_queue(),
            Act::GotoAlbum => match self.tab {
                Tab::Queue => self.goto_album(),
                Tab::Search => self.reveal_search(),
                _ => {}
            },
            Act::JumpToPlaying => self.jump_to_playing(),
            Act::Back => self.back(),

            Act::StartFilter => {
                self.pause_queue_follow();
                let l = self.active_list();
                if let Some(f) = self.filter_mut(l) {
                    f.typing = true;
                }
            }
            Act::StartSearch => {
                self.goto_tab(Tab::Search);
                self.search.focused = true;
                self.search.history_pos = None;
            }
            Act::OpenCommand => {
                self.command.typing = true;
                self.command.text.clear();
            }

            Act::TogglePause => {
                if !self.stopped {
                    let _ = self.mpv.tx.send(MpvCmd::TogglePause);
                }
            }
            Act::NextTrack => {
                // the consume guard would swallow the position mpv reports now
                self.expect_pos = None;
                if self.stopped {
                    // mpv ignores playlist-next while idle
                    self.play_from_stopped();
                } else {
                    let _ = self.mpv.tx.send(MpvCmd::Next);
                    // Consume: advance first (removing the playing entry would
                    // stop mpv), then remove, unless `>` went nowhere.
                    if self.consume {
                        if let Some(i) = self.current {
                            let moves = self.queue.len() > 1
                                && (i + 1 < self.queue.len() || self.loop_playlist);
                            if moves {
                                self.consume_finished(i);
                            }
                        }
                    }
                }
            }
            Act::PrevTrack => {
                let nothing_before = self.current == Some(0) && !self.loop_playlist;
                let played = self.position > self.prev_restart_secs as f64;
                let restart =
                    !self.stopped && (nothing_before || (self.prev_restart_secs > 0 && played));
                if restart {
                    // so a second `<` goes back before mpv reports the new time
                    self.position = 0.0;
                    let _ = self.mpv.tx.send(MpvCmd::SeekAbs(0.0));
                } else {
                    self.expect_pos = None;
                    let _ = self.mpv.tx.send(MpvCmd::Prev);
                }
            }
            Act::VolUp => self.set_volume(self.volume + 5.0),
            Act::VolDown => self.set_volume(self.volume - 5.0),
            Act::SeekFwd => {
                let _ = self.mpv.tx.send(MpvCmd::SeekRel(5.0));
            }
            Act::SeekBack => {
                let _ = self.mpv.tx.send(MpvCmd::SeekRel(-5.0));
            }
            Act::ToggleRepeat => {
                self.loop_playlist = !self.loop_playlist;
                let on = self.loop_playlist;
                let _ = self.mpv.tx.send(MpvCmd::SetLoop { property: "loop-playlist", on });
                self.transient(format!("repeat {}", if on { "on" } else { "off" }));
            }
            Act::ToggleShuffle => self.toggle_shuffle(),
            Act::Reshuffle => self.reshuffle_queue(),
            Act::ToggleSingle => {
                self.loop_file = !self.loop_file;
                let on = self.loop_file;
                let _ = self.mpv.tx.send(MpvCmd::SetLoop { property: "loop-file", on });
                self.transient(format!("single {}", if on { "on" } else { "off" }));
            }
            Act::ToggleConsume => {
                self.consume = !self.consume;
                let on = self.consume;
                self.transient(format!("consume {}", if on { "on" } else { "off" }));
            }
            Act::SleepTimer => self.cycle_sleep_timer(),
        }
    }

    fn toggle_shuffle(&mut self) {
        if self.random {
            self.disable_shuffle();
        } else {
            self.enable_shuffle();
        }
    }

    fn enable_shuffle(&mut self) {
        if self.random {
            return;
        }
        if self.queue.is_empty() {
            self.random = true;
            self.shuffle_snapshot = None;
            self.transient("shuffle on".into());
            return;
        }
        self.shuffle_snapshot = Some(ShuffleSnapshot { order: self.queue.clone() });
        let order = self.shuffled_order_keep_current();
        self.apply_queue_order(order);
        self.random = true;
        self.transient("shuffle on".into());
    }

    fn disable_shuffle(&mut self) {
        if !self.random {
            return;
        }
        let snapshot = self.shuffle_snapshot.take();
        if let Some(snapshot) = snapshot {
            // Match snapshot entries to queue slots by rating key: first unused
            // match, with slots bucketed per key to stay O(n).
            let order = {
                let mut remaining: HashMap<&str, VecDeque<usize>> = HashMap::new();
                for (i, t) in self.queue.iter().enumerate() {
                    remaining.entry(t.rating_key.as_str()).or_default().push_back(i);
                }
                let mut used = vec![false; self.queue.len()];
                let mut order = Vec::with_capacity(self.queue.len());
                for wanted in &snapshot.order {
                    if let Some(idx) =
                        remaining.get_mut(wanted.rating_key.as_str()).and_then(VecDeque::pop_front)
                    {
                        used[idx] = true;
                        order.push(idx);
                    }
                }
                // tracks the snapshot did not mention keep the tail slots
                for (i, used) in used.iter().enumerate() {
                    if !used {
                        order.push(i);
                    }
                }
                order
            };
            if order.len() == self.queue.len() {
                self.apply_queue_order(order);
            }
        }
        self.random = false;
        self.transient("shuffle off".into());
    }

    fn reshuffle_queue(&mut self) {
        if self.queue.len() < 2 {
            self.transient("queue too short to shuffle".into());
            return;
        }
        if !self.random {
            self.enable_shuffle();
            return;
        }
        let order = self.shuffled_order_keep_current();
        self.apply_queue_order(order);
        self.transient("queue reshuffled".into());
    }

    fn shuffle_new_queue(&mut self, tracks: &mut Vec<Track>) {
        if !self.random {
            self.shuffle_snapshot = None;
            return;
        }
        self.shuffle_snapshot = Some(ShuffleSnapshot { order: tracks.clone() });
        if tracks.len() < 2 {
            return;
        }
        let rest = tracks.split_off(1);
        let order = shuffled_indices(rest.len());
        for i in order {
            tracks.push(rest[i].clone());
        }
    }

    fn shuffled_order_keep_current(&self) -> Vec<usize> {
        let mut rest: Vec<usize> = (0..self.queue.len()).collect();
        let mut order = Vec::with_capacity(self.queue.len());
        if let Some(current) = self.current {
            if current < self.queue.len() {
                order.push(current);
                rest.retain(|&i| i != current);
            }
        }
        for i in shuffled_indices(rest.len()) {
            order.push(rest[i]);
        }
        order
    }

    /// Reorders the app queue and mpv's playlist in lockstep (`order[new] =
    /// old`), one track at a time from slot 0.
    fn apply_queue_order(&mut self, order: Vec<usize>) {
        if order.len() != self.queue.len() {
            return;
        }
        let old_current = self.current.filter(|&c| c < self.queue.len());
        let old_selected = self.queue_state.selected();
        // the cursor counts filtered rows; the plan is by real slot
        let old_selected_real = self.real_at(ListRef::Queue);
        let Some(plan) = queue_move_plan(&order, old_current) else {
            return;
        };
        for &(from, to) in &plan.moves {
            let _ = self.mpv.tx.send(MpvCmd::Move { from, to });
        }
        // move, don't clone: a long queue is a lot to duplicate on the UI thread
        let mut old: Vec<Option<Track>> =
            std::mem::take(&mut self.queue).into_iter().map(Some).collect();
        self.queue = order.iter().filter_map(|&i| old.get_mut(i).and_then(Option::take)).collect();
        debug_assert_eq!(self.queue.len(), order.len(), "the plan validated a permutation");
        self.removed.clear();
        self.drop_visual(ListRef::Queue);
        self.current = plan.play;
        // remap the failure marker rather than leave it on a stranger
        self.failed_at = self.failed_at.and_then(|f| plan.position.get(f).copied());
        // mpv reports every intermediate slot the playing entry crosses
        self.expect_pos = match (self.current, plan.play_moved) {
            (Some(expected), true) => Some(PosGuard::reorder(expected)),
            _ => None,
        };
        self.refilter(ListRef::Queue);
        let vis = self.vis_len(ListRef::Queue);
        if vis == 0 {
            self.queue_state.select(None);
            self.follow_queue();
            return;
        }
        // highlighted row -> new real slot -> filtered cursor; falls back to
        // the playing track, then the old cursor
        let selected = old_selected_real
            .and_then(|s| plan.position.get(s).copied())
            .and_then(|real| self.queue_filter.cursor_of(real))
            .or_else(|| self.current.and_then(|c| self.queue_filter.cursor_of(c)))
            .or_else(|| old_selected.map(|s| s.min(vis - 1)))
            .unwrap_or(0);
        self.queue_state.select(Some(selected.min(vis - 1)));
        self.follow_queue();
    }

    /// `>` while stopped: after a failure the track following the broken one,
    /// otherwise the cursor's row.
    fn play_from_stopped(&mut self) {
        if self.queue.is_empty() {
            self.transient("queue is empty".into());
            return;
        }
        let idx = match self.failed_at {
            Some(f) if self.loop_playlist => (f + 1) % self.queue.len(),
            Some(f) => f + 1,
            None => match self.real_at(ListRef::Queue) {
                Some(real) => real,
                None => self.queue_state.selected().unwrap_or(0),
            },
        };
        if idx >= self.queue.len() {
            self.transient("end of the queue".into());
            return;
        }
        self.play_queue_index(idx);
    }

    /// Uses the pane height recorded by the last draw.
    fn page(&mut self, dir: i32) {
        let l = self.active_list();
        let visible = self.scroll.visible(scroll_id(l)).max(1);
        let vis = self.vis_len(l);
        self.with_sel(|st, _| {
            if vis == 0 {
                st.select(None);
                return;
            }
            let cur = st.selected().unwrap_or(0);
            let next = scroll::page_jump(cur, visible, vis, dir);
            st.select(Some(next));
        });
    }

    fn column_left(&mut self) {
        match self.tab {
            Tab::Browser => {
                self.browser.focus = match self.browser.focus {
                    BrowserCol::Tracks => BrowserCol::Albums,
                    BrowserCol::Albums => BrowserCol::Artists,
                    BrowserCol::Artists => BrowserCol::Artists,
                };
            }
            Tab::Playlists if self.playlists.focus == PlaylistsCol::Items => {
                // going back to the playlist list clears its track filter
                self.pl_items_filter.clear();
                self.playlists.focus = PlaylistsCol::List;
            }
            _ => {}
        }
    }

    fn column_right(&mut self) {
        match self.tab {
            Tab::Browser => match self.browser.focus {
                BrowserCol::Artists => {
                    // like Enter, but no refetch when the albums shown are this
                    // artist's
                    let Some(real) = self.real_at(ListRef::BArtists) else { return };
                    let Some(a) = self.browser.artists.get(real) else { return };
                    if self.browser.albums_for.as_deref() == Some(a.rating_key.as_str()) {
                        self.browser.focus = BrowserCol::Albums;
                    } else {
                        self.activate();
                    }
                }
                BrowserCol::Albums => {
                    if let Some(real) = self.real_at(ListRef::BAlbums) {
                        if let Some(album) = self.browser.albums.get(real) {
                            let key = album.rating_key.clone();
                            self.browser.focus = BrowserCol::Tracks;
                            self.load_tracks(key);
                        }
                    }
                }
                _ => {}
            },
            Tab::Playlists if !self.playlists.items.is_empty() => {
                self.playlists.focus = PlaylistsCol::Items;
            }
            _ => {}
        }
    }

    /// Keeps the items pane on the highlighted playlist. Previews are bounded
    /// and debounced so a huge playlist is not fetched whole.
    fn preview_playlist(&mut self) {
        if self.tab != Tab::Playlists
            || self.playlists.focus != PlaylistsCol::List
            || self.pl_list_filter.typing
        {
            self.preview_blocked = None;
            return;
        }
        let Some(real) = self.real_at(ListRef::PlList) else { return };
        let Some(pl) = self.playlists.playlists.get(real) else { return };
        let key = pl.rating_key.clone();
        if self.preview_blocked.as_deref() == Some(key.as_str()) {
            return;
        }
        self.preview_blocked = None;
        let total = pl.item_count.map(|n| n as usize);
        if self.playlists.items_for.as_deref() == Some(key.as_str()) {
            return;
        }
        self.preview_epoch = self.preview_epoch.wrapping_add(1);
        self.playlists.items_for = Some(key.clone());
        self.playlists.items_total = total;
        self.playlists.items.clear();
        self.drop_visual(ListRef::PlItems);
        self.playlists.items_state.select(Some(0));
        self.pl_items_filter.clear();

        if self.preview_tracks == 0 {
            self.playlists.items_for = None;
            return;
        }
        if let Some(cached) = self.playlists.preview_cache.get(&key).cloned() {
            self.playlists.items = cached;
            self.drop_visual(ListRef::PlItems);
            self.announce_preview_cap();
            return;
        }
        if self.preview_debounce_ms == 0 {
            self.load_playlist_preview(key);
        } else {
            let tx = self.msg_tx.clone();
            let epoch = self.preview_epoch;
            let delay = self.preview_debounce_ms;
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(delay));
                let _ = tx.send(Msg::PreviewPlaylist { key, epoch });
            });
        }
    }

    fn selected_playlist_key(&self) -> Option<String> {
        let real = self.real_at(ListRef::PlList)?;
        self.playlists.playlists.get(real).map(|p| p.rating_key.clone())
    }

    fn load_playlist_preview(&mut self, key: String) {
        let size = self.preview_tracks.max(1);
        let _ = self.plex.tx.send(PlexCmd::LoadPlaylistPreview { key, size });
    }

    /// The status line is the only place the preview cap shows.
    fn announce_preview_cap(&mut self) {
        if self.tab != Tab::Playlists {
            return;
        }
        let Some(total) = self.playlists.items_total else { return };
        if self.preview_tracks == 0 || total <= self.preview_tracks {
            return;
        }
        let shown = self.playlists.items.len();
        let mut msg = format!("preview: {shown} of {total} tracks");
        if let Some(key) = self.keymap.first_key(KeyCtx::Playlists, &Act::Activate) {
            msg.push_str(&format!("; {key} adds all"));
        }
        self.transient(msg);
    }

    /// Asks first when it is very large. `count` is what would actually be
    /// queued (the remainder when starting mid-list).
    fn queue_playlist(
        &mut self,
        key: String,
        confirm: Confirm,
        action: QueueAction,
        count: Option<u32>,
        start: Option<String>,
    ) {
        let n = count.unwrap_or(0) as usize;
        if self.confirm_playlist_over > 0 && n > self.confirm_playlist_over {
            self.pending_clear = None;
            self.pending_playlist = Some(PendingPlaylist {
                key,
                confirm,
                action,
                start,
                count,
                armed_at: Instant::now(),
            });
            let verb = match action {
                QueueAction::Append => "Add",
                QueueAction::Replace => "Replace queue with",
                QueueAction::PlayNext => "Insert",
            };
            self.transient_secs(
                format!("{verb} {n} tracks? (y/n)"),
                self.confirm_timeout_secs,
                false,
            );
            return;
        }
        self.send_resolve_playlist(key, confirm, action, start, count);
    }

    /// A replace streams unless shuffle is on, which needs the whole list.
    fn send_resolve_playlist(
        &mut self,
        key: String,
        confirm: Confirm,
        action: QueueAction,
        start: Option<String>,
        count: Option<u32>,
    ) {
        if action != QueueAction::Replace && self.refuse_while_loading() {
            return;
        }
        let seq = self.issue_seq(action == QueueAction::Replace);
        let stream = action == QueueAction::Replace && !self.random;
        let mut cancel = Arc::new(AtomicBool::new(false));
        if stream {
            let load = PlaylistLoad::new(seq);
            cancel = load.cancel.clone();
            self.playlist_load = Some(load);
        }
        let _ = self.plex.tx.send(PlexCmd::ResolvePlaylist {
            seq,
            key,
            confirm,
            action,
            start,
            stream,
            expected: count.unwrap_or(0) as usize,
            cancel,
        });
        self.transient(
            match action {
                QueueAction::Append => "adding playlist…",
                QueueAction::Replace => "loading playlist…",
                QueueAction::PlayNext => "inserting playlist…",
            }
            .into(),
        );
    }

    /// Enter. Albums drill with Right/l so Enter can play at once.
    fn activate(&mut self) {
        match self.active_list() {
            ListRef::Queue => {
                if let Some(real) = self.real_at(ListRef::Queue) {
                    self.play_queue_index(real);
                }
            }
            ListRef::BArtists => {
                let Some(real) = self.real_at(ListRef::BArtists) else { return };
                let Some(a) = self.browser.artists.get(real) else { return };
                let key = a.rating_key.clone();
                self.browser_filters[0].clear();
                self.browser.focus = BrowserCol::Albums;
                self.load_albums(key);
            }
            ListRef::BAlbums => {
                let Some(real) = self.real_at(ListRef::BAlbums) else { return };
                let Some(al) = self.browser.albums.get(real) else { return };
                let confirm = Confirm::Album { name: al.title.clone() };
                let key = al.rating_key.clone();
                let seq = self.issue_seq(true);
                let _ = self.plex.tx.send(PlexCmd::ResolveAlbum {
                    seq,
                    key,
                    confirm,
                    action: QueueAction::Replace,
                });
                self.transient("loading album…".into());
            }
            ListRef::BTracks => {
                let Some(real) = self.real_at(ListRef::BTracks) else { return };
                let Some(t) = self.browser.tracks.get(real).cloned() else { return };
                let tracks = self.browser.tracks[real..].to_vec();
                let confirm = Confirm::FromList { name: t.album.clone() };
                self.queue_local(tracks, confirm, QueueAction::Replace);
            }
            ListRef::PlList => {
                let Some(real) = self.real_at(ListRef::PlList) else { return };
                let Some(pl) = self.playlists.playlists.get(real) else { return };
                let confirm = Confirm::Playlist { name: pl.title.clone() };
                let key = pl.rating_key.clone();
                let count = pl.item_count;
                self.queue_playlist(key, confirm, QueueAction::Replace, count, None);
            }
            ListRef::PlItems => {
                let Some(real) = self.real_at(ListRef::PlItems) else { return };
                let Some(t) = self.playlists.items.get(real).cloned() else { return };
                let Some(key) = self.playlists.items_for.clone() else { return };
                let row = self.playlists.playlists.iter().find(|p| p.rating_key == key);
                let name = row.map(|p| p.title.clone()).unwrap_or_else(|| "playlist".into());
                // the preview is the first page, so its index is the playlist's
                // index, and the remainder is what the question must name
                let remainder =
                    row.and_then(|p| p.item_count).map(|total| total.saturating_sub(real as u32));
                let confirm = Confirm::FromList { name };
                self.queue_playlist(
                    key,
                    confirm,
                    QueueAction::Replace,
                    remainder,
                    Some(crate::plex::client::entry_key(&t)),
                );
            }
            ListRef::Search => {
                let Some(i) = self.search.state.selected() else { return };
                let Some(hit) = self.search.hits.get(i).cloned() else { return };
                use crate::plex::HitKind;
                match hit.kind {
                    HitKind::Track => {
                        let seq = self.issue_seq(true);
                        let _ = self.plex.tx.send(PlexCmd::ResolveTrack {
                            seq,
                            key: hit.rating_key,
                            confirm: Confirm::Track { title: hit.title },
                            action: QueueAction::Replace,
                            rest_of_album: true,
                        });
                        self.transient("loading…".into());
                    }
                    HitKind::Album => {
                        self.search.detail_request = Some(hit.rating_key.clone());
                        let _ = self.plex.tx.send(PlexCmd::LoadAlbumDetail {
                            key: hit.rating_key,
                            label: hit.title,
                        });
                        self.transient("loading album…".into());
                    }
                    HitKind::Artist => self.reveal(Some(hit.rating_key), None, None),
                }
            }
            ListRef::SearchDetail => {
                let Some(d) = self.search.detail.as_ref() else { return };
                let Some(i) = d.state.selected() else { return };
                let tracks = d.tracks[i..].to_vec();
                let confirm = Confirm::FromList { name: d.label.clone() };
                self.queue_local(tracks, confirm, QueueAction::Replace);
            }
            ListRef::Help => {}
        }
    }

    fn add_selected(&mut self, action: QueueAction) {
        match self.active_list() {
            ListRef::BArtists => {
                let Some(real) = self.real_at(ListRef::BArtists) else { return };
                let Some(a) = self.browser.artists.get(real) else { return };
                let confirm = Confirm::Artist { name: a.title.clone() };
                let key = a.rating_key.clone();
                let seq = self.issue_seq(action == QueueAction::Replace);
                let _ =
                    self.plex.tx.send(PlexCmd::ResolveArtistTracks { seq, key, confirm, action });
                self.transient(
                    match action {
                        QueueAction::Append => "loading artist…",
                        QueueAction::Replace => "loading artist…",
                        QueueAction::PlayNext => "inserting artist…",
                    }
                    .into(),
                );
            }
            ListRef::BAlbums => {
                let Some(real) = self.real_at(ListRef::BAlbums) else { return };
                let Some(al) = self.browser.albums.get(real) else { return };
                let confirm = Confirm::Album { name: al.title.clone() };
                let key = al.rating_key.clone();
                let seq = self.issue_seq(action == QueueAction::Replace);
                let _ = self.plex.tx.send(PlexCmd::ResolveAlbum { seq, key, confirm, action });
                self.transient(
                    match action {
                        QueueAction::Append => "loading album…",
                        QueueAction::Replace => "loading album…",
                        QueueAction::PlayNext => "inserting album…",
                    }
                    .into(),
                );
            }
            ListRef::BTracks => {
                let Some(real) = self.real_at(ListRef::BTracks) else { return };
                let Some(t) = self.browser.tracks.get(real).cloned() else { return };
                let confirm = Confirm::Track { title: t.title.clone() };
                self.queue_local(vec![t], confirm, action);
            }
            ListRef::PlList => {
                let Some(real) = self.real_at(ListRef::PlList) else { return };
                let Some(pl) = self.playlists.playlists.get(real) else { return };
                let confirm = Confirm::Playlist { name: pl.title.clone() };
                let key = pl.rating_key.clone();
                let count = pl.item_count;
                self.queue_playlist(key, confirm, action, count, None);
            }
            ListRef::PlItems => {
                let Some(real) = self.real_at(ListRef::PlItems) else { return };
                let Some(t) = self.playlists.items.get(real).cloned() else { return };
                if action == QueueAction::PlayNext {
                    let confirm = Confirm::Track { title: t.title.clone() };
                    self.queue_local(vec![t], confirm, QueueAction::PlayNext);
                } else {
                    let Some(key) = self.playlists.items_for.clone() else { return };
                    let row = self.playlists.playlists.iter().find(|p| p.rating_key == key);
                    let name = row.map(|p| p.title.clone()).unwrap_or_else(|| "playlist".into());
                    // `real` is the position in the playlist (first-page preview)
                    let remainder = row
                        .and_then(|p| p.item_count)
                        .map(|total| total.saturating_sub(real as u32));
                    self.queue_playlist(
                        key,
                        Confirm::FromList { name },
                        action,
                        remainder,
                        Some(crate::plex::client::entry_key(&t)),
                    );
                }
            }
            ListRef::Search => {
                let Some(i) = self.search.state.selected() else { return };
                let Some(hit) = self.search.hits.get(i).cloned() else { return };
                use crate::plex::HitKind;
                match hit.kind {
                    HitKind::Track => {
                        let seq = self.issue_seq(action == QueueAction::Replace);
                        let _ = self.plex.tx.send(PlexCmd::ResolveTrack {
                            seq,
                            key: hit.rating_key,
                            confirm: Confirm::Track { title: hit.title },
                            action,
                            rest_of_album: false,
                        });
                        self.transient(
                            if action == QueueAction::PlayNext {
                                "inserting…"
                            } else {
                                "adding…"
                            }
                            .into(),
                        );
                    }
                    HitKind::Album => {
                        let seq = self.issue_seq(action == QueueAction::Replace);
                        let _ = self.plex.tx.send(PlexCmd::ResolveAlbum {
                            seq,
                            key: hit.rating_key,
                            confirm: Confirm::Album { name: hit.title },
                            action,
                        });
                        self.transient(
                            if action == QueueAction::PlayNext {
                                "inserting album…"
                            } else {
                                "adding album…"
                            }
                            .into(),
                        );
                    }
                    HitKind::Artist => {}
                }
            }
            ListRef::SearchDetail => {
                let Some(d) = self.search.detail.as_ref() else { return };
                let Some(i) = d.state.selected() else { return };
                let Some(t) = d.tracks.get(i).cloned() else { return };
                let confirm = Confirm::Track { title: t.title.clone() };
                self.queue_local(vec![t], confirm, action);
            }
            ListRef::Queue | ListRef::Help => {}
        }
    }

    /// Not while filtered: the playing row may be hidden, and filtering is
    /// browsing.
    fn follow_queue(&mut self) {
        if !self.auto_follow
            || !self.queue_follow
            || self.tab != Tab::Queue
            || self.queue_filter.is_active()
        {
            return;
        }
        let Some(cur) = self.current else { return };
        if cur >= self.queue.len() {
            return;
        }
        self.queue_state.select(Some(cur));
        self.scroll.pin_top(ScrollId::Queue);
    }

    fn pause_queue_follow(&mut self) {
        if self.active_list() != ListRef::Queue {
            return;
        }
        // also cancels a pending one-shot pin, or `o` then a move would pin
        // the new cursor (messages are coalesced, so that window is real)
        self.scroll.release_top(ScrollId::Queue);
        if self.queue_follow {
            self.queue_follow = false;
            let key = self
                .keymap
                .first_key(KeyCtx::Queue, &Act::JumpToPlaying)
                .unwrap_or_else(|| ":playing".into());
            self.transient(format!("queue follow: paused ({key} to resume)"));
        }
    }

    /// `g <letter>`: first visible row whose title starts with it.
    fn jump_to_letter(&mut self, needle: char) {
        let l = self.active_list();
        if matches!(l, ListRef::Help) {
            return;
        }
        let total = self.total_len(l);
        let visible = self.vis_len(l);
        if total == 0 || visible == 0 {
            return;
        }
        let Some(needle) = needle.to_lowercase().next() else { return };
        let filter = self.filter(l);
        let mut target = None;
        for vi in 0..visible {
            let real = match filter {
                Some(f) => f.real(vi, total),
                None => vi,
            };
            let Some(title) = self.row_title(l, real) else { continue };
            let first = title.trim_start().chars().next().and_then(|c| c.to_lowercase().next());
            if first == Some(needle) {
                target = Some(vi);
                break;
            }
        }
        match target {
            Some(vi) => {
                self.with_sel(|st, _| st.select(Some(vi)));
                self.transient(format!("jump: {}", needle.to_uppercase()));
            }
            None => self.transient(format!("no row starts with {}", needle.to_uppercase())),
        }
    }

    fn row_title(&self, l: ListRef, real: usize) -> Option<&str> {
        match l {
            ListRef::Queue => self.queue.get(real).map(|t| t.title.as_str()),
            ListRef::BArtists => self.browser.artists.get(real).map(|a| a.title.as_str()),
            ListRef::BAlbums => self.browser.albums.get(real).map(|a| a.title.as_str()),
            ListRef::BTracks => self.browser.tracks.get(real).map(|t| t.title.as_str()),
            ListRef::PlList => self.playlists.playlists.get(real).map(|p| p.title.as_str()),
            ListRef::PlItems => self.playlists.items.get(real).map(|t| t.title.as_str()),
            ListRef::Search => self.search.hits.get(real).map(|h| h.title.as_str()),
            ListRef::SearchDetail => self
                .search
                .detail
                .as_ref()
                .and_then(|d| d.tracks.get(real))
                .map(|t| t.title.as_str()),
            ListRef::Help => None,
        }
    }

    /// `o`. In the Queue this also resumes auto-follow; other lists center as
    /// configured.
    fn jump_to_playing(&mut self) {
        let Some(cur) = self.current else {
            self.transient("nothing playing".into());
            return;
        };
        let Some(key) = self.queue.get(cur).map(|t| t.rating_key.clone()) else { return };
        let center = self.center_cursor;
        let in_queue = self.active_list() == ListRef::Queue;
        if in_queue {
            self.queue_follow = self.auto_follow;
        }
        match self.active_list() {
            ListRef::Queue => {
                let Some(p) = self.queue_filter.cursor_of(cur) else {
                    self.transient("playing track is filtered out".into());
                    return;
                };
                self.queue_state.select(Some(p));
                // with auto-follow off `o` still jumps, once
                if self.auto_follow {
                    self.scroll.pin_top(ScrollId::Queue);
                    self.transient("queue follow: on".into());
                } else {
                    self.scroll.pin_top_once(ScrollId::Queue);
                    self.transient("queue follow: off".into());
                }
            }
            ListRef::BTracks => {
                let pos = self.browser.tracks.iter().position(|t| t.rating_key == key);
                if let Some(p) = pos.and_then(|r| self.browser_filters[2].cursor_of(r)) {
                    self.browser.tracks_state.select(Some(p));
                    if center {
                        self.scroll.center_once = Some(ScrollId::BrowserTracks);
                    }
                }
            }
            ListRef::PlItems => {
                let pos = self.playlists.items.iter().position(|t| t.rating_key == key);
                if let Some(p) = pos.and_then(|r| self.pl_items_filter.cursor_of(r)) {
                    self.playlists.items_state.select(Some(p));
                    if center {
                        self.scroll.center_once = Some(ScrollId::PlaylistItems);
                    }
                }
            }
            _ => {}
        }
    }

    /// Esc / Backspace.
    fn back(&mut self) {
        match self.tab {
            Tab::Browser => match self.browser.focus {
                BrowserCol::Tracks => {
                    self.browser_filters[2].clear();
                    self.browser.focus = BrowserCol::Albums;
                }
                BrowserCol::Albums => {
                    self.browser_filters[1].clear();
                    self.browser.focus = BrowserCol::Artists;
                }
                BrowserCol::Artists => {
                    self.browser_filters[0].clear();
                    self.goto_tab(Tab::Queue);
                }
            },
            Tab::Search => {
                self.search.detail = None;
                self.search.detail_request = None;
            }
            Tab::Help => {
                let t = self.help_prev;
                self.goto_tab(t);
            }
            _ => {}
        }
    }

    /// `(input, scope, shown, total)` while typing.
    pub fn filter_typing(&self) -> Option<(String, &'static str, usize, usize)> {
        let l = self.active_list();
        let f = self.filter(l)?;
        if !f.typing {
            return None;
        }
        let total = self.total_len(l);
        Some((f.text.clone(), scope_label(l), f.len(total), total))
    }

    /// Re-applies the active filter and puts the cursor on the first match.
    fn refilter_active_and_reset(&mut self) {
        let l = self.active_list();
        self.refilter(l);
        self.with_sel(|st, vis| {
            st.select(if vis == 0 { None } else { Some(0) });
        });
    }

    /// Also called whenever the underlying list changes, so a refresh keeps
    /// the filter applied.
    fn refilter(&mut self, l: ListRef) {
        let Some(text) = self.filter(l).map(|f| f.text.clone()) else {
            return;
        };
        if text.is_empty() {
            if let Some(f) = self.filter_mut(l) {
                f.indices.clear();
            }
            return;
        }
        let needle = filter::Needle::new(&text);
        match l {
            ListRef::Queue => {
                let items = &self.queue;
                self.queue_filter.apply(items.len(), |i| {
                    let t = &items[i];
                    needle.found_in(&t.title)
                        || needle.found_in(&t.artist)
                        || needle.found_in(&t.album)
                });
            }
            ListRef::BArtists => {
                let items = &self.browser.artists;
                self.browser_filters[0].apply(items.len(), |i| needle.found_in(&items[i].title));
            }
            ListRef::BAlbums => {
                let items = &self.browser.albums;
                self.browser_filters[1].apply(items.len(), |i| {
                    let a = &items[i];
                    needle.found_in(&a.title) || needle.found_in(&a.artist)
                });
            }
            ListRef::BTracks => {
                let items = &self.browser.tracks;
                self.browser_filters[2].apply(items.len(), |i| {
                    let t = &items[i];
                    needle.found_in(&t.title)
                        || needle.found_in(&t.artist)
                        || needle.found_in(&t.album)
                });
            }
            ListRef::PlList => {
                let items = &self.playlists.playlists;
                self.pl_list_filter.apply(items.len(), |i| needle.found_in(&items[i].title));
            }
            ListRef::PlItems => {
                let items = &self.playlists.items;
                self.pl_items_filter.apply(items.len(), |i| {
                    let t = &items[i];
                    needle.found_in(&t.title)
                        || needle.found_in(&t.artist)
                        || needle.found_in(&t.album)
                });
            }
            ListRef::Search | ListRef::SearchDetail | ListRef::Help => {}
        }
    }
}

/// mpv booleans come as bool, "no"/"inf"/"force" or numbers.
fn prop_on(value: &Option<serde_json::Value>) -> bool {
    use serde_json::Value;
    match value {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !matches!(s.as_str(), "no" | "false" | "0" | "off" | "none" | ""),
        Some(Value::Number(n)) => n.as_f64().map(|x| x != 0.0).unwrap_or(false),
        _ => false,
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

/// "year" (the fallback), "year_desc" or "title". No year sorts last and ties
/// are broken, so the server's order never shows through.
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

pub fn spawn_plex_worker(
    client: PlexClient,
    section: String,
    tx: UnboundedSender<Msg>,
    art_cache_mb: u64,
    art_max_size_px: [u32; 2],
) -> UnboundedSender<PlexCmd> {
    let (ctx, mut crx) = tokio::sync::mpsc::unbounded_channel::<PlexCmd>();
    tokio::spawn(async move {
        while let Some(cmd) = crx.recv().await {
            let client = client.clone();
            let tx = tx.clone();
            let section = section.clone();
            tokio::spawn(async move {
                match cmd {
                    PlexCmd::LoadArtists => match client.artists(&section).await {
                        Ok(a) => {
                            let _ = tx.send(Msg::ArtistsLoaded(a));
                        }
                        Err(e) => {
                            let _ = tx.send(Msg::Error(format!("loading artists: {e:#}")));
                        }
                    },
                    PlexCmd::LoadAlbums(key) => match client.albums(&section, &key).await {
                        Ok(albums) => {
                            let _ = tx.send(Msg::AlbumsLoaded { artist_key: key, albums });
                        }
                        Err(e) => {
                            let _ = tx.send(Msg::LoadFailed {
                                key,
                                error: format!("loading albums: {e:#}"),
                            });
                        }
                    },
                    PlexCmd::LoadTracks(key) => match client.album_tracks(&key).await {
                        Ok(tracks) => {
                            let _ = tx.send(Msg::TracksLoaded { album_key: key, tracks });
                        }
                        Err(e) => {
                            let _ = tx.send(Msg::LoadFailed {
                                key,
                                error: format!("loading tracks: {e:#}"),
                            });
                        }
                    },
                    PlexCmd::Search(q) => match client.search(&q).await {
                        Ok(hits) => {
                            let _ = tx.send(Msg::SearchResults { query: q, hits });
                        }
                        Err(e) => {
                            let _ = tx.send(Msg::Error(format!("searching: {e:#}")));
                        }
                    },
                    PlexCmd::LoadPlaylists => match client.playlists().await {
                        Ok(list) => {
                            let _ = tx.send(Msg::PlaylistsLoaded(list));
                        }
                        Err(e) => {
                            let _ = tx.send(Msg::Error(format!("loading playlists: {e:#}")));
                        }
                    },
                    PlexCmd::SavePlaylist { name, keys } => {
                        let outcome = client.save_playlist(&name, &keys).await;
                        let _ = tx.send(Msg::PlaylistSaved(outcome));
                    }
                    PlexCmd::LoadPlaylistPreview { key, size } => {
                        match client.playlist_items_page(&key, 0, size).await {
                            Ok(tracks) => {
                                let _ =
                                    tx.send(Msg::PlaylistItemsLoaded { rating_key: key, tracks });
                            }
                            Err(e) => {
                                tracing::warn!("playlist preview: {e:#}");
                                let _ = tx.send(Msg::PreviewFailed { key });
                            }
                        }
                    }
                    PlexCmd::ResolvePlaylist {
                        seq,
                        key,
                        confirm,
                        action,
                        start,
                        stream,
                        expected,
                        cancel,
                    } => {
                        crate::ui::playlist_load::run(
                            client,
                            tx,
                            PlaylistRequest {
                                seq,
                                key,
                                confirm,
                                action,
                                start,
                                stream,
                                expected,
                                cancel,
                            },
                        )
                        .await
                    }
                    PlexCmd::ResolveTrack { seq, key, confirm, action, rest_of_album } => {
                        let reply = QueueReplyGuard::new(tx, seq);
                        match client.track(&key).await {
                            Ok(t) => {
                                let mut tracks = vec![t.clone()];
                                if rest_of_album {
                                    if let Some(album_key) = t.parent_rating_key.as_deref() {
                                        if let Ok(mut album) = client.album_tracks(album_key).await
                                        {
                                            if let Some(pos) = album
                                                .iter()
                                                .position(|x| x.rating_key == t.rating_key)
                                            {
                                                tracks = album.split_off(pos);
                                            }
                                        }
                                    }
                                }
                                reply.send(QueueOutcome::Tracks { tracks, confirm, action });
                            }
                            Err(e) => {
                                reply.send(QueueOutcome::Failed(format!("resolving track: {e:#}")))
                            }
                        }
                    }
                    PlexCmd::ResolveArtistTracks { seq, key, confirm, action } => {
                        let reply = QueueReplyGuard::new(tx, seq);
                        match client.artist_tracks(&section, &key).await {
                            Ok(tracks) => {
                                reply.send(QueueOutcome::Tracks { tracks, confirm, action })
                            }
                            Err(e) => {
                                reply.send(QueueOutcome::Failed(format!("resolving artist: {e:#}")))
                            }
                        }
                    }
                    PlexCmd::ResolveAlbum { seq, key, confirm, action } => {
                        let reply = QueueReplyGuard::new(tx, seq);
                        match client.album_tracks(&key).await {
                            Ok(tracks) => {
                                reply.send(QueueOutcome::Tracks { tracks, confirm, action })
                            }
                            Err(e) => {
                                reply.send(QueueOutcome::Failed(format!("resolving album: {e:#}")))
                            }
                        }
                    }
                    PlexCmd::ResolveArtist(key) => match client.artist(&key).await {
                        Ok(a) => {
                            let _ = tx.send(Msg::GotoArtist(a));
                        }
                        Err(e) => {
                            let _ = tx.send(Msg::LoadFailed {
                                key,
                                error: format!("resolving artist: {e:#}"),
                            });
                        }
                    },
                    PlexCmd::LoadAlbumDetail { key, label } => {
                        match client.album_tracks(&key).await {
                            Ok(tracks) => {
                                let _ = tx.send(Msg::SearchAlbumLoaded { key, label, tracks });
                            }
                            Err(e) => {
                                let _ = tx.send(Msg::Error(format!("loading album: {e:#}")));
                            }
                        }
                    }
                    PlexCmd::LoadThumb { path } => {
                        match load_thumb(&client, &path, art_cache_mb, art_max_size_px).await {
                            Ok(img) => {
                                let _ = tx.send(Msg::ArtReady { thumb_path: path, img });
                            }
                            Err(e) => {
                                tracing::warn!("album art: {e:#}");
                                let _ = tx.send(Msg::ArtFailed { thumb_path: path });
                            }
                        }
                    }
                    PlexCmd::LoadTrackInfo(key) => match client.track(&key).await {
                        Ok(t) => {
                            let _ = tx.send(Msg::TrackInfo { rating_key: key, audio: t.audio });
                        }
                        Err(e) => tracing::warn!("track info: {e:#}"),
                    },
                }
            });
        }
    });
    ctx
}

async fn load_thumb(
    client: &PlexClient,
    path: &str,
    cache_mb: u64,
    max_size_px: [u32; 2],
) -> anyhow::Result<Arc<DynamicImage>> {
    // no private cache dir means no art cache, never a shared one
    let cache_mb = match crate::cache::try_ensure_cache_dir() {
        Ok(_) => cache_mb,
        Err(e) => {
            tracing::warn!("album art cache disabled, cache dir is not private: {e}");
            0
        }
    };
    // None only without a home directory, when the cache is already off.
    load_cached_art(
        &crate::cache::art_path(path).unwrap_or_default(),
        cache_mb,
        max_size_px,
        || client.thumb_bytes(path),
    )
    .await
}

/// Decoded before it is cached, so a bad response never lands there; a cached
/// file that fails to decode is fetched again. `cache_mb == 0` disables it.
async fn load_cached_art<F, Fut>(
    cache_file: &std::path::Path,
    cache_mb: u64,
    max_size_px: [u32; 2],
    fetch: F,
) -> anyhow::Result<Arc<DynamicImage>>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<Vec<u8>>>,
{
    async fn decode(bytes: Vec<u8>, max_size_px: [u32; 2]) -> anyhow::Result<DynamicImage> {
        tokio::task::spawn_blocking(move || decode_image(&bytes, max_size_px)).await?
    }

    if cache_mb > 0 {
        if let Ok(bytes) = tokio::fs::read(cache_file).await {
            match decode(bytes, max_size_px).await {
                Ok(img) => {
                    crate::cache::touch(cache_file); // LRU: keep recently used art
                    return Ok(Arc::new(img));
                }
                Err(e) => {
                    tracing::warn!("cached cover is unusable, fetching it again: {e:#}");
                    let _ = tokio::fs::remove_file(cache_file).await;
                }
            }
        }
    }
    let bytes = fetch().await?;
    if cache_mb == 0 {
        return Ok(Arc::new(decode(bytes, max_size_px).await?));
    }
    let to_cache = bytes.clone();
    let img = decode(bytes, max_size_px).await?;
    if let Some(dir) = cache_file.parent() {
        let dest = cache_file.to_path_buf();
        let dir = dir.to_path_buf();
        let _ = tokio::task::spawn_blocking(move || {
            let written = crate::cache::ensure_private_dir(&dir)
                .and_then(|()| crate::cache::write_private_atomic(&dest, &to_cache));
            if let Err(e) = written {
                tracing::warn!("cannot cache the cover: {e}");
            }
            crate::cache::prune_art_at(&dir, cache_mb);
        })
        .await;
    }
    Ok(Arc::new(img))
}

/// The decoder's allocation ceiling, applied before it allocates: a hostile
/// server must not make shmc reserve gigabytes before `max_size_px` applies.
const MAX_IMAGE_ALLOC: u64 = 64 * 1024 * 1024;

/// The budget follows `album_art.max_size_px` (RGBA at that size), with a floor
/// for the "unlimited" default.
fn decode_limits(max_size_px: [u32; 2]) -> image::Limits {
    let pixels = u64::from(max_size_px[0]) * u64::from(max_size_px[1]);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(pixels.saturating_mul(4).max(MAX_IMAGE_ALLOC));
    limits
}

fn decode_image(bytes: &[u8], max_size_px: [u32; 2]) -> anyhow::Result<DynamicImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .context("guessing image format")?;
    reader.limits(decode_limits(max_size_px));
    reader.decode().context("decoding image")
}

use anyhow::Context;

#[cfg(test)]
mod tests {
    use super::*;

    /// splitmix64: deterministic, so a failing random case can be replayed.
    fn prng(seed: &mut u64) -> u64 {
        *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn random_permutation(n: usize, seed: &mut u64) -> Vec<usize> {
        let mut order: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            let j = (prng(seed) as usize) % (i + 1);
            order.swap(i, j);
        }
        order
    }

    /// The plain O(n²) version: the moves mpv must receive.
    fn reference_moves(order: &[usize]) -> (Vec<(usize, usize)>, Vec<usize>) {
        let mut working: Vec<usize> = (0..order.len()).collect();
        let mut moves = Vec::new();
        for (target, &item) in order.iter().enumerate() {
            let from =
                working.iter().position(|&i| i == item).expect("order must be a permutation");
            if from != target {
                moves.push((from, target));
                working.remove(from);
                working.insert(target, item);
            }
        }
        (moves, working)
    }

    /// `moved`: some position the entry passed through differed from its final
    /// one, i.e. mpv reports intermediate events.
    fn reference_play(order: &[usize], play: usize) -> (usize, bool) {
        let mut working: Vec<usize> = (0..order.len()).collect();
        let final_pos = order.iter().position(|&i| i == play).expect("permutation");
        let at = |w: &[usize]| w.iter().position(|&i| i == play).expect("still queued");
        let mut moved = at(&working) != final_pos;
        for (target, &item) in order.iter().enumerate() {
            let from =
                working.iter().position(|&i| i == item).expect("order must be a permutation");
            if from != target {
                working.remove(from);
                working.insert(target, item);
            }
            moved |= at(&working) != final_pos;
        }
        (final_pos, moved)
    }

    /// The app queue and mpv stay in lockstep only if they see the same moves.
    #[test]
    fn queue_move_plan_matches_the_plain_version() {
        fn check(order: &[usize], play: Option<usize>, tag: &str) {
            let plan = queue_move_plan(order, play)
                .unwrap_or_else(|| panic!("{tag}: rejected a permutation"));
            let (moves, final_order) = reference_moves(order);
            assert_eq!(plan.moves, moves, "{tag}: order = {order:?}");
            assert_eq!(final_order, order, "{tag}: the walk lands on `order`");
            for (new, &old) in order.iter().enumerate() {
                assert_eq!(plan.position[old], new, "{tag}: inverse mapping");
            }
            match play {
                Some(p) => {
                    let (final_pos, moved) = reference_play(order, p);
                    assert_eq!(plan.play, Some(final_pos), "{tag}");
                    assert_eq!(plan.play_moved, moved, "{tag}");
                }
                None => {
                    assert_eq!(plan.play, None, "{tag}");
                    assert!(!plan.play_moved, "{tag}");
                }
            }
        }

        let mut seed = 0x5EED_1234_5678_9ABC_u64;
        for round in 0..1000 {
            let n = (prng(&mut seed) % 65) as usize;
            let order = random_permutation(n, &mut seed);
            let play = (n > 0).then(|| (prng(&mut seed) as usize) % n);
            check(&order, play, &format!("round {round}"));
        }
        // larger trees, where the Fenwick tree does real work
        for n in [0, 1, 2, 3, 127, 512] {
            let order = random_permutation(n, &mut seed);
            let play = (n > 0).then(|| (prng(&mut seed) as usize) % n);
            check(&order, play, &format!("n = {n}"));
        }
    }

    /// Applying a broken order would duplicate or drop tracks in both playlists.
    #[test]
    fn queue_move_plan_refuses_a_broken_order() {
        assert!(queue_move_plan(&[0, 0], None).is_none());
        assert!(queue_move_plan(&[0, 2], None).is_none());
        assert!(queue_move_plan(&[0, 1], None).is_some());
        assert!(queue_move_plan(&[], None).is_some());
    }

    /// A quadratic restore takes seconds at 50k. The ceiling is coarse to
    /// stay unflaky under load; the scaling ratio is the real guard.
    #[test]
    fn queue_move_plan_scales_linearly() {
        let mut seed = 42_u64;
        let mut timed = |n: usize| {
            let order = random_permutation(n, &mut seed);
            let started = Instant::now();
            let plan = queue_move_plan(&order, Some(n / 2)).expect("permutation");
            let elapsed = started.elapsed();
            assert!(!plan.moves.is_empty());
            elapsed
        };
        // warm the allocator
        let _ = timed(1_000);
        let small = timed(25_000).min(timed(25_000));
        let large = timed(50_000).min(timed(50_000));
        assert!(large < Duration::from_millis(1500), "50k restore plan took {large:?}");
        assert!(
            large < small * 4 + Duration::from_millis(50),
            "doubling n scaled super-linearly: {small:?} -> {large:?}"
        );
    }

    #[test]
    fn command_parser_accepts_colon_or_not() {
        assert_eq!(parse_command("pause"), Command::Pause);
        assert_eq!(parse_command(":pause"), Command::Pause);
        assert_eq!(parse_command("::pause"), Command::Pause);
        assert_eq!(parse_command(" q "), Command::Quit);
        assert_eq!(parse_command("quit"), Command::Quit);
        assert_eq!(parse_command("next"), Command::Next);
        assert_eq!(parse_command("prev"), Command::Prev);
        assert_eq!(parse_command("play"), Command::Play);
        assert_eq!(parse_command("resume"), Command::Play);
        assert_eq!(parse_command("stop"), Command::Stop);
        assert_eq!(parse_command("seek -10"), Command::Seek("-10"));
        assert_eq!(parse_command("vol 42"), Command::Volume("42"));
        assert_eq!(parse_command(":vol +5"), Command::Volume("+5"));
        assert_eq!(parse_command("save  Road   trip "), Command::Save("Road   trip"));
        assert_eq!(parse_command(":save"), Command::BadSave);
        assert_eq!(parse_command("saves"), Command::Unknown("saves"));
        assert_eq!(parse_command("clear"), Command::Clear);
        assert_eq!(parse_command("shuffle"), Command::Shuffle);
        assert_eq!(parse_command("random"), Command::Unknown("random"));
        assert_eq!(parse_command("repeat"), Command::Repeat);
        assert_eq!(parse_command("single"), Command::Single);
        assert_eq!(parse_command("consume"), Command::Consume);
        assert_eq!(parse_command("reshuffle"), Command::Reshuffle);
        assert_eq!(parse_command("playing"), Command::Playing);
        assert_eq!(parse_command("sleep"), Command::Sleep(""));
        assert_eq!(parse_command("sleep 30"), Command::Sleep("30"));
        assert_eq!(parse_command("sleep off"), Command::Sleep("off"));
        assert_eq!(parse_command("sleep 1 2"), Command::BadSleep);
        assert_eq!(parse_command("queue"), Command::Tab(Tab::Queue));
        assert_eq!(parse_command("browser"), Command::Tab(Tab::Browser));
        assert_eq!(parse_command("playlists"), Command::Tab(Tab::Playlists));
        assert_eq!(parse_command("search"), Command::Tab(Tab::Search));
        assert_eq!(parse_command("help"), Command::Tab(Tab::Help));
    }

    #[test]
    fn command_parser_reports_bad_volume_and_unknown() {
        assert_eq!(parse_command("vol"), Command::BadVolume);
        assert_eq!(parse_command("vol 1 2"), Command::BadVolume);
        assert_eq!(parse_command("seek"), Command::BadSeek);
        assert_eq!(parse_command("seek 1 2"), Command::BadSeek);
        assert_eq!(parse_command(""), Command::Empty);
        assert_eq!(parse_command(":"), Command::Empty);
        assert_eq!(parse_command("wat"), Command::Unknown("wat"));
        assert_eq!(parse_command(":wat now"), Command::Unknown("wat now"));
    }

    /// An IHDR claiming `w` x `h` RGBA and no pixels (the empty IDAT is there
    /// because the decoder will not start without one).
    fn png_claiming(w: u32, h: u32) -> Vec<u8> {
        fn crc32(bytes: &[u8]) -> u32 {
            let mut crc = !0u32;
            for &b in bytes {
                crc ^= u32::from(b);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
                }
            }
            !crc
        }
        fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
            out.extend((data.len() as u32).to_be_bytes());
            let mut body = kind.to_vec();
            body.extend(data);
            out.extend(&body);
            out.extend(crc32(&body).to_be_bytes());
        }
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend(w.to_be_bytes());
        ihdr.extend(h.to_be_bytes());
        ihdr.extend([8, 6, 0, 0, 0]); // 8 bit, RGBA, deflate, no filter, no interlace
        chunk(&mut png, b"IHDR", &ihdr);
        chunk(&mut png, b"IDAT", &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01]);
        chunk(&mut png, b"IEND", &[]);
        png
    }

    #[test]
    fn art_decoding_bounded_and_works() {
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let decoded = decode_image(&png, [0, 0]).expect("a real cover decodes");
        assert_eq!(decoded.width(), 1);

        // 128 MB: over shmc's ceiling, under the image crate's default
        let bomb = png_claiming(8000, 4000);
        let err = decode_image(&bomb, [0, 0]).expect_err("a decompression bomb is refused");
        assert!(
            matches!(err.downcast_ref::<image::ImageError>(), Some(image::ImageError::Limits(_))),
            "refused by the limits check, not by a malformed file: {err:#}"
        );

        assert_eq!(decode_limits([0, 0]).max_alloc, Some(MAX_IMAGE_ALLOC));
        // a configured cap gets room for RGBA at that size
        assert_eq!(decode_limits([5000, 5000]).max_alloc, Some(5000u64 * 5000 * 4));
    }

    fn tiny_png() -> Vec<u8> {
        let mut out = Vec::new();
        DynamicImage::new_rgb8(4, 4)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[tokio::test]
    async fn corrupt_cached_cover_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let cache_file = dir.path().join("art").join("c.img");
        std::fs::create_dir_all(cache_file.parent().unwrap()).unwrap();
        std::fs::write(&cache_file, b"<html>proxy page</html>").unwrap();

        let good = tiny_png();
        let img = load_cached_art(&cache_file, 10, [0, 0], || {
            let good = good.clone();
            async move { Ok(good) }
        })
        .await
        .expect("fetched again after the cached copy failed to decode");
        assert_eq!(img.width(), 4);
        assert_eq!(std::fs::read(&cache_file).unwrap(), good);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&cache_file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        // a 200 that is not an image never reaches the cache
        let other = dir.path().join("art").join("d.img");
        let res =
            load_cached_art(&other, 10, [0, 0], || async { Ok(b"not an image".to_vec()) }).await;
        assert!(res.is_err());
        assert!(!other.exists());
    }

    fn cover_track(i: usize) -> Track {
        Track {
            rating_key: format!("t{i}"),
            title: format!("T{i}"),
            artist: "A".into(),
            album: "B".into(),
            track_no: None,
            year: None,
            duration_ms: None,
            parent_rating_key: Some(format!("al{i}")),
            grandparent_rating_key: None,
            thumb: None,
            part_key: Some(format!("/p/{i}")),
            container: None,
            audio: AudioInfo::default(),
            playlist_item_id: None,
        }
    }

    fn play_pos(app: &mut App, i: u64) {
        app.handle_msg(Msg::Mpv(MpvEvent::Property {
            name: "playlist-pos".into(),
            value: Some(serde_json::json!(i)),
        }));
    }

    /// A late cover must not leave its path marked in flight.
    #[tokio::test]
    async fn late_cover_can_be_requested_again() {
        let cfg = crate::config::MpvConfig {
            path: "mpv".into(),
            extra_args: vec!["--ao=null".into()],
            volume: 50.0,
        };
        let (mpv, _events) = crate::playback::mpv::spawn(&cfg).await.expect("spawn mpv");
        let (plex_tx, mut plex_rx) = tokio::sync::mpsc::unbounded_channel();
        let (msg_tx, _msg_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            "http://127.0.0.1:1".into(),
            "TOK".into(),
            PlexHandle { tx: plex_tx },
            mpv,
            50.0,
            None,
            true,
            [0, 0],
            msg_tx,
            UiConfig::default(),
            &KeyTable::default(),
        );
        app.queue = (0..2).map(cover_track).collect();
        let thumb = |i: usize| format!("/library/metadata/al{i}/thumb");
        let loads = |rx: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>| {
            let mut paths = Vec::new();
            while let Ok(cmd) = rx.try_recv() {
                if let PlexCmd::LoadThumb { path } = cmd {
                    paths.push(path);
                }
            }
            paths
        };

        let cover = || Arc::new(DynamicImage::new_rgba8(2, 2));
        play_pos(&mut app, 0);
        assert_eq!(loads(&mut plex_rx), [thumb(0)]);
        app.handle_msg(Msg::ArtReady { thumb_path: thumb(0), img: cover() });
        play_pos(&mut app, 1);
        assert_eq!(loads(&mut plex_rx), [thumb(1)]);
        // back on the cover that is showing, then the other one lands too late
        play_pos(&mut app, 0);
        app.handle_msg(Msg::ArtReady { thumb_path: thumb(1), img: cover() });
        play_pos(&mut app, 1);
        assert_eq!(loads(&mut plex_rx), [thumb(1)]);
    }
}

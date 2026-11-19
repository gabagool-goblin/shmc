use super::{move_sel, App, ListRef, PlexCmd, QueueAction, Tab};
use crate::playback::{MediaKey, MpvCmd};
use crate::ui::keymap::{self, Act, KeyCtx};
use std::time::Duration;

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
        ["q" | "quit"] => Command::Quit,
        ["n" | "next"] => Command::Next,
        ["p" | "prev"] => Command::Prev,
        ["pause"] => Command::Pause,
        ["play" | "resume"] => Command::Play,
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

/// Ctrl-/Alt-letters are shortcuts, never text for an input box.
fn has_ctrl_or_alt(key: &crossterm::event::KeyEvent) -> bool {
    use crossterm::event::KeyModifiers;
    key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

impl App {
    pub(super) fn handle_key(&mut self, ev: crossterm::event::Event) {
        use crossterm::event::{Event, KeyEvent, KeyEventKind};

        let Event::Key(key @ KeyEvent { kind: KeyEventKind::Press | KeyEventKind::Repeat, .. }) =
            ev
        else {
            return;
        };

        // a held `D` must not answer its own "clear the queue?"
        if key.kind == KeyEventKind::Repeat
            && (self.pending_clear.is_some() || self.has_pending_rating())
        {
            return;
        }

        self.dismiss_status();

        // Ctrl-C quits from anywhere, even mid-input
        if keymap::key_id(&key) == "C-c" {
            self.exec(Act::Quit);
            return;
        }

        if self.answer_rating(key.code) {
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
                crossterm::event::KeyCode::Char('y' | 'Y') => {
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
                crossterm::event::KeyCode::Char('y' | 'Y') => {
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
                        Act::StartFilter
                        | Act::StartSearch
                        | Act::ColumnLeft
                        | Act::ColumnRight
                        | Act::JumpToPlaying,
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
                    self.set_search_detail(None);
                    self.clear_list_requests();
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
    pub(super) fn handle_paste(&mut self, text: String) {
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
        if self.queue.items.is_empty() {
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
        let keys = self.queue.items.iter().map(|t| t.rating_key.clone()).collect();
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
            Command::Stop => self.stop(),
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
                            self.transient_err(format!("usage: :sleep minutes|off (got {arg:?})"));
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

    fn stop(&mut self) {
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

    pub(super) fn media_key(&mut self, key: MediaKey) {
        match key {
            MediaKey::Toggle => self.exec(Act::TogglePause),
            // Set rather than toggle, so the play-only key never pauses a playing track.
            MediaKey::Play | MediaKey::Pause => {
                if !self.stopped {
                    let _ = self.mpv.tx.send(MpvCmd::SetPause { paused: key == MediaKey::Pause });
                }
            }
            MediaKey::Stop => self.stop(),
            MediaKey::Next => self.exec(Act::NextTrack),
            MediaKey::Prev => self.exec(Act::PrevTrack),
            MediaKey::Forward => self.exec(Act::SeekFwd),
            MediaKey::Rewind => self.exec(Act::SeekBack),
        }
    }

    pub(super) fn key_ctx(&self) -> KeyCtx {
        match self.tab {
            Tab::Queue => KeyCtx::Queue,
            Tab::Browser => KeyCtx::Browser,
            Tab::Playlists => KeyCtx::Playlists,
            Tab::Search => KeyCtx::Search,
            Tab::Help => KeyCtx::Help,
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
                            let moves = self.queue.items.len() > 1
                                && (i + 1 < self.queue.items.len() || self.loop_playlist);
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
            Act::Rate => self.rate_highlighted(),
            Act::RatePlaying => self.rate_playing(),
            Act::Similar => self.find_similar(),
            Act::Radio => self.start_radio(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn every_listed_command_parses() {
        for (cmds, _) in keymap::COMMANDS {
            for cmd in cmds.split(" / ") {
                assert!(!matches!(parse_command(cmd), Command::Unknown(_)), "{cmd} does not parse");
            }
        }
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
}

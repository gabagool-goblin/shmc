//! Glyph sets for the transport and status icons, per `ui.icons`.

/// Nerd glyphs are one cell wide; the ASCII `||` is two.
pub struct Transport {
    pub play: &'static str,
    pub pause: &'static str,
    pub stop: &'static str,
}

pub struct Status {
    pub repeat: &'static str,
    pub random: &'static str,
    pub single: &'static str,
    pub consume: &'static str,
}

pub const TRANSPORT_NERD: Transport = Transport {
    play: "\u{f04b}",  // nf-fa-play
    pause: "\u{f04c}", // nf-fa-pause
    stop: "\u{f04d}",  // nf-fa-stop
};

pub const TRANSPORT_ASCII: Transport = Transport {
    play: ">",
    pause: "||",
    stop: "\u{25a0}", // ■
};

/// Material Design glyphs, one cell each.
pub const STATUS_NERD: Status = Status {
    repeat: "\u{f0456}",  // nf-md-repeat
    random: "\u{f049d}",  // nf-md-shuffle
    single: "\u{f0458}",  // nf-md-repeat_once
    consume: "\u{f0413}", // nf-md-playlist_remove
};

pub const STATUS_ASCII: Status =
    Status { repeat: "rep", random: "shf", single: "one", consume: "con" };

pub fn transport(ascii: bool) -> &'static Transport {
    if ascii {
        &TRANSPORT_ASCII
    } else {
        &TRANSPORT_NERD
    }
}

pub fn status(ascii: bool) -> &'static Status {
    if ascii {
        &STATUS_ASCII
    } else {
        &STATUS_NERD
    }
}

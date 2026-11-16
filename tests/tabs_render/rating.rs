//! Star ratings: what `r` sends and what the header shows.

use super::*;
use shmc::config::RatingStyle;

/// Three tracks with `t1` playing and writes allowed.
async fn playing(style: RatingStyle) -> (TestApp, tokio::sync::mpsc::UnboundedReceiver<PlexCmd>) {
    let (mut app, plex) = new_app_with_plex().await;
    app.queue = (0..3).map(track).collect();
    app.current = Some(1);
    app.stopped = false;
    app.set_ratings = true;
    app.rating_style = style;
    (app, plex)
}

fn rates(plex: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>) -> Vec<(String, Option<u8>)> {
    sent(plex)
        .into_iter()
        .filter_map(|c| match c {
            PlexCmd::Rate { key, rating } => Some((key, rating)),
            _ => None,
        })
        .collect()
}

/// A digit answers the question, `0` clears.
#[tokio::test]
async fn five_style_digit_rates_and_zero_clears() {
    let (mut app, mut plex) = playing(RatingStyle::Five).await;
    app.handle_msg(key('r'));
    assert!(rates(&mut plex).is_empty(), "asking sends nothing");
    app.handle_msg(key('4'));
    assert_eq!(rates(&mut plex), [("t1".to_string(), Some(8))]);
    app.handle_msg(key('r'));
    app.handle_msg(key('0'));
    assert_eq!(rates(&mut plex), [("t1".to_string(), None)]);
}

/// A track change while the question is open does not redirect the answer.
#[tokio::test]
async fn open_question_rates_the_track_it_named() {
    let (mut app, mut plex) = playing(RatingStyle::Five).await;
    app.handle_msg(key('r'));
    app.handle_msg(playlist_pos(2));
    assert_eq!(app.current, Some(2));
    app.handle_msg(key('3'));
    assert_eq!(rates(&mut plex), [("t1".to_string(), Some(6))]);
}

/// Any other key cancels and is not run as its own action.
#[tokio::test]
async fn other_key_cancels_the_question_and_is_swallowed() {
    let (mut app, mut plex) = playing(RatingStyle::Five).await;
    app.handle_msg(key('r'));
    app.handle_msg(key('x'));
    assert!(!app.loop_playlist, "x (repeat) must not run");
    app.handle_msg(key('4'));
    assert!(rates(&mut plex).is_empty());
}

/// One star: unrated is starred, anything rated is cleared, with no question.
#[tokio::test]
async fn one_style_toggles_without_asking() {
    let (mut app, mut plex) = playing(RatingStyle::One).await;
    app.handle_msg(key('r'));
    assert_eq!(rates(&mut plex), [("t1".to_string(), Some(10))]);
    assert!(app.status.is_empty(), "no question: {}", app.status);
    app.ratings.insert("t1".into(), Some(6));
    app.handle_msg(key('r'));
    assert_eq!(rates(&mut plex), [("t1".to_string(), None)]);
}

/// Writing off, the style off or nothing playing sends nothing.
#[tokio::test]
async fn refused_rating_sends_nothing() {
    let (mut app, mut plex) = playing(RatingStyle::Five).await;
    app.set_ratings = false;
    app.handle_msg(key('r'));
    assert!(app.status.contains("plex.set_ratings"), "{}", app.status);
    app.handle_msg(key('4'));

    app.set_ratings = true;
    app.rating_style = RatingStyle::Off;
    app.handle_msg(key('r'));
    assert!(app.status.contains("ui.rating_style"), "{}", app.status);

    app.rating_style = RatingStyle::Five;
    app.stopped = true;
    app.handle_msg(key('r'));
    app.handle_msg(key('4'));
    assert!(rates(&mut plex).is_empty());
}

fn title_row(app: &mut App, w: u16) -> Vec<char> {
    render(app, w, 30).lines().nth(1).unwrap().chars().collect()
}

/// Metadata and a confirmed write both change what the header shows.
#[tokio::test]
async fn header_follows_metadata_and_confirmed_writes() {
    let (mut app, _plex) = playing(RatingStyle::Five).await;
    assert!(!title_row(&mut app, 100).iter().collect::<String>().contains('*'));
    app.handle_msg(Msg::TrackInfo {
        rating_key: "t1".into(),
        audio: AudioInfo::default(),
        user_rating: Some(7),
    });
    let row: String = title_row(&mut app, 100).iter().collect();
    assert!(row.contains("Starfall! 1 ****."), "{row}");
    app.handle_msg(Msg::Rated { rating_key: "t1".into(), rating: Some(10) });
    let row: String = title_row(&mut app, 100).iter().collect();
    assert!(row.contains("Starfall! 1 *****"), "{row}");
    app.handle_msg(Msg::Rated { rating_key: "t1".into(), rating: None });
    let row: String = title_row(&mut app, 100).iter().collect();
    assert!(!row.contains('*'), "{row}");
}

#[tokio::test]
async fn one_star_style_shows_a_single_star() {
    let (mut app, _plex) = playing(RatingStyle::One).await;
    app.ratings.insert("t1".into(), Some(4));
    let row: String = title_row(&mut app, 100).iter().collect();
    assert!(row.contains("Starfall! 1 *") && !row.contains("**"), "{row}");
}

/// A fetched rating is not fetched again for another entry of the track.
#[tokio::test]
async fn known_rating_is_not_fetched_again() {
    let (mut app, mut plex) = playing(RatingStyle::Five).await;
    let full = AudioInfo { sample_rate_hz: Some(44100), ..Default::default() };
    app.queue = (0..2).map(|_| Track { audio: full.clone(), ..track(0) }).collect();
    app.handle_msg(playlist_pos(0));
    let asked = |plex: &mut _| {
        sent(plex).iter().filter(|c| matches!(c, PlexCmd::LoadTrackInfo(k) if k == "t0")).count()
    };
    assert_eq!(asked(&mut plex), 1);
    app.handle_msg(Msg::TrackInfo { rating_key: "t0".into(), audio: full, user_rating: Some(6) });
    app.handle_msg(playlist_pos(1));
    assert_eq!(asked(&mut plex), 0);
    let row: String = title_row(&mut app, 100).iter().collect();
    assert!(row.contains("***.."), "{row}");
}

/// The title keeps its column rated or not; the rating is dropped when it does not fit.
#[tokio::test]
async fn rating_never_moves_the_title() {
    // (width, whether the rating fits beside a short title)
    for (w, fits) in [(100, true), (80, true), (72, false)] {
        let (mut app, _plex) = playing(RatingStyle::Five).await;
        let bare = title_row(&mut app, w);
        app.ratings.insert("t1".into(), Some(7));
        let rated = title_row(&mut app, w);
        let at = |row: &[char], s: &str| {
            let want: Vec<char> = s.chars().collect();
            row.windows(want.len()).position(|win| win == want)
        };
        let col = at(&bare, "Starfall! 1").unwrap();
        assert_eq!(at(&rated, "Starfall! 1"), Some(col), "{w}");
        let after: String = rated[col + 11..col + 17].iter().collect();
        if fits {
            assert_eq!(after, " ****.", "{w}");
        } else {
            assert_eq!(rated, bare, "{w}");
        }

        app.queue[1].title = "Long title ".repeat(8);
        app.ratings.remove("t1");
        let bare = title_row(&mut app, w);
        app.ratings.insert("t1".into(), Some(7));
        assert_eq!(title_row(&mut app, w), bare, "a full-width title drops the rating at {w}");
    }
}

/// The compact layout has no header: the now-playing line carries the rating
/// in every border style, and drops it when the title would be cut.
#[tokio::test]
async fn compact_line_shows_the_rating_after_the_title() {
    use super::theme::{with_borders, LookGuard, STYLES};
    let (mut app, _plex) = playing(RatingStyle::Five).await;
    app.header_compact = true;
    app.tab = Tab::Help;
    app.ratings.insert("t1".into(), Some(7));
    for style in STYLES {
        let _look = LookGuard::set(with_borders(style));
        let screen = render(&mut app, 80, 24);
        assert!(screen.contains("Starfall! 1 ****. - Northwind Sounds"), "{style:?}\n{screen}");

        app.queue[1].title = "x".repeat(40);
        let rated = render(&mut app, 50, 24);
        app.ratings.remove("t1");
        assert_eq!(rated, render(&mut app, 50, 24), "{style:?}");
        app.queue[1].title = "Starfall! 1".into();
        app.ratings.insert("t1".into(), Some(7));
    }
}

/// Where the cover's metadata replaces the now-playing line, its title carries
/// the rating; the full layout leaves it to the header.
#[tokio::test]
async fn cover_metadata_shows_the_rating_in_compact_only() {
    let (mut app, _plex) = playing(RatingStyle::Five).await;
    queue_app_with_art(&mut app, true);
    app.art_metadata = true;
    app.ratings.insert("t2".into(), Some(7));
    let meta_title = |app: &mut App| {
        render(app, 80, 30)
            .lines()
            .find(|l| l.contains("│ Starfall! 2 ") && !l.contains("4:17"))
            .map(str::to_string)
    };
    app.header_compact = true;
    let row = meta_title(&mut app).expect("cover metadata");
    assert!(row.contains("│ Starfall! 2 ****. "), "{row}");
    app.header_compact = false;
    let row = meta_title(&mut app).expect("cover metadata");
    assert!(!row.contains('*'), "{row}");
}

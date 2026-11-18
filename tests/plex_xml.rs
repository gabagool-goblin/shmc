//! Parsing tests against XML captured from a real Plex Media Server.

use shmc::plex::models::MediaContainer;
use shmc::plex::HitKind;

#[test]
fn parse_artists_fixture() {
    let c: MediaContainer = quick_xml::de::from_str(include_str!("fixtures/artists.xml")).unwrap();
    assert_eq!(c.directories.len(), 3);
    let artists: Vec<_> = c.directories.into_iter().map(|d| d.into_artist()).collect();
    assert_eq!(artists[0].title, "Artist 1");
    assert_eq!(artists[0].rating_key, "20041");
    assert!(artists[0].thumb.as_deref().unwrap().starts_with("/library/metadata/"));
    assert_eq!(artists[1].title, "Artist 2");
    assert_eq!(artists[2].title, "テスト歌手 1");
}

#[test]
fn parse_albums_fixture() {
    let c: MediaContainer = quick_xml::de::from_str(include_str!("fixtures/albums.xml")).unwrap();
    assert!(!c.directories.is_empty());
    let albums: Vec<_> = c.directories.into_iter().map(|d| d.into_album()).collect();
    let a = &albums[0];
    assert_eq!(a.title, "Album 1");
    assert_eq!(a.year, Some(2008));
    assert_eq!(a.artist, "Artist 1");
}

#[test]
fn parse_track_audio_stream_details() {
    let xml = r#"<MediaContainer><Track ratingKey="1" key="/library/metadata/1" title="x"><Media bitrate="857" audioCodec="flac" container="flac" audioChannels="2"><Part id="1" key="/p" container="flac"><Stream streamType="2" selected="1" codec="flac" bitrate="857" bitDepth="16" samplingRate="44100" channels="2"/></Part></Media></Track></MediaContainer>"#;
    let c: MediaContainer = quick_xml::de::from_str(xml).unwrap();
    let t = c.tracks.into_iter().next().unwrap().into_track();
    assert_eq!(t.audio.codec.as_deref(), Some("flac"));
    assert_eq!(t.audio.bitrate_kbps, Some(857));
    assert_eq!(t.audio.sample_rate_hz, Some(44100));
    assert_eq!(t.audio.bit_depth, Some(16));
    assert_eq!(t.audio.channels, Some(2));
}

#[test]
fn track_artist_prefers_original_title() {
    let xml = r#"<MediaContainer>
<Track ratingKey="1" key="/library/metadata/1" title="One" grandparentTitle="Album Artist" originalTitle="Track Artist"/>
<Track ratingKey="2" key="/library/metadata/2" title="Two" grandparentTitle="Album Artist"/>
<Track ratingKey="3" key="/library/metadata/3" title="Three" grandparentTitle="Album Artist" originalTitle="  "/>
</MediaContainer>"#;
    let c: MediaContainer = quick_xml::de::from_str(xml).unwrap();
    let tracks: Vec<_> = c.tracks.into_iter().map(|t| t.into_track()).collect();
    assert_eq!(tracks[0].artist, "Track Artist");
    assert_eq!(tracks[1].artist, "Album Artist");
    assert_eq!(tracks[2].artist, "Album Artist");
}

#[test]
fn parse_tracks_fixture_and_stream_url() {
    let c: MediaContainer = quick_xml::de::from_str(include_str!("fixtures/tracks.xml")).unwrap();
    assert_eq!(c.tracks.len(), 1);
    let t = c.tracks[0].clone().into_track();
    assert_eq!(t.title, "テスト曲 25");
    assert_eq!(t.artist, "Artist 1");
    assert_eq!(t.album, "Album 1");
    assert_eq!(t.track_no, Some(13));
    assert_eq!(t.year, Some(2008));
    assert_eq!(t.duration_ms, Some(245054));
    // Bulk XML has <Media> but no <Stream>.
    assert_eq!(t.audio.codec.as_deref(), Some("mp3"));
    assert_eq!(t.audio.bitrate_kbps, Some(243));
    assert_eq!(t.audio.channels, Some(2));
    assert_eq!(t.audio.sample_rate_hz, None);
    assert_eq!(t.audio.bit_depth, None);
    let part_key = t.part_key.as_deref().unwrap();
    assert!(part_key.starts_with("/library/parts/"), "got {part_key}");
    assert_eq!(t.container.as_deref(), Some("mp3"));
    assert!(t.thumb.is_some());

    let url = t.stream_url("http://192.168.1.100:32400", "TOK").unwrap();
    assert!(url.starts_with("http://192.168.1.100:32400/library/parts/"));
    assert!(url.ends_with("X-Plex-Token=TOK"));
    assert_eq!(t.user_rating, Some(10));
}

/// Plex sends a float, omits it for an unrated track, and 0 means unrated.
#[test]
fn user_rating_is_rounded_and_zero_is_unrated() {
    let xml = r#"<MediaContainer><Track ratingKey="1" key="/k" userRating="6.5"/><Track ratingKey="2" key="/k" userRating="0.0"/><Track ratingKey="3" key="/k"/></MediaContainer>"#;
    let c: MediaContainer = quick_xml::de::from_str(xml).unwrap();
    let r: Vec<_> = c.tracks.into_iter().map(|t| t.into_track().user_rating).collect();
    assert_eq!(r, [Some(7), None, None]);
}

#[test]
fn parse_search_fixture() {
    let c: MediaContainer = quick_xml::de::from_str(include_str!("fixtures/search.xml")).unwrap();
    assert!(!c.directories.is_empty(), "search should return Directory entries");
    for d in &c.directories {
        let _ = d.clone().into_artist();
        let _ = d.clone().into_album();
    }
    for t in &c.tracks {
        let _ = t.clone().into_track();
    }
}

/// Sections carry `key`, not `ratingKey`; `shmc config --check` reads it.
#[test]
fn parse_sections_fixture() {
    let c: MediaContainer = quick_xml::de::from_str(include_str!("fixtures/sections.xml")).unwrap();
    assert_eq!(c.directories.len(), 4);
    let music: Vec<_> =
        c.directories.iter().filter(|d| d.type_.as_deref() == Some("artist")).collect();
    assert_eq!(music.len(), 1);
    assert_eq!(music[0].key, "1");
    assert_eq!(music[0].title.as_deref(), Some("Music"));
    assert!(music[0].rating_key.is_none(), "sections must not require ratingKey");
}

/// `<Playlist>` elements, smart and regular, with nested `<Image>` and
/// `<UltraBlurColors>` children the wire model must skip.
#[test]
fn parse_playlists_fixture() {
    let c: MediaContainer =
        quick_xml::de::from_str(include_str!("fixtures/playlists.xml")).unwrap();
    assert_eq!(c.playlists.len(), 25);
    let list: Vec<_> = c.playlists.into_iter().map(|p| p.into_playlist()).collect();

    let first = &list[0];
    assert_eq!(first.rating_key, "1276");
    assert!(first.smart);
    assert_eq!(first.item_count, Some(2236));
    assert!(first.thumb.as_deref().unwrap().starts_with("/playlists/"));
    assert!(first.title.contains('\u{2764}'), "got {:?}", first.title);

    let best = list.iter().find(|p| p.rating_key == "27261").unwrap();
    assert!(!best.smart);
    assert_eq!(best.title, "Playlist 15");
    assert_eq!(best.item_count, Some(635));

    assert!(list.iter().all(|p| p.duration_ms.is_some()));
}

/// Playlist order is meaningful and must be preserved.
#[test]
fn parse_playlist_items_fixture() {
    let c: MediaContainer =
        quick_xml::de::from_str(include_str!("fixtures/playlist_items.xml")).unwrap();
    assert_eq!(c.tracks.len(), 39);
    let tracks: Vec<_> = c.tracks.into_iter().map(|t| t.into_track()).collect();

    assert_eq!(tracks[0].title, "Track 8");
    assert_eq!(tracks[0].artist, "Artist 12");
    assert_eq!(tracks[1].artist, "Artist 28");
    assert_eq!(tracks[0].track_no, Some(10));
    assert!(tracks[0].part_key.as_deref().unwrap().starts_with("/library/parts/"));
    assert!(tracks[0].stream_url("http://x", "TOK").is_some());
}

/// `/hubs/search` groups results in `<Hub>` elements; plain `/search`
/// returns only stubs.
#[test]
fn parse_hubs_search_fixture() {
    let c: MediaContainer =
        quick_xml::de::from_str(include_str!("fixtures/hubs_search_starfall.xml")).unwrap();
    assert_eq!(c.hubs.len(), 17);

    let hits = c.into_search_hits();
    let tracks = hits.iter().filter(|h| h.kind == HitKind::Track).count();
    assert_eq!(tracks, 14);
    assert_eq!(hits.iter().filter(|h| h.kind == HitKind::Album).count(), 2);
    let first_album = hits.iter().position(|h| h.kind == HitKind::Album).unwrap();
    assert_eq!(first_album, tracks);

    let first = &hits[0];
    assert_eq!(first.kind, HitKind::Track);
    assert_eq!(first.title, "Starfall!");
    assert_eq!(first.artist, "Northwind Sounds");
    assert_eq!(first.subtitle, "Evening Satellite");
    assert_eq!(first.duration_ms, Some(257293));
    assert!(first.rating_key.parse::<u64>().is_ok());

    let credited = hits.iter().find(|h| h.rating_key == "20776").expect("track 20776");
    assert_eq!(credited.artist, "Featured Acts");

    let album = &hits[first_album];
    assert_eq!(album.title, "Starfall!");
    assert_eq!(album.artist, "Northwind Sounds");
    assert_eq!(album.subtitle, "2020");
    assert_eq!(album.artist_key.as_deref(), Some("8735"));
    assert_eq!(album.album_key, None);

    let track = hits.iter().find(|h| h.rating_key == "20545").expect("track 20545");
    assert_eq!(track.album_key.as_deref(), Some("20544"));
    assert_eq!(track.artist_key.as_deref(), Some("8735"));
}

/// Entity-encoded Japanese titles and credits decode.
#[test]
fn parse_hubs_search_cjk_fixture() {
    let c: MediaContainer =
        quick_xml::de::from_str(include_str!("fixtures/hubs_search_cjk.xml")).unwrap();
    let hits = c.into_search_hits();
    assert!(hits.iter().any(|h| h.kind == HitKind::Track));
    assert!(hits.iter().any(|h| h.title.contains('ス')), "CJK titles must decode");
    assert!(hits.iter().all(|h| h.kind != HitKind::Track || !h.artist.is_empty()));
}

#[test]
fn unparsable_number_does_not_fail_listing() {
    let xml = r#"<MediaContainer>
  <Track ratingKey="1" key="/k/1" title="good" index="3" duration="1000">
    <Media bitrate="320"><Part id="1" key="/p/1"><Stream streamType="2" samplingRate="44100" bitDepth="16"/></Part></Media>
  </Track>
  <Track ratingKey="2" key="/k/2" title="odd" index="" duration="-1">
    <Media bitrate="99999999999"><Part id="2" key="/p/2" duration="x"><Stream streamType="2" samplingRate="" bitDepth="-3"/></Part></Media>
  </Track>
  <Track ratingKey="3" key="/k/3" title="fine" index="4" duration="2000"/>
  <Directory key="/d/1" year="" leafCount="5" duration="-1"/>
</MediaContainer>"#;
    let c: MediaContainer = quick_xml::de::from_str(xml).expect("a bad number is not fatal");
    let t = &c.tracks;
    assert_eq!((t[0].index, t[0].duration), (Some(3), Some(1000)));
    let stream = &t[0].media[0].parts[0].streams[0];
    assert_eq!((stream.sampling_rate, stream.bit_depth), (Some(44100), Some(16)));
    assert_eq!((t[1].index, t[1].duration), (None, None));
    assert_eq!(t[1].media[0].bitrate, None);
    let odd = &t[1].media[0].parts[0];
    assert_eq!(odd.duration, None);
    assert_eq!(odd.streams[0].sampling_rate, None);
    assert_eq!(odd.streams[0].bit_depth, None);
    assert_eq!((t[2].index, t[2].duration), (Some(4), Some(2000)));
    assert_eq!(c.directories[0].year, None);
}

/// Plex's order is kept, and the match rounds up: 0.0412 is 95.88%, 0.05 exactly 95%.
#[test]
fn parse_nearest_fixture_keeps_order_and_rounds_matches_up() {
    let c: MediaContainer = quick_xml::de::from_str(include_str!("fixtures/nearest.xml")).unwrap();
    let rows = shmc::plex::sonic::similar_from(c);
    let got: Vec<_> = rows.iter().map(|(t, p)| (t.title.as_str(), *p)).collect();
    assert_eq!(
        got,
        [
            ("Lantern Road", Some(96)),
            ("Glass Orchard", Some(95)),
            ("Slow Meridian", Some(90)),
            ("No Distance", None),
        ]
    );
    assert_eq!(rows[1].0.user_rating, Some(8));
    assert_eq!(rows[0].0.grandparent_rating_key.as_deref(), Some("5001"));
}

/// Similar albums keep Plex's order and carry their artist's key and a rounded-up match.
#[test]
fn parse_nearest_albums_fixture() {
    use shmc::plex::HitKind;
    let c: MediaContainer =
        quick_xml::de::from_str(include_str!("fixtures/nearest_albums.xml")).unwrap();
    let rows = shmc::plex::sonic::similar_hits_from(c);
    let got: Vec<_> = rows
        .iter()
        .map(|(h, p)| {
            (
                h.kind,
                h.rating_key.as_str(),
                h.title.as_str(),
                h.artist.as_str(),
                h.artist_key.as_deref(),
                *p,
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (HitKind::Album, "6101", "Salt Flats", "Violet Ferry", Some("5101"), Some(96)),
            (HitKind::Album, "6102", "Night Ledger", "Paper Harbour", Some("5102"), Some(95)),
            (HitKind::Album, "6103", "Low Signal", "Violet Ferry", Some("5101"), Some(90)),
        ]
    );
}

/// Similar artists keep Plex's order; the name is the hit's `artist`.
#[test]
fn parse_nearest_artists_fixture() {
    use shmc::plex::HitKind;
    let c: MediaContainer =
        quick_xml::de::from_str(include_str!("fixtures/nearest_artists.xml")).unwrap();
    let rows = shmc::plex::sonic::similar_hits_from(c);
    let got: Vec<_> =
        rows.iter().map(|(h, p)| (h.kind, h.rating_key.as_str(), h.artist.as_str(), *p)).collect();
    assert_eq!(
        got,
        [
            (HitKind::Artist, "5201", "Violet Ferry", Some(96)),
            (HitKind::Artist, "5202", "Paper Harbour", Some(95)),
            (HitKind::Artist, "5203", "Mira Vale", Some(90)),
        ]
    );
}

/// A distance outside 0..=1 stays a percentage.
#[test]
fn match_percent_is_clamped() {
    assert_eq!(shmc::plex::sonic::match_percent(0.0), 100);
    assert_eq!(shmc::plex::sonic::match_percent(1.0), 0);
    assert_eq!(shmc::plex::sonic::match_percent(-0.2), 100);
    assert_eq!(shmc::plex::sonic::match_percent(1.7), 0);
}

#[test]
fn parse_root_fixture_reads_owner_features() {
    let c: MediaContainer = quick_xml::de::from_str(include_str!("fixtures/root.xml")).unwrap();
    let f = shmc::plex::sonic::Features::from_container(&c);
    assert!(f.sonic_analysis);
    assert!(f.radio);
    assert!(!f.loudness);
}

#[test]
fn parse_artist_station_fixture() {
    let c: MediaContainer =
        quick_xml::de::from_str(include_str!("fixtures/artist_station.xml")).unwrap();
    let s = shmc::plex::radio::station_from(c).unwrap();
    assert_eq!(s.title, "Mira Vale Radio");
    assert_eq!(
        s.key,
        "/library/metadata/31007/station/0b5f3c1e-5a9e-4c52-9d0e-1b9a52c4a7f3?type=10"
    );
}

#[test]
fn parse_play_queue_fixture() {
    let c: MediaContainer =
        quick_xml::de::from_str(include_str!("fixtures/play_queue.xml")).unwrap();
    let (id, rows) = shmc::plex::radio::play_queue_from(c);
    assert_eq!(id.as_deref(), Some("123"));
    let got: Vec<_> = rows.iter().map(|(t, i)| (t.title.as_str(), i.as_str())).collect();
    assert_eq!(
        got,
        [
            ("Song 1", "701"),
            ("Song 2", "702"),
            ("Song 3", "703"),
            ("Song 4", "704"),
            ("Song 5", "705")
        ]
    );
}

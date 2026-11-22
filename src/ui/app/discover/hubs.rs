use super::super::plex_worker::cached_features;
use super::DiscoverMsg;
use crate::plex::sonic::Features;
use crate::plex::PlexClient;
use crate::ui::app::Msg;
use std::sync::Mutex;
use tokio::sync::mpsc::UnboundedSender;

pub(super) async fn load(
    client: &PlexClient,
    section: &str,
    features: &Mutex<Option<Features>>,
    tx: &UnboundedSender<Msg>,
) {
    let radio = !cached_features(client, features).await.is_some_and(|f| !f.radio);
    let msg = match client.discover_hubs(section).await {
        Ok(mut hubs) => {
            if !radio {
                hubs.stations.clear();
            }
            DiscoverMsg::Loaded { hubs, radio }
        }
        Err(e) => {
            tracing::warn!("loading Discover: {e:#}");
            DiscoverMsg::LoadFailed
        }
    };
    let _ = tx.send(Msg::Discover(msg));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::client::testing::serve_full;
    use crate::plex::discover::ShelfItems;
    use tokio::sync::mpsc::unbounded_channel;

    const HUBS: &str = include_str!("../../../../tests/fixtures/hubs_discover.xml");

    async fn loaded(features: &'static str) -> (DiscoverMsg, Vec<String>) {
        let (base, seen, _) = serve_full(move |line| {
            let body = if line.starts_with("GET / ") {
                format!("<MediaContainer ownerFeatures=\"{features}\"/>")
            } else if line.starts_with("GET /hubs/sections/1?") {
                HUBS.into()
            } else {
                return (404, String::new());
            };
            (200, body)
        })
        .await;
        let client = PlexClient::new(&base, "faketoken-hubs");
        let (tx, mut rx) = unbounded_channel();
        load(&client, "1", &Mutex::new(None), &tx).await;
        let Ok(Msg::Discover(msg)) = rx.try_recv() else { panic!("expected a Discover reply") };
        let lines = seen.lock().unwrap().clone();
        (msg, lines)
    }

    #[tokio::test]
    async fn one_request_brings_shelves_and_stations() {
        let (msg, seen) = loaded("radio").await;
        let DiscoverMsg::Loaded { hubs, radio } = msg else { panic!("expected Loaded") };
        assert!(radio);
        assert_eq!(hubs.stations.len(), 4);
        assert_eq!(hubs.shelves.len(), 3);
        assert!(matches!(hubs.shelves[2].items, ShelfItems::Playlists(_)));
        let hub_requests = seen.iter().filter(|l| l.starts_with("GET /hubs/")).count();
        assert_eq!(hub_requests, 1);
        assert!(seen.iter().any(|l| l.contains("includeLibraryPlaylists=1")), "{seen:?}");
    }

    #[tokio::test]
    async fn every_load_asks_plex_with_a_new_url() {
        let (base, seen, _) = serve_full(|line| match line.starts_with("GET /hubs/") {
            true => (200, HUBS.into()),
            false => (200, "<MediaContainer ownerFeatures=\"radio\"/>".into()),
        })
        .await;
        let client = PlexClient::new(&base, "faketoken-hubs");
        let (tx, _rx) = unbounded_channel();
        let features = Mutex::new(None);
        load(&client, "1", &features, &tx).await;
        load(&client, "1", &features, &tx).await;
        let lines = seen.lock().unwrap().clone();
        let hubs: Vec<_> = lines.iter().filter(|l| l.starts_with("GET /hubs/")).collect();
        assert_eq!(hubs.len(), 2);
        assert_ne!(hubs[0], hubs[1], "Plex would answer the same URL from its cache");
    }

    #[tokio::test]
    async fn without_radio_the_shelves_stay_and_the_stations_go() {
        let (msg, _) = loaded("music-analysis").await;
        let DiscoverMsg::Loaded { hubs, radio } = msg else { panic!("expected Loaded") };
        assert!(!radio);
        assert!(hubs.stations.is_empty());
        assert_eq!(hubs.shelves.len(), 3);
    }
}

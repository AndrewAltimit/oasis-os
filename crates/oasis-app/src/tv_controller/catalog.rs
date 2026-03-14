//! TV catalog fetch logic: polling background fetch results and starting new fetches.

use crate::app_state::AppState;

/// Poll pending TV catalog fetch (non-blocking).
pub(super) fn poll_catalog_fetch(state: &mut AppState) {
    let Some(ref rx) = state.pending_tv_catalog_fetch else {
        return;
    };
    match rx.try_recv() {
        Ok(Ok(catalogs)) => {
            let loaded = catalogs.iter().filter(|c| c.is_some()).count();
            let total = catalogs.len();
            log::info!("TV catalog fetch result: {loaded}/{total} channels have episodes");
            state.pending_tv_catalog_fetch = None;
            let runner = super::find_tv_guide_runner(
                &mut state.content.app_runner,
                &mut state.content.open_runners,
            );
            if let Some(runner) = runner {
                if let Some(guide) = runner.tv_guide_state() {
                    guide.fetch_in_progress = false;
                    let all_none = catalogs.iter().all(|c| c.is_none());
                    for (i, cat) in catalogs.into_iter().enumerate() {
                        if let Some(c) = cat
                            && i < guide.catalogs.len()
                        {
                            guide.catalogs[i] = Some(c);
                            guide.rebuild_cached_schedule(i);
                        }
                    }
                    if all_none {
                        log::warn!("TV: all channel catalogs empty");
                        guide.fetch_error = Some("No episodes found for any channel".into());
                    }

                    // Auto-tune if OASIS_TV_CHANNEL is set (for automated testing).
                    if let Ok(ch_str) = std::env::var("OASIS_TV_CHANNEL")
                        && let Ok(ch_num) = ch_str.parse::<u32>()
                    {
                        if let Some(idx) = guide.channels.iter().position(|c| c.number == ch_num) {
                            guide.selected_channel = idx;
                            if let Some(req) = guide.tune() {
                                use oasis_core::apps::tv_guide::TV_REQUEST_PATH;
                                let url = oasis_core::apps::tv_guide::catalog::ChannelCatalog::download_url(&req.episode);
                                let seek_secs = std::env::var("OASIS_TV_SEEK")
                                    .ok()
                                    .and_then(|s| s.parse::<u64>().ok())
                                    .unwrap_or(req.seek_secs);
                                let data = format!("tune_url {url} {seek_secs}");
                                log::info!("TV: auto-tune CH{} -> {}", ch_num, req.episode.title,);
                                runner.set_pending_request(TV_REQUEST_PATH.to_string(), data);
                            } else if let Some(catalog) =
                                guide.catalogs.get(idx).and_then(|c| c.as_ref())
                                && let Some(ep) = catalog.episodes.first()
                            {
                                // Force-tune to first episode (schedule has no slot
                                // at current time, but env var testing needs it).
                                use oasis_core::apps::tv_guide::TV_REQUEST_PATH;
                                let url = oasis_core::apps::tv_guide::catalog::ChannelCatalog::download_url(ep);
                                let seek_secs = std::env::var("OASIS_TV_SEEK")
                                    .ok()
                                    .and_then(|s| s.parse::<u64>().ok())
                                    .unwrap_or(0);
                                let data = format!("tune_url {url} {seek_secs}");
                                log::info!(
                                    "TV: force-tune CH{} -> {} (no schedule slot)",
                                    ch_num,
                                    ep.title,
                                );
                                guide.tuned_channel = Some(idx);
                                runner.set_pending_request(TV_REQUEST_PATH.to_string(), data);
                            } else {
                                log::warn!("TV: auto-tune CH{ch_num} failed (no episodes)");
                            }
                        } else {
                            log::warn!("TV: OASIS_TV_CHANNEL={ch_num} not found in channels");
                        }
                    }
                }
                runner.refresh_tv_text();
            } else {
                log::warn!("TV: catalogs arrived but no TV Guide runner found");
            }
        },
        Ok(Err(e)) => {
            state.pending_tv_catalog_fetch = None;
            log::error!("TV catalog fetch failed: {e}");
            let runner = super::find_tv_guide_runner(
                &mut state.content.app_runner,
                &mut state.content.open_runners,
            );
            if let Some(runner) = runner {
                if let Some(guide) = runner.tv_guide_state() {
                    guide.fetch_in_progress = false;
                    guide.fetch_error = Some(e);
                }
                runner.refresh_tv_text();
            } else {
                log::warn!("TV: error arrived but no TV Guide runner found");
            }
        },
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            state.pending_tv_catalog_fetch = None;
            log::error!("TV catalog fetch thread died");
            let runner = super::find_tv_guide_runner(
                &mut state.content.app_runner,
                &mut state.content.open_runners,
            );
            if let Some(runner) = runner {
                if let Some(guide) = runner.tv_guide_state() {
                    guide.fetch_in_progress = false;
                    guide.fetch_error = Some("catalog fetch failed".into());
                }
                runner.refresh_tv_text();
            }
        },
        Err(std::sync::mpsc::TryRecvError::Empty) => {
            // Timeout after 2 minutes.
            if let Some(start) = state.tv_fetch_start
                && start.elapsed().as_secs() >= 120
            {
                log::warn!("TV: catalog fetch timed out after 120s");
                state.pending_tv_catalog_fetch = None;
                state.tv_fetch_start = None;
                let runner = super::find_tv_guide_runner(
                    &mut state.content.app_runner,
                    &mut state.content.open_runners,
                );
                if let Some(runner) = runner {
                    if let Some(guide) = runner.tv_guide_state() {
                        guide.fetch_in_progress = false;
                        guide.fetch_error = Some("Fetch timed out (2 min)".into());
                    }
                    runner.refresh_tv_text();
                }
            }
        },
    }
}

/// Start TV catalog fetch if a TV Guide app needs it.
pub(super) fn start_catalog_fetch_if_needed(state: &mut AppState) {
    if state.pending_tv_catalog_fetch.is_some() {
        return;
    }
    let runner = super::find_tv_guide_runner(
        &mut state.content.app_runner,
        &mut state.content.open_runners,
    );
    if let Some(runner) = runner
        && let Some(guide) = runner.tv_guide_state()
        && !guide.fetch_attempted
        && guide.catalogs.iter().all(|c| c.is_none())
    {
        log::info!(
            "TV: starting catalog fetch for {} channels",
            guide.channels.len(),
        );
        guide.fetch_attempted = true;
        guide.fetch_in_progress = true;
        let channels = guide.channels.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let tls = state.net.tls_provider.clone();
        std::thread::spawn(move || {
            log::info!("TV: background fetch thread started");
            let result = super::super::fetch_tv_catalogs_blocking(&channels, &tls);
            log::info!(
                "TV: background fetch thread finished (ok={})",
                result.is_ok(),
            );
            let _ = tx.send(result);
        });
        state.pending_tv_catalog_fetch = Some(rx);
        state.tv_fetch_start = Some(std::time::Instant::now());
    }
}

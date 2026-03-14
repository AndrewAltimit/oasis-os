//! SDI view: Radio Stations (list view).

use oasis_core::active_theme::ActiveTheme;
use oasis_core::sdi::SdiRegistry;

use crate::theme::*;
use crate::types::RADIO_STATIONS;

use super::helpers::sdi_key;
use super::list_view::{
    hide_unused_rows, setup_list_bg, update_list_bg, update_list_row, update_scrollbar, LIST_ROWS,
};

/// Create SDI objects for the radio stations list.
pub(crate) fn setup_radio(sdi: &mut SdiRegistry) {
    setup_list_bg(sdi, "radio_");
}

/// Update SDI objects for the radio stations list each frame.
pub(crate) fn update_radio(
    sdi: &mut SdiRegistry,
    selected: usize,
    scroll: usize,
    at: &ActiveTheme,
) {
    let mut buf = String::with_capacity(32);
    let accent = at.app.selected_text;
    update_list_bg(sdi, &mut buf, "radio_", "RADIO", accent, at, None);

    let end = (scroll + LIST_ROWS).min(RADIO_STATIONS.len());
    let visible = end - scroll;

    for row in 0..visible {
        let i = scroll + row;
        let station = &RADIO_STATIONS[i];
        let br_str = format!("{}k", station.bitrate);
        let br_x = 480 - (br_str.len() as i32 * CHAR_W) - 4;
        update_list_row(
            sdi,
            &mut buf,
            "radio_",
            row,
            i == selected,
            "[R]",
            accent,
            station.name,
            at.app.text,
            Some((station.genre, at.app.dim_text, 230)),
            Some((&br_str, at.app.dim_text, br_x)),
            oasis_core::color::with_alpha(accent, 100),
            at,
        );
    }
    hide_unused_rows(sdi, &mut buf, "radio_", visible);
    update_scrollbar(sdi, &mut buf, "radio_", selected, RADIO_STATIONS.len(), at);
}

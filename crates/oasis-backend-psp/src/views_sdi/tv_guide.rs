//! SDI view: TV Guide Channels (list view).
//!
//! The channel list itself is drawn by `desktop::draw_tvguide_windowed`;
//! this only creates the shared list background objects under `tv_`.

use oasis_core::sdi::SdiRegistry;

use super::list_view::setup_list_bg;

pub(crate) fn setup_tv_channels(sdi: &mut SdiRegistry) {
    setup_list_bg(sdi, "tv_");
}

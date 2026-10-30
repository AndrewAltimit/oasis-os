//! Dashboard input: d-pad cursor movement, page switching, pointer hover.

use super::DashboardState;
use super::animation::PageSlideAnim;
use crate::input::Button;

impl DashboardState {
    /// Handle a button press for cursor navigation.
    pub fn handle_input(&mut self, button: &Button) {
        let cols = self.config.grid_cols as usize;
        let page_apps = self.current_page_apps().len();
        if page_apps == 0 {
            return;
        }

        match button {
            Button::Right => {
                self.selected = (self.selected + 1) % page_apps;
            },
            Button::Left => {
                if self.selected == 0 {
                    self.selected = page_apps - 1;
                } else {
                    self.selected -= 1;
                }
            },
            Button::Down => {
                let next = self.selected + cols;
                if next < page_apps {
                    self.selected = next;
                }
            },
            Button::Up if self.selected >= cols => {
                self.selected -= cols;
            },
            _ => {},
        }
    }

    /// Update the pointer-hover icon (`None` = pointer over no icon).
    /// Indices past the current page's apps are treated as `None`.
    /// Returns whether the hover target changed.
    pub fn set_hover(&mut self, index: Option<usize>) -> bool {
        let index = index.filter(|&i| i < self.current_page_apps().len());
        let changed = self.hover_index != index;
        self.hover_index = index;
        changed
    }

    /// Switch to the next page (wraps around) with slide animation.
    pub fn next_page(&mut self) {
        let count = self.page_count();
        self.page = (self.page + 1) % count;
        self.enter_page(-1); // slide left (next)
    }

    /// Switch to the previous page (wraps around) with slide animation.
    pub fn prev_page(&mut self) {
        let count = self.page_count();
        self.page = self.page.checked_sub(1).unwrap_or(count - 1);
        self.enter_page(1); // slide right (prev)
    }

    /// Shared page-change bookkeeping after `page` moved: clamp the
    /// selection to the new page, start the slide (unless the skin fades
    /// pages) and the entrance animation, and drop stale hover state.
    fn enter_page(&mut self, direction: i32) {
        let page_apps = self.current_page_apps().len();
        if self.selected >= page_apps && page_apps > 0 {
            self.selected = page_apps - 1;
        }
        self.page_anim = (self.config.page_style != "fade").then_some(PageSlideAnim {
            frame: 0,
            duration: self.config.page_slide_duration,
            direction,
        });
        self.entrance_elapsed_ms = Some(0);
        self.reset_hover();
    }
}

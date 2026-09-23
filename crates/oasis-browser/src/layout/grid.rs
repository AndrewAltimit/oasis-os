//! CSS Grid layout algorithm.
//!
//! Implements CSS Grid Layout for the OASIS browser engine:
//!
//! - track lists with `px` / `%` / `fr` / `auto` / `min-content` /
//!   `max-content` / `fit-content()` / `minmax()` tracks,
//! - `repeat(auto-fill | auto-fit, ...)` resolved against the container
//!   size (CSS Grid §7.2.3.2), with empty `auto-fit` tracks collapsed,
//! - line-based placement (`1 / 3`, `span 2`, negative lines, named
//!   areas) and the auto-placement algorithm in row or column flow,
//!   sparse or `dense`,
//! - a simplified track sizing algorithm (base sizes, maximize tracks,
//!   expand flexible tracks, stretch `auto` tracks).
//!
//! Subgrid, named lines from `[name]` brackets, and baseline alignment
//! are not supported.

use std::collections::HashMap;
use std::ops::Range;

use super::block::{TextMeasurer, layout_block, resolve_edge_sizes};
use super::box_model::*;
use crate::css::values::{
    BoxSizing, Dimension, GridLine, GridTemplate, GridTrackSize, TrackBreadth,
};

/// Hard cap on the number of tracks in either axis, so hostile CSS
/// (`grid-column: 1 / 100000000`) cannot allocate unbounded grids.
const MAX_TRACKS: usize = 1000;

/// A resolved item placement: `(col, row, col_span, row_span)`, 0-based.
type Placement = (usize, usize, usize, usize);

/// Named-area rectangles from `grid-template-areas`:
/// name -> `(col, row, col_span, row_span)`.
type AreaMap<'a> = HashMap<&'a str, Placement>;

/// Lay out a grid container and all its children.
///
/// The grid container's `content.x`, `content.y`, and `content.width`
/// must already be set by the caller. This function resolves track
/// sizes, places children into grid cells, and positions them.
pub fn layout_grid(container: &mut LayoutBox, _containing_width: f32, measurer: &dyn TextMeasurer) {
    let content_width = container.dimensions.content.width;
    let content_x = container.dimensions.content.x;
    let content_y = container.dimensions.content.y;
    let col_gap = container.style.column_gap;
    let row_gap = container.style.row_gap;
    let definite_height = definite_content_height(container);

    if container.children.is_empty() {
        set_container_height(container, definite_height, 0.0);
        return;
    }

    // -- Phase 1: Resolve each child's edge sizes ------------------------
    for child in &mut container.children {
        resolve_edge_sizes(child, content_width);
    }

    // -- Phase 2: Explicit grid (auto-repeat resolved here) --------------
    let (col_explicit, col_fit) = expand_template(
        &container.style.grid_template_columns,
        Some(content_width),
        col_gap,
    );
    let (row_explicit, row_fit) = expand_template(
        &container.style.grid_template_rows,
        definite_height,
        row_gap,
    );
    let areas = container.style.grid_template_areas.clone();
    let area_map = build_area_map(&areas);
    let area_cols = areas.iter().map(Vec::len).max().unwrap_or(0);
    let explicit_cols = col_explicit.len().max(area_cols);
    let explicit_rows = row_explicit.len().max(areas.len());

    // -- Phase 3: Placement ----------------------------------------------
    let flow_column = container.style.grid_auto_flow_column;
    let dense = container.style.grid_auto_flow_dense;
    let items: Vec<ItemSpec> = container
        .children
        .iter()
        .map(|c| item_spec(&c.style, &area_map, explicit_cols, explicit_rows))
        .collect();
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| container.children[i].style.order);

    let mut placements = if flow_column {
        // Column flow: rows are the fixed axis, columns grow.
        let swapped: Vec<ItemSpec> = items.iter().map(ItemSpec::swapped).collect();
        let placed = auto_place(&swapped, explicit_rows, dense, &order);
        placed
            .into_iter()
            .map(|(r, c, rs, cs)| (c, r, cs, rs))
            .collect::<Vec<_>>()
    } else {
        auto_place(&items, explicit_cols, dense, &order)
    };

    let mut num_cols = placements
        .iter()
        .map(|p| p.0 + p.2)
        .max()
        .unwrap_or(0)
        .max(explicit_cols)
        .max(1);
    let mut num_rows = placements
        .iter()
        .map(|p| p.1 + p.3)
        .max()
        .unwrap_or(0)
        .max(explicit_rows)
        .max(1);

    let auto_cols = container.style.grid_auto_columns.clone();
    let auto_rows = container.style.grid_auto_rows.clone();
    let mut col_tracks = full_track_list(&col_explicit, &auto_cols, num_cols);
    let mut row_tracks = full_track_list(&row_explicit, &auto_rows, num_rows);

    // auto-fit: collapse empty repeated tracks (their gutters go too).
    if let Some(range) = col_fit {
        num_cols = collapse_empty_tracks(&mut col_tracks, &mut placements, range, true);
    }
    if let Some(range) = row_fit {
        num_rows = collapse_empty_tracks(&mut row_tracks, &mut placements, range, false);
    }

    // -- Phase 4: Column sizes -------------------------------------------
    let total_col_gaps = col_gap * (num_cols as f32 - 1.0);
    let available_for_cols = (content_width - total_col_gaps).max(0.0);
    let mut col_content = vec![0.0f32; num_cols];
    if col_tracks.iter().any(|t| is_intrinsic(*t)) {
        // Tentatively lay out each item to measure its width. This is a
        // coarse stand-in for real min/max-content contributions.
        let temp_width = content_width / num_cols as f32;
        for (i, child) in container.children.iter_mut().enumerate() {
            let (col, _, col_span, _) = placements[i];
            if col_span != 1 {
                continue;
            }
            child.dimensions.content.x = 0.0;
            child.dimensions.content.y = 0.0;
            layout_block(child, temp_width, measurer);
            let w = child.dimensions.margin_box().width;
            col_content[col] = col_content[col].max(w);
        }
    }
    let col_widths = resolve_track_sizes(
        &col_tracks,
        &col_content,
        Some(available_for_cols),
        Some(content_width),
    );
    let col_offsets = cumulative_offsets(&col_widths, col_gap);

    // -- Phase 5: Lay out items at their final widths --------------------
    // Items are laid out at their column position (row offset applied
    // afterwards) so descendants land at the right x.
    let mut item_heights = Vec::with_capacity(container.children.len());
    for (i, child) in container.children.iter_mut().enumerate() {
        let (col, _, col_span, _) = placements[i];
        let cell_w = span_size(&col_widths, col, col_span, col_gap);
        // Grid items stretch to fill their cell by default. Override
        // the child's width to auto so that `calculate_block_width`
        // expands it to fill the cell (containing width).
        child.style.width = Dimension::Auto;
        resolve_edge_sizes(child, cell_w);
        child.dimensions.content.x = content_x
            + col_offsets.get(col).copied().unwrap_or(0.0)
            + child.dimensions.margin.left
            + child.dimensions.border.left
            + child.dimensions.padding.left;
        child.dimensions.content.y = content_y
            + child.dimensions.margin.top
            + child.dimensions.border.top
            + child.dimensions.padding.top;
        layout_block(child, cell_w, measurer);
        item_heights.push(child.dimensions.margin_box().height);
    }

    // -- Phase 6: Row sizes ----------------------------------------------
    let mut row_content = vec![0.0f32; num_rows];
    for (i, &(_, row, _, row_span)) in placements.iter().enumerate() {
        if row_span == 1 && row < num_rows {
            row_content[row] = row_content[row].max(item_heights[i]);
        }
    }
    let row_available = definite_height.map(|h| (h - row_gap * (num_rows as f32 - 1.0)).max(0.0));
    let mut row_heights =
        resolve_track_sizes(&row_tracks, &row_content, row_available, definite_height);
    grow_rows_for_spanning_items(
        &mut row_heights,
        &row_tracks,
        &placements,
        &item_heights,
        row_gap,
    );
    let row_offsets = cumulative_offsets(&row_heights, row_gap);

    // -- Phase 7: Move items down to their rows --------------------------
    for (i, child) in container.children.iter_mut().enumerate() {
        let row = placements[i].1;
        let dy = row_offsets.get(row).copied().unwrap_or(0.0);
        if dy != 0.0 {
            shift_subtree(child, 0.0, dy);
        }
    }

    // -- Phase 8: Container height = sum of row heights + gaps -----------
    let total_height = row_heights.iter().sum::<f32>() + row_gap * (num_rows as f32 - 1.0);
    set_container_height(container, definite_height, total_height);
}

// -------------------------------------------------------------------
// Container sizing helpers
// -------------------------------------------------------------------

/// The container's definite content-box height, if `height` is a length.
fn definite_content_height(container: &LayoutBox) -> Option<f32> {
    match container.style.height {
        Dimension::Px(h) => {
            let d = &container.dimensions;
            Some(if container.style.box_sizing == BoxSizing::BorderBox {
                (h - d.padding.vertical() - d.border.vertical()).max(0.0)
            } else {
                h
            })
        },
        _ => None,
    }
}

fn set_container_height(container: &mut LayoutBox, definite: Option<f32>, content: f32) {
    container.dimensions.content.height = definite.unwrap_or(content.max(0.0));
}

/// Offset a box and all its descendants.
fn shift_subtree(b: &mut LayoutBox, dx: f32, dy: f32) {
    b.dimensions.content.x += dx;
    b.dimensions.content.y += dy;
    for child in &mut b.children {
        shift_subtree(child, dx, dy);
    }
}

// -------------------------------------------------------------------
// Explicit grid
// -------------------------------------------------------------------

/// Size of a track for the auto-repeat count computation: its max
/// sizing function if definite, else its min if definite (§7.2.3.2).
fn definite_track_size(track: GridTrackSize, basis: Option<f32>) -> Option<f32> {
    let breadth = |b: TrackBreadth| match b {
        TrackBreadth::Px(p) => Some(p),
        TrackBreadth::Percent(p) => basis.map(|w| w * p / 100.0),
        _ => None,
    };
    match track {
        GridTrackSize::Px(p) => Some(p),
        GridTrackSize::Percent(p) => basis.map(|w| w * p / 100.0),
        GridTrackSize::Minmax(min, max) => match (breadth(min), breadth(max)) {
            (Some(mn), Some(mx)) => Some(mx.max(mn)),
            (None, Some(mx)) => Some(mx),
            (Some(mn), None) => Some(mn),
            (None, None) => None,
        },
        _ => None,
    }
}

/// Number of repetitions for `repeat(auto-fill | auto-fit, ...)`: the
/// largest count whose tracks and gaps fit in `available`, at least 1.
fn auto_repeat_count(
    available: Option<f32>,
    fixed: &[GridTrackSize],
    repeated: &[GridTrackSize],
    gap: f32,
) -> usize {
    let Some(avail) = available else {
        return 1;
    };
    let mut rep_sum = 0.0;
    for t in repeated {
        match definite_track_size(*t, Some(avail)) {
            Some(s) => rep_sum += s,
            None => return 1,
        }
    }
    let other: f32 = fixed
        .iter()
        .map(|t| definite_track_size(*t, Some(avail)).unwrap_or(0.0))
        .sum();
    let per_repetition = rep_sum + gap * repeated.len() as f32;
    if per_repetition <= 0.0 {
        return 1;
    }
    // total = other + n*rep_sum + gap*(fixed + n*len - 1) <= avail
    let free = avail + gap - other - gap * fixed.len() as f32;
    let n = (free / per_repetition + 1e-4).floor();
    let cap = (MAX_TRACKS / repeated.len().max(1)).max(1);
    if n.is_finite() && n >= 1.0 {
        (n as usize).min(cap)
    } else {
        1
    }
}

/// Expand a template's auto-repeat block. Returns the explicit track
/// list and, for `auto-fit`, the range of repeated tracks that may be
/// collapsed when empty.
fn expand_template(
    template: &GridTemplate,
    available: Option<f32>,
    gap: f32,
) -> (Vec<GridTrackSize>, Option<Range<usize>>) {
    let Some(rep) = &template.auto_repeat else {
        return (template.tracks.clone(), None);
    };
    let count = auto_repeat_count(available, &template.tracks, &rep.tracks, gap);
    let at = rep.insert_at.min(template.tracks.len());
    let mut out = Vec::with_capacity(template.tracks.len() + count * rep.tracks.len());
    out.extend_from_slice(&template.tracks[..at]);
    for _ in 0..count {
        out.extend_from_slice(&rep.tracks);
    }
    let end = out.len();
    out.extend_from_slice(&template.tracks[at..]);
    (out, rep.fit.then_some(at..end))
}

/// Explicit tracks followed by implicit tracks (cycling through
/// `grid-auto-*`, default `auto`) up to `count`.
fn full_track_list(
    explicit: &[GridTrackSize],
    auto_tracks: &[GridTrackSize],
    count: usize,
) -> Vec<GridTrackSize> {
    (0..count)
        .map(|i| {
            if i < explicit.len() {
                explicit[i]
            } else if !auto_tracks.is_empty() {
                auto_tracks[(i - explicit.len()) % auto_tracks.len()]
            } else {
                GridTrackSize::Auto
            }
        })
        .collect()
}

/// Remove `auto-fit` tracks in `range` that no item occupies and remap
/// placements. Returns the new track count.
fn collapse_empty_tracks(
    tracks: &mut Vec<GridTrackSize>,
    placements: &mut [Placement],
    range: Range<usize>,
    columns: bool,
) -> usize {
    let axis = |p: &Placement| if columns { (p.0, p.2) } else { (p.1, p.3) };
    let mut used = vec![false; tracks.len()];
    for p in placements.iter() {
        let (start, span) = axis(p);
        for u in used.iter_mut().skip(start).take(span) {
            *u = true;
        }
    }
    let collapsed: Vec<bool> = (0..tracks.len())
        .map(|i| range.contains(&i) && !used[i])
        .collect();
    if !collapsed.iter().any(|&c| c) {
        return tracks.len();
    }
    // new_index[i] = number of surviving tracks before i.
    let mut new_index = Vec::with_capacity(tracks.len() + 1);
    let mut n = 0;
    for &c in &collapsed {
        new_index.push(n);
        if !c {
            n += 1;
        }
    }
    for p in placements.iter_mut() {
        if columns {
            p.0 = new_index[p.0];
        } else {
            p.1 = new_index[p.1];
        }
    }
    let mut i = 0;
    tracks.retain(|_| {
        let keep = !collapsed[i];
        i += 1;
        keep
    });
    tracks.len().max(1)
}

// -------------------------------------------------------------------
// Placement
// -------------------------------------------------------------------

/// Per-axis placement request: a definite start track (0-based) or
/// `None` for auto, plus a span. `f` is the fixed axis of the flow
/// (columns in row flow), `g` the growing one.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ItemSpec {
    f: Option<usize>,
    fs: usize,
    g: Option<usize>,
    gs: usize,
}

impl ItemSpec {
    fn swapped(&self) -> Self {
        Self {
            f: self.g,
            fs: self.gs,
            g: self.f,
            gs: self.fs,
        }
    }
}

/// Convert a numeric line to a 0-based line index. Negative lines count
/// back from the end of the explicit grid (`-1` = last explicit line).
/// Lines before the start of the grid clamp to 0 (no negative implicit
/// tracks).
fn line_index(n: i32, explicit: usize) -> usize {
    let idx = if n > 0 {
        n as i64 - 1
    } else {
        explicit as i64 + 1 + n as i64
    };
    (idx.max(0) as usize).min(MAX_TRACKS)
}

/// Resolve a named line against the area map: `name` / `name-start`
/// for a start line, `name` / `name-end` for an end line.
fn named_line(name: &str, is_end: bool, areas: &AreaMap<'_>, columns: bool) -> Option<usize> {
    let suffix = if is_end { "-end" } else { "-start" };
    let area_name = name.strip_suffix(suffix).unwrap_or(name);
    let &(c, r, cs, rs) = areas.get(area_name)?;
    Some(match (columns, is_end) {
        (true, false) => c,
        (true, true) => c + cs,
        (false, false) => r,
        (false, true) => r + rs,
    })
}

fn definite_line(
    line: &GridLine,
    is_end: bool,
    explicit: usize,
    areas: &AreaMap<'_>,
    columns: bool,
) -> Option<usize> {
    match line {
        GridLine::Line(n) => Some(line_index(*n, explicit)),
        GridLine::Named(name) => named_line(name, is_end, areas, columns),
        GridLine::Auto | GridLine::Span(_) => None,
    }
}

fn span_of(line: &GridLine) -> Option<usize> {
    match line {
        GridLine::Span(n) => Some((*n as usize).clamp(1, MAX_TRACKS)),
        _ => None,
    }
}

/// Resolve one axis of an item's placement (CSS Grid §8.3.1).
fn resolve_axis(
    start: &GridLine,
    end: &GridLine,
    explicit: usize,
    areas: &AreaMap<'_>,
    columns: bool,
) -> (Option<usize>, usize) {
    let s = definite_line(start, false, explicit, areas, columns);
    let e = definite_line(end, true, explicit, areas, columns);
    match (s, e) {
        (Some(s), Some(e)) if e > s => (Some(s), e - s),
        (Some(s), Some(e)) if e < s => (Some(e), s - e),
        (Some(s), Some(_)) => (Some(s), 1),
        (Some(s), None) => (Some(s), span_of(end).unwrap_or(1)),
        (None, Some(e)) => {
            let span = span_of(start).unwrap_or(1);
            let s = e.saturating_sub(span);
            (Some(s), (e - s).max(1))
        },
        (None, None) => (None, span_of(start).or(span_of(end)).unwrap_or(1)),
    }
}

/// Build an item's placement request (row flow orientation).
fn item_spec(
    style: &crate::css::values::ComputedStyle,
    areas: &AreaMap<'_>,
    explicit_cols: usize,
    explicit_rows: usize,
) -> ItemSpec {
    if let Some(&(c, r, cs, rs)) = style.grid_area.as_deref().and_then(|n| areas.get(n)) {
        return ItemSpec {
            f: Some(c),
            fs: cs,
            g: Some(r),
            gs: rs,
        };
    }
    let (f, fs) = resolve_axis(
        &style.grid_column_start,
        &style.grid_column_end,
        explicit_cols,
        areas,
        true,
    );
    let (g, gs) = resolve_axis(
        &style.grid_row_start,
        &style.grid_row_end,
        explicit_rows,
        areas,
        false,
    );
    ItemSpec { f, fs, g, gs }
}

/// Occupancy matrix indexed `[g][f]`, growing along `g` on demand.
struct Occupancy {
    cells: Vec<Vec<bool>>,
    nf: usize,
}

impl Occupancy {
    fn fits(&self, f: usize, fs: usize, g: usize, gs: usize) -> bool {
        if f + fs > self.nf {
            return false;
        }
        (g..g + gs).all(|gg| {
            self.cells
                .get(gg)
                .is_none_or(|row| row[f..f + fs].iter().all(|&o| !o))
        })
    }

    fn mark(&mut self, f: usize, fs: usize, g: usize, gs: usize) {
        let g_end = (g + gs).min(MAX_TRACKS);
        if self.cells.len() < g_end {
            self.cells.resize(g_end, vec![false; self.nf]);
        }
        for row in &mut self.cells[g.min(g_end)..g_end] {
            let f_end = (f + fs).min(self.nf);
            for cell in &mut row[f.min(f_end)..f_end] {
                *cell = true;
            }
        }
    }
}

/// The CSS Grid auto-placement algorithm (§8.5), written for row flow:
/// `f` is the fixed axis (columns), `g` grows (rows). Returns
/// `(f, g, f_span, g_span)` per item, in item order.
fn auto_place(
    items: &[ItemSpec],
    explicit_f: usize,
    dense: bool,
    order: &[usize],
) -> Vec<Placement> {
    // The fixed axis is wide enough for every definite item and span.
    let nf = items
        .iter()
        .map(|it| it.f.map_or(it.fs, |f| f + it.fs))
        .max()
        .unwrap_or(1)
        .max(explicit_f)
        .clamp(1, MAX_TRACKS);
    let mut occ = Occupancy {
        cells: Vec::new(),
        nf,
    };
    let mut out: Vec<Option<Placement>> = vec![None; items.len()];

    // 1. Items with a definite position in both axes.
    for &i in order {
        let it = items[i];
        if let (Some(f), Some(g)) = (it.f, it.g) {
            occ.mark(f, it.fs, g, it.gs);
            out[i] = Some((f, g, it.fs, it.gs));
        }
    }

    // 2. Items locked to a `g` line (definite g, auto f).
    let mut line_cursor: HashMap<usize, usize> = HashMap::new();
    for &i in order {
        let it = items[i];
        let (None, Some(g)) = (it.f, it.g) else {
            continue;
        };
        let fs = it.fs.min(nf);
        let from = if dense {
            0
        } else {
            line_cursor.get(&g).copied().unwrap_or(0)
        };
        let f = (from..=nf - fs)
            .find(|&f| occ.fits(f, fs, g, it.gs))
            .unwrap_or(0);
        occ.mark(f, fs, g, it.gs);
        line_cursor.insert(g, f + fs);
        out[i] = Some((f, g, fs, it.gs));
    }

    // 3. Everything else, in order, with the auto-placement cursor.
    let (mut cur_g, mut cur_f) = (0usize, 0usize);
    for &i in order {
        if out[i].is_some() {
            continue;
        }
        let it = items[i];
        let fs = it.fs.min(nf);
        if dense {
            cur_g = 0;
            cur_f = 0;
        }
        let placed = if let Some(f) = it.f {
            // Definite f, auto g.
            if !dense && f < cur_f {
                cur_g += 1;
            }
            let mut g = cur_g;
            while !occ.fits(f, fs, g, it.gs) && g < MAX_TRACKS {
                g += 1;
            }
            cur_f = f;
            (f, g)
        } else {
            // Fully automatic.
            let mut g = cur_g;
            let mut f = cur_f;
            loop {
                if f + fs > nf {
                    g += 1;
                    f = 0;
                    if g >= MAX_TRACKS {
                        break;
                    }
                    continue;
                }
                if occ.fits(f, fs, g, it.gs) {
                    break;
                }
                f += 1;
            }
            cur_f = f;
            (f, g)
        };
        cur_g = placed.1;
        occ.mark(placed.0, fs, placed.1, it.gs);
        out[i] = Some((placed.0, placed.1, fs, it.gs));
    }

    out.into_iter().map(|p| p.unwrap_or((0, 0, 1, 1))).collect()
}

// -------------------------------------------------------------------
// Track sizing
// -------------------------------------------------------------------

/// True when a track's size depends on item content.
fn is_intrinsic(track: GridTrackSize) -> bool {
    let b = |x: TrackBreadth| {
        matches!(
            x,
            TrackBreadth::Auto | TrackBreadth::MinContent | TrackBreadth::MaxContent
        )
    };
    match track {
        GridTrackSize::Auto
        | GridTrackSize::MinContent
        | GridTrackSize::MaxContent
        | GridTrackSize::FitContent(_) => true,
        GridTrackSize::Minmax(min, max) => b(min) || b(max),
        GridTrackSize::Px(_) | GridTrackSize::Percent(_) | GridTrackSize::Fr(_) => false,
    }
}

/// Resolve track sizes for one axis.
///
/// `content[i]` is the largest content contribution of the non-spanning
/// items in track `i`. `available` is the space for tracks (container
/// size minus gutters) when definite; `basis` is what percentages
/// resolve against. Follows the shape of CSS Grid §11: initialise base
/// sizes and growth limits, maximize tracks, expand flexible tracks,
/// then stretch `auto` tracks.
fn resolve_track_sizes(
    tracks: &[GridTrackSize],
    content: &[f32],
    available: Option<f32>,
    basis: Option<f32>,
) -> Vec<f32> {
    let n = tracks.len();
    let mut base = vec![0.0f32; n];
    let mut limit = vec![0.0f32; n];
    let mut flex = vec![0.0f32; n];
    let mut stretchy = vec![false; n];
    let pct = |p: f32| basis.map(|b| b * p / 100.0);

    for i in 0..n {
        let c = content.get(i).copied().unwrap_or(0.0);
        let (b, l, f, s) = match tracks[i] {
            GridTrackSize::Px(p) => (p, p, 0.0, false),
            GridTrackSize::Percent(p) => match pct(p) {
                Some(v) => (v, v, 0.0, false),
                None => (c, c, 0.0, true),
            },
            GridTrackSize::Auto => (c, c, 0.0, true),
            GridTrackSize::MinContent | GridTrackSize::MaxContent => (c, c, 0.0, false),
            GridTrackSize::FitContent(max) => {
                let v = c.min(max);
                (v, v, 0.0, false)
            },
            GridTrackSize::Fr(f) => (0.0, 0.0, f, false),
            GridTrackSize::Minmax(min, max) => {
                let b = match min {
                    TrackBreadth::Px(p) => p,
                    TrackBreadth::Percent(p) => pct(p).unwrap_or(c),
                    _ => c,
                };
                let (l, f, s) = match max {
                    TrackBreadth::Px(p) => (p, 0.0, false),
                    TrackBreadth::Percent(p) => match pct(p) {
                        Some(v) => (v, 0.0, false),
                        None => (c, 0.0, true),
                    },
                    TrackBreadth::Fr(f) => (b, f, false),
                    TrackBreadth::Auto => (b.max(c), 0.0, true),
                    TrackBreadth::MinContent | TrackBreadth::MaxContent => (b.max(c), 0.0, false),
                };
                let l = l.max(b);
                // Grow a content-sized track toward its content, capped
                // at the growth limit (items should fit when they can).
                let b = if f > 0.0 { b } else { b.max(c.min(l)) };
                (b, l, f, s)
            },
        };
        base[i] = b.max(0.0);
        limit[i] = l.max(base[i]);
        flex[i] = f;
        stretchy[i] = s;
    }

    let Some(avail) = available else {
        // Indefinite free space: flexible tracks size to their content.
        for i in 0..n {
            if flex[i] > 0.0 {
                base[i] = base[i].max(content.get(i).copied().unwrap_or(0.0));
            }
        }
        return base;
    };

    // Maximize tracks: share free space among non-flexible tracks that
    // are below their growth limit.
    let mut free = avail - base.iter().sum::<f32>();
    while free > 0.01 {
        let growable: Vec<usize> = (0..n)
            .filter(|&i| flex[i] == 0.0 && limit[i] > base[i] + 0.01)
            .collect();
        if growable.is_empty() {
            break;
        }
        let share = free / growable.len() as f32;
        for i in growable {
            let g = share.min(limit[i] - base[i]);
            base[i] += g;
            free -= g;
        }
    }

    if flex.iter().any(|&f| f > 0.0) {
        // Expand flexible tracks: find the size of 1fr, freezing tracks
        // whose base size exceeds their flexible share (§12.7.1).
        let mut frozen = vec![false; n];
        loop {
            let used: f32 = (0..n)
                .filter(|&i| flex[i] == 0.0 || frozen[i])
                .map(|i| base[i])
                .sum();
            let flex_sum: f32 = (0..n)
                .filter(|&i| flex[i] > 0.0 && !frozen[i])
                .map(|i| flex[i])
                .sum();
            if flex_sum <= 0.0 {
                break;
            }
            let fr = (avail - used).max(0.0) / flex_sum.max(1.0);
            let mut changed = false;
            for i in 0..n {
                if flex[i] > 0.0 && !frozen[i] && fr * flex[i] < base[i] {
                    frozen[i] = true;
                    changed = true;
                }
            }
            if !changed {
                for i in 0..n {
                    if flex[i] > 0.0 && !frozen[i] {
                        base[i] = fr * flex[i];
                    }
                }
                break;
            }
        }
    } else {
        // Stretch `auto` tracks into any remaining space.
        let free = avail - base.iter().sum::<f32>();
        let count = stretchy.iter().filter(|&&s| s).count();
        if free > 0.0 && count > 0 {
            let share = free / count as f32;
            for i in 0..n {
                if stretchy[i] {
                    base[i] += share;
                }
            }
        }
    }

    base
}

/// Make rows tall enough for items spanning several rows, spreading any
/// deficit over the spanned content-sized rows (or all spanned rows when
/// none are content-sized).
fn grow_rows_for_spanning_items(
    rows: &mut [f32],
    tracks: &[GridTrackSize],
    placements: &[Placement],
    heights: &[f32],
    gap: f32,
) {
    for (i, &(_, row, _, span)) in placements.iter().enumerate() {
        if span < 2 || row >= rows.len() {
            continue;
        }
        let end = (row + span).min(rows.len());
        let have = span_size(rows, row, end - row, gap);
        let deficit = heights[i] - have;
        if deficit <= 0.0 {
            continue;
        }
        let mut targets: Vec<usize> = (row..end)
            .filter(|&r| tracks.get(r).is_some_and(|t| is_intrinsic(*t)))
            .collect();
        if targets.is_empty() {
            targets = (row..end).collect();
        }
        let share = deficit / targets.len() as f32;
        for r in targets {
            rows[r] += share;
        }
    }
}

// -------------------------------------------------------------------
// Internal helpers
// -------------------------------------------------------------------

/// Total size of `span` tracks starting at `start`, including the gaps
/// between them.
fn span_size(sizes: &[f32], start: usize, span: usize, gap: f32) -> f32 {
    let end = (start + span).min(sizes.len());
    if start >= end {
        return 0.0;
    }
    sizes[start..end].iter().sum::<f32>() + gap * (end - start - 1) as f32
}

/// Build a map from area name to `(col, row, col_span, row_span)` from
/// `grid-template-areas`. Dots (`.`) represent unnamed cells and are skipped.
///
/// Example:
/// ```text
/// "header header"
/// "sidebar main"
/// ```
/// Produces: `{ "header": (0, 0, 2, 1), "sidebar": (0, 1, 1, 1), "main": (1, 1, 1, 1) }`
fn build_area_map(areas: &[Vec<String>]) -> AreaMap<'_> {
    let mut map = HashMap::new();
    for (row, cells) in areas.iter().enumerate() {
        for (col, name) in cells.iter().enumerate() {
            if name.starts_with('.') || name.is_empty() {
                continue;
            }
            map.entry(name.as_str())
                .and_modify(|&mut (start_col, start_row, ref mut cs, ref mut rs)| {
                    // Extend the area to cover this cell.
                    let end_col = (col + 1).max(start_col + *cs);
                    let end_row = (row + 1).max(start_row + *rs);
                    *cs = end_col - start_col;
                    *rs = end_row - start_row;
                })
                .or_insert((col, row, 1, 1));
        }
    }
    map
}

/// Calculate cumulative offsets from track sizes and gap.
fn cumulative_offsets(sizes: &[f32], gap: f32) -> Vec<f32> {
    let mut offsets = Vec::with_capacity(sizes.len());
    let mut offset: f32 = 0.0;
    for &size in sizes {
        offsets.push(offset);
        offset += size + gap;
    }
    offsets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::values::{ComputedStyle, Display, GridTrackSize};
    use crate::layout::block::TextMeasurer;

    struct FixedMeasurer;

    impl TextMeasurer for FixedMeasurer {
        fn measure_text(&self, text: &str, font_size: u16) -> u32 {
            oasis_types::backend::bitmap_measure_text(text, font_size)
        }
    }

    fn grid_style() -> ComputedStyle {
        ComputedStyle {
            display: Display::Grid,
            ..Default::default()
        }
    }

    fn item_style(width: f32, height: f32) -> ComputedStyle {
        ComputedStyle {
            display: Display::Block,
            width: Dimension::Px(width),
            height: Dimension::Px(height),
            ..Default::default()
        }
    }

    fn make_grid_container(style: ComputedStyle, children: Vec<LayoutBox>) -> LayoutBox {
        let mut lb = LayoutBox::new(BoxType::Grid, style, None);
        lb.children = children;
        lb.dimensions.content.x = 0.0;
        lb.dimensions.content.y = 0.0;
        lb.dimensions.content.width = 480.0;
        lb
    }

    fn make_item(width: f32, height: f32) -> LayoutBox {
        LayoutBox::new(BoxType::Block, item_style(width, height), None)
    }

    // -- Basic 2-column grid ---------------------------------------------

    #[test]
    fn grid_two_columns_px() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns =
            vec![GridTrackSize::Px(200.0), GridTrackSize::Px(200.0)].into();
        let mut container = make_grid_container(
            style,
            vec![
                make_item(100.0, 30.0),
                make_item(100.0, 30.0),
                make_item(100.0, 40.0),
                make_item(100.0, 40.0),
            ],
        );
        layout_grid(&mut container, 480.0, &m);

        // First row: items at col 0 and col 1.
        let x0 = container.children[0].dimensions.content.x;
        let x1 = container.children[1].dimensions.content.x;
        assert!(
            x0 < x1,
            "col 0 item should be left of col 1: x0={x0}, x1={x1}",
        );

        // Second row: items below first row.
        let y0 = container.children[0].dimensions.content.y;
        let y2 = container.children[2].dimensions.content.y;
        assert!(y2 > y0, "row 1 should be below row 0: y0={y0}, y2={y2}",);
    }

    // -- Fr units --------------------------------------------------------

    #[test]
    fn grid_fr_units_distribute_space() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns = vec![GridTrackSize::Fr(1.0), GridTrackSize::Fr(1.0)].into();
        let mut container =
            make_grid_container(style, vec![make_item(50.0, 30.0), make_item(50.0, 30.0)]);
        layout_grid(&mut container, 480.0, &m);

        // Each column should be 240px (480 / 2).
        let w0 = container.children[0].dimensions.content.width;
        let w1 = container.children[1].dimensions.content.width;
        assert!((w0 - 240.0).abs() < 2.0, "col 0 should be ~240px: got {w0}",);
        assert!((w1 - 240.0).abs() < 2.0, "col 1 should be ~240px: got {w1}",);
    }

    #[test]
    fn grid_mixed_px_and_fr() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns = vec![GridTrackSize::Px(100.0), GridTrackSize::Fr(1.0)].into();
        let mut container =
            make_grid_container(style, vec![make_item(50.0, 30.0), make_item(50.0, 30.0)]);
        layout_grid(&mut container, 480.0, &m);

        // Col 0 = 100px fixed, Col 1 = 380px (remaining).
        let w0 = container.children[0].dimensions.content.width;
        let w1 = container.children[1].dimensions.content.width;
        assert!((w0 - 100.0).abs() < 2.0, "col 0 should be ~100px: got {w0}",);
        assert!((w1 - 380.0).abs() < 2.0, "col 1 should be ~380px: got {w1}",);
    }

    // -- Gap -------------------------------------------------------------

    #[test]
    fn grid_gap_adds_spacing() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns = vec![GridTrackSize::Fr(1.0), GridTrackSize::Fr(1.0)].into();
        style.column_gap = 10.0;
        style.row_gap = 10.0;
        let mut container =
            make_grid_container(style, vec![make_item(50.0, 30.0), make_item(50.0, 30.0)]);
        layout_grid(&mut container, 480.0, &m);

        // Available = 480 - 10 (gap) = 470. Each col = 235.
        let w0 = container.children[0].dimensions.content.width;
        assert!(
            (w0 - 235.0).abs() < 2.0,
            "col width with gap should be ~235px: got {w0}",
        );

        // Second item should be offset by col_width + gap.
        let x0 = container.children[0].dimensions.content.x;
        let x1 = container.children[1].dimensions.content.x;
        let spacing = x1 - x0;
        assert!(
            (spacing - 245.0).abs() < 2.0,
            "spacing between items should be ~245px: got {spacing}",
        );
    }

    // -- Row gap ---------------------------------------------------------

    #[test]
    fn grid_row_gap() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns = vec![GridTrackSize::Fr(1.0)].into();
        style.column_gap = 10.0;
        style.row_gap = 10.0;
        let mut container =
            make_grid_container(style, vec![make_item(100.0, 30.0), make_item(100.0, 30.0)]);
        layout_grid(&mut container, 480.0, &m);

        let y0 = container.children[0].dimensions.content.y;
        let y1 = container.children[1].dimensions.content.y;
        let row_spacing = y1 - y0;
        // Should be item_height + gap = 30 + 10 = 40.
        assert!(
            (row_spacing - 40.0).abs() < 2.0,
            "row spacing should be ~40px: got {row_spacing}",
        );
    }

    // -- Empty container -------------------------------------------------

    #[test]
    fn grid_empty_container() {
        let m = FixedMeasurer;
        let mut container = make_grid_container(grid_style(), vec![]);
        layout_grid(&mut container, 480.0, &m);
        assert!(
            container.dimensions.content.height.abs() < 1.0,
            "empty grid height should be ~0",
        );
    }

    // -- Auto-placement with more items than columns --------------------

    #[test]
    fn grid_auto_placement_wraps_rows() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns = vec![
            GridTrackSize::Fr(1.0),
            GridTrackSize::Fr(1.0),
            GridTrackSize::Fr(1.0),
        ]
        .into();
        let mut container = make_grid_container(
            style,
            vec![
                make_item(50.0, 30.0),
                make_item(50.0, 30.0),
                make_item(50.0, 30.0),
                make_item(50.0, 30.0), // wraps to row 1
            ],
        );
        layout_grid(&mut container, 480.0, &m);

        // Item 3 (index 3) should be in row 1, col 0.
        let y0 = container.children[0].dimensions.content.y;
        let y3 = container.children[3].dimensions.content.y;
        assert!(y3 > y0, "item 3 should wrap to next row: y0={y0}, y3={y3}",);

        let x3 = container.children[3].dimensions.content.x;
        let x0 = container.children[0].dimensions.content.x;
        assert!(
            (x3 - x0).abs() < 2.0,
            "item 3 should be in col 0: x0={x0}, x3={x3}",
        );
    }

    // -- Container height calculation ------------------------------------

    #[test]
    fn grid_container_height_matches_rows() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns = vec![GridTrackSize::Fr(1.0), GridTrackSize::Fr(1.0)].into();
        style.column_gap = 5.0;
        style.row_gap = 5.0;
        let mut container = make_grid_container(
            style,
            vec![
                make_item(50.0, 30.0),
                make_item(50.0, 30.0),
                make_item(50.0, 40.0),
                make_item(50.0, 40.0),
            ],
        );
        layout_grid(&mut container, 480.0, &m);

        // 2 rows: 30 + 5 (gap) + 40 = 75, plus padding (0).
        let h = container.dimensions.content.height;
        assert!(
            (h - 75.0).abs() < 2.0,
            "container height should be ~75px: got {h}",
        );
    }

    // -- Helper tests ----------------------------------------------------

    #[test]
    fn cumulative_offsets_basic() {
        let sizes = [100.0, 200.0, 150.0];
        let offsets = cumulative_offsets(&sizes, 10.0);
        assert_eq!(offsets.len(), 3);
        assert!((offsets[0]).abs() < f32::EPSILON);
        assert!((offsets[1] - 110.0).abs() < f32::EPSILON);
        assert!((offsets[2] - 320.0).abs() < f32::EPSILON);
    }

    #[test]
    fn resolve_track_sizes_all_fr() {
        let templates = vec![
            GridTrackSize::Fr(1.0),
            GridTrackSize::Fr(2.0),
            GridTrackSize::Fr(1.0),
        ];
        let child_sizes = vec![50.0, 50.0, 50.0];
        let sizes = resolve_track_sizes(&templates, &child_sizes, Some(400.0), Some(400.0));
        assert!((sizes[0] - 100.0).abs() < 0.1);
        assert!((sizes[1] - 200.0).abs() < 0.1);
        assert!((sizes[2] - 100.0).abs() < 0.1);
    }

    #[test]
    fn resolve_track_sizes_mixed() {
        let templates = vec![GridTrackSize::Px(100.0), GridTrackSize::Fr(1.0)];
        let child_sizes = vec![50.0, 50.0];
        let sizes = resolve_track_sizes(&templates, &child_sizes, Some(400.0), Some(400.0));
        assert!((sizes[0] - 100.0).abs() < 0.1);
        assert!((sizes[1] - 300.0).abs() < 0.1);
    }

    // -- Parse grid template tests (via ComputedStyle) -------------------

    #[test]
    fn parse_grid_template_fr_units() {
        use crate::css::parser::CssValue;
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "grid-template-columns",
            &CssValue::Keyword("1fr 2fr 1fr".into()),
            8.0,
        );
        assert_eq!(s.grid_template_columns.len(), 3);
        assert_eq!(s.grid_template_columns[0], GridTrackSize::Fr(1.0));
        assert_eq!(s.grid_template_columns[1], GridTrackSize::Fr(2.0));
        assert_eq!(s.grid_template_columns[2], GridTrackSize::Fr(1.0));
    }

    #[test]
    fn parse_grid_template_repeat() {
        use crate::css::parser::CssValue;
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "grid-template-columns",
            &CssValue::Keyword("repeat(3, 1fr)".into()),
            8.0,
        );
        assert_eq!(s.grid_template_columns.len(), 3);
        for track in s.grid_template_columns.iter() {
            assert_eq!(*track, GridTrackSize::Fr(1.0));
        }
    }

    #[test]
    fn parse_grid_template_mixed() {
        use crate::css::parser::CssValue;
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "grid-template-columns",
            &CssValue::Keyword("100px 1fr auto".into()),
            8.0,
        );
        assert_eq!(s.grid_template_columns.len(), 3);
        assert_eq!(s.grid_template_columns[0], GridTrackSize::Px(100.0),);
        assert_eq!(s.grid_template_columns[1], GridTrackSize::Fr(1.0));
        assert_eq!(s.grid_template_columns[2], GridTrackSize::Auto);
    }

    // -- Auto-repeat -----------------------------------------------------

    fn minmax_px_fr(px: f32) -> GridTrackSize {
        GridTrackSize::Minmax(TrackBreadth::Px(px), TrackBreadth::Fr(1.0))
    }

    fn auto_repeat_template(min_px: f32, fit: bool) -> GridTemplate {
        GridTemplate {
            tracks: Vec::new(),
            auto_repeat: Some(crate::css::values::GridAutoRepeat {
                insert_at: 0,
                tracks: vec![minmax_px_fr(min_px)],
                fit,
            }),
        }
    }

    fn xs(container: &LayoutBox) -> Vec<f32> {
        container
            .children
            .iter()
            .map(|c| c.dimensions.content.x)
            .collect()
    }

    fn ys(container: &LayoutBox) -> Vec<f32> {
        container
            .children
            .iter()
            .map(|c| c.dimensions.content.y)
            .collect()
    }

    fn widths(container: &LayoutBox) -> Vec<f32> {
        container
            .children
            .iter()
            .map(|c| c.dimensions.content.width)
            .collect()
    }

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.5
    }

    #[test]
    fn auto_repeat_count_matches_spec_formula() {
        let rep = [minmax_px_fr(200.0)];
        // 3 * 200 + 2 * 16 = 632 <= 800; 4 tracks would need 848.
        assert_eq!(auto_repeat_count(Some(800.0), &[], &rep, 16.0), 3);
        assert_eq!(auto_repeat_count(Some(1000.0), &[], &rep, 16.0), 4);
        // Never fewer than one repetition, even when nothing fits.
        assert_eq!(auto_repeat_count(Some(50.0), &[], &rep, 16.0), 1);
        // Indefinite container: one repetition.
        assert_eq!(auto_repeat_count(None, &[], &rep, 16.0), 1);
        // Fixed tracks and their gutters are subtracted first:
        // 100 + 10 + n*(200 + 10) <= 800 + ... -> n = 3.
        assert_eq!(
            auto_repeat_count(Some(800.0), &[GridTrackSize::Px(100.0)], &rep, 10.0),
            3
        );
        // Exact fit counts: 4 * 100 + 3 * 0 = 400.
        assert_eq!(
            auto_repeat_count(Some(400.0), &[], &[GridTrackSize::Px(100.0)], 0.0),
            4
        );
    }

    #[test]
    fn auto_fill_keeps_empty_tracks() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns = auto_repeat_template(100.0, false);
        style.column_gap = 10.0;
        let mut c = make_grid_container(style, vec![make_item(10.0, 20.0), make_item(10.0, 20.0)]);
        layout_grid(&mut c, 480.0, &m);
        // 4 columns fit (4*100 + 3*10 = 430); each is (480 - 30) / 4.
        let w = widths(&c);
        assert!(approx(w[0], 112.5), "auto-fill column width: {w:?}");
        assert!(approx(xs(&c)[1], 122.5), "second column x: {:?}", xs(&c));
    }

    #[test]
    fn auto_fit_collapses_empty_tracks() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns = auto_repeat_template(100.0, true);
        style.column_gap = 10.0;
        let mut c = make_grid_container(style, vec![make_item(10.0, 20.0), make_item(10.0, 20.0)]);
        layout_grid(&mut c, 480.0, &m);
        // Two used tracks share the whole width: (480 - 10) / 2.
        let w = widths(&c);
        assert!(approx(w[0], 235.0) && approx(w[1], 235.0), "{w:?}");
        assert!(approx(xs(&c)[1], 245.0));
    }

    // -- Placement -------------------------------------------------------

    fn three_col_style() -> ComputedStyle {
        let mut style = grid_style();
        style.grid_template_columns = vec![GridTrackSize::Px(100.0); 3].into();
        style.column_gap = 10.0;
        style
    }

    #[test]
    fn span_placement_widens_item() {
        let m = FixedMeasurer;
        let mut first = make_item(10.0, 20.0);
        first.style.grid_column_start = GridLine::Span(2);
        let mut c = make_grid_container(three_col_style(), vec![first, make_item(10.0, 20.0)]);
        layout_grid(&mut c, 480.0, &m);
        assert!(approx(widths(&c)[0], 210.0), "{:?}", widths(&c));
        // The next item auto-places into column 3 of the same row.
        assert!(approx(xs(&c)[1], 220.0));
        assert!(approx(ys(&c)[1], ys(&c)[0]));
    }

    #[test]
    fn negative_line_spans_full_row() {
        let m = FixedMeasurer;
        let mut header = make_item(10.0, 20.0);
        header.style.grid_column_start = GridLine::Line(1);
        header.style.grid_column_end = GridLine::Line(-1);
        let mut c = make_grid_container(three_col_style(), vec![header, make_item(10.0, 20.0)]);
        layout_grid(&mut c, 480.0, &m);
        assert!(approx(widths(&c)[0], 320.0), "{:?}", widths(&c));
        // The second item wraps below the full-width header.
        assert!(ys(&c)[1] > ys(&c)[0]);
        assert!(approx(xs(&c)[1], 0.0));
    }

    #[test]
    fn explicit_line_placement() {
        let m = FixedMeasurer;
        let mut item = make_item(10.0, 20.0);
        item.style.grid_column_start = GridLine::Line(3);
        item.style.grid_row_start = GridLine::Line(2);
        let mut c = make_grid_container(three_col_style(), vec![make_item(10.0, 30.0), item]);
        layout_grid(&mut c, 480.0, &m);
        assert!(approx(xs(&c)[1], 220.0));
        assert!(
            approx(ys(&c)[1], 30.0),
            "row 2 starts below row 1: {:?}",
            ys(&c)
        );
    }

    #[test]
    fn slash_syntax_via_apply_declaration() {
        use crate::css::parser::CssValue;
        let mut s = ComputedStyle::default();
        s.apply_declaration("grid-column", &CssValue::Keyword("2 / span 2".into()), 16.0);
        assert_eq!(s.grid_column_start, GridLine::Line(2));
        assert_eq!(s.grid_column_end, GridLine::Span(2));
        s.apply_declaration("grid-row", &CssValue::Keyword("1 / -1".into()), 16.0);
        assert_eq!(s.grid_row_start, GridLine::Line(1));
        assert_eq!(s.grid_row_end, GridLine::Line(-1));
        s.apply_declaration(
            "grid-area",
            &CssValue::Keyword("2 / 1 / 4 / 3".into()),
            16.0,
        );
        assert_eq!(s.grid_row_end, GridLine::Line(4));
        assert_eq!(s.grid_column_end, GridLine::Line(3));
        s.apply_declaration(
            "grid-auto-flow",
            &CssValue::Keyword("row dense".into()),
            16.0,
        );
        assert!(s.grid_auto_flow_dense && !s.grid_auto_flow_column);
    }

    fn span_items() -> Vec<LayoutBox> {
        let mut a = make_item(10.0, 20.0);
        a.style.grid_column_start = GridLine::Span(2);
        let mut b = make_item(10.0, 20.0);
        b.style.grid_column_start = GridLine::Span(2);
        vec![a, b, make_item(10.0, 20.0)]
    }

    #[test]
    fn sparse_flow_leaves_holes() {
        let m = FixedMeasurer;
        let mut c = make_grid_container(three_col_style(), span_items());
        layout_grid(&mut c, 480.0, &m);
        // Item 1 doesn't fit next to item 0, so it wraps; item 2 follows
        // it (sparse never goes back to the hole at row 1, column 3).
        assert!(approx(ys(&c)[2], ys(&c)[1]));
        assert!(approx(xs(&c)[2], 220.0));
    }

    #[test]
    fn dense_flow_backfills_holes() {
        let m = FixedMeasurer;
        let mut style = three_col_style();
        style.grid_auto_flow_dense = true;
        let mut c = make_grid_container(style, span_items());
        layout_grid(&mut c, 480.0, &m);
        assert!(approx(ys(&c)[2], ys(&c)[0]), "dense backfills row 1");
        assert!(approx(xs(&c)[2], 220.0));
    }

    #[test]
    fn named_areas_still_place_items() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns = vec![GridTrackSize::Px(100.0), GridTrackSize::Fr(1.0)].into();
        style.grid_template_areas = vec![
            vec!["head".into(), "head".into()],
            vec!["side".into(), "main".into()],
        ];
        let mut main = make_item(10.0, 20.0);
        main.style.grid_area = Some("main".into());
        let mut head = make_item(10.0, 20.0);
        head.style.grid_area = Some("head".into());
        let mut side = make_item(10.0, 20.0);
        side.style.grid_column_start = GridLine::Named("side".into());
        side.style.grid_column_end = GridLine::Named("side".into());
        side.style.grid_row_start = GridLine::Named("side-start".into());
        let mut c = make_grid_container(style, vec![main, head, side]);
        layout_grid(&mut c, 480.0, &m);
        assert!(approx(xs(&c)[0], 100.0) && approx(ys(&c)[0], 20.0));
        assert!(approx(widths(&c)[1], 480.0) && approx(ys(&c)[1], 0.0));
        assert!(approx(xs(&c)[2], 0.0) && approx(ys(&c)[2], 20.0));
    }

    // -- Track sizing ----------------------------------------------------

    #[test]
    fn percent_tracks_resolve_against_container() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns =
            vec![GridTrackSize::Percent(25.0), GridTrackSize::Percent(75.0)].into();
        let mut c = make_grid_container(style, vec![make_item(10.0, 20.0), make_item(10.0, 20.0)]);
        layout_grid(&mut c, 480.0, &m);
        assert!(approx(widths(&c)[0], 120.0) && approx(widths(&c)[1], 360.0));
    }

    #[test]
    fn minmax_flexible_track_respects_minimum() {
        // 1fr of 300px would be 100px, below the 150px floor: the minmax
        // track freezes at 150 and the plain fr tracks share the rest.
        let tracks = vec![
            minmax_px_fr(150.0),
            GridTrackSize::Fr(1.0),
            GridTrackSize::Fr(1.0),
        ];
        let sizes = resolve_track_sizes(&tracks, &[0.0; 3], Some(300.0), Some(300.0));
        assert!(
            approx(sizes[0], 150.0) && approx(sizes[1], 75.0),
            "{sizes:?}"
        );
    }

    #[test]
    fn fr_rows_fill_definite_height() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.height = Dimension::Px(300.0);
        style.grid_template_rows = vec![GridTrackSize::Fr(1.0), GridTrackSize::Fr(2.0)].into();
        let mut c = make_grid_container(style, vec![make_item(10.0, 20.0), make_item(10.0, 20.0)]);
        layout_grid(&mut c, 480.0, &m);
        assert!(approx(ys(&c)[1], 100.0), "{:?}", ys(&c));
        assert!(approx(c.dimensions.content.height, 300.0));
    }

    #[test]
    fn items_positioned_inside_container_padding_box() {
        // content.x/y is the content box; padding must not be added again.
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns = vec![GridTrackSize::Fr(1.0)].into();
        let mut c = make_grid_container(style, vec![make_item(10.0, 20.0)]);
        c.dimensions.padding.left = 12.0;
        c.dimensions.padding.top = 7.0;
        c.dimensions.content.x = 12.0;
        c.dimensions.content.y = 7.0;
        layout_grid(&mut c, 480.0, &m);
        assert!(approx(xs(&c)[0], 12.0) && approx(ys(&c)[0], 7.0));
        assert!(approx(c.dimensions.content.height, 20.0));
    }

    #[test]
    fn descendants_follow_their_grid_item() {
        let m = FixedMeasurer;
        let mut style = grid_style();
        style.grid_template_columns = vec![GridTrackSize::Fr(1.0), GridTrackSize::Fr(1.0)].into();
        let mut second = make_item(10.0, 20.0);
        second.style.height = Dimension::Auto;
        second.children = vec![make_item(30.0, 15.0)];
        let mut c = make_grid_container(
            style,
            vec![make_item(10.0, 40.0), make_item(10.0, 40.0), second],
        );
        layout_grid(&mut c, 480.0, &m);
        let item = &c.children[2];
        let inner = &item.children[0];
        assert!(approx(
            inner.dimensions.content.x,
            item.dimensions.content.x
        ));
        assert!(approx(
            inner.dimensions.content.y,
            item.dimensions.content.y
        ));
        assert!(approx(item.dimensions.content.y, 40.0));
    }

    // -- End-to-end: stylesheet -> cascade -> layout ---------------------

    /// Lay out `html` with `css` at `width` and return the border boxes
    /// `(x, y, w)` of the first grid container's children.
    fn card_boxes(html: &str, css: &str, width: f32) -> Vec<(f32, f32, f32)> {
        use crate::css::cascade::{CascadeContext, style_tree};
        use crate::css::parser::Stylesheet;
        use crate::html::tokenizer::Tokenizer;
        use crate::html::tree_builder::TreeBuilder;
        use crate::layout::block::build_layout_tree;

        fn find_grid(b: &LayoutBox) -> Option<&LayoutBox> {
            if matches!(b.box_type, BoxType::Grid) {
                return Some(b);
            }
            b.children.iter().find_map(find_grid)
        }

        let doc = TreeBuilder::build(Tokenizer::new(html).tokenize());
        let ua = crate::css::default::default_stylesheet();
        let author = Stylesheet::parse(css);
        let sheets = vec![ua, &author];
        let styles = style_tree(&doc, &sheets, &[], &CascadeContext::default());
        let root = build_layout_tree(
            &doc,
            &styles,
            &crate::SimpleTextMeasurer,
            width,
            600.0,
            None,
            &std::collections::HashMap::new(),
        );
        let grid = find_grid(&root).expect("grid container");
        grid.children
            .iter()
            .map(|c| {
                let bb = c.dimensions.border_box();
                (bb.x, bb.y, bb.width)
            })
            .collect()
    }

    #[test]
    fn responsive_card_grid_reflows_with_width() {
        let html = "<html><body><div class=\"cards\">\
            <div class=\"card\">a</div><div class=\"card\">b</div>\
            <div class=\"card\">c</div><div class=\"card\">d</div>\
            <div class=\"card\">e</div></div></body></html>";
        let css = "body { margin: 0 } \
            .cards { display: grid; gap: 20px; \
                     grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); } \
            .card { height: 50px; } \
            .card:first-child { grid-column: 1 / -1; }";

        // 800px: 3 columns ((800 + 20) / 220 = 3.7) of (800 - 40) / 3.
        let wide = card_boxes(html, css, 800.0);
        assert_eq!(wide.len(), 5);
        assert!(approx(wide[0].2, 800.0), "hero spans the row: {wide:?}");
        let col = (800.0 - 40.0) / 3.0;
        assert!(approx(wide[1].2, col), "{wide:?}");
        assert!(approx(wide[2].0, col + 20.0), "{wide:?}");
        assert!(
            approx(wide[1].1, wide[3].1),
            "b, c, d share a row: {wide:?}"
        );
        assert!(approx(wide[4].1, wide[1].1 + 70.0), "e wraps: {wide:?}");

        // 440px: 2 columns ((440 + 20) / 220 = 2.09) of 210px.
        let narrow = card_boxes(html, css, 440.0);
        assert!(approx(narrow[0].2, 440.0), "{narrow:?}");
        assert!(approx(narrow[1].2, 210.0), "{narrow:?}");
        assert!(approx(narrow[2].0, 230.0), "{narrow:?}");
        assert!(
            approx(narrow[3].0, narrow[1].0),
            "d wraps under b: {narrow:?}"
        );
        assert!(approx(narrow[3].1, narrow[1].1 + 70.0), "{narrow:?}");
    }
}

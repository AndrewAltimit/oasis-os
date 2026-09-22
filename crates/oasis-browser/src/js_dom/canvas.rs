//! Canvas 2D context bindings (`__oasis_canvas_*`).

use std::rc::Rc;

use oasis_js::rquickjs::{Ctx, Function, Result as JsResult};

use crate::html::dom::NodeId;

/// Install `__oasis_canvas_*` globals for `<canvas>` 2D context support.
///
/// Must be called after [`super::install_document_global_full`] since the
/// JS bootstrap below extends `Element.prototype` with `getContext()`.
#[cfg(feature = "canvas")]
pub fn install_canvas_bindings(
    ctx: &Ctx<'_>,
    canvas_map: &crate::canvas::SharedCanvasMap,
) -> JsResult<()> {
    let globals = ctx.globals();

    // -- __oasis_canvas_fill_rect(nid, x, y, w, h) -------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_fill_rect",
            Function::new(
                ctx.clone(),
                move |nid: i32, x: f64, y: f64, w: f64, h: f64| {
                    let map = m.borrow();
                    if let Some(state) = map.get(&(nid as NodeId)) {
                        let mut s = state.borrow_mut();
                        let color = s.fill_color;
                        s.commands.push(crate::canvas::CanvasCommand::FillRect {
                            x: x as f32,
                            y: y as f32,
                            w: w as f32,
                            h: h as f32,
                            color,
                        });
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_canvas_stroke_rect(nid, x, y, w, h) -----------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_stroke_rect",
            Function::new(
                ctx.clone(),
                move |nid: i32, x: f64, y: f64, w: f64, h: f64| {
                    let map = m.borrow();
                    if let Some(state) = map.get(&(nid as NodeId)) {
                        let mut s = state.borrow_mut();
                        let color = s.stroke_color;
                        let lw = s.line_width;
                        s.commands.push(crate::canvas::CanvasCommand::StrokeRect {
                            x: x as f32,
                            y: y as f32,
                            w: w as f32,
                            h: h as f32,
                            color,
                            line_width: lw,
                        });
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_canvas_clear_rect(nid, x, y, w, h) ------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_clear_rect",
            Function::new(
                ctx.clone(),
                move |nid: i32, x: f64, y: f64, w: f64, h: f64| {
                    let map = m.borrow();
                    if let Some(state) = map.get(&(nid as NodeId)) {
                        let mut s = state.borrow_mut();
                        s.commands.push(crate::canvas::CanvasCommand::ClearRect {
                            x: x as f32,
                            y: y as f32,
                            w: w as f32,
                            h: h as f32,
                        });
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_canvas_fill_text(nid, text, x, y) --------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_fill_text",
            Function::new(
                ctx.clone(),
                move |nid: i32, text: String, x: f64, y: f64| {
                    let map = m.borrow();
                    if let Some(state) = map.get(&(nid as NodeId)) {
                        let mut s = state.borrow_mut();
                        let color = s.fill_color;
                        let font_size = s.font_size;
                        s.commands.push(crate::canvas::CanvasCommand::FillText {
                            text,
                            x: x as f32,
                            y: y as f32,
                            color,
                            font_size,
                        });
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_canvas_set_fill(nid, color_str) ----------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_set_fill",
            Function::new(ctx.clone(), move |nid: i32, color: String| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId))
                    && let Some(c) = crate::svg::parse_svg_color(&color)
                {
                    state.borrow_mut().fill_color = c;
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_set_stroke(nid, color_str) --------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_set_stroke",
            Function::new(ctx.clone(), move |nid: i32, color: String| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId))
                    && let Some(c) = crate::svg::parse_svg_color(&color)
                {
                    state.borrow_mut().stroke_color = c;
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_set_line_width(nid, width) --------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_set_line_width",
            Function::new(ctx.clone(), move |nid: i32, width: f64| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    state.borrow_mut().line_width = width as f32;
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_set_font(nid, font_str) -----------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_set_font",
            Function::new(ctx.clone(), move |nid: i32, font: String| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    // Extract pixel size from font string, e.g. "12px sans-serif".
                    for part in font.split_whitespace() {
                        if let Some(px) = part.strip_suffix("px")
                            && let Ok(size) = px.parse::<f32>()
                        {
                            state.borrow_mut().font_size = size;
                            break;
                        }
                    }
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_line(nid, x1, y1, x2, y2, is_fill) -----------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_line",
            Function::new(
                ctx.clone(),
                move |nid: i32, x1: f64, y1: f64, x2: f64, y2: f64, is_fill: bool| {
                    let map = m.borrow();
                    if let Some(state) = map.get(&(nid as NodeId)) {
                        let mut s = state.borrow_mut();
                        let color = if is_fill {
                            s.fill_color
                        } else {
                            s.stroke_color
                        };
                        let lw = s.line_width;
                        s.commands.push(crate::canvas::CanvasCommand::Line {
                            x1: x1 as f32,
                            y1: y1 as f32,
                            x2: x2 as f32,
                            y2: y2 as f32,
                            color,
                            line_width: lw,
                        });
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_canvas_arc(nid, cx, cy, r, fill) ---------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_arc",
            Function::new(
                ctx.clone(),
                move |nid: i32, cx: f64, cy: f64, r: f64, fill: bool| {
                    let map = m.borrow();
                    if let Some(state) = map.get(&(nid as NodeId)) {
                        let mut s = state.borrow_mut();
                        let color = if fill { s.fill_color } else { s.stroke_color };
                        s.commands.push(crate::canvas::CanvasCommand::Arc {
                            cx: cx as f32,
                            cy: cy as f32,
                            r: r as f32,
                            color,
                            fill,
                        });
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_canvas_begin_path(nid) ---------------------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_begin_path",
            Function::new(ctx.clone(), move |nid: i32| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    let mut s = state.borrow_mut();
                    s.current_path.clear();
                    s.path_start = None;
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_move_to(nid, x, y) ----------------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_move_to",
            Function::new(ctx.clone(), move |nid: i32, x: f64, y: f64| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    let mut s = state.borrow_mut();
                    let pt = (x as f32, y as f32);
                    s.current_path.push(pt);
                    s.path_start = Some(pt);
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_line_to(nid, x, y) ----------------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_line_to",
            Function::new(ctx.clone(), move |nid: i32, x: f64, y: f64| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    state.borrow_mut().current_path.push((x as f32, y as f32));
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_bezier_curve_to(nid, cp1x, cp1y, cp2x, cp2y, x, y)
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_bezier_curve_to",
            Function::new(
                ctx.clone(),
                move |nid: i32, cp1x: f64, cp1y: f64, cp2x: f64, cp2y: f64, x: f64, y: f64| {
                    let map = m.borrow();
                    if let Some(state) = map.get(&(nid as NodeId)) {
                        let mut s = state.borrow_mut();
                        let (cx, cy) = s.current_path.last().copied().unwrap_or((0.0, 0.0));
                        crate::svg::flatten_cubic(
                            &mut s.current_path,
                            cx,
                            cy,
                            cp1x as f32,
                            cp1y as f32,
                            cp2x as f32,
                            cp2y as f32,
                            x as f32,
                            y as f32,
                        );
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_canvas_quadratic_curve_to(nid, cpx, cpy, x, y) ------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_quadratic_curve_to",
            Function::new(
                ctx.clone(),
                move |nid: i32, cpx: f64, cpy: f64, x: f64, y: f64| {
                    let map = m.borrow();
                    if let Some(state) = map.get(&(nid as NodeId)) {
                        let mut s = state.borrow_mut();
                        let (cx, cy) = s.current_path.last().copied().unwrap_or((0.0, 0.0));
                        crate::svg::flatten_quad(
                            &mut s.current_path,
                            cx,
                            cy,
                            cpx as f32,
                            cpy as f32,
                            x as f32,
                            y as f32,
                        );
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_canvas_close_path(nid) -------------------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_close_path",
            Function::new(ctx.clone(), move |nid: i32| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    let mut s = state.borrow_mut();
                    if let Some(start) = s.path_start {
                        s.current_path.push(start);
                    }
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_fill_path(nid) --------------------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_fill_path",
            Function::new(ctx.clone(), move |nid: i32| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    let mut s = state.borrow_mut();
                    if s.current_path.len() >= 3 {
                        let color = s.fill_color;
                        let points = std::mem::take(&mut s.current_path);
                        s.commands
                            .push(crate::canvas::CanvasCommand::FillPath { points, color });
                    }
                    s.current_path.clear();
                    s.path_start = None;
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_stroke_path(nid) ------------------------------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_stroke_path",
            Function::new(ctx.clone(), move |nid: i32| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    let mut s = state.borrow_mut();
                    if s.current_path.len() >= 2 {
                        let color = s.stroke_color;
                        let lw = s.line_width;
                        let points = std::mem::take(&mut s.current_path);
                        s.commands.push(crate::canvas::CanvasCommand::StrokePath {
                            points,
                            color,
                            line_width: lw,
                        });
                    }
                    s.current_path.clear();
                    s.path_start = None;
                }
            })?,
        )?;
    }

    // -- __oasis_canvas_save(nid) / __oasis_canvas_restore(nid) -------
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_save",
            Function::new(ctx.clone(), move |nid: i32| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    state.borrow_mut().save();
                }
            })?,
        )?;
    }
    {
        let m = Rc::clone(canvas_map);
        globals.set(
            "__oasis_canvas_restore",
            Function::new(ctx.clone(), move |nid: i32| {
                let map = m.borrow();
                if let Some(state) = map.get(&(nid as NodeId)) {
                    state.borrow_mut().restore();
                }
            })?,
        )?;
    }

    // -- JavaScript CanvasRenderingContext2D class ---------------------
    #[cfg(feature = "canvas")]
    {
        let _: () = ctx.eval(JS_CANVAS_BOOTSTRAP)?;
    }

    Ok(())
}

/// JavaScript code for the CanvasRenderingContext2D class and
/// `Element.prototype.getContext()`.
const JS_CANVAS_BOOTSTRAP: &str = include_str!("canvas.js");

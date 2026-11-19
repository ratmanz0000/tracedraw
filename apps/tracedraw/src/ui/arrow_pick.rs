//! The arrowhead pick: a button that shows the head's silhouette and
//! opens a grid of silhouettes, the presets and the custom heads of the
//! outline library, where a list of names used to be.

use egui::{pos2, Color32, CursorIcon, Id, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};
use tracedraw_core::Arrowhead;

use crate::i18n::tr;
use crate::theme::{paint_icon_button, Tokens};
use crate::ui::menus;
use crate::ui::tips::RichTip;

/// The size of one silhouette cell of the pick menu.
const CELL: Vec2 = Vec2::new(30.0, 22.0);

/// A head's silhouette in `rect`: the tip at the right edge, the shape's
/// middle at the centre, filled. `None` draws the plain line's end.
pub(crate) fn paint_head(painter: &egui::Painter, rect: Rect, kind: &Arrowhead, ink: Color32) {
    if matches!(kind, Arrowhead::None) {
        let y = rect.center().y;
        painter.line_segment(
            [pos2(rect.left() + 1.0, y), pos2(rect.right() - 1.0, y)],
            Stroke::new(1.4, ink),
        );
        return;
    }
    let rings = head_rings(kind);
    let Some(b) = rings_bounds(&rings) else {
        return;
    };
    let (x0, y0, x1, y1) = b;
    let w = (x1 - x0).max(1e-6);
    let h = (y1 - y0).max(1e-6);
    let s = (rect.width() / w).min(rect.height() / h);
    let dx = (rect.width() - w * s) / 2.0;
    let ym = (y0 + y1) / 2.0;
    let cy = rect.center().y;
    for ring in &rings {
        let pts: Vec<Pos2> = ring
            .iter()
            .map(|p| pos2(rect.left() + dx + (p.x - x0) * s, cy - (p.y - ym) * s))
            .collect();
        // A convex ring fills as one polygon; a concave one clips into
        // triangles (a fan of thin triangles shows gaps).
        if convex(&pts) {
            painter.add(Shape::convex_polygon(pts, ink, Stroke::NONE));
        } else {
            for t in triangulate(&pts) {
                painter.add(Shape::convex_polygon(
                    vec![pts[t[0]], pts[t[1]], pts[t[2]]],
                    ink,
                    Stroke::NONE,
                ));
            }
        }
    }
}

/// Whether the closed ring of points turns the same way everywhere.
fn convex(pts: &[Pos2]) -> bool {
    let n = pts.len();
    let mut sign = 0.0;
    for i in 0..n {
        let t = area2(pts[i], pts[(i + 1) % n], pts[(i + 2) % n]);
        if t.abs() < 1e-6 {
            continue;
        }
        if sign == 0.0 {
            sign = t.signum();
        } else if t.signum() != sign {
            return false;
        }
    }
    true
}

/// The head's outline as closed rings of head-space points (the tip at
/// x = 0 pointing towards +x, about one unit tall). A custom head of
/// several subpaths gives several rings; each fills on its own.
fn head_rings(kind: &Arrowhead) -> Vec<Vec<Pos2>> {
    let Some(outline) = tracedraw_core::style::arrowhead_preset_outline(kind) else {
        return Vec::new();
    };
    let mut rings: Vec<Vec<Pos2>> = Vec::new();
    kurbo::flatten(
        outline.elements().iter().copied(),
        0.005,
        &mut |el| match el {
            kurbo::PathEl::MoveTo(p) => rings.push(vec![pos2(p.x as f32, p.y as f32)]),
            kurbo::PathEl::LineTo(p) => {
                if let Some(last) = rings.last_mut() {
                    last.push(pos2(p.x as f32, p.y as f32));
                }
            }
            _ => {}
        },
    );
    for ring in &mut rings {
        ring.dedup_by(|a, b| (a.x - b.x).abs() < 1e-4 && (a.y - b.y).abs() < 1e-4);
        if ring.len() > 2 {
            let a = ring[0];
            let b = ring[ring.len() - 1];
            if (a.x - b.x).abs() < 1e-4 && (a.y - b.y).abs() < 1e-4 {
                ring.pop();
            }
        }
    }
    rings.retain(|r| r.len() >= 3);
    rings
}

/// The smallest box around the rings: `(x0, y0, x1, y1)`.
fn rings_bounds(rings: &[Vec<Pos2>]) -> Option<(f32, f32, f32, f32)> {
    let mut b: Option<(f32, f32, f32, f32)> = None;
    for ring in rings {
        for p in ring {
            b = Some(match b {
                None => (p.x, p.y, p.x, p.y),
                Some((x0, y0, x1, y1)) => (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)),
            });
        }
    }
    b
}

/// The double area of the triangle `a b c`.
fn area2(a: Pos2, b: Pos2, c: Pos2) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

/// Ear clipping for the simple closed polygons of the head outlines: the
/// vertex indices of the covering triangles. A ring that would not clip
/// (never for these outlines) fans from its first vertex, so a shape
/// still shows.
fn triangulate(pts: &[Pos2]) -> Vec<[usize; 3]> {
    let n = pts.len();
    if n < 3 {
        return Vec::new();
    }
    let mut ring: Vec<usize> = (0..n).collect();
    // A positive turn puts the ring clockwise on the screen (y down),
    // the turn ears need.
    let mut turn = 0.0;
    for i in 0..n {
        turn += area2(pts[i], pts[(i + 1) % n], pts[(i + 2) % n]);
    }
    if turn < 0.0 {
        ring.reverse();
    }
    let mut out = Vec::new();
    loop {
        let m = ring.len();
        if m < 3 {
            break;
        }
        let mut clipped = false;
        for k in 0..m {
            let (i0, i1, i2) = (ring[(k + m - 1) % m], ring[k], ring[(k + 1) % m]);
            let (a, b, c) = (pts[i0], pts[i1], pts[i2]);
            if area2(a, b, c) <= 1e-6 {
                continue;
            }
            let inside = ring.iter().all(|&j| {
                j == i0
                    || j == i1
                    || j == i2
                    || area2(a, b, pts[j]) < -1e-9
                    || area2(b, c, pts[j]) < -1e-9
                    || area2(c, a, pts[j]) < -1e-9
            });
            if inside {
                out.push([i0, i1, i2]);
                ring.remove(k);
                clipped = true;
                break;
            }
        }
        if !clipped {
            break;
        }
    }
    if ring.len() == 3 {
        out.push([ring[0], ring[1], ring[2]]);
    } else {
        for j in 1..ring.len().saturating_sub(1) {
            out.push([ring[0], ring[j], ring[j + 1]]);
        }
    }
    out
}

/// The head pick button: the head's silhouette with a caret; the menu
/// shows the silhouettes of the presets and the library's custom heads.
/// True when a pick changed the head.
pub(crate) fn head_pick(
    ui: &mut Ui,
    id_salt: &str,
    a: &mut Arrowhead,
    custom: &[Arrowhead],
    mixed: bool,
) -> bool {
    let mut changed = false;
    let tip = if id_salt.to_lowercase().ends_with("end") {
        "toolbar.end_arrow"
    } else {
        "toolbar.start_arrow"
    };
    let size = egui::vec2(34.0, ui.spacing().interact_size.y);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let resp = ui
        .interact(rect, Id::new(("head_pick", id_salt)), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .rich_tip(tr(tip));
    let p = ui.painter();
    paint_icon_button(p, rect, resp.hovered(), false);
    // The silhouette on the left, the caret on the right.
    let head_rect = Rect::from_min_max(
        pos2(rect.left() + 3.0, rect.top() + 2.0),
        pos2(rect.right() - 9.0, rect.bottom() - 2.0),
    );
    if mixed {
        // Several heads at once: a broken line, no single silhouette.
        let y = head_rect.center().y;
        let w = head_rect.width();
        for (f, g) in [(0.05, 0.4), (0.6, 0.95)] {
            p.line_segment(
                [
                    pos2(head_rect.left() + w * f, y),
                    pos2(head_rect.left() + w * g, y),
                ],
                Stroke::new(1.2, Tokens::text_dim()),
            );
        }
    } else {
        paint_head(p, head_rect, a, Tokens::text());
    }
    let cx = rect.right() - 5.5;
    let cy = rect.center().y + 0.5;
    p.add(Shape::convex_polygon(
        vec![
            pos2(cx - 3.0, cy - 1.5),
            pos2(cx + 3.0, cy - 1.5),
            pos2(cx, cy + 1.5),
        ],
        Tokens::text_dim(),
        Stroke::NONE,
    ));
    let current = a.clone();
    let mut picked = None;
    egui::Popup::menu(&resp)
        .id(Id::new(id_salt))
        .style(menus::menu_popup_style)
        .show(|ui| {
            picked = head_menu(ui, id_salt, &current, custom, mixed);
        });
    if let Some(k) = picked {
        if *a != k {
            *a = k;
            changed = true;
        }
    }
    changed
}

/// The grid of head silhouettes of the pick menu: the presets, then the
/// custom heads of the library. The head a cell picked, if any.
fn head_menu(
    ui: &mut Ui,
    id_salt: &str,
    current: &Arrowhead,
    custom: &[Arrowhead],
    mixed: bool,
) -> Option<Arrowhead> {
    let mut heads: Vec<Arrowhead> = Arrowhead::ALL.to_vec();
    heads.extend(custom.iter().cloned());
    let mut picked = None;
    for (row, chunk) in heads.chunks(4).enumerate() {
        ui.horizontal(|ui| {
            for (c, k) in chunk.iter().enumerate() {
                let n = row * 4 + c;
                if head_cell(
                    ui,
                    Id::new(("arrow_pick_cell", id_salt, n)),
                    k,
                    !mixed && *current == *k,
                ) {
                    picked = Some(k.clone());
                    ui.close();
                }
            }
        });
    }
    picked
}

/// One silhouette cell of the pick menu; true on a click.
fn head_cell(ui: &mut Ui, id: Id, kind: &Arrowhead, on: bool) -> bool {
    let (rect, _) = ui.allocate_exact_size(CELL, Sense::hover());
    let resp = ui.interact(rect, id, Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 2.0, Tokens::tool_hover());
    }
    if on {
        p.rect_stroke(
            rect,
            2.0,
            Stroke::new(1.2, Tokens::ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    paint_head(p, rect.shrink(3.0), kind, Tokens::text());
    resp.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The area of a closed ring of points, signed.
    fn ring_area(pts: &[Pos2]) -> f32 {
        let mut s = 0.0;
        for i in 0..pts.len() {
            let a = pts[i];
            let b = pts[(i + 1) % pts.len()];
            s += a.x * b.y - b.x * a.y;
        }
        s / 2.0
    }

    /// The concave arrow outline triangulates exactly: the triangles
    /// cover the outline and nothing more.
    #[test]
    fn the_concave_arrow_triangulates_exactly() {
        let rings = head_rings(&Arrowhead::Arrow);
        assert_eq!(rings.len(), 1);
        let pts = &rings[0];
        assert_eq!(pts.len(), 4, "the notched quad");
        let tri = triangulate(pts);
        assert_eq!(tri.len(), 2);
        let covered: f32 = tri
            .iter()
            .map(|t| ring_area(&[pts[t[0]], pts[t[1]], pts[t[2]]]).abs())
            .sum();
        let want = ring_area(pts).abs();
        assert!((covered - want).abs() < 1e-4, "{covered} vs {want}");
    }

    /// Every preset silhouette keeps its turn in its triangles and
    /// covers exactly its ring's area.
    #[test]
    fn every_preset_silhouette_triangulates_exactly() {
        for k in Arrowhead::ALL {
            if matches!(k, Arrowhead::None) {
                continue;
            }
            for ring in head_rings(&k) {
                let tri = triangulate(&ring);
                assert!(!tri.is_empty(), "{k:?}");
                let covered: f32 = tri
                    .iter()
                    .map(|t| ring_area(&[ring[t[0]], ring[t[1]], ring[t[2]]]).abs())
                    .sum();
                let want = ring_area(&ring).abs();
                assert!(
                    (covered - want).abs() < 1e-3 * want.max(1.0),
                    "{k:?}: {covered} vs {want}"
                );
                let cw = ring_area(&ring) < 0.0;
                for t in &tri {
                    let area = ring_area(&[ring[t[0]], ring[t[1]], ring[t[2]]]);
                    assert!(
                        if cw { area <= 1e-6 } else { area >= -1e-6 },
                        "{k:?}: a triangle against the ring's turn"
                    );
                }
            }
        }
    }

    /// A custom head of two rings, a frame with a hole, fills each ring
    /// on its own: no ring spans across the hole.
    #[test]
    fn a_custom_head_of_two_rings_fills_each_on_its_own() {
        let mut p = tracedraw_core::geometry::BezPath::new();
        p.move_to((10.0, -5.0));
        p.line_to((-10.0, -5.0));
        p.line_to((-10.0, 5.0));
        p.line_to((10.0, 5.0));
        p.close_path();
        p.move_to((4.0, -2.0));
        p.line_to((-4.0, -2.0));
        p.line_to((-4.0, 2.0));
        p.line_to((4.0, 2.0));
        p.close_path();
        let head = Arrowhead::from_shape_path(&p, "Frame");
        let rings = head_rings(&head);
        assert_eq!(rings.len(), 2, "{rings:?}");
        for ring in &rings {
            assert!(!triangulate(ring).is_empty());
        }
    }

    /// The pick opens a menu of silhouettes; a click on the diamond's
    /// cell picks the diamond.
    #[test]
    fn the_pick_menu_sets_the_head_from_a_cell() {
        let ctx = crate::theme::ui_context();
        let screen = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
        let run = |events: Vec<egui::Event>| -> Option<Arrowhead> {
            let mut picked = None;
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            ctx.run_ui(input, |ui| {
                ui.set_min_size(egui::vec2(400.0, 300.0));
                picked = head_menu(ui, "test_start", &Arrowhead::Arrow, &[], false);
            })
            .textures_delta
            .clear();
            picked
        };
        run(Vec::new());
        let cell = ctx
            .read_response(Id::new(("arrow_pick_cell", "test_start", 6)))
            .expect("the diamond cell")
            .rect;
        let press = |pressed: bool, at: Pos2| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        assert_eq!(run(vec![egui::Event::PointerMoved(cell.center())]), None);
        run(vec![press(true, cell.center())]);
        let picked = run(vec![press(false, cell.center())]);
        assert_eq!(picked, Some(Arrowhead::Diamond));
    }

    /// A click on the pick button opens its menu.
    #[test]
    fn the_pick_button_opens_its_menu() {
        let ctx = crate::theme::ui_context();
        let screen = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
        let mut a = Arrowhead::None;
        let mut run = |events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            ctx.run_ui(input, |ui| {
                ui.set_min_size(egui::vec2(400.0, 300.0));
                head_pick(ui, "test_start", &mut a, &[], false);
            })
            .textures_delta
            .clear();
        };
        run(Vec::new());
        let btn = ctx
            .read_response(Id::new(("head_pick", "test_start")))
            .expect("the pick button")
            .rect;
        let press = |pressed: bool, at: Pos2| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        run(vec![egui::Event::PointerMoved(btn.center())]);
        run(vec![press(true, btn.center())]);
        run(vec![press(false, btn.center())]);
        run(Vec::new());
        assert!(egui::Popup::is_any_open(&ctx), "the menu opened");
        assert_eq!(a, Arrowhead::None);
    }
}

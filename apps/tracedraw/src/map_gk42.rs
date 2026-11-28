//! GK-42 map calibration (a local feature): three reference markers on
//! the page tie the page millimetres to true GK-42 metres, and clicks
//! with the mode on pick up points whose coordinates the panel lists.
//!
//! The canvas keeps only the markers and the marks: the markers are
//! real shapes on a layer of their own (they move with any tool and
//! recalibrate at once), the picked points are overlay-only marks so
//! the drawing itself stays clean. Everything else lives in the GK-42
//! docker: the placement steps, the scales the calibration implies,
//! the table of the markers and the picked points, the clipboard and
//! the apply and reset buttons. See the module docs of
//! `tracedraw_core::gk42` for the marker maths.

use crate::app::App;
use crate::i18n::{tr, trf};
use crate::theme::Tokens;
use crate::ui::tips::{keyed, RichTip};
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect as ERect, Stroke, TextEdit, Ui, Vec2};
use tracedraw_core::document::{spans_text, Shape, ShapeKind, Symbol};
use tracedraw_core::geometry::{Affine, Point, Rect};
use tracedraw_core::gk42::{zones_disagree, GkCalibration, GkPoint, GkTransform};
use tracedraw_core::{Color, Command, Document, Fill, LayerId, ShapeId, Stroke as CoreStroke};

/// The page coordinates of the marker rows and the picked rows go out
/// with two decimals; the GK-42 metres with centimetre precision.
const MM_DIGITS: usize = 2;

/// App-side state of the module: the mode, the placement flow, the
/// picked points and the draft coordinate fields of the panel.
#[derive(Debug, Clone, PartialEq)]
pub struct Gk42State {
    /// The mode is on: a click on the page places the markers or adds a
    /// picked point (the top-bar dot is pressed).
    pub on: bool,
    /// The placement step: 0 to 2 = place marker 1, 2 or 3; 3 = pick points.
    pub step: u8,
    /// The first marker's shape, placed but not applied yet.
    pub marker1: Option<ShapeId>,
    /// The second marker's shape, placed but not applied yet.
    pub marker2: Option<ShapeId>,
    /// The third marker's shape, placed but not applied yet.
    pub marker3: Option<ShapeId>,
    /// The picked points, page millimetres, in click order.
    pub picked: Vec<Point>,
    cleared_points: Vec<Point>,
    /// The typed true coordinates of marker 1, metres (northing, easting).
    pub gk1: (String, String),
    /// The typed true coordinates of marker 2, metres (northing, easting).
    pub gk2: (String, String),
    /// The typed true coordinates of marker 3, metres (northing, easting).
    pub gk3: (String, String),
    /// The millimetre fields of the table, mirrored from the markers
    /// while their fields are not being edited.
    pub mm1: (String, String),
    pub mm2: (String, String),
    pub mm3: (String, String),
    /// The coordinate drafts were seeded from a saved calibration once
    /// (so an erased field is not refilled behind the user's back).
    pub seeded: bool,
    /// The last problem to show in the panel, an i18n key.
    pub error: Option<String>,
    /// A short note to show until the next action ("copied").
    pub note: Option<String>,
}

impl Default for Gk42State {
    fn default() -> Self {
        Gk42State {
            on: false,
            step: 0,
            marker1: None,
            marker2: None,
            marker3: None,
            picked: Vec::new(),
            cleared_points: Vec::new(),
            gk1: (String::new(), String::new()),
            gk2: (String::new(), String::new()),
            gk3: (String::new(), String::new()),
            mm1: (String::new(), String::new()),
            mm2: (String::new(), String::new()),
            mm3: (String::new(), String::new()),
            seeded: false,
            error: None,
            note: None,
        }
    }
}

/// The next placement step: the first marker missing, or 3 (pick
/// points) once all three markers are there.
fn next_step(app: &App) -> u8 {
    if marker_id(app, true).is_none() {
        0
    } else if marker_id(app, false).is_none() {
        1
    } else if marker3_id(app).is_none() {
        2
    } else {
        3
    }
}

/// Turning the mode on picks the flow up where the markers left it:
/// with all three markers there the clicks add picked points, otherwise
/// the next click places the first missing marker.
pub fn set_mode(app: &mut App, on: bool) {
    app.map_gk42.on = on;
    if on {
        app.map_gk42.step = next_step(app);
    }
}

/// A click on the page with the mode on: the first three clicks place
/// the calibration markers, the rest add picked points. Drags never
/// come here: the tool under the mode keeps doing its work.
pub fn click(app: &mut App, p: Point) {
    let step = next_step(app);
    if step < 3 {
        if place_marker(app, p, step) {
            app.map_gk42.step = step + 1;
        }
    } else {
        app.map_gk42.picked.push(p);
    }
}

/// The markers' layer on the current page, found by name or added.
fn marker_layer(app: &mut App) -> Option<LayerId> {
    let name = tr("gk42.layer");
    let page = app.page;
    {
        let doc = app.doc();
        if let Ok(p) = doc.page(page) {
            if let Some(l) = p.layers.iter().find(|l| l.name == name) {
                return Some(l.id);
            }
        }
    }
    app.run(Command::AddLayer {
        page,
        name: name.clone(),
    });
    app.doc()
        .page(page)
        .ok()?
        .layers
        .iter()
        .find(|l| l.name == name)
        .map(|l| l.id)
}

/// Placing one marker: a small filled circle on the markers' layer,
/// red for marker 1, blue for marker 2 and amber for marker 3, named
/// for the Objects docker. `index` is 0 to 2.
fn place_marker(app: &mut App, p: Point, index: u8) -> bool {
    let Some(layer) = marker_layer(app) else {
        return false;
    };
    let id = app.engine.new_shape_id();
    // A 6 mm circle centred on the click.
    let rect = Rect::new(p.x - 3.0, p.y - 3.0, p.x + 3.0, p.y + 3.0);
    let mut shape = Shape::new(id, ShapeKind::Ellipse { rect, arc: None });
    shape.fill = Fill::Solid(match index {
        0 => Color::rgb8(214, 48, 49),
        1 => Color::rgb8(9, 132, 227),
        _ => Color::rgb8(230, 150, 0),
    });
    shape.stroke = Some(CoreStroke::hairline(Color::BLACK));
    let n = (index + 1).to_string();
    shape.name = Some(trf("gk42.marker_name", &[("n", n.as_str())]));
    app.run(Command::AddShape { layer, shape });
    match index {
        0 => app.map_gk42.marker1 = Some(id),
        1 => app.map_gk42.marker2 = Some(id),
        _ => app.map_gk42.marker3 = Some(id),
    }
    true
}

/// The centre of a marker shape in page millimetres.
fn marker_position(doc: &Document, id: ShapeId) -> Option<Point> {
    let shape = doc.find_shape(id)?;
    let page = doc
        .parent_transform(id)
        .unwrap_or(tracedraw_core::geometry::Affine::IDENTITY)
        * shape.transform;
    match &shape.kind {
        ShapeKind::Ellipse { rect, .. } => Some(page * rect.center()),
        _ => Some(page * shape.local_bounds().center()),
    }
}

/// The marker ids in force: the state's ones while they are being
/// placed, the saved calibration's after it (or after a reload).
#[allow(dead_code)]
pub fn marker_ids(app: &App) -> Option<(ShapeId, ShapeId)> {
    let st = &app.map_gk42;
    if let (Some(a), Some(b)) = (st.marker1, st.marker2) {
        return Some((a, b));
    }
    app.doc().gk42.as_ref().map(|c| (c.marker1, c.marker2))
}

/// One marker's id from either source.
fn marker_id(app: &App, first: bool) -> Option<ShapeId> {
    let st = &app.map_gk42;
    let state_one = if first { st.marker1 } else { st.marker2 };
    state_one.or_else(|| {
        let cal = app.doc().gk42.as_ref()?;
        Some(if first { cal.marker1 } else { cal.marker2 })
    })
}

/// The third marker's id from either source; `None` for a calibration
/// saved with two markers.
fn marker3_id(app: &App) -> Option<ShapeId> {
    app.map_gk42
        .marker3
        .or_else(|| app.doc().gk42.as_ref()?.marker3)
}

/// The marker ids in use with their page points: two, or three once the
/// third marker is placed or saved.
fn marker_pages(app: &App) -> Option<(Vec<ShapeId>, Vec<Point>)> {
    let doc = app.doc();
    let mut ids = vec![marker_id(app, true)?, marker_id(app, false)?];
    if let Some(i3) = marker3_id(app) {
        ids.push(i3);
    }
    let pages = ids
        .iter()
        .map(|&i| marker_position(doc, i))
        .collect::<Option<Vec<_>>>()?;
    Some((ids, pages))
}

/// The typed draft as `(northing, easting)` pairs, marker 3 only when it
/// is in use.
fn typed_draft(st: &Gk42State, three: bool) -> Option<Vec<(f64, f64)>> {
    fn pair(p: &(String, String)) -> Option<(f64, f64)> {
        Some((parse_m(&p.0)?, parse_m(&p.1)?))
    }
    let mut out = vec![pair(&st.gk1)?, pair(&st.gk2)?];
    if three {
        out.push(pair(&st.gk3)?);
    }
    Some(out)
}

/// The calibration record from the ids and the true coordinates.
fn build_cal(ids: &[ShapeId], gk: &[GkPoint], layer: LayerId) -> Option<GkCalibration> {
    Some(GkCalibration {
        layer,
        marker1: *ids.first()?,
        marker2: *ids.get(1)?,
        gk1: *gk.first()?,
        gk2: *gk.get(1)?,
        marker3: ids.get(2).copied(),
        gk3: gk.get(2).copied(),
    })
}

/// The transform a calibration implies at the given page points.
fn cal_transform(cal: &GkCalibration, pages: &[Point]) -> Option<GkTransform> {
    cal.transform_at(*pages.first()?, *pages.get(1)?, pages.get(2).copied())
}

/// True when the marker positions cannot define a map: two markers on
/// one horizontal or vertical line, or three on one straight line.
fn pages_degenerate(pages: &[Point]) -> bool {
    match pages {
        [a, b] => (b.x - a.x).abs() < 1e-9 || (b.y - a.y).abs() < 1e-9,
        [a, b, c] => ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() < 1e-9,
        _ => true,
    }
}

/// The applied calibration with the markers' current page positions
/// and the transform it implies: `None` when there is none, a marker
/// is gone, or the geometry is degenerate (mirrored, on one line).
pub fn transform_of(doc: &Document) -> Option<(GkTransform, GkCalibration)> {
    let cal = doc.gk42.as_ref()?;
    let p1 = marker_position(doc, cal.marker1)?;
    let p2 = marker_position(doc, cal.marker2)?;
    let p3 = match cal.marker3 {
        Some(id) => Some(marker_position(doc, id)?),
        None => None,
    };
    let t = cal.transform_at(p1, p2, p3)?;
    Some((t, cal.clone()))
}

/// Convert the explicitly labelled X (northing) and Y (easting) fields.
fn typed_points(typed: &[(f64, f64)]) -> Vec<GkPoint> {
    typed.iter().map(|&(x, y)| GkPoint { x, y }).collect()
}

/// Diagnose why the typed coordinates do not resolve against the markers,
/// returning `None` when the inputs are incomplete or valid.
fn diagnose_draft(app: &App) -> Option<&'static str> {
    let (ids, pages) = marker_pages(app)?;
    if pages.len() != 3 {
        return Some("gk42.need_markers");
    }
    let Some(typed) = typed_draft(&app.map_gk42, true) else {
        return Some("gk42.bad_number");
    };
    if pages_degenerate(&pages) {
        return Some("gk42.broken");
    }
    let gk = typed_points(&typed);
    if zones_disagree(&gk) {
        return Some("gk42.zone_mismatch");
    }
    let cal = build_cal(&ids, &gk, LayerId(0))?;
    if cal_transform(&cal, &pages).is_none() {
        return Some("gk42.bad_axes");
    }
    None
}

/// The transform the panel and cursor readout use: the markers' positions
/// against the typed coordinates, falling back to the saved calibration's.
/// The bool says whether the coordinates came from the saved one.
pub fn preview_transform(app: &App) -> Option<(GkTransform, bool)> {
    let draft = || {
        let (ids, pages) = marker_pages(app)?;
        let typed = typed_draft(&app.map_gk42, pages.len() == 3)?;
        let gk = typed_points(&typed);
        if zones_disagree(&gk) {
            return None;
        }
        let cal = build_cal(&ids, &gk, LayerId(0))?;
        cal_transform(&cal, &pages)
    };
    draft()
        .map(|t| (t, false))
        .or_else(|| transform_of(app.doc()).map(|(t, _)| (t, true)))
}

/// Applying the draft: the typed coordinates against the markers'
/// positions, one undo step. Fills the panel's error line and returns
/// false when the draft does not parse, the markers are incomplete or
/// the zones of the two eastings disagree.
pub fn apply(app: &mut App) -> bool {
    let Some((ids, gk)) = parse_draft(app) else {
        return false;
    };
    let Some(&first) = ids.first() else {
        return false;
    };
    let layer = match app.doc().locate(first) {
        Ok((l, _)) => l,
        Err(_) => {
            app.map_gk42.error = Some(tr("gk42.broken"));
            return false;
        }
    };
    let Some(cal) = build_cal(&ids, &gk, layer) else {
        app.map_gk42.error = Some(tr("gk42.broken"));
        return false;
    };
    app.run(Command::SetGkCalibration { cal: Some(cal.clone()) });
    if app.doc().gk42.as_ref() != Some(&cal) {
        app.map_gk42.error = Some(tr("gk42.broken"));
        return false;
    }
    app.map_gk42.error = None;
    app.map_gk42.note = None;
    true
}

/// Parsing the panel's coordinate fields into the calibration data. All
/// three markers must be placed, and their labelled coordinates must fit.
fn parse_draft(app: &mut App) -> Option<(Vec<ShapeId>, Vec<GkPoint>)> {
    let Some((ids, pages)) = marker_pages(app) else {
        app.map_gk42.error = Some(tr("gk42.need_markers"));
        return None;
    };
    if pages.len() < 3 {
        app.map_gk42.error = Some(tr("gk42.need_markers"));
        return None;
    }
    let st = &mut app.map_gk42;
    let Some(typed) = typed_draft(st, true) else {
        st.error = Some(tr("gk42.bad_number"));
        return None;
    };
    if pages_degenerate(&pages) {
        st.error = Some(tr("gk42.broken"));
        return None;
    }
    let gk = typed_points(&typed);
    if zones_disagree(&gk) {
        st.error = Some(tr("gk42.zone_mismatch"));
        return None;
    }
    let Some(cal) = build_cal(&ids, &gk, LayerId(0)) else {
        st.error = Some(tr("gk42.broken"));
        return None;
    };
    if cal_transform(&cal, &pages).is_none() {
        st.error = Some(tr("gk42.bad_axes"));
        return None;
    }
    st.error = None;
    Some((ids, gk))
}

/// Resetting everything: the saved calibration goes, the marker shapes
/// go, the picked list and the fields empty, one undo step for all of
/// it. The mode stays as it was.
pub fn reset(app: &mut App) {
    let saved = app.doc().gk42.clone();
    let mut ids: Vec<ShapeId> = Vec::new();
    if let Some(i) = app.map_gk42.marker1 {
        ids.push(i);
    }
    if let Some(i) = app.map_gk42.marker2 {
        ids.push(i);
    }
    if let Some(i) = app.map_gk42.marker3 {
        ids.push(i);
    }
    if let Some(c) = &saved {
        ids.push(c.marker1);
        ids.push(c.marker2);
        ids.extend(c.marker3);
    }
    ids.sort();
    ids.dedup();
    ids.retain(|i| app.doc().find_shape(*i).is_some());
    let mut cmds = Vec::new();
    if saved.is_some() {
        cmds.push(Command::SetGkCalibration { cal: None });
    }
    if !ids.is_empty() {
        cmds.push(Command::DeleteShapes { shapes: ids });
    }
    if let Some(first) = cmds.first() {
        let label = first.label();
        let cmds: Vec<Command> = cmds;
        let _ = app.engine.run_batch(label, &cmds);
    }
    let st = &mut app.map_gk42;
    st.marker1 = None;
    st.marker2 = None;
    st.marker3 = None;
    st.picked.clear();
    st.cleared_points.clear();
    st.gk1 = (String::new(), String::new());
    st.gk2 = (String::new(), String::new());
    st.gk3 = (String::new(), String::new());
    st.mm1 = (String::new(), String::new());
    st.mm2 = (String::new(), String::new());
    st.mm3 = (String::new(), String::new());
    st.step = 0;
    st.seeded = false;
    st.error = None;
    st.note = None;
}

/// Moving a marker to an absolute page position (the table's "Abs., mm"
/// fields commit with Enter): one translate command, one undo step.
fn move_marker(app: &mut App, id: ShapeId, target: Point) -> bool {
    let Some(cur) = marker_position(app.doc(), id) else {
        return false;
    };
    let v = tracedraw_core::Vec2::new(target.x - cur.x, target.y - cur.y);
    app.run(Command::TransformShapes {
        shapes: vec![id],
        transform: tracedraw_core::geometry::Affine::translate(v),
    });
    true
}

/// A GK-42 value as typed: the group spaces (plain, narrow, the
/// apostrophe), the comma and the dot all parse.
pub fn parse_m(s: &str) -> Option<f64> {
    let cleaned: String = s
        .chars()
        .filter(|c| !matches!(c, ' ' | '\u{00A0}' | '\u{2009}' | '\''))
        .collect();
    let cleaned = cleaned.trim().replace(',', ".");
    if cleaned.is_empty() {
        return None;
    }
    cleaned.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Grouping an integer part by thousands: 6519000 to "6 519 000".
fn group(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (k, c) in s.chars().enumerate() {
        if k > 0 && (s.len() - k) % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// GK-42 metres for display: thousands with spaces, centimetres when
/// there is a fraction (a comma as the decimal mark, as the maps do).
#[cfg_attr(not(test), allow(dead_code))]
pub fn fmt_m(v: f64) -> String {
    let sign = if v < 0.0 { "-" } else { "" };
    let a = (v.abs() * 100.0).round() / 100.0;
    let mut int = a.floor() as i64;
    let mut frac = ((a - int as f64) * 100.0).round() as i64;
    if frac >= 100 {
        int += 1;
        frac = 0;
    }
    if frac == 0 {
        format!("{sign}{}", group(int))
    } else {
        format!("{sign}{},{:02}", group(int), frac)
    }
}

/// GK-42 metres as whole metres without spaces or decimals, for the
/// coordinate fields and the table ("7490000").
pub fn fmt_gk_plain(v: f64) -> String {
    format!("{}", v.round() as i64)
}

/// Write the same Y easting and X northing shown in the picked-point table.
pub(crate) fn gk_readout(g: GkPoint) -> String {
    format!("GK-42: Y {}  X {}", fmt_gk_plain(g.y), fmt_gk_plain(g.x))
}

/// Page millimetres for display.
pub fn fmt_mm(v: f64) -> String {
    format!("{:.*}", MM_DIGITS, v)
}

/// The distance between two page points in map metres, with the scales
/// and the turn of the map.
fn delta_m(a: Point, b: Point, t: &GkTransform) -> f64 {
    t.distance_m(a, b)
}

/// A dashed line: short segments, plain line shapes only.
fn dashed(painter: &Painter, a: Pos2, b: Pos2, stroke: Stroke) {
    let len = a.distance(b);
    if !(len > 1.0) {
        return;
    }
    let dir = (b - a) / len;
    let mut d = 0.0;
    while d < len {
        let e = (d + 4.0).min(len);
        painter.line_segment([a + dir * d, a + dir * e], stroke);
        d = e + 3.0;
    }
}

/// A numbered badge near a marker.
fn badge(painter: &Painter, at: Pos2, label: &str, color: Color32) {
    let c = at + Vec2::new(9.0, -9.0);
    painter.circle_filled(c, 8.0, color);
    painter.circle_stroke(c, 8.0, Stroke::new(1.0, Color32::WHITE));
    painter.text(
        c,
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(10.0),
        Color32::WHITE,
    );
}

/// A plate with the coordinates near the cursor, growing up from
/// `anchor`.
fn plate(painter: &Painter, anchor: Pos2, lines: &[String]) {
    let font = FontId::monospace(11.0);
    let line_h = 15.0;
    let w = lines
        .iter()
        .map(|l| {
            painter
                .layout_no_wrap(l.clone(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
        .fold(0.0, f32::max)
        + 10.0;
    let h = line_h * lines.len() as f32 + 8.0;
    let rect = ERect::from_min_size(Pos2::new(anchor.x, anchor.y - h), Vec2::new(w, h));
    painter.rect_filled(rect, 3.0, Color32::from_rgba_unmultiplied(20, 20, 20, 230));
    for (k, l) in lines.iter().enumerate() {
        painter.text(
            Pos2::new(rect.left() + 5.0, rect.top() + 4.0 + k as f32 * line_h),
            Align2::LEFT_TOP,
            l.clone(),
            font.clone(),
            Color32::from_rgb(235, 235, 235),
        );
    }
}

/// Page millimetres and GK-42 metres at the cursor, in the same Y, X
/// order and whole-metre format as the picked-point table.
pub fn cursor_lines(app: &App) -> Option<Vec<String>> {
    let active_t = preview_transform(app).map(|(t, _)| t);
    if !app.map_gk42.on && active_t.is_none() {
        return None;
    }
    let p = app.pointer_page?;
    let mut lines = vec![format!("Y: {}  X: {} mm", fmt_mm(p.y), fmt_mm(p.x),)];
    if let Some(t) = active_t {
        lines.push(gk_readout(t.to_gk(p)));
    }
    Some(lines)
}

/// The canvas overlay: badges at the markers, the dashed line between
/// them, the picked points with their numbers and the coordinate plate
/// next to the cursor (real coordinates + GK-42). Plain shapes only, so
/// it redraws every frame for free.
pub fn overlay(app: &App, painter: &Painter) {
    let st = &app.map_gk42;
    let doc = app.doc();
    let view = &app.view;
    let red = Color32::from_rgb(214, 48, 49);
    let blue = Color32::from_rgb(9, 132, 227);
    let green = Color32::from_rgb(0, 150, 100);
    let p1 = marker_id(app, true)
        .and_then(|i| marker_position(doc, i))
        .map(|p| view.to_screen(p));
    let p2 = marker_id(app, false)
        .and_then(|i| marker_position(doc, i))
        .map(|p| view.to_screen(p));
    let p3 = marker3_id(app)
        .and_then(|i| marker_position(doc, i))
        .map(|p| view.to_screen(p));
    let line = Stroke::new(1.0, Color32::from_rgb(0, 120, 215));
    if let (Some(a), Some(b)) = (p1, p2) {
        dashed(painter, a, b, line);
    }
    if let (Some(b), Some(c)) = (p2, p3) {
        dashed(painter, b, c, line);
    }
    if let (Some(c), Some(a)) = (p3, p1) {
        dashed(painter, c, a, line);
    }
    if let Some(a) = p1 {
        badge(painter, a, "1", red);
    }
    if let Some(b) = p2 {
        badge(painter, b, "2", blue);
    }
    if let Some(c) = p3 {
        badge(painter, c, "3", Color32::from_rgb(230, 150, 0));
    }
    for (k, p) in st.picked.iter().enumerate() {
        let s = view.to_screen(*p);
        painter.circle_filled(s, 2.5, green);
        painter.circle_stroke(s, 3.5, Stroke::new(1.0, green));
        painter.text(
            s + Vec2::new(7.0, -7.0),
            Align2::LEFT_BOTTOM,
            (k + 1).to_string(),
            FontId::monospace(10.0),
            green,
        );
    }
    if let (Some(p), Some(lines)) = (app.pointer_page, cursor_lines(app)) {
        let at = view.to_screen(p);
        plate(painter, at + Vec2::new(14.0, -12.0), &lines);
    }
}

/// The docker panel body.
pub fn panel(app: &mut App, ui: &mut Ui) {
    // The document's side first, copied, so the widgets below never
    // hold the document borrow while the state changes.
    let mm1 = marker_id(app, true).and_then(|i| marker_position(app.doc(), i));
    let mm2 = marker_id(app, false).and_then(|i| marker_position(app.doc(), i));
    let mm3 = marker3_id(app).and_then(|i| marker_position(app.doc(), i));
    let saved = app.doc().gk42.clone();
    let applied_ok = transform_of(app.doc()).is_some();
    seed(app, &saved);

    let preview = preview_transform(app);
    let step = next_step(app);
    ui.horizontal_wrapped(|ui| {
        let on = app.map_gk42.on;
        let label = if step < 3 {
            "gk42.place_points"
        } else {
            "gk42.pick_points"
        };
        let caption = if on && step < 3 {
            trf("gk42.point_progress", &[("n", &(step + 1).to_string())])
        } else {
            tr(label)
        };
        let mut capture = egui::Button::new(caption).min_size(Vec2::new(100.0, 25.0));
        if on {
            capture = capture.fill(Tokens::tool_active());
        }
        if ui.add(capture).clicked() {
            set_mode(app, !on);
        }
        if on {
            let hint = match step {
                0..=2 => None,
                _ => Some("gk42.pick_on_page"),
            };
            if let Some(hint) = hint {
                ui.label(dim(&tr(hint)));
            }
        }
    });
    if let Some((t, _)) = &preview {
        ui.label(dim(&trf(
            "gk42.scale_line",
            &[("sx", &fmt_1n(t.scale_x())), ("sy", &fmt_1n(t.scale_y()))],
        )));
    }

    // The calibration is saved but its markers are gone or degenerate.
    if saved.is_some() && !applied_ok {
        ui.label(rt(tr("gk42.broken")).color(Color32::from_rgb(230, 90, 90)));
    } else if applied_ok {
        ui.label(rt(tr("gk42.saved_hint")).color(Tokens::text_dim()));
    }

    ui.add_space(3.0);
    ui.separator();
    // The table.
    let cols = GkColumns::for_width(ui.available_width());
    let (mm_w, gk_w) = (cols.mm, cols.gk);
    let mut commits: Vec<(ShapeId, Point)> = Vec::new();
    egui::ScrollArea::horizontal()
        .id_salt("gk42_columns")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GkColumns::GAP;
                ui.add_space(GkColumns::NUMBER + GkColumns::GAP);
                centered_header(ui, 2.0 * mm_w + GkColumns::GAP, &tr("gk42.page_group"));
                centered_header(ui, 2.0 * gk_w + GkColumns::GAP, &tr("gk42.table_gk"));
            });
            egui::Grid::new("gk42_table")
                .num_columns(6)
                .striped(true)
                .spacing([GkColumns::GAP, 3.0])
                .show(ui, |ui| {
                    centered_header(ui, GkColumns::NUMBER, &tr("gk42.table_n"));
                    for (axis, width) in [("Y", mm_w), ("X", mm_w), ("Y", gk_w), ("X", gk_w)] {
                        centered_header(ui, width, axis);
                    }
                    centered_header(ui, GkColumns::DELTA, &tr("gk42.table_delta"))
                        .on_hover_text(tr("gk42.delta_tip"));
                    ui.end_row();
                    let t = preview.as_ref().map(|(t, _)| t);
                    if let Some(a) = marker_id(app, true) {
                        if let Some((id, target)) =
                            marker_row(ui, &mut app.map_gk42, 1, 0, a, mm1, mm_w, gk_w, None)
                        {
                            commits.push((id, target));
                        }
                        ui.end_row();
                    }
                    if let Some(b) = marker_id(app, false) {
                        let delta = match (mm1, mm2, t) {
                            (Some(p1), Some(p2), Some(t)) => Some(fmt_gk_plain(delta_m(p1, p2, t))),
                            _ => None,
                        };
                        if let Some((id, target)) =
                            marker_row(ui, &mut app.map_gk42, 2, 1, b, mm2, mm_w, gk_w, delta)
                        {
                            commits.push((id, target));
                        }
                        ui.end_row();
                    }
                    if let Some(c) = marker3_id(app) {
                        let delta = match (mm2, mm3, t) {
                            (Some(p2), Some(p3), Some(t)) => Some(fmt_gk_plain(delta_m(p2, p3, t))),
                            _ => None,
                        };
                        if let Some((id, target)) =
                            marker_row(ui, &mut app.map_gk42, 3, 2, c, mm3, mm_w, gk_w, delta)
                        {
                            commits.push((id, target));
                        }
                        ui.end_row();
                    }
                    let n = app.map_gk42.picked.len();
                    for k in 0..n {
                        picked_row(ui, &mut app.map_gk42, k, mm1, t);
                        ui.end_row();
                    }
                });
        });
    for (id, target) in commits {
        move_marker(app, id, target);
    }

    ui.separator();
    let draft_error = diagnose_draft(app);
    let can_apply = next_step(app) == 3 && draft_error.is_none();
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(tr("gk42.reset"))
            .rich_tip(keyed("gk42.reset"))
            .clicked()
        {
            reset(app);
        } else if ui
            .add_enabled(
                can_apply,
                if can_apply {
                    egui::Button::new(tr("gk42.apply")).fill(Tokens::tool_active())
                } else {
                    egui::Button::new(tr("gk42.apply"))
                },
            )
            .rich_tip_always(keyed("gk42.apply"))
            .clicked()
        {
            apply(app);
        }
    });
    if let Some(key) = draft_error.filter(|_| next_step(app) == 3) {
        ui.label(dim(&tr(key)));
    }

    ui.separator();
    ui.horizontal_wrapped(|ui| {
        for (key, sep, decimals) in [("gk42.copy", '\t', true), ("gk42.csv", ';', false)] {
            if ui.button(tr(key)).rich_tip(keyed(key)).clicked() {
                let copied = crate::clipboard::write_text(&table_text(app, sep, decimals));
                app.map_gk42.note = Some(tr(if copied {
                    "gk42.copied"
                } else {
                    "gk42.copy_failed"
                }));
            }
        }
        let st = &mut app.map_gk42;
        if ui
            .add_enabled(!st.picked.is_empty(), egui::Button::new(tr("gk42.clear")))
            .clicked()
        {
            st.cleared_points = std::mem::take(&mut st.picked);
        }
        if !st.cleared_points.is_empty() && ui.button(tr("gk42.restore_points")).clicked() {
            st.cleared_points.append(&mut st.picked);
            st.picked = std::mem::take(&mut st.cleared_points);
        }
    });

    // The note and the error of the last action.
    if let Some(note) = app.map_gk42.note.clone() {
        ui.label(rt(note).color(Tokens::text_dim()).size(11.0));
    }
    if let Some(e) = app.map_gk42.error.clone() {
        ui.label(rt(e).color(Color32::from_rgb(230, 90, 90)).size(11.0));
    }
}

struct GkColumns {
    mm: f32,
    gk: f32,
}

impl GkColumns {
    const NUMBER: f32 = 20.0;
    const DELTA: f32 = 50.0;
    const GAP: f32 = 3.0;
    const FIXED: f32 = Self::NUMBER + Self::DELTA + 5.0 * Self::GAP;

    fn for_width(width: f32) -> Self {
        let fields = (width - Self::FIXED - 4.0).clamp(196.0, 360.0);
        Self {
            mm: fields * (42.0 / 196.0),
            gk: fields * (56.0 / 196.0),
        }
    }
}

fn centered_header(ui: &mut Ui, width: f32, label: &str) -> egui::Response {
    ui.allocate_ui_with_layout(
        Vec2::new(width, 13.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| ui.label(dim(label)),
    )
    .inner
}

/// Seeding the coordinate fields from a saved calibration once, so a
/// reload shows what the file carries and an erased field stays empty.
fn seed(app: &mut App, saved: &Option<GkCalibration>) {
    if app.map_gk42.seeded {
        return;
    }
    let st = &mut app.map_gk42;
    st.seeded = true;
    if let Some(c) = saved {
        st.gk1 = (fmt_gk_plain(c.gk1.x), fmt_gk_plain(c.gk1.y));
        st.gk2 = (fmt_gk_plain(c.gk2.x), fmt_gk_plain(c.gk2.y));
        if let Some(g3) = c.gk3 {
            st.gk3 = (fmt_gk_plain(g3.x), fmt_gk_plain(g3.y));
        }
    }
}

fn dim(text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .color(Tokens::text_dim())
        .size(11.0)
}

fn rt(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text)
}

/// One marker row of the table: the two millimetre fields (the pair
/// moves the marker when either commits with Enter), the two GK-42
/// fields (the apply takes them) and the distance in the last cell.
/// `which` selects the state's field pair (0 to 2 = markers 1 to 3).
/// The columns read Y then X: the first field holds the axis's own
/// second value (mm y / easting), the second its first (mm x /
/// northing). Returns the move to run after the widgets, so the borrow
/// of the state ends first.
#[allow(clippy::too_many_arguments)]
fn marker_row(
    ui: &mut Ui,
    st: &mut Gk42State,
    n: u8,
    which: usize,
    id: ShapeId,
    mm: Option<Point>,
    mm_w: f32,
    gk_w: f32,
    delta: Option<String>,
) -> Option<(ShapeId, Point)> {
    let (mm_edit, gk_edit) = match which {
        0 => (&mut st.mm1, &mut st.gk1),
        1 => (&mut st.mm2, &mut st.gk2),
        _ => (&mut st.mm3, &mut st.gk3),
    };
    let [my, mx, gy, gx] = match which {
        0 => ["m1y", "m1x", "g1y", "g1x"],
        1 => ["m2y", "m2x", "g2y", "g2x"],
        _ => ["m3y", "m3x", "g3y", "g3x"],
    };
    let marker = match which {
        0 => Color32::from_rgb(228, 111, 114),
        1 => Color32::from_rgb(100, 169, 244),
        _ => Color32::from_rgb(240, 173, 78),
    };
    ui.label(rt(n.to_string()).monospace().color(marker));
    let live_y = mm.map(|p| fmt_mm(p.y)).unwrap_or_default();
    let live_x = mm.map(|p| fmt_mm(p.x)).unwrap_or_default();
    let commit_y = mm_field(
        ui,
        &mut mm_edit.1,
        &live_y,
        mm_w,
        ("gk42", my),
        "gk42.mm_y_tip",
    );
    let commit_x = mm_field(
        ui,
        &mut mm_edit.0,
        &live_x,
        mm_w,
        ("gk42", mx),
        "gk42.mm_x_tip",
    );
    gk_field(ui, &mut gk_edit.1, gk_w, ("gk42", gy), "gk42.gk_y_tip");
    gk_field(ui, &mut gk_edit.0, gk_w, ("gk42", gx), "gk42.gk_x_tip");
    ui.label(match delta {
        Some(d) => dim(&d),
        None => dim(""),
    });
    if commit_y || commit_x {
        if let (Some(x), Some(y)) = (parse_m(&mm_edit.0), parse_m(&mm_edit.1)) {
            return Some((id, Point::new(x, y)));
        }
    }
    None
}

/// One picked row: the click's millimetres, its GK-42 metres and the
/// distance to the previous point (the first one: to marker 1, whose
/// position comes in as `mm1`).
fn picked_row(
    ui: &mut Ui,
    st: &mut Gk42State,
    k: usize,
    mm1: Option<Point>,
    t: Option<&GkTransform>,
) {
    let Some(p) = st.picked.get(k).copied() else {
        return;
    };
    ui.label(dim(&(k + 1).to_string()));
    ui.label(dim(&fmt_mm(p.y)));
    ui.label(dim(&fmt_mm(p.x)));
    let Some(t) = t else {
        ui.label(dim(""));
        ui.label(dim(""));
        ui.label(dim(""));
        return;
    };
    let g = t.to_gk(p);
    ui.label(dim(&fmt_gk_plain(g.y)));
    ui.label(dim(&fmt_gk_plain(g.x)));
    let prev = if k == 0 {
        mm1
    } else {
        st.picked.get(k - 1).copied()
    };
    ui.label(match prev {
        Some(prev) => dim(&fmt_gk_plain(delta_m(prev, p, t))),
        None => dim(""),
    });
}

/// A millimetre field of a marker row: mirrors the marker while not
/// focused (the commit frame keeps the typed text), and says when the
/// field committed with Enter, so the caller moves the marker once.
fn mm_field(
    ui: &mut Ui,
    text: &mut String,
    live: &str,
    width: f32,
    id: (&str, &str),
    tip: &str,
) -> bool {
    let r = ui
        .add(
            TextEdit::singleline(text)
                .desired_width(width)
                .min_size(Vec2::new(width, 20.0))
                .margin(egui::Margin::symmetric(4, 2))
                .vertical_align(egui::Align::Center)
                .font(FontId::monospace(11.5))
                .id(egui::Id::new(id)),
        )
        .on_hover_text(tr(tip));
    let committed = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if !r.has_focus() && !committed {
        *text = live.to_string();
    }
    committed
}

/// A GK-42 metres field of the panel: the user's draft, committed by
/// the apply button.
fn gk_field(ui: &mut Ui, text: &mut String, width: f32, id: (&str, &str), tip: &str) {
    ui.add(
        TextEdit::singleline(text)
            .desired_width(width)
            .min_size(Vec2::new(width, 20.0))
            .margin(egui::Margin::symmetric(4, 2))
            .vertical_align(egui::Align::Center)
            .font(FontId::proportional(11.5))
            .id(egui::Id::new(id)),
    )
    .on_hover_text(tr(tip));
}

/// A scale as 1:N with the N grouped: 10000 to "1:10 000".
fn fmt_1n(n: f64) -> String {
    let n = (n * 100.0).round() / 100.0;
    let int = n.floor() as i64;
    let frac = ((n - int as f64) * 100.0).round() as i64;
    if frac == 0 {
        format!("1:{}", group(int))
    } else {
        format!("1:{},{:02}", group(int), frac)
    }
}

/// The table as text: the markers and the picked points with their
/// coordinates. `human` uses the display formats (spaces, commas),
/// otherwise plain numbers with dots, as CSV wants. The columns read Y
/// then X, matching the panel.
fn table_text(app: &App, sep: char, human: bool) -> String {
    let st = &app.map_gk42;
    let doc = app.doc();
    let fmt = |v: f64| fmt_gk_plain(v);
    let mm = |v: f64| {
        if human {
            fmt_mm(v)
        } else {
            format!("{v}")
        }
    };
    let mut out = String::new();
    let head = [
        tr("gk42.table_n"),
        format!("{} Y", tr("gk42.table_mm")),
        format!("{} X", tr("gk42.table_mm")),
        format!("Y, {}", tr("gk42.table_gk")),
        format!("X, {}", tr("gk42.table_gk")),
        tr("gk42.table_delta"),
    ]
    .join(&sep.to_string());
    out.push_str(&head);
    out.push('\n');
    let t = preview_transform(app).map(|(t, _)| t);
    for (n, id) in [
        (1, marker_id(app, true)),
        (2, marker_id(app, false)),
        (3, marker3_id(app)),
    ] {
        let Some(id) = id else { continue };
        let Some(p) = marker_position(doc, id) else {
            continue;
        };
        let (gx, gy, d) = match &t {
            Some(t) => {
                let g = t.to_gk(p);
                (fmt(g.x), fmt(g.y), String::new())
            }
            None => (String::new(), String::new(), String::new()),
        };
        out.push_str(&[n.to_string(), mm(p.y), mm(p.x), gy, gx, d].join(&sep.to_string()));
        out.push('\n');
    }
    for (k, p) in st.picked.iter().enumerate() {
        let (gx, gy, d) = match &t {
            Some(t) => {
                let g = t.to_gk(*p);
                let prev = if k == 0 {
                    marker_id(app, true).and_then(|i| marker_position(doc, i))
                } else {
                    st.picked.get(k - 1).copied()
                };
                let d = prev
                    .map(|prev| fmt(delta_m(prev, *p, t)))
                    .unwrap_or_default();
                (fmt(g.x), fmt(g.y), d)
            }
            None => (String::new(), String::new(), String::new()),
        };
        out.push_str(&[(k + 1).to_string(), mm(p.y), mm(p.x), gy, gx, d].join(&sep.to_string()));
        out.push('\n');
    }
    out
}

/// Export the objects directly on the layer, with the same axis order as the docker.
pub(crate) fn export_layer(app: &mut App, layer_id: LayerId) {
    let Some((transform, _)) = preview_transform(app) else {
        app.status = tr("gk42.export_calibrate");
        return;
    };
    let doc = app.doc();
    let Ok(layer) = doc.layer(layer_id) else {
        return;
    };
    let rows = layer_export_rows(&layer.shapes, &doc.symbols, &transform);
    let bytes = match tracedraw_io::xlsx::write_rows_with_text_columns(&rows, &[], &[0]) {
        Ok(bytes) => bytes,
        Err(error) => {
            app.status = error;
            return;
        }
    };
    let name = crate::files::clean_file_name(&layer.name);
    crate::files::Dialog::new()
        .add_filter(tr("gk42.export_workbook"), &["xlsx"])
        .set_file_name(format!("{name}-GK42.xlsx"))
        .save_bytes(app, bytes);
}

fn layer_export_rows(
    shapes: &[Shape],
    symbols: &[Symbol],
    transform: &GkTransform,
) -> Vec<Vec<String>> {
    let mut rows = vec![vec![
        tr("gk42.export_label"),
        format!("Y, {}", tr("gk42.table_gk")),
        format!("X, {}", tr("gk42.table_gk")),
    ]];
    rows.extend(shapes.iter().map(|shape| {
        let (label, anchor) = match &shape.kind {
            ShapeKind::Text { spans, .. } => (spans_text(spans), Some((shape, Affine::IDENTITY))),
            ShapeKind::Group { children } => {
                let (text, anchor) = group_export_fields(children, shape.transform, symbols);
                let label = if text.is_empty() {
                    crate::ui::objects_docker::object_label(shape)
                } else {
                    text.join("\n")
                };
                (label, anchor)
            }
            _ => (
                crate::ui::objects_docker::object_label(shape),
                Some((shape, Affine::IDENTITY)),
            ),
        };
        let (y, x) = anchor
            .map(|(anchor, parent)| {
                transform.to_gk(parent * anchor.visual_bounds_with(symbols).center())
            })
            .filter(|gk| gk.y.is_finite() && gk.x.is_finite())
            .map(|gk| (fmt_gk_plain(gk.y), fmt_gk_plain(gk.x)))
            .unwrap_or_default();
        vec![label, y, x]
    }));
    rows
}

/// Visit each direct child's subtree in drawing order. A text-free direct
/// subgroup is a fallback, and a symbol anywhere in the group takes precedence.
fn group_export_fields<'a>(
    children: &'a [Shape],
    parent: Affine,
    symbols: &[Symbol],
) -> (Vec<String>, Option<(&'a Shape, Affine)>) {
    let mut text = Vec::new();
    let mut first_symbol = None;
    let mut fallback = None;
    let mut stack = Vec::new();
    for child in children {
        let mut has_text = false;
        stack.push((child, parent));
        while let Some((shape, transform)) = stack.pop() {
            match &shape.kind {
                ShapeKind::Text { spans, .. } => {
                    has_text = true;
                    let value = spans_text(spans);
                    if !value.trim().is_empty() {
                        text.push(value);
                    }
                }
                ShapeKind::Group { children } => {
                    let transform = transform * shape.transform;
                    stack.extend(children.iter().rev().map(|child| (child, transform)));
                }
                ShapeKind::SymbolInstance { index } if symbols.get(*index).is_some() => {
                    first_symbol.get_or_insert((shape, transform));
                }
                _ => {}
            }
        }
        if !has_text && matches!(&child.kind, ShapeKind::Group { .. }) {
            fallback.get_or_insert((child, parent));
        }
    }
    (text, first_symbol.or(fallback))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracedraw_core::document::TextSpan;

    #[test]
    fn layer_export_uses_group_text_and_symbol_or_text_free_subgroup_centres() {
        let mut next_id = 0;
        let mut shape = |kind, x, y| {
            next_id += 1;
            let mut shape = Shape::new(ShapeId(next_id), kind);
            shape.transform = Affine::translate((x, y));
            shape
        };
        let rect = || ShapeKind::Rect {
            rect: Rect::new(0.0, 0.0, 4.0, 6.0),
            radius: 0.0,
            corners: None,
        };
        let text = |value| ShapeKind::Text {
            spans: vec![TextSpan::new(value, "sans-serif", 12.0)],
            origin: Point::ORIGIN,
            frame: None,
            align: Default::default(),
            para: Default::default(),
            on_path: None,
        };
        let mut named = shape(rect(), 0.0, 0.0);
        named.name = Some("0042".into());
        let standalone = shape(text("Station"), 0.0, 0.0);
        let a = shape(text("A"), 0.0, 0.0);
        let with_text = shape(ShapeKind::Group { children: vec![a] }, 2.0, 4.0);
        let rectangle = shape(rect(), 5.0, 6.0);
        let fallback = shape(
            ShapeKind::Group {
                children: vec![rectangle],
            },
            7.0,
            8.0,
        );
        let b = shape(text("B"), 0.0, 0.0);
        let fallback_group = shape(
            ShapeKind::Group {
                children: vec![with_text, fallback.clone(), b],
            },
            20.0,
            30.0,
        );
        let symbol = shape(ShapeKind::SymbolInstance { index: 0 }, 10.0, 20.0);
        let nested = shape(
            ShapeKind::Group {
                children: vec![symbol],
            },
            30.0,
            40.0,
        );
        let c = shape(text("C"), 0.0, 0.0);
        let symbol_group = shape(
            ShapeKind::Group {
                children: vec![c, fallback, nested],
            },
            50.0,
            60.0,
        );
        let d = shape(text("D"), 0.0, 0.0);
        let no_anchor = shape(ShapeKind::Group { children: vec![d] }, 0.0, 0.0);
        let symbols = [Symbol {
            name: "Marker".into(),
            shape: shape(rect(), 0.0, 0.0),
        }];
        let transform = GkTransform {
            e: [1.0, 0.0, 7_000_000.0],
            n: [0.0, 1.0, 6_000_000.0],
        };
        let rows = layer_export_rows(
            &[named, standalone, fallback_group, symbol_group, no_anchor],
            &symbols,
            &transform,
        );
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[1], ["0042", "7000002", "6000003"]);
        assert_eq!(rows[2][0], "Station");
        assert_eq!(rows[3], ["A\nB", "7000034", "6000047"]);
        assert_eq!(rows[4], ["C", "7000092", "6000123"]);
        assert_eq!(rows[5], ["D", "", ""]);
        let bytes = tracedraw_io::xlsx::write_rows_with_text_columns(&rows, &[], &[0]).unwrap();
        assert_eq!(tracedraw_io::xlsx::read_grid(&bytes).unwrap(), rows);
    }

    use crate::tools::Tool;

    /// The parser takes the map's thousand groups, the comma and the
    /// apostrophe, and refuses the empty and the broken.
    #[test]
    fn parse_m_takes_spaces_commas_and_refuses_garbage() {
        assert_eq!(parse_m("6 519 000"), Some(6_519_000.0));
        assert_eq!(parse_m("6320669"), Some(6_320_669.0));
        assert_eq!(parse_m("42\u{00A0}531\u{2009}250"), Some(42_531_250.0));
        assert_eq!(parse_m("6523'200"), Some(6523200.0));
        assert_eq!(parse_m("42 531 250,5"), Some(42_531_250.5));
        assert_eq!(parse_m("42 531 250.5"), Some(42_531_250.5));
        assert_eq!(parse_m(""), None);
        assert_eq!(parse_m("   "), None);
        assert_eq!(parse_m("abc"), None);
        assert_eq!(parse_m("1e999"), None);
    }

    /// The display format groups by thousands and keeps the fraction,
    /// and the parser reads its output back.
    #[test]
    fn fmt_m_groups_and_round_trips_through_parse_m() {
        assert_eq!(fmt_m(6_519_000.0), "6 519 000");
        assert_eq!(fmt_m(-42_531_250.5), "-42 531 250,50");
        assert_eq!(fmt_m(1234.567), "1 234,57");
        assert_eq!(fmt_m(0.999), "1");
        for v in [0.0, 1234.56, -42_531_250.5, 6_519_000.0] {
            let back = parse_m(&fmt_m(v)).unwrap();
            assert!((back - v).abs() < 0.005, "{v} -> {back}");
        }
    }

    /// Plain GK-42 fields carry whole metres without spaces or decimals;
    /// the cursor plate uses the same digits and rounding as the table.
    #[test]
    fn gk_readout_formats_whole_metres_in_table_order() {
        assert_eq!(fmt_gk_plain(7_491_069.93), "7491070");
        assert_eq!(fmt_gk_plain(5_416_374.28), "5416374");
        let g = GkPoint {
            x: 5_416_374.28,
            y: 42_531_000.0,
        };
        assert_eq!(gk_readout(g), "GK-42: Y 42531000  X 5416374");
    }

    /// The deltas use each axis's own scale: a millimetre right and a
    /// millimetre up on a stretched map are different distances.
    #[test]
    fn coordinate_columns_fit_a_narrow_docker() {
        for width in [285.0, 320.0, 385.0] {
            let cols = GkColumns::for_width(width);
            assert!(cols.mm >= 42.0 && cols.gk >= 56.0);
            assert!(2.0 * (cols.mm + cols.gk) + GkColumns::FIXED <= width);
        }
    }

    #[test]
    fn delta_m_scales_each_axis() {
        let t = GkTransform {
            e: [10.0, 0.0, 0.0],
            n: [0.0, 12.0, 0.0],
        };
        assert_eq!(
            delta_m(Point::new(0.0, 0.0), Point::new(3.0, 0.0), &t),
            30.0
        );
        assert_eq!(
            delta_m(Point::new(0.0, 0.0), Point::new(0.0, 3.0), &t),
            36.0
        );
        // 3 and 4 scale along their own axes (30 and 48) before the sum
        // goes under the root, so the diagonal is not a 3-4-5 any more.
        assert!(
            (delta_m(Point::new(0.0, 0.0), Point::new(3.0, 4.0), &t)
                - (30.0f64 * 30.0 + 48.0 * 48.0).sqrt())
            .abs()
                < 1e-9
        );
    }

    /// A scale like 1:10 000 prints with the group spaces.
    #[test]
    fn fmt_1n_groups_the_denominator() {
        assert_eq!(fmt_1n(10_000.0), "1:10 000");
        assert_eq!(fmt_1n(55_555.555), "1:55 555,56");
    }

    /// The full flow headlessly: two clicks place the markers, the
    /// typed coordinates apply, the transform comes out, the reset
    /// takes everything back including the marker shapes.
    #[test]
    fn click_flow_places_applies_and_resets() {
        let mut app = App::headless();
        app.set_tool(Tool::Pick);
        set_mode(&mut app, true);
        assert!(app.map_gk42.on);
        click(&mut app, Point::new(50.0, 50.0));
        assert_eq!(app.map_gk42.step, 1);
        click(&mut app, Point::new(150.0, 150.0));
        assert_eq!(app.map_gk42.step, 2);
        click(&mut app, Point::new(50.0, 150.0));
        assert_eq!(app.map_gk42.step, 3);
        let layer_name = tr("gk42.layer");
        let doc = app.doc();
        let layer = doc
            .pages
            .iter()
            .flat_map(|p| &p.layers)
            .find(|l| l.name == layer_name)
            .expect("the markers layer");
        assert_eq!(layer.shapes.len(), 3);
        let id1 = app.map_gk42.marker1.unwrap();
        let id2 = app.map_gk42.marker2.unwrap();
        let id3 = app.map_gk42.marker3.unwrap();
        let s1 = doc.find_shape(id1).expect("marker 1");
        match &s1.kind {
            ShapeKind::Ellipse { rect, .. } => {
                let c = rect.center();
                assert!((c.x - 50.0).abs() < 1e-9 && (c.y - 50.0).abs() < 1e-9);
            }
            other => panic!("an ellipse marker, got {other:?}"),
        }
        // The typed coordinates: zone 42, the scales 1:50 000 and
        // 1:42 000 for the 100 mm between the markers.
        app.map_gk42.gk1 = ("6 519 000".into(), "42 531 000".into());
        app.map_gk42.gk2 = ("6 523 200".into(), "42 536 000".into());
        app.map_gk42.gk3 = ("6 523 200".into(), "42 531 000".into());
        assert!(apply(&mut app));
        let (t, cal) = transform_of(app.doc()).expect("applied");
        assert_eq!(cal.zone(), Some(42));
        assert!((t.e[0] - 50.0).abs() < 1e-9);
        assert!((t.n[1] - 42.0).abs() < 1e-9);
        assert!((t.scale_x() - 50_000.0).abs() < 1e-6);
        // Two more clicks: picked points, overlay-only.
        click(&mut app, Point::new(100.0, 100.0));
        click(&mut app, Point::new(120.0, 110.0));
        assert_eq!(app.map_gk42.picked.len(), 2);
        // The reset: the calibration and the markers go, and the whole
        // reset comes back as one undo step.
        reset(&mut app);
        assert_eq!(app.doc().gk42, None);
        assert!(app.doc().find_shape(id1).is_none());
        assert!(app.doc().find_shape(id2).is_none());
        assert!(app.doc().find_shape(id3).is_none());
        assert!(app.map_gk42.picked.is_empty());
        app.engine.undo().expect("the reset is one step");
        assert!(app.doc().gk42.is_some());
        assert!(app.doc().find_shape(id1).is_some());
        assert!(app.doc().find_shape(id2).is_some());
        assert!(app.doc().find_shape(id3).is_some());
    }

    /// The mm fields move the marker: moving marker 1 stretches the
    /// scales the calibration implies.
    #[test]
    fn mm_fields_move_the_marker_and_recalibrate() {
        let mut app = App::headless();
        set_mode(&mut app, true);
        click(&mut app, Point::new(50.0, 50.0));
        click(&mut app, Point::new(150.0, 150.0));
        click(&mut app, Point::new(50.0, 150.0));
        app.map_gk42.gk1 = ("6519000".into(), "42531000".into());
        app.map_gk42.gk2 = ("6523200".into(), "42536000".into());
        app.map_gk42.gk3 = ("6523200".into(), "42531000".into());
        assert!(apply(&mut app));
        let id1 = app.map_gk42.marker1.unwrap();
        assert!(move_marker(&mut app, id1, Point::new(60.0, 50.0)));
        let (t, _) = transform_of(app.doc()).expect("still applied");
        // Marker 1 moved 10 mm right: the three markers now imply a shear
        // of 5 m per mm along Y, the X scale stays 50 m per mm.
        assert!((t.e[0] - 50.0).abs() < 1e-9);
        assert!((t.e[1] - 5.0).abs() < 1e-9);
    }

    /// The apply refuses disagreeing zones and non-parsing fields, and
    /// says so in the panel's error line.
    #[test]
    fn apply_refuses_zones_that_disagree_and_garbage() {
        let mut app = App::headless();
        set_mode(&mut app, true);
        click(&mut app, Point::new(50.0, 50.0));
        click(&mut app, Point::new(150.0, 150.0));
        click(&mut app, Point::new(50.0, 150.0));
        app.map_gk42.gk1 = ("6519000".into(), "42531000".into());
        app.map_gk42.gk2 = ("6523200".into(), "43536000".into());
        app.map_gk42.gk3 = ("6523200".into(), "42531000".into());
        assert!(!apply(&mut app));
        assert_eq!(app.doc().gk42, None);
        assert!(app.map_gk42.error.is_some());
        app.map_gk42.gk1 = ("6519000".into(), "42531000".into());
        app.map_gk42.gk2 = ("bicycle".into(), "42536000".into());
        assert!(!apply(&mut app));
        app.map_gk42.gk2 = ("6523200".into(), "42536000".into());
        assert!(apply(&mut app));
        assert!(app.map_gk42.error.is_none());
    }

    /// The floating window next to the cursor carries both the real
    /// coordinates (Y then X, as in the table) and the GK-42 coordinates
    /// (whole metres, Y easting then X northing) as soon
    /// as the markers have GK-42 values, before and after Apply, and
    /// even when the pick mode is switched off.
    #[test]
    fn cursor_plate_matches_the_picked_point_table() {
        let mut app = App::headless();
        set_mode(&mut app, true);
        click(&mut app, Point::new(50.0, 50.0));
        click(&mut app, Point::new(150.0, 150.0));
        click(&mut app, Point::new(50.0, 150.0));
        app.map_gk42.gk1 = ("5416000".into(), "7490000".into());
        app.map_gk42.gk2 = ("5416748.56".into(), "7492139.86".into());
        app.map_gk42.gk3 = ("5416748.56".into(), "7490000".into());
        app.pointer_page = Some(Point::new(120.0, 80.0));

        // Before Apply: draft GK-42 coordinates already resolve on the cursor plate.
        let before = cursor_lines(&app).expect("cursor lines");
        assert_eq!(before.len(), 2, "{before:?}");
        assert_eq!(before[0], "Y: 80.00  X: 120.00 mm");
        assert_eq!(before[1], "GK-42: Y 7491498  X 5416225");
        assert!(!before[1].contains(','));

        // After Apply with pick mode turned off: the cursor plate still shows both.
        assert!(apply(&mut app));
        set_mode(&mut app, false);
        let after = cursor_lines(&app).expect("cursor lines with mode off");
        assert_eq!(after, before);
        let point = Point::new(120.0, 80.0);
        click(&mut app, point);
        let table = table_text(&app, ';', false);
        let row: Vec<_> = table
            .lines()
            .last()
            .expect("picked point")
            .split(';')
            .collect();
        assert_eq!(&row[1..3], &["80", "120"]);
        assert_eq!(after[1], format!("GK-42: Y {}  X {}", row[3], row[4]));
    }

    /// The copied table and the CSV put the Y column before the X one,
    /// in the header and in every row, just like the docker's grid.
    /// The saved calibration still carries the same two axes.
    #[test]
    fn table_text_writes_y_then_x() {
        let mut app = App::headless();
        set_mode(&mut app, true);
        click(&mut app, Point::new(50.0, 50.0));
        click(&mut app, Point::new(150.0, 150.0));
        click(&mut app, Point::new(50.0, 150.0));
        app.map_gk42.gk1 = ("6 519 000".into(), "42 531 000".into());
        app.map_gk42.gk2 = ("6 523 200".into(), "42 536 000".into());
        app.map_gk42.gk3 = ("6 523 200".into(), "42 531 000".into());
        assert!(apply(&mut app), "error: {:?}", app.map_gk42.error);
        click(&mut app, Point::new(100.0, 100.0));
        for sep in ['\t', ';'] {
            let text = table_text(&app, sep, sep == '\t');
            let lines: Vec<&str> = text.lines().collect();
            let head: Vec<&str> = lines[0].split(sep).collect();
            assert_eq!(head[1], format!("{} Y", tr("gk42.table_mm")));
            assert_eq!(head[2], format!("{} X", tr("gk42.table_mm")));
            assert_eq!(head[3], format!("Y, {}", tr("gk42.table_gk")));
            assert_eq!(head[4], format!("X, {}", tr("gk42.table_gk")));
            let mark: Vec<&str> = lines[1].split(sep).collect();
            assert_eq!((mark[3], mark[4]), ("42531000", "6519000"));
            let pick: Vec<&str> = lines[4].split(sep).collect();
            assert_eq!(pick[1], if sep == '\t' { "100.00" } else { "100" });
            assert_eq!((pick[3], pick[4]), ("42533500", "6521100"));
        }
        let (_, cal) = transform_of(app.doc()).expect("calibration");
        assert_eq!(cal.gk1.x, 6_519_000.0);
        assert_eq!(cal.gk1.y, 42_531_000.0);
    }

    #[test]
    fn labelled_axes_reject_swapped_seven_digit_coordinates() {
        let mut app = App::headless();
        set_mode(&mut app, true);
        click(&mut app, Point::new(50.0, 50.0));
        click(&mut app, Point::new(150.0, 150.0));
        click(&mut app, Point::new(50.0, 150.0));
        app.map_gk42.gk1 = ("5998000".into(), "4312000".into());
        app.map_gk42.gk2 = ("6002000".into(), "4316000".into());
        app.map_gk42.gk3 = ("6002000".into(), "4312000".into());
        assert!(apply(&mut app));
        let (t, cal) = transform_of(app.doc()).expect("valid calibration");
        assert_eq!(cal.zone(), Some(4));
        assert!((t.e[0] - 40.0).abs() < 1e-9);
        assert!((t.n[1] - 40.0).abs() < 1e-9);
        for pair in [
            &mut app.map_gk42.gk1,
            &mut app.map_gk42.gk2,
            &mut app.map_gk42.gk3,
        ] {
            std::mem::swap(&mut pair.0, &mut pair.1);
        }
        assert!(!apply(&mut app));
        assert_eq!(app.doc().gk42.as_ref(), Some(&cal));
        assert!(preview_transform(&app).expect("saved fallback").1);
    }
}

//! Safe recompression of a plain stored PNG without changing its samples.
use crate::app::App;
use crate::i18n::{tr, trf};
use egui::{Color32, ColorImage, TextureHandle};
use image::RgbaImage;
use std::sync::{Arc, Mutex};
use tracedraw_core::{Command, Shape, ShapeId, ShapeKind, SharedBytes};

const MAX_PIXELS: u64 = 12_000_000;
const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
const UNSUPPORTED: &str = "opt_raster.unsupported";
const FAILED: &str = "opt_raster.failed";

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Color,
    Alpha,
    Difference,
}

#[derive(Debug)]
struct Candidate {
    bytes: SharedBytes,
    before: RgbaImage,
    after: RgbaImage,
    colors: usize,
    alpha_used: bool,
    alpha_channel: bool,
}

type Job = Arc<Mutex<Option<Result<Arc<Candidate>, &'static str>>>>;

#[derive(Clone)]
pub struct State {
    id: ShapeId,
    instance: u64,
    revision: u64,
    source: SharedBytes,
    dims: (u32, u32),
    dpi: (f64, f64),
    job: Option<Job>,
    result: Option<Arc<Candidate>>,
    error: Option<&'static str>,
    best: bool,
    mode: Mode,
    background: u8,
    x: f32,
    y: f32,
    zoom: f32,
    textures: Option<(TextureHandle, TextureHandle)>,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OptimizeRaster")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl PartialEq for State {
    fn eq(&self, other: &Self) -> bool {
        (self.id, self.instance, self.revision) == (other.id, other.instance, other.revision)
    }
}

fn source(shape: &Shape) -> Option<(&SharedBytes, (u32, u32))> {
    if shape.cmyk_bitmap || shape.picture_inks.is_some() || shape.locked {
        return None;
    }
    match &shape.kind {
        ShapeKind::Bitmap {
            png,
            width_px,
            height_px,
            fx: None,
            ..
        } if tracedraw_core::jpeg_source::jpeg_of(png).is_none() => {
            Some((png, (*width_px, *height_px)))
        }
        _ => None,
    }
}

pub fn open(app: &mut App) {
    let Some(id) = app.selected_bitmap().filter(|_| app.selection.len() == 1) else {
        return;
    };
    let Ok((layer, shape)) = app.doc().shape(id) else {
        return;
    };
    let dims = match shape.kind {
        ShapeKind::Bitmap {
            width_px,
            height_px,
            ..
        } => (width_px, height_px),
        _ => return,
    };
    let size = app
        .doc()
        .shape_on_page(id)
        .and_then(|s| {
            let ShapeKind::Bitmap { rect, .. } = s.kind else {
                return None;
            };
            let t = s.transform.as_coeffs();
            Some((
                rect.width().abs() * t[0].hypot(t[1]),
                rect.height().abs() * t[2].hypot(t[3]),
            ))
        })
        .unwrap_or_default();
    let dpi = (
        f64::from(dims.0) * 25.4 / size.0,
        f64::from(dims.1) * 25.4 / size.1,
    );
    let data = source(shape).filter(|_| !layer.locked && !app.selected_bitmap_is_linked());
    let mut st = State {
        id,
        dims,
        dpi,
        instance: app.engine.instance(),
        revision: app.engine.revision(),
        source: data.map_or_else(|| Vec::new().into(), |(bytes, _)| bytes.clone()),
        job: None,
        result: None,
        error: data.is_none().then_some(UNSUPPORTED),
        best: true,
        mode: Mode::Color,
        background: 0,
        x: 0.5,
        y: 0.5,
        zoom: 1.0,
        textures: None,
    };
    if st.error.is_none() {
        start(&mut st);
    }
    app.dialog = super::dialogs::Dialog::OptimizeRaster(Box::new(st));
}

/// Only allow chunks whose semantics are covered by raw RGB(A) comparison.
/// In particular do not drop profiles, print metadata or additional frames.
fn plain_png(data: &[u8], dims: (u32, u32)) -> Result<bool, &'static str> {
    if data.len() > MAX_SOURCE_BYTES || u64::from(dims.0) * u64::from(dims.1) > MAX_PIXELS {
        return Err("opt_raster.limit");
    }
    let [w, h] = [dims.0, dims.1];
    if w == 0
        || h == 0
        || !data.starts_with(b"\x89PNG\r\n\x1a\n")
        || data.get(8..16) != Some(b"\0\0\0\x0dIHDR")
        || data.get(16..20) != Some(&w.to_be_bytes())
        || data.get(20..24) != Some(&h.to_be_bytes())
        || data.get(24) != Some(&8)
    {
        return Err(UNSUPPORTED);
    }
    let alpha = match data.get(25) {
        Some(2) => false,
        Some(6) => true,
        _ => return Err(UNSUPPORTED),
    };
    let mut rest = data.get(33..).ok_or(FAILED)?;
    while let Some([a, b, c, d, kind @ ..]) = rest.get(..8) {
        let size = u32::from_be_bytes([*a, *b, *c, *d]) as usize;
        let end = size.checked_add(12).ok_or(FAILED)?;
        let next = rest.get(end..).ok_or(FAILED)?;
        match kind {
            b"IDAT" => rest = next,
            b"IEND" if size == 0 && next.is_empty() => return Ok(alpha),
            _ => return Err(UNSUPPORTED),
        }
    }
    Err(FAILED)
}

fn analyze(bytes: &[u8], dims: (u32, u32), best: bool) -> Result<Candidate, &'static str> {
    use image::{
        ColorType, ImageEncoder, ImageFormat,
        codecs::png::{CompressionType, FilterType, PngEncoder},
    };
    let alpha_channel = plain_png(bytes, dims)?;
    let decode = |b| image::load_from_memory_with_format(b, ImageFormat::Png).map_err(|_| FAILED);
    let (before, encoded) = {
        let image = decode(bytes)?;
        let mut encoded = Vec::new();
        PngEncoder::new_with_quality(
            &mut encoded,
            if best {
                CompressionType::Best
            } else {
                CompressionType::Default
            },
            FilterType::Adaptive,
        )
        .write_image(
            image.as_bytes(),
            dims.0,
            dims.1,
            if alpha_channel {
                ColorType::Rgba8
            } else {
                ColorType::Rgb8
            }
            .into(),
        )
        .map_err(|_| FAILED)?;
        (image.into_rgba8(), encoded)
    };
    let after = decode(&encoded)?.into_rgba8();
    if before != after {
        return Err(FAILED);
    }
    let mut colors = std::collections::HashSet::new();
    let mut alpha_used = false;
    for px in before.pixels() {
        alpha_used |= px.0[3] != 255;
        if colors.len() < 257 {
            colors.insert(px.0);
        }
    }
    Ok(Candidate {
        bytes: encoded.into(),
        before,
        after,
        colors: colors.len(),
        alpha_used,
        alpha_channel,
    })
}

fn start(st: &mut State) {
    if crate::files::WEB || st.job.is_some() {
        return;
    }
    st.result = None;
    st.error = None;
    st.textures = None;
    let bytes = st.source.clone();
    let (dims, best) = (st.dims, st.best);
    let job = Job::default();
    let output = job.clone();
    #[cfg(not(target_arch = "wasm32"))]
    if std::thread::Builder::new()
        .name("optimize-raster".into())
        .spawn(move || {
            let value = analyze(&bytes, dims, best).map(Arc::new);
            if let Ok(mut result) = output.lock() {
                *result = Some(value);
            }
        })
        .is_ok()
    {
        st.job = Some(job);
        return;
    }
    st.error = Some(FAILED);
}

fn replacement(app: &App, st: &State, candidate: &Candidate) -> Option<Command> {
    if app.engine.instance() != st.instance
        || app.engine.revision() != st.revision
        || candidate.bytes.len() >= st.source.len()
    {
        return None;
    }
    let (layer, shape) = app.doc().shape(st.id).ok()?;
    if layer.locked || app.selected_bitmap_is_linked() {
        return None;
    }
    let (bytes, dims) = source(shape)?;
    if dims != st.dims || !SharedBytes::ptr_eq(bytes, &st.source) {
        return None;
    }
    let mut updated = shape.clone();
    if let ShapeKind::Bitmap { png, .. } = &mut updated.kind {
        *png = candidate.bytes.clone();
    }
    Some(Command::ReplaceShapes {
        replacements: vec![(st.id, updated)],
    })
}

fn preview(result: &Candidate, st: &State, after: bool) -> ColorImage {
    let image = if after { &result.after } else { &result.before };
    let w = image.width();
    let h = image.height();
    let scale = (w as f32 / 300.0).max(h as f32 / 220.0) / st.zoom;
    let span_w = (300.0 * scale).min(w as f32);
    let span_h = (220.0 * scale).min(h as f32);
    let x0 = (w as f32 - span_w) * st.x;
    let y0 = (h as f32 - span_h) * st.y;
    let sample = |px: u32, py: u32| {
        // Fit first, then zoom and pan the same undistorted region on both sides.
        let rx = (px as f32 + 0.5 - 150.0) * scale + span_w * 0.5;
        let ry = (py as f32 + 0.5 - 110.0) * scale + span_h * 0.5;
        if rx < 0.0 || ry < 0.0 || rx >= span_w || ry >= span_h {
            return Color32::from_gray(45);
        }
        let x = ((x0 + rx) as u32).min(w - 1);
        let y = ((y0 + ry) as u32).min(h - 1);
        let [r, g, b, a] = image.get_pixel(x, y).0;
        match st.mode {
            Mode::Alpha => Color32::from_gray(a),
            Mode::Difference if after => {
                if result.before.get_pixel(x, y) == result.after.get_pixel(x, y) {
                    Color32::BLACK
                } else {
                    Color32::LIGHT_RED
                }
            }
            _ => {
                let bg: u8 = match st.background {
                    1 => 255,
                    2 => 0,
                    _ if (px / 12 + py / 12) % 2 == 0 => 184,
                    _ => 225,
                };
                let mix = |v| {
                    ((u16::from(v) * u16::from(a) + u16::from(bg) * u16::from(255 - a) + 127) / 255)
                        as u8
                };
                Color32::from_rgb(mix(r), mix(g), mix(b))
            }
        }
    };
    ColorImage::new(
        [300, 220],
        (0..220)
            .flat_map(|py| (0..300).map(move |px| sample(px, py)))
            .collect(),
    )
}

pub fn dialog(app: &mut App, ctx: &egui::Context, st: &mut State, close: &mut bool) {
    if let Some(job) = &st.job {
        let done = job.lock().ok().and_then(|mut slot| slot.take());
        if let Some(done) = done {
            st.job = None;
            match done {
                Ok(result) => st.result = Some(result),
                Err(err) => st.error = Some(err),
            }
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
    }
    let mut open = true;
    egui::Window::new(tr("menu.bitmaps.optimize"))
        .id(egui::Id::new("optimize_raster"))
        .open(&mut open)
        .default_width(640.0)
        .show(ctx, |ui| {
            let (w, h) = st.dims;
            ui.label(trf(
                "opt_raster.info",
                &[
                    ("w", &w.to_string()),
                    ("h", &h.to_string()),
                    ("x", &format!("{:.0}", st.dpi.0)),
                    ("y", &format!("{:.0}", st.dpi.1)),
                    (
                        "mb",
                        &format!("{:.1}", f64::from(w) * f64::from(h) * 4.0 / 1_048_576.0),
                    ),
                ],
            ));
            if let Some(err) = st.error {
                ui.label(tr(err));
            }
            if st.job.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tr("opt_raster.working"));
                });
            }
            if let Some(result) = st.result.clone() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(tr("opt_raster.compression"));
                    if ui
                        .selectable_label(st.best, tr("opt_raster.best"))
                        .clicked()
                        && !st.best
                    {
                        st.best = true;
                        start(st);
                    }
                    if ui
                        .selectable_label(!st.best, tr("opt_raster.standard"))
                        .clicked()
                        && st.best
                    {
                        st.best = false;
                        start(st);
                    }
                });
                if st.job.is_some() {
                    return;
                }
                ui.label(trf(
                    "opt_raster.sizes",
                    &[
                        ("a", &st.source.len().to_string()),
                        ("b", &result.bytes.len().to_string()),
                    ],
                ));
                let colors = if result.colors > 256 {
                    ">256".into()
                } else {
                    result.colors.to_string()
                };
                ui.label(trf("opt_raster.colors", &[("n", &colors)]));
                ui.label(tr(if result.alpha_used {
                    "opt_raster.alpha_used"
                } else if result.alpha_channel {
                    "opt_raster.alpha_opaque"
                } else {
                    "opt_raster.alpha_none"
                }));
                ui.label(tr("opt_raster.verified"));
                if result.bytes.len() >= st.source.len() {
                    ui.label(tr("opt_raster.no_savings"));
                }
                let mut changed = false;
                ui.horizontal_wrapped(|ui| {
                    for (mode, key) in [
                        (Mode::Color, "opt_raster.color"),
                        (Mode::Alpha, "opt_raster.alpha"),
                        (Mode::Difference, "opt_raster.difference"),
                    ] {
                        changed |= ui.selectable_value(&mut st.mode, mode, tr(key)).changed();
                    }
                    for (value, key) in [
                        (0, "opt_raster.checker"),
                        (1, "opt_raster.white"),
                        (2, "opt_raster.black"),
                    ] {
                        changed |= ui
                            .selectable_value(&mut st.background, value, tr(key))
                            .changed();
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut st.zoom, 1.0..=16.0).text(tr("opt_raster.zoom")),
                        )
                        .changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut st.x, 0.0..=1.0).text("X"))
                        .changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut st.y, 0.0..=1.0).text("Y"))
                        .changed();
                });
                if changed || st.textures.is_none() {
                    st.textures = Some((
                        ctx.load_texture(
                            "opt-raster-original",
                            preview(&result, st, false),
                            egui::TextureOptions::NEAREST,
                        ),
                        ctx.load_texture(
                            "opt-raster-result",
                            preview(&result, st, true),
                            egui::TextureOptions::NEAREST,
                        ),
                    ));
                }
                if let Some((left, right)) = &st.textures {
                    let width = ((ui.available_width() - 10.0) * 0.5).clamp(80.0, 300.0);
                    ui.horizontal(|ui| {
                        for (tex, title) in
                            [(left, "opt_raster.original"), (right, "opt_raster.result")]
                        {
                            ui.vertical(|ui| {
                                ui.label(tr(title));
                                ui.image((tex.id(), egui::vec2(width, width * 220.0 / 300.0)));
                            });
                        }
                    });
                }
                let fresh =
                    app.engine.revision() == st.revision && app.engine.instance() == st.instance;
                if !fresh {
                    ui.label(tr("opt_raster.stale"));
                }
                if ui
                    .add_enabled(
                        fresh && result.bytes.len() < st.source.len(),
                        egui::Button::new(tr("docker.apply")),
                    )
                    .clicked()
                {
                    if let Some(cmd) = replacement(app, st, &result) {
                        app.run(cmd);
                        *close = true;
                    }
                }
            }
            if ui.button(tr("dialog.cancel")).clicked() {
                *close = true;
            }
        });
    *close |= !open;
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageEncoder, codecs::png::PngEncoder};
    #[test]
    fn optimize_raster_keeps_hidden_rgb_and_alpha() {
        let pixels = [210, 17, 93, 0, 111, 77, 233, 128, 55, 22, 11, 255];
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(&pixels, 3, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        for best in [true, false] {
            let candidate = analyze(&png, (3, 1), best).unwrap();
            assert_eq!(candidate.after.as_raw().as_slice(), &pixels);
            assert!(candidate.alpha_used);
            assert_eq!(candidate.colors, 3);
        }
        let mut profiled = png.clone();
        profiled.splice(33..33, [0, 0, 0, 1, b's', b'R', b'G', b'B', 0, 0, 0, 0, 0]);
        assert_eq!(analyze(&profiled, (3, 1), true).unwrap_err(), UNSUPPORTED);
        assert_eq!(plain_png(&png, (4000, 4000)), Err("opt_raster.limit"));
    }

    #[test]
    fn optimize_raster_applies_verified_bytes_once_and_undo_restores_original() {
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, 32, 32);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_compression(png::Compression::NoCompression);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[81; 32 * 32 * 3])
                .unwrap();
        }
        let mut app = App::headless();
        let id = app
            .new_shape(ShapeKind::Bitmap {
                rect: tracedraw_core::Rect::new(10.0, 20.0, 40.0, 60.0),
                width_px: 32,
                height_px: 32,
                png: encoded.clone().into(),
                fx: None,
            })
            .unwrap();
        app.select(vec![id]);
        open(&mut app);
        let super::super::dialogs::Dialog::OptimizeRaster(st) = app.dialog.clone() else {
            panic!("opened optimize window");
        };
        let candidate = analyze(&st.source, st.dims, true).unwrap();
        assert!(candidate.bytes.len() < encoded.len());
        let original = app.doc().find_shape(id).unwrap().clone();
        let command = replacement(&app, &st, &candidate).unwrap();
        app.run(command);
        let updated = app.doc().find_shape(id).unwrap();
        assert_eq!(source(updated).unwrap().0, &candidate.bytes);
        assert_eq!(updated.transform, original.transform);
        assert_eq!(updated.bounds(), original.bounds());
        assert!(replacement(&app, &st, &candidate).is_none());
        app.undo();
        assert_eq!(app.doc().find_shape(id).unwrap(), &original);
    }
}

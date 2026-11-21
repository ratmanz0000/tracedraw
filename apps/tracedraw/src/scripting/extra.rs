//! Script host methods for dimensions (#2348) and symbols (#2343):
//! `create_dimension`, `dimension_info`, `symbols`, `create_symbol`,
//! `place_symbol`, `shape_children` and `symbolize_groups`. See
//! docs/behavior/scripting.md, "Dimensions" and "Symbols".

use super::{id_arg, num, HostInner};
use crate::dimensions::{Dimension, Measure};
use serde_json::{json, Value};
use tracedraw_core::{
    document::{Shape, ShapeKind},
    geometry::{Affine, Point},
    Command, Fill, LayerId, ShapeId, Symbol,
};

fn is_text(s: &Shape) -> bool {
    matches!(s.kind, ShapeKind::Text { .. })
}

/// How near an object a script's dimension point has to be to hold to
/// it, mm: the 3 decimals a recorded macro writes, and some.
const SCRIPT_ANCHOR_REACH: f64 = 0.01;

fn point(a: &Value, x: &str, y: &str) -> Point {
    Point::new(num(a, x, 0.0), num(a, y, 0.0))
}

impl HostInner {
    pub(super) fn extra_call(&mut self, method: &str, a: &Value) -> Result<Option<Value>, String> {
        Ok(Some(match method {
            // A live dimension: from (x1, y1) to (x2, y2), its line (or
            // an angle's arc) through (tx, ty), in the Dimension tool's
            // style and units. `kind`: "parallel" (as measured),
            // "horizontal", "vertical", "auto" (the larger of the two
            // spans, as the Horizontal or Vertical tool does) or
            // "angular" (at (cx, cy) from the first point to the second).
            "create_dimension" => {
                let layer = self.layer_for(a)?;
                let p1 = point(a, "x1", "y1");
                let p2 = point(a, "x2", "y2");
                let at = point(a, "tx", "ty");
                let kind = a.get("kind").and_then(|v| v.as_str()).unwrap_or("parallel");
                let flat = |horizontal: bool| {
                    if horizontal {
                        Point::new(p2.x, p1.y)
                    } else {
                        Point::new(p1.x, p2.y)
                    }
                };
                let measure = match kind {
                    "parallel" | "slanted" => Measure::Linear { a: p1, b: p2, at },
                    "horizontal" => Measure::Linear {
                        a: p1,
                        b: flat(true),
                        at,
                    },
                    "vertical" => Measure::Linear {
                        a: p1,
                        b: flat(false),
                        at,
                    },
                    "auto" => Measure::Linear {
                        a: p1,
                        b: flat((p2.x - p1.x).abs() >= (p2.y - p1.y).abs()),
                        at,
                    },
                    "angular" => Measure::Angular {
                        center: point(a, "cx", "cy"),
                        a: p1,
                        b: p2,
                        at,
                    },
                    other => return Err(format!("unknown dimension kind {other}")),
                };
                let mut d = Dimension::new(measure, self.dims.style.clone());
                // Held to the objects its points are on, as the tool's
                // dimensions are, so it follows them (#2348).
                d.anchors = crate::dimensions::anchors_on(
                    &self.doc.layers_for_page(self.page).unwrap_or_default(),
                    &d,
                    SCRIPT_ANCHOR_REACH,
                );
                let ids = [self.new_id(), self.new_id(), self.new_id()];
                let group = self.new_id();
                let g =
                    crate::dimensions::group_of(d, ids, group, &self.dims.font, self.dims.units);
                self.run(Command::AddShape { layer, shape: g })?;
                self.selection = vec![group];
                let info = self.dimension_info(group)?;
                json!({"id": group.raw(), "value": info["value"], "text": info["text"]})
            }
            "dimension_info" => self.dimension_info(ShapeId(id_arg(a, "id")?))?,
            "symbols" => {
                let list: Vec<Value> = self
                    .doc
                    .symbols
                    .iter()
                    .enumerate()
                    .map(|(i, s)| json!({"index": i, "name": s.name}))
                    .collect();
                json!({ "symbols": list })
            }
            // A symbol from the objects `ids` (else the selection), which
            // an instance of it replaces where they were.
            "create_symbol" => {
                let mut ids: Vec<ShapeId> = a
                    .get("ids")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_u64()).map(ShapeId).collect())
                    .unwrap_or_default();
                if ids.is_empty() {
                    ids = self.selection.clone();
                }
                let shapes: Vec<Shape> = ids
                    .iter()
                    .map(|id| self.shape(id.raw()))
                    .collect::<Result<_, _>>()?;
                let Some(first) = ids.first() else {
                    return Err("create_symbol needs objects".into());
                };
                let layer = self.layer_of(*first)?;
                let b = shapes
                    .iter()
                    .map(|s| s.bounds())
                    .reduce(|x, y| x.union(y))
                    .unwrap_or_default();
                let mut def = if shapes.len() == 1 {
                    shapes[0].clone()
                } else {
                    let mut g = Shape::new(ShapeId(0), ShapeKind::Group { children: shapes });
                    g.fill = Fill::None;
                    g
                };
                def.transform_by(Affine::translate((-b.x0, -b.y0)));
                def.id = ShapeId(0);
                let index = self.doc.symbols.len();
                let name = self.symbol_name(a, index);
                self.run(Command::AddSymbol {
                    symbol: Symbol {
                        name: name.clone(),
                        shape: def,
                    },
                })?;
                self.run(Command::DeleteShapes { shapes: ids })?;
                let id = self.place(layer, index, Affine::translate((b.x0, b.y0)))?;
                self.selection = vec![id];
                json!({"index": index, "name": name, "id": id.raw()})
            }
            // An instance of symbol `index` with its corner at (x, y).
            "place_symbol" => {
                let layer = self.layer_for(a)?;
                let index = num(a, "index", 0.0).max(0.0) as usize;
                if index >= self.doc.symbols.len() {
                    return Err(format!("no symbol {index}"));
                }
                let t = Affine::translate((num(a, "x", 0.0), num(a, "y", 0.0)));
                let id = self.place(layer, index, t)?;
                self.selection = vec![id];
                json!({"id": id.raw()})
            }
            "shape_children" => {
                let s = self.shape(id_arg(a, "id")?)?;
                let children: Vec<Value> = match &s.kind {
                    ShapeKind::Group { children } => children
                        .iter()
                        .map(|c| {
                            let b = s.transform.transform_rect_bbox(c.bounds());
                            json!({
                                "id": c.id.raw(),
                                "name": c.name.clone().unwrap_or_default(),
                                "type": crate::ui::dockers::kind_id(&c.kind),
                                "is_text": is_text(c),
                                "bbox": [b.x0, b.y0, b.width(), b.height()],
                            })
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                json!({ "children": children })
            }
            "symbolize_groups" => self.symbolize_groups(ShapeId(id_arg(a, "id")?), a)?,
            _ => return Ok(None),
        }))
    }

    fn dimension_info(&self, id: ShapeId) -> Result<Value, String> {
        let s = self.shape(id.raw())?;
        let d = Dimension::of(&s)
            .ok_or_else(|| format!("{} is not a dimension", id.raw()))?
            .transformed(s.transform);
        let pts: Vec<[f64; 2]> = (0..3u8)
            .filter_map(|i| d.point(i))
            .map(|p| [p.x, p.y])
            .collect();
        let kind = match d.measure {
            Measure::Linear { .. } => "linear",
            Measure::Angular { .. } => "angular",
        };
        Ok(json!({
            "kind": kind,
            "value": d.value(),
            "text": d.label(self.dims.units),
            "points": pts,
        }))
    }

    fn symbol_name(&self, a: &Value, index: usize) -> String {
        match a.get("name").and_then(|v| v.as_str()) {
            Some(n) if !n.trim().is_empty() => n.to_string(),
            _ => format!("Symbol {}", index + 1),
        }
    }

    /// Add an instance of symbol `index` placed by `t` to `layer`.
    fn place(&mut self, layer: LayerId, index: usize, t: Affine) -> Result<ShapeId, String> {
        let id = self.new_id();
        self.run(Command::AddShape {
            layer,
            shape: instance(id, index, t),
        })?;
        Ok(id)
    }

    /// Every top-level group whose objects that are not text copy those
    /// of the group `sample` (see [`fingerprint`]) becomes an instance of
    /// one symbol, made from the sample's objects or an existing symbol
    /// that draws the same. A group with text keeps its text beside the
    /// instance, in a group, so each label stays editable (#2343).
    fn symbolize_groups(&mut self, sample: ShapeId, a: &Value) -> Result<Value, String> {
        let name = a.get("name").and_then(|v| v.as_str());
        let plan = tracedraw_core::symbols::plan(&self.doc, sample, name)?;
        let mut cmds = Vec::new();
        if let Some(sym) = &plan.new_symbol {
            cmds.push(Command::AddSymbol { symbol: sym.clone() });
        }
        let mut ids = Vec::new();
        let mut replacements = Vec::new();
        for m in &plan.matches {
            let inst = self.new_id();
            let group = if m.texts.is_empty() { inst } else { self.new_id() };
            ids.push(group.raw());
            replacements.push((
                m.group.id,
                tracedraw_core::symbols::replacement(m, plan.index, inst, group),
            ));
        }
        if !replacements.is_empty() {
            cmds.push(Command::ReplaceShapes { replacements });
        }
        for c in cmds {
            self.run(c)?;
        }
        self.drop_missing_selection();
        Ok(json!({
            "replaced": plan.matches.len(),
            "with_text": plan.with_text,
            "skipped": plan.skipped,
            "symbol_index": plan.index,
            "symbol_name": plan.name,
            "ids": ids,
        }))
    }
}

/// An instance of symbol `index` placed by `t`.
fn instance(id: ShapeId, index: usize, t: Affine) -> Shape {
    let mut inst = Shape::new(id, ShapeKind::SymbolInstance { index });
    inst.transform = t;
    inst.fill = Fill::None;
    inst.stroke = None;
    inst
}

#[cfg(test)]
#[path = "extra_tests.rs"]
mod tests;

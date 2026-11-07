use super::*;

#[derive(Clone, Copy)]
pub(super) struct Xf {
    pub(super) origin: Pos2,
    pub(super) view: View,
}

impl Xf {
    pub(super) fn pos(self, p: Pos2) -> Pos2 {
        self.origin + self.view.pan + p.to_vec2() * self.view.zoom
    }

    pub(super) fn world(self, p: Pos2) -> Pos2 {
        ((p - self.origin - self.view.pan) / self.view.zoom).to_pos2()
    }

    pub(super) fn rect(self, r: Rect) -> Rect {
        Rect::from_min_max(self.pos(r.min), self.pos(r.max))
    }
}

/// Sheet laid out in world coordinates.
#[derive(Debug, Clone, Copy)]
pub(super) struct Band {
    pub(super) sheet: u32,
    pub(super) top: f32,
    pub(super) height: f32,
    pub(super) collapsed: bool,
}

impl Band {
    pub(super) fn body_top(self) -> f32 {
        self.top + SHEET_HEADER
    }
}

/// Drawn shape of a block: a box with ports, or a variable tag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Kind {
    Box,
    /// `en`: a write with the enable input, drawn twice as tall with «En» below.
    Tag {
        write: bool,
        en: bool,
    },
}

/// Approximate text width; exact layout is not needed for box sizing.
pub(super) fn text_w(text: &str, size: f32) -> f32 {
    text.chars().count() as f32 * size * 0.58
}

pub(super) fn label(block: &Block, def: Option<&BlockDef>) -> String {
    if let Some(var) = block.var_name() {
        return var.to_owned();
    }
    def.map_or_else(
        || block.kind.clone(),
        |d| {
            d.info
                .symbol
                .clone()
                .unwrap_or_else(|| crate::i18n::tr(&d.info.name).to_owned())
        },
    )
}

pub(super) fn kind_of(block: &Block, def: Option<&BlockDef>) -> Kind {
    match block.kind.as_str() {
        VAR_GET => Kind::Tag {
            write: false,
            en: false,
        },
        VAR_SET => Kind::Tag {
            write: true,
            en: def.is_some_and(|d| d.inputs.len() > 1),
        },
        _ => Kind::Box,
    }
}

/// Size of a block in world units.
pub(super) fn size(block: &Block, def: Option<&BlockDef>) -> Vec2 {
    let text = label(block, def);
    match kind_of(block, def) {
        Kind::Tag { en, .. } => Vec2::new(
            ((text_w(&text, 11.0) + 18.0) / GRID).ceil() * GRID,
            if en { TAG_H * 2.0 } else { TAG_H },
        ),
        Kind::Box => {
            let (ins, outs) = def.map_or((Vec::new(), Vec::new()), |d| {
                (
                    d.active_inputs(&block.params).collect(),
                    d.active_outputs(&block.params).collect(),
                )
            });
            let rows = ins.len().max(outs.len()).max(1);
            let longest = |ports: &[&PortDef]| {
                ports
                    .iter()
                    .map(|p| text_w(crate::i18n::tr(p.title()), 9.5))
                    .fold(0.0, f32::max)
            };
            let ports_w = longest(&ins) + longest(&outs) + 22.0;
            let w = (text_w(&text, 11.0) + 16.0).max(ports_w).max(64.0);
            Vec2::new(
                (w / GRID).ceil() * GRID,
                HEAD_H + rows as f32 * ROW_H + 10.0,
            )
        }
    }
}

/// Whether a variable block already has a link on either side. A block of a variable
/// bound to a port of a service block (Wi-Fi `En`, `IP`...) has its side fixed from the
/// start: a read-only value gets no writing side. Parameters work both ways.
pub(super) fn tag_linked(logic: &Logic, lib: &crab_core::Library, id: &str) -> bool {
    logic
        .block(id)
        .and_then(|b| b.var_name())
        .is_some_and(|v| crab_core::vars::var_fixed(logic, lib, v))
        || logic
            .links
            .iter()
            .any(|l| l.from.block == id || l.to.block == id)
}

pub(super) fn port_pos(rect: Rect, kind: Kind, index: usize, output: bool, z: f32) -> Pos2 {
    match kind {
        Kind::Tag { en: false, .. } => {
            if output {
                rect.right_center()
            } else {
                rect.left_center()
            }
        }
        // Value on the upper half, `en` on the lower one.
        Kind::Tag { en: true, .. } => {
            let x = if output { rect.max.x } else { rect.min.x };
            let row = if output { 0.0 } else { index as f32 };
            Pos2::new(x, rect.min.y + rect.height() * (0.25 + 0.5 * row))
        }
        Kind::Box => {
            let x = if output { rect.max.x } else { rect.min.x };
            Pos2::new(x, rect.min.y + (HEAD_H + ROW_H * (index as f32 + 0.5)) * z)
        }
    }
}

pub(super) fn signal_color(p: &Palette, ty: SignalType) -> Color32 {
    match ty {
        SignalType::Bool => p.bool_signal,
        SignalType::Int => p.int_signal,
        SignalType::Float => p.float_signal,
        SignalType::String => p.string_signal,
        SignalType::Any | SignalType::Number | SignalType::Same => p.text,
    }
}

pub(super) fn value_text(v: &ParamValue) -> String {
    match v {
        ParamValue::Bool(b) => b.to_string(),
        ParamValue::Int(i) => i.to_string(),
        ParamValue::Float(f) => format!("{f}"),
        ParamValue::Text(s) => s.clone(),
    }
}

/// Whether a link runs left to right (one vertical segment) or has to go around.
pub(super) fn is_forward(a: Pos2, b: Pos2, step: f32) -> bool {
    b.x - a.x >= 2.0 * step
}

/// Orthogonal route from an output to an input, in screen space. `bend` moves the
/// middle segment: its x for a forward link, its y for a backward one.
pub(super) fn route(a: Pos2, b: Pos2, step: f32, bend: Option<f32>) -> Vec<Pos2> {
    if is_forward(a, b, step) {
        let mx = bend.map_or((a.x + b.x) * 0.5, |x| x.clamp(a.x + 2.0, b.x - 2.0));
        vec![a, Pos2::new(mx, a.y), Pos2::new(mx, b.y), b]
    } else {
        // Backwards: go around below the lower end.
        let my = bend.unwrap_or(if (b.y - a.y).abs() > 3.0 * step {
            (a.y + b.y) * 0.5
        } else {
            a.y.max(b.y) + 2.5 * step
        });
        vec![
            a,
            Pos2::new(a.x + step, a.y),
            Pos2::new(a.x + step, my),
            Pos2::new(b.x - step, my),
            Pos2::new(b.x - step, b.y),
            b,
        ]
    }
}

/// Replaces sharp corners of a polyline with short arcs.
pub(super) fn rounded(points: &[Pos2], radius: f32) -> Vec<Pos2> {
    let mut out = vec![points[0]];
    for w in points.windows(3) {
        let (p0, p, p1) = (w[0], w[1], w[2]);
        let (d0, d1) = (p - p0, p1 - p);
        let (l0, l1) = (d0.length(), d1.length());
        if l0 < 0.5 || l1 < 0.5 {
            out.push(p);
            continue;
        }
        let r = radius.min(l0 * 0.5).min(l1 * 0.5);
        let s = p - d0 / l0 * r;
        let e = p + d1 / l1 * r;
        for k in 0..=6 {
            let t = k as f32 / 6.0;
            let q = s.to_vec2() * (1.0 - t) * (1.0 - t)
                + p.to_vec2() * 2.0 * (1.0 - t) * t
                + e.to_vec2() * t * t;
            out.push(q.to_pos2());
        }
    }
    out.push(*points.last().expect("route has points"));
    out
}

pub(super) fn near_polyline(points: &[Pos2], p: Pos2, tolerance: f32) -> bool {
    points.windows(2).any(|w| {
        let (s, d) = (w[0], w[1] - w[0]);
        let t = ((p - s).dot(d) / d.length_sq().max(1e-6)).clamp(0.0, 1.0);
        (s + d * t).distance(p) <= tolerance
    })
}

/// Sheets stacked top to bottom; heights follow their content. A block being dragged is
/// left out (it may be on its way to another sheet) unless `grow` is set: with Shift held it
/// stays on its sheet and the sheet stretches after it.
pub(super) fn bands(app: &EditorApp, sizes: &HashMap<&str, Vec2>, grow: bool) -> Vec<Band> {
    let doc = app.doc.as_ref().expect("canvas shown with a document");
    // The "add sheet" strip takes the top.
    let mut top = ADD_SHEET_H + SHEET_GAP;
    app.sheets()
        .into_iter()
        .map(|sheet| {
            let bottom = doc
                .project
                .layout
                .blocks
                .iter()
                .filter(|(id, p)| {
                    p.sheet == sheet && (grow || app.canvas.moving.as_deref() != Some(id))
                })
                .map(|(id, p)| p.y + sizes.get(id.as_str()).map_or(0.0, |s| s.y))
                .fold(0.0, f32::max);
            let collapsed = app.canvas.collapsed.contains(&sheet);
            let band = Band {
                sheet,
                top,
                height: if collapsed {
                    SHEET_HEADER
                } else {
                    SHEET_HEADER + (bottom + SHEET_PAD).max(SHEET_MIN_BODY)
                },
                collapsed,
            };
            top += band.height + SHEET_GAP;
            band
        })
        .collect()
}

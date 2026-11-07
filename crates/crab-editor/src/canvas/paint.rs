use super::*;

/// Parameter line of a block that is cut to the block width but hovered: the caller
/// paints it in full over everything else.
pub(super) struct FullNote {
    pub(super) pos: Pos2,
    pub(super) text: String,
}

/// Draws a block box: header with the label, id above, ports with their names,
/// defaults of unconnected inputs and parameter values below. The parameter line is
/// cut to the block width; under the pointer it is returned to be shown in full.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_box(
    painter: &egui::Painter,
    r: Rect,
    z: f32,
    block: &Block,
    def: Option<&BlockDef>,
    pal: &Palette,
    border: Stroke,
    pointer: Option<Pos2>,
    fed: impl Fn(&str) -> bool,
) -> Option<FullNote> {
    let mut full_note = None;
    let kind = Kind::Box;
    let font = |size: f32| FontId::proportional(size * z.clamp(0.5, 2.0));
    painter.rect(
        r,
        CornerRadius::same(2),
        pal.block_fill,
        border,
        StrokeKind::Inside,
    );
    let head = Rect::from_min_size(r.min, Vec2::new(r.width(), HEAD_H * z)).shrink(border.width);
    painter.rect_filled(
        head,
        CornerRadius {
            nw: 2,
            ne: 2,
            sw: 0,
            se: 0,
        },
        pal.block_header,
    );
    painter.text(
        head.center(),
        Align2::CENTER_CENTER,
        label(block, def),
        font(10.5),
        pal.text,
    );
    // Block label (id) above the box; a web widget also shows its caption.
    let caption = match block.params.get("caption") {
        Some(crab_core::ParamValue::Text(c)) if block.kind.starts_with("web_") && !c.is_empty() => {
            format!("{} · {c}", block.id)
        }
        _ => block.id.clone(),
    };
    painter.text(
        r.left_top() - Vec2::new(0.0, 2.0),
        Align2::LEFT_BOTTOM,
        &caption,
        font(9.5),
        pal.weak,
    );
    if let Some(def) = def {
        for (i, port) in def.active_inputs(&block.params).enumerate() {
            let p = port_pos(r, kind, i, false, z);
            painter.circle_filled(p, PORT_R * z.sqrt(), signal_color(pal, port.ty));
            painter.text(
                p + Vec2::new(5.0 * z, 0.0),
                Align2::LEFT_CENTER,
                crate::i18n::tr(port.title()),
                font(9.0),
                pal.weak,
            );
            if !fed(&port.name) {
                if let Some(d) = &port.default {
                    painter.text(
                        p - Vec2::new(5.0 * z, 0.0),
                        Align2::RIGHT_CENTER,
                        value_text(d),
                        font(9.0),
                        pal.weak,
                    );
                }
            }
        }
        for (i, port) in def.active_outputs(&block.params).enumerate() {
            let p = port_pos(r, kind, i, true, z);
            painter.circle_filled(p, PORT_R * z.sqrt(), signal_color(pal, port.ty));
            painter.text(
                p - Vec2::new(5.0 * z, 0.0),
                Align2::RIGHT_CENTER,
                crate::i18n::tr(port.title()),
                font(9.0),
                pal.weak,
            );
        }
        // Parameter values under the box, like constants.
        let summary: Vec<String> = def
            .params
            .iter()
            .filter_map(|p| {
                block
                    .params
                    .get(&p.name)
                    .and_then(|v| p.caption_with(v, &|t| crate::i18n::tr(t).to_owned()))
            })
            .collect();
        if !summary.is_empty() {
            let full = summary.join(" ");
            let pos = r.left_bottom() + Vec2::new(0.0, 2.0);
            let width = |text: &str| {
                painter
                    .layout_no_wrap(text.to_owned(), font(9.0), pal.weak)
                    .size()
                    .x
            };
            let mut shown = full.clone();
            if width(&full) > r.width() {
                let mut chars: Vec<char> = full.chars().collect();
                while !chars.is_empty()
                    && width(&format!("{}…", chars.iter().collect::<String>())) > r.width()
                {
                    chars.pop();
                }
                shown = format!("{}…", chars.iter().collect::<String>());
            }
            let galley = painter.layout_no_wrap(shown.clone(), font(9.0), pal.weak);
            let line = Rect::from_min_size(pos, Vec2::new(r.width(), galley.size().y));
            if shown != full && pointer.is_some_and(|p| line.contains(p)) {
                full_note = Some(FullNote { pos, text: full });
            } else {
                painter.galley(pos, galley, pal.weak);
            }
        }
    } else {
        painter.text(
            r.center() + Vec2::new(0.0, 6.0 * z),
            Align2::CENTER_CENTER,
            "?",
            font(12.0),
            pal.error,
        );
    }
    full_note
}

/// Block as it will look on a sheet, for the block editor: an instance with default
/// parameters and nothing connected.
pub fn block_preview(ui: &mut Ui, def: &BlockDef, pal: &Palette) {
    let params = def
        .params
        .iter()
        .filter_map(|p| Some((p.name.clone(), p.default.clone()?)))
        .collect();
    let block = Block {
        id: crab_codegen::preview_id(def),
        kind: def.info.kind.clone(),
        params,
        ..Default::default()
    };
    let body = size(&block, Some(def));
    let margin = Vec2::new(48.0, 20.0);
    let (rect, _) = ui.allocate_exact_size(body + margin * 2.0, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, pal.sheet);
    let r = Rect::from_min_size(rect.min + margin, body);
    let border = Stroke::new(1.0, pal.block_border);
    paint_box(
        &painter,
        r,
        1.0,
        &block,
        Some(def),
        pal,
        border,
        None,
        |_| false,
    );
}

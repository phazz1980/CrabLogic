use super::*;

/// Everything the drawing pass wants to change in the document; applied once the document
/// is no longer borrowed.
pub(super) struct Edits {
    pub(super) moves: Vec<(String, Vec2)>,
    pub(super) drag_end: Option<String>,
    pub(super) landing: Option<BlockPos>,
    pub(super) wire_edit: Option<(PortRef, Option<f32>)>,
    pub(super) wire_end: bool,
    pub(super) wire_break: Option<(PortRef, bool)>,
    pub(super) connect: Option<(PortRef, PortRef)>,
    pub(super) drop: Option<(DragItem, BlockPos)>,
}

/// Applies `edits` to the document; returns the problems to report and the id of a block that
/// was added (to select it).
pub(super) fn apply_edits(
    app: &mut EditorApp,
    lib: &crab_core::Library,
    edits: Edits,
) -> (Vec<String>, Option<String>) {
    let Edits {
        moves,
        drag_end,
        landing,
        wire_edit,
        wire_end,
        wire_break,
        connect,
        drop,
    } = edits;
    let mut problems = Vec::new();
    let mut added = None;
    if let Some(doc) = &mut app.doc {
        let group = moves.len() > 1;
        for (id, delta) in moves {
            if let Some(&pos) = doc.project.layout.blocks.get(&id) {
                let moved = BlockPos {
                    x: pos.x + delta.x,
                    y: pos.y + delta.y,
                    ..pos
                };
                let key = if group {
                    "move:group".to_owned()
                } else {
                    format!("move:{id}")
                };
                let _ = doc.edit(lib, Some(key), |p| p.move_block(&id, moved));
            }
        }
        let group_end = drag_end.as_ref().and_then(|id| {
            let ids = app.selection.as_ref()?.blocks();
            (ids.len() > 1 && ids.contains(id)).then(|| ids.to_vec())
        });
        if let Some(ids) = group_end {
            // A group stays on its sheets; positions snap to the grid.
            let _ = doc.edit(lib, Some("move:group".to_owned()), |p| {
                for id in &ids {
                    if let Some(&pos) = p.layout.blocks.get(id) {
                        let snap = |v: f32| (v / GRID).round().max(0.0) * GRID;
                        p.move_block(
                            id,
                            BlockPos {
                                x: snap(pos.x),
                                y: snap(pos.y),
                                ..pos
                            },
                        )?;
                    }
                }
                Ok::<_, crab_core::EditError>(())
            });
            doc.end_edit();
        } else if let (Some(id), Some(pos)) = (&drag_end, landing) {
            let _ = doc.edit(lib, Some(format!("move:{id}")), |p| p.move_block(id, pos));
            doc.end_edit();
        }
        if let Some((to, bend)) = wire_edit {
            let key = format!("wire:{}.{}", to.block, to.port);
            let _ = doc.edit(lib, Some(key), |p| {
                p.set_wire_bend(&to, bend);
                Ok::<_, crab_core::EditError>(())
            });
        }
        if wire_end {
            doc.end_edit();
        }
        if let Some((to, broken)) = wire_break {
            let _ = doc.edit(lib, None, |p| {
                p.set_wire_broken(&to, broken);
                Ok::<_, crab_core::EditError>(())
            });
        }
        if let Some((from, to)) = connect {
            if let Err(e) = doc.edit(lib, None, |p| p.connect(lib, from, to)) {
                problems.push(e.to_string());
            }
        }
        if let Some((item, pos)) = drop {
            let result = match item {
                DragItem::Block(kind) => doc.edit(lib, None, |p| p.add_block(lib, &kind, pos)),
                DragItem::Var(name) => doc.edit(lib, None, |p| p.add_var_block(&name, false, pos)),
                DragItem::Setting { id, name, ty, feed } => doc.edit(lib, None, |p| {
                    crate::netpanel::pull_out(p, lib, &id, &name, ty, feed, pos)
                }),
                // A tag already on the board gets one more copy; otherwise the
                // block itself is placed (or moved).
                DragItem::Topic(id) => doc.edit(lib, None, |p| {
                    let tag = p.logic.tag_of(&id).to_owned();
                    let is_tag = p.is_widget(&tag)
                        || p.logic.block(&tag).is_some_and(|b| {
                            // A display has pins but is not a tag: it has no copies.
                            !crate::displaypanel::is_display(&b.kind)
                                && lib.resolve(&p.logic, b).is_some_and(|l| {
                                    l.def
                                        .params
                                        .iter()
                                        .any(|d| d.kind == crab_core::ParamKind::Pin)
                                })
                        });
                    let placed = std::iter::once(tag.as_str())
                        .chain(p.logic.tag_copies(&tag))
                        .any(|b| p.layout.blocks.contains_key(b));
                    // A widget never stands on a sheet itself, only its images do.
                    if is_tag && (placed || p.is_widget(&tag)) {
                        p.add_tag_copy(&tag, pos)
                    } else {
                        p.layout.blocks.insert(id.clone(), pos);
                        Ok(id)
                    }
                }),
            };
            match result {
                Ok(id) => added = Some(id),
                Err(e) => problems.push(e.to_string()),
            }
        }
    }
    (problems, added)
}

/// Swaps two sheets in the document and in the editor state; the focus and the view follow
/// the moved sheet.
#[allow(clippy::too_many_arguments)]
pub(super) fn swap_sheets(
    app: &mut EditorApp,
    ui: &mut Ui,
    lib: &crab_core::Library,
    a: u32,
    b: u32,
    bands: &[Band],
    xf: &Xf,
    rect: Rect,
) {
    if let Some(doc) = &mut app.doc {
        let _ = doc.edit(lib, None, |p| {
            p.swap_sheets(a, b);
            Ok::<_, crab_core::EditError>(())
        });
    }
    let (ea, eb) = (app.extra_sheets.remove(&a), app.extra_sheets.remove(&b));
    if ea {
        app.extra_sheets.insert(b);
    }
    if eb {
        app.extra_sheets.insert(a);
    }
    let (ca, cb) = (
        app.canvas.collapsed.remove(&a),
        app.canvas.collapsed.remove(&b),
    );
    if ca {
        app.canvas.collapsed.insert(b);
    }
    if cb {
        app.canvas.collapsed.insert(a);
    }
    // The focus and the view follow the moved sheet.
    app.sheet = b;
    if let Some(band) = bands.iter().find(|x| x.sheet == a) {
        let offset = xf.pos(Pos2::new(0.0, band.top)).y - rect.min.y;
        app.canvas.follow_sheet = Some((b, offset));
        ui.ctx().request_repaint();
    }
}

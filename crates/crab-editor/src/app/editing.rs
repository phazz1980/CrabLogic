use super::*;

impl EditorApp {
    pub fn undo(&mut self) {
        if let Some(doc) = &mut self.doc {
            doc.undo(&self.lib);
            self.selection = None;
        }
    }

    pub fn redo(&mut self) {
        if let Some(doc) = &mut self.doc {
            doc.redo(&self.lib);
            self.selection = None;
        }
    }

    /// Arrow keys move the selected blocks by one grid step (five with Shift) on their sheets.
    pub(super) fn nudge_selection(&mut self, ctx: &egui::Context) {
        let step = if ctx.input(|i| i.modifiers.shift) {
            50.0
        } else {
            10.0
        };
        let mut delta = (0.0f32, 0.0f32);
        ctx.input_mut(|i| {
            for (key, d) in [
                (Key::ArrowLeft, (-step, 0.0)),
                (Key::ArrowRight, (step, 0.0)),
                (Key::ArrowUp, (0.0, -step)),
                (Key::ArrowDown, (0.0, step)),
            ] {
                let mods = if i.modifiers.shift {
                    Modifiers::SHIFT
                } else {
                    Modifiers::NONE
                };
                while i.consume_key(mods, key) {
                    delta = (delta.0 + d.0, delta.1 + d.1);
                }
            }
        });
        if delta == (0.0, 0.0) {
            return;
        }
        let (Some(doc), Some(Selection::Block(_) | Selection::Blocks(_))) =
            (&mut self.doc, &self.selection)
        else {
            return;
        };
        let ids = self
            .selection
            .as_ref()
            .map(|s| s.blocks().to_vec())
            .unwrap_or_default();
        let result = doc.edit(&self.lib, Some("nudge".to_owned()), |p| {
            for id in &ids {
                if let Some(&pos) = p.layout.blocks.get(id) {
                    p.move_block(
                        id,
                        crab_core::layout::BlockPos {
                            x: (pos.x + delta.0).max(0.0),
                            y: (pos.y + delta.1).max(0.0),
                            ..pos
                        },
                    )?;
                }
            }
            Ok::<_, crab_core::EditError>(())
        });
        if let Err(e) = result {
            self.info(LogLevel::Error, e.to_string());
        }
    }

    pub fn delete_selection(&mut self) {
        let (Some(doc), Some(sel)) = (&mut self.doc, self.selection.take()) else {
            return;
        };
        let result = match &sel {
            Selection::Block(_) | Selection::Blocks(_) => doc
                .edit(&self.lib, None, |p| {
                    // Copies go first: removing a tag folds its first copy into it.
                    let mut ids = sel.blocks().to_vec();
                    ids.sort_by_key(|id| p.logic.block(id).is_none_or(|b| b.tag.is_none()));
                    // An input or output leaves the sheet but stays in the project tree.
                    ids.iter().try_for_each(|id| {
                        let kind = p.logic.block(id).map(|b| b.kind.as_str());
                        if matches!(
                            kind,
                            Some(
                                "wifi"
                                    | "ethernet"
                                    | "mqtt"
                                    | "mqtt_publish"
                                    | "mqtt_subscribe"
                                    | "web_ui"
                            )
                        ) {
                            p.take_service_off_board(id)
                        } else if p.is_tag(&self.lib, p.logic.tag_of(id)) || p.is_widget(id) {
                            p.take_off_board(id)
                        } else {
                            p.remove_block(id)
                        }
                    })?;
                    p.release_unused_services(&self.lib)
                })
                .map_err(|e| e.to_string()),
            Selection::Link(to) => doc.edit(&self.lib, None, |p| {
                if p.disconnect(to) {
                    Ok(())
                } else {
                    Err(crate::i18n::tr("связь не найдена").to_owned())
                }
            }),
        };
        if let Err(e) = result {
            self.info(LogLevel::Error, e);
        }
    }

    /// Puts the selected blocks on the system clipboard as JSON, so they can be
    /// pasted in another editor window or project.
    pub fn copy_selection(&mut self, ctx: &egui::Context) {
        let (Some(doc), Some(sel)) = (&self.doc, &self.selection) else {
            return;
        };
        if sel.blocks().is_empty() {
            return;
        }
        let frag = doc.project.copy_fragment(sel.blocks());
        let n = frag.blocks.len();
        ctx.copy_text(frag.to_clipboard());
        self.info(
            LogLevel::Info,
            crate::i18n::msg("Скопировано блоков: {0}", &[&n]),
        );
    }

    /// Copies the selected blocks to the clipboard and removes them.
    pub fn cut_selection(&mut self, ctx: &egui::Context) {
        if self
            .selection
            .as_ref()
            .is_none_or(|s| s.blocks().is_empty())
        {
            return;
        }
        self.copy_selection(ctx);
        self.delete_selection();
    }

    /// Inserts blocks from clipboard text at `at` (or a free spot on the active sheet).
    pub fn paste(&mut self, text: &str, at: Option<BlockPos>) {
        let Some(frag) = Fragment::from_clipboard(text) else {
            return;
        };
        let Some(doc) = &mut self.doc else { return };
        let at = at.unwrap_or_else(|| doc.project.free_position(self.sheet));
        let lib = self.lib.clone();
        match doc.edit(&lib, None, |p| p.paste_fragment(&lib, &frag, at)) {
            Ok(ids) => {
                self.sheet = at.sheet;
                self.selection = Selection::of_blocks(ids);
            }
            Err(e) => self.info(LogLevel::Error, crate::i18n::msg("Вставка: {0}", &[&e])),
        }
    }
}

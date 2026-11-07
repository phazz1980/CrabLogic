use super::*;

/// Text inserted into the template from the substitution menu or buttons.
pub(super) struct Snippet {
    pub(super) label: String,
    pub(super) text: String,
    pub(super) hint: String,
}

impl Snippet {
    pub(super) fn new(
        label: impl Into<String>,
        text: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self {
            label: label.into(),
            text: text.into(),
            hint: hint.into(),
        }
    }
}

/// Name with the caption, for menus: `in (Вход)`.
pub(super) fn row_title(name: &str, label: &str) -> String {
    match label.trim() {
        "" => name.to_owned(),
        l => format!("{name} ({l})"),
    }
}

/// Replaces `group.old` and `group["old"]` in a template by the new name.
pub(super) fn rename_in_template(template: &str, group: &str, old: &str, new: &str) -> String {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    let dotted = format!("{group}.{old}");
    while let Some(at) = rest.find(group) {
        let (head, tail) = rest.split_at(at);
        out.push_str(head);
        let before_ok = !head.chars().next_back().is_some_and(is_word);
        if before_ok && tail.starts_with(&dotted) && !tail[dotted.len()..].starts_with(is_word) {
            out.push_str(&format!("{group}.{new}"));
            rest = &tail[dotted.len()..];
            continue;
        }
        let mut done = false;
        for q in ['"', '\''] {
            let quoted = format!("{group}[{q}{old}{q}]");
            if before_ok && tail.starts_with(&quoted) {
                out.push_str(&format!("{group}[{q}{new}{q}]"));
                rest = &tail[quoted.len()..];
                done = true;
                break;
            }
        }
        if !done {
            out.push_str(group);
            rest = &tail[group.len()..];
        }
    }
    out.push_str(rest);
    out
}

/// Port substitutions: `{{ input.x }}` or `{{ output.x }}`.
pub(super) fn port_snippets(rows: &[PortRow], kind: &str) -> Vec<Snippet> {
    rows.iter()
        .map(|p| {
            let hint = if p.description.is_empty() {
                type_name(p.ty).to_owned()
            } else {
                format!("{}, {}", type_name(p.ty), p.description)
            };
            Snippet::new(
                row_title(&p.name, &p.label),
                format!("{{{{ {kind}.{} }}}}", p.name),
                hint,
            )
        })
        .collect()
}

/// Submenu of snippets; the chosen text goes to `insert`.
pub(super) fn snippet_menu(
    ui: &mut Ui,
    title: &str,
    items: &[Snippet],
    insert: &mut Option<String>,
) {
    ui.add_enabled_ui(!items.is_empty(), |ui| {
        ui.menu_button(title, |ui| {
            for s in items {
                let text = RichText::new(format!("{}   {}", s.label, s.text.trim_end()));
                if ui.button(text).on_hover_text(&s.hint).clicked() {
                    *insert = Some(s.text.clone());
                    ui.close();
                }
            }
        });
    });
}

/// Inserts `text` at the cursor of the text edit `id` (replacing the selection,
/// or at the end without a cursor) and puts the cursor after it.
pub(super) fn insert_at_cursor(ctx: &egui::Context, id: Id, template: &mut String, text: &str) {
    use egui::text::{CCursor, CCursorRange};
    let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    let chars = template.chars().count();
    let (start, end) = state
        .cursor
        .char_range()
        .map(|r| {
            let (a, b) = (r.primary.index.0, r.secondary.index.0);
            (a.min(b).min(chars), a.max(b).min(chars))
        })
        .unwrap_or((chars, chars));
    let byte = |i: usize| {
        template
            .char_indices()
            .nth(i)
            .map_or(template.len(), |(b, _)| b)
    };
    let (from, to) = (byte(start), byte(end));
    template.replace_range(from..to, text);
    let cursor = CCursor::new(start + text.chars().count());
    state.cursor.set_char_range(Some(CCursorRange::one(cursor)));
    state.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
}

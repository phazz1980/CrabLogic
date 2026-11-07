use super::*;

/// A Markdown template of the docs toolbar.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Fmt {
    Bold,
    Italic,
    Code,
    Heading(usize),
    Bullets,
    Numbered,
    Quote,
    CodeBlock,
    Table,
    Link,
    /// Picture `![описание](target)` on a line of its own.
    Image(String),
}

/// Applies `fmt` to the chars `start..end` of `text`; returns the new text and the new selection.
pub(super) fn format_selection(
    text: &str,
    start: usize,
    end: usize,
    fmt: &Fmt,
) -> (String, usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let (start, end) = (
        start.min(end).min(chars.len()),
        start.max(end).min(chars.len()),
    );
    let sel: String = chars[start..end].iter().collect();
    let before: String = chars[..start].iter().collect();
    let after: String = chars[end..].iter().collect();
    let len = |s: &str| s.chars().count();
    // Text put in place of the selection, and the selection inside it.
    let (insert, from, to) = match fmt {
        Fmt::Bold | Fmt::Italic | Fmt::Code => {
            let mark = match fmt {
                Fmt::Bold => "**",
                Fmt::Italic => "*",
                _ => "`",
            };
            let inner = if sel.is_empty() {
                crate::i18n::tr("текст")
            } else {
                &sel
            };
            let m = len(mark);
            (format!("{mark}{inner}{mark}"), m, m + len(inner))
        }
        Fmt::Link => {
            let url = "https://";
            if sel.is_empty() {
                (
                    crate::i18n::fmt("[текст]({0})", &[&url]),
                    1,
                    1 + len(crate::i18n::tr("текст")),
                )
            } else {
                let at = len(&sel) + 3;
                (format!("[{sel}]({url})"), at, at + len(url))
            }
        }
        Fmt::Heading(_) | Fmt::Bullets | Fmt::Numbered | Fmt::Quote => {
            return prefix_lines(&chars, start, end, fmt);
        }
        Fmt::CodeBlock | Fmt::Table | Fmt::Image(_) => {
            // A block stands on lines of its own.
            let lead = if before.is_empty() || before.ends_with('\n') {
                ""
            } else {
                "\n"
            };
            let tail = if after.starts_with('\n') { "" } else { "\n" };
            let inner = if sel.is_empty() {
                crate::i18n::tr("код")
            } else {
                &sel
            };
            let (body, from, to) = match fmt {
                Fmt::CodeBlock => (format!("```\n{inner}\n```"), 4, 4 + len(inner)),
                Fmt::Image(target) => (
                    crate::i18n::fmt("![описание]({0})", &[&target]),
                    2,
                    2 + len(crate::i18n::tr("описание")),
                ),
                _ => {
                    let table = crate::i18n::tr(
                        "| Колонка 1 | Колонка 2 |\n|---|---|\n| значение | значение |",
                    );
                    (table.to_owned(), len(table), len(table))
                }
            };
            let lead_len = len(lead);
            (
                format!("{lead}{body}{tail}"),
                lead_len + from,
                lead_len + to,
            )
        }
    };
    (format!("{before}{insert}{after}"), start + from, start + to)
}

/// Adds a line prefix to the lines the selection touches, or removes it when every line has it.
pub(super) fn prefix_lines(
    chars: &[char],
    start: usize,
    end: usize,
    fmt: &Fmt,
) -> (String, usize, usize) {
    let line_start = chars[..start]
        .iter()
        .rposition(|&c| c == '\n')
        .map_or(0, |i| i + 1);
    // A selection ending right after a line break does not touch the next line.
    let last = if end > start && chars[end - 1] == '\n' {
        end - 1
    } else {
        end
    };
    let line_end = chars[last..]
        .iter()
        .position(|&c| c == '\n')
        .map_or(chars.len(), |i| last + i);
    let block: String = chars[line_start..line_end].iter().collect();
    let lines: Vec<&str> = block.split('\n').collect();

    let prefix = |n: usize| match fmt {
        Fmt::Heading(level) => format!("{} ", "#".repeat(*level)),
        Fmt::Bullets => "- ".to_owned(),
        Fmt::Numbered => format!("{}. ", n + 1),
        _ => "> ".to_owned(),
    };
    // Bytes of the prefix of this kind that the line already has.
    let existing = |line: &str| -> usize {
        match fmt {
            Fmt::Heading(_) => {
                let hashes = line.bytes().take_while(|&b| b == b'#').count();
                if hashes > 0 && line[hashes..].starts_with(' ') {
                    hashes + 1
                } else {
                    0
                }
            }
            Fmt::Bullets if line.starts_with("- ") => 2,
            Fmt::Quote if line.starts_with("> ") => 2,
            Fmt::Numbered => {
                let digits = line.bytes().take_while(u8::is_ascii_digit).count();
                if digits > 0 && line[digits..].starts_with(". ") {
                    digits + 2
                } else {
                    0
                }
            }
            _ => 0,
        }
    };
    let remove = match fmt {
        // The same heading level again removes it; another level replaces it.
        Fmt::Heading(level) => lines.iter().all(|l| existing(l) == level + 1),
        _ => lines.iter().all(|l| existing(l) > 0),
    };
    let out: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(n, line)| {
            let rest = &line[existing(line)..];
            if remove {
                rest.to_owned()
            } else {
                format!("{}{rest}", prefix(n))
            }
        })
        .collect();
    let out = out.join("\n");
    let before: String = chars[..line_start].iter().collect();
    let after: String = chars[line_end..].iter().collect();
    let to = line_start + out.chars().count();
    (format!("{before}{out}{after}"), line_start, to)
}

/// Applies `fmt` to the selection of the text field `id` and keeps the result selected.
pub(super) fn format_at_cursor(ctx: &egui::Context, id: Id, text: &mut String, fmt: &Fmt) {
    use egui::text::{CCursor, CCursorRange};
    let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    let chars = text.chars().count();
    let (start, end) = state
        .cursor
        .char_range()
        .map_or((chars, chars), |r| (r.primary.index.0, r.secondary.index.0));
    let (new, from, to) = format_selection(text, start, end, fmt);
    *text = new;
    let range = CCursorRange::two(CCursor::new(from), CCursor::new(to));
    state.cursor.set_char_range(Some(range));
    state.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
}

/// Tab «Описание»: formatting buttons, the Markdown text and a live preview.
pub(super) fn docs_tab(ui: &mut Ui, f: &mut Form, editable: bool) {
    let id = Id::unique("block_docs_text");
    ui.weak(crate::i18n::tr(
        "docs.md: пояснение и пример. По нему ИИ-помощник подбирает блоки.",
    ));
    docs_editor(
        ui,
        id,
        &mut f.docs,
        &mut f.attach.pictures,
        editable,
        crate::i18n::tr("блока"),
    );
}

/// Markdown text with the format buttons and a preview; pictures go to `pictures`.
/// `owner` is where they are copied on save, for the hint ("блока", "платы").
pub fn docs_editor(
    ui: &mut Ui,
    id: Id,
    docs: &mut String,
    pictures: &mut Pictures,
    editable: bool,
    owner: &str,
) {
    if editable {
        let mut apply = None;
        ui.horizontal_wrapped(|ui| {
            let buttons = [
                (
                    crate::i18n::tr("Ж"),
                    crate::i18n::tr("Жирный: **текст**"),
                    Fmt::Bold,
                ),
                (
                    crate::i18n::tr("К"),
                    crate::i18n::tr("Курсив: *текст*"),
                    Fmt::Italic,
                ),
                (
                    crate::i18n::tr("`код`"),
                    crate::i18n::tr("Код в строке"),
                    Fmt::Code,
                ),
                (
                    "H1",
                    crate::i18n::tr("Заголовок первого уровня"),
                    Fmt::Heading(1),
                ),
                (
                    "H2",
                    crate::i18n::tr("Заголовок второго уровня"),
                    Fmt::Heading(2),
                ),
                (
                    "H3",
                    crate::i18n::tr("Заголовок третьего уровня"),
                    Fmt::Heading(3),
                ),
                (
                    crate::i18n::tr("• Список"),
                    crate::i18n::tr("Маркированный список; повторное нажатие снимает"),
                    Fmt::Bullets,
                ),
                (
                    crate::i18n::tr("1. Список"),
                    crate::i18n::tr("Нумерованный список; повторное нажатие снимает"),
                    Fmt::Numbered,
                ),
                (
                    crate::i18n::tr("Цитата"),
                    crate::i18n::tr("Цитата: > текст"),
                    Fmt::Quote,
                ),
                (
                    crate::i18n::tr("Блок кода"),
                    crate::i18n::tr("Блок кода в ```"),
                    Fmt::CodeBlock,
                ),
                (
                    crate::i18n::tr("Таблица"),
                    crate::i18n::tr("Таблица 2×2"),
                    Fmt::Table,
                ),
                (
                    crate::i18n::tr("Ссылка"),
                    crate::i18n::tr("[текст](адрес)"),
                    Fmt::Link,
                ),
            ];
            for (text, hint, fmt) in buttons {
                if ui.button(text).on_hover_text(hint).clicked() {
                    apply = Some(fmt);
                }
            }
            let hint = crate::i18n::fmt(
                "Вставить картинку из файла (png, jpg, gif, bmp). \
                 При сохранении она копируется в папку {0}",
                &[&owner],
            );
            if ui
                .button(crate::i18n::tr("Картинка..."))
                .on_hover_text(hint)
                .clicked()
            {
                let file = rfd::FileDialog::new()
                    .set_title(crate::i18n::tr("Картинка для описания"))
                    .add_filter(
                        crate::i18n::tr("Изображения"),
                        &["png", "jpg", "jpeg", "gif", "bmp"],
                    )
                    .pick_file();
                if let Some(file) = file {
                    apply = Some(Fmt::Image(pictures.add(file)));
                }
            }
        });
        if let Some(fmt) = apply {
            format_at_cursor(ui.ctx(), id, docs, &fmt);
        }
    }
    ui.add_space(4.0);
    ui.columns(2, |cols| {
        cols[0].add(
            TextEdit::multiline(docs)
                .id(id)
                .interactive(editable)
                .desired_width(f32::INFINITY)
                .desired_rows(20),
        );
        cols[1].label(RichText::new(crate::i18n::tr("Предпросмотр")).weak());
        let find = |target: &str| pictures.path(target);
        crate::markdown::show_with_images(&mut cols[1], docs, None, Some(&find));
    });
}

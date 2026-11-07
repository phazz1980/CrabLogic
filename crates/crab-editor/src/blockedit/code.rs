use super::*;

pub(super) fn code_tab(ui: &mut Ui, f: &mut Form, editable: bool) {
    ui.horizontal(|ui| {
        ui.label(crate::i18n::tr("Заголовки"));
        ui.add_enabled(
            editable,
            TextEdit::singleline(&mut f.includes)
                .hint_text("DHT.h, Wire.h")
                .desired_width(260.0),
        )
        .on_hover_text(crate::i18n::tr("Файлы для #include через запятую"));
    });
    ui.weak(crate::i18n::tr(SECTIONS_HINT));
    let inputs = port_snippets(&f.inputs, "input");
    let outputs = port_snippets(&f.outputs, "output");
    let literal: Vec<Snippet> = f
        .params
        .iter()
        .map(|p| {
            let hint = crate::i18n::fmt("{0} — значение как литерал C++", &[&kind_label(p.kind)]);
            Snippet::new(
                row_title(&p.name, &p.label),
                format!("{{{{ param.{} }}}}", p.name),
                hint,
            )
        })
        .collect();
    let conditions: Vec<Snippet> = f
        .params
        .iter()
        .map(|p| {
            let hint = crate::i18n::tr("Код внутри попадёт в скетч, только если условие выполнено");
            let text = match p.kind {
                ParamKind::Bool => format!(
                    "{{% if value.{} %}}

{{% endif %}}
",
                    p.name
                ),
                ParamKind::Text => {
                    format!(
                        "{{% if value.{} == \"\" %}}

{{% endif %}}
",
                        p.name
                    )
                }
                _ => format!(
                    "{{% if value.{} > 0 %}}

{{% endif %}}
",
                    p.name
                ),
            };
            Snippet::new(row_title(&p.name, &p.label), text, hint)
        })
        .collect();
    let sections: Vec<Snippet> = [
        (
            "// @global",
            crate::i18n::tr("Переменные блока, по одной на экземпляр"),
        ),
        ("// @setup", crate::i18n::tr("Код в setup()")),
        ("// @loop", crate::i18n::tr("Код цикла, в порядке схемы")),
        (
            "// @functions",
            crate::i18n::tr("Функции блока, перед setup()"),
        ),
        (
            "// @global once",
            crate::i18n::tr("Общие переменные, один раз на проект"),
        ),
        (
            "// @setup once",
            crate::i18n::tr("Общий код в setup(), один раз"),
        ),
        (
            "// @functions once",
            crate::i18n::tr("Общие функции, один раз на проект"),
        ),
    ]
    .into_iter()
    .map(|(text, hint)| {
        Snippet::new(
            "",
            format!(
                "{text}
"
            ),
            hint,
        )
    })
    .collect();

    if !editable {
        let mut text = f.template.as_str();
        let mut layouter = crate::cpphl::layouter(true);
        ui.add(
            TextEdit::multiline(&mut text)
                .code_editor()
                .layouter(&mut layouter)
                .desired_width(f32::INFINITY)
                .desired_rows(24),
        );
        return;
    }
    let id = Id::unique("block_template");
    let mut insert = None;
    ui.horizontal_wrapped(|ui| {
        ui.label(crate::i18n::tr("Вставить:"));
        let buttons = inputs.iter().chain(&outputs).chain(&literal);
        for s in std::iter::once(&Snippet::new("", "{{ id }}", "")).chain(buttons) {
            if ui
                .small_button(RichText::new(&s.text).monospace())
                .clicked()
            {
                insert = Some(s.text.clone());
            }
        }
    });
    ui.weak(crate::i18n::tr(
        "Правая кнопка в поле кода — меню подстановок; вставка идёт в место курсора.",
    ));
    let mut layouter = crate::cpphl::layouter(true);
    let output = TextEdit::multiline(&mut f.template)
        .id(id)
        .code_editor()
        .layouter(&mut layouter)
        .desired_width(f32::INFINITY)
        .desired_rows(24)
        .show(ui);
    output.response.context_menu(|ui| {
        if ui
            .button(crate::i18n::tr("{{ id }} — префикс переменных блока"))
            .on_hover_text(crate::i18n::tr(
                "Имена в разделах без once начинайте с него: {{ id }}_start",
            ))
            .clicked()
        {
            insert = Some("{{ id }}".into());
            ui.close();
        }
        ui.separator();
        snippet_menu(ui, crate::i18n::tr("Вход"), &inputs, &mut insert);
        snippet_menu(ui, crate::i18n::tr("Выход"), &outputs, &mut insert);
        snippet_menu(ui, crate::i18n::tr("Параметр"), &literal, &mut insert);
        snippet_menu(
            ui,
            crate::i18n::tr("Условие по параметру"),
            &conditions,
            &mut insert,
        );
        ui.separator();
        snippet_menu(ui, crate::i18n::tr("Раздел"), &sections, &mut insert);
    });
    if let Some(text) = insert {
        insert_at_cursor(ui.ctx(), id, &mut f.template, &text);
    }
}

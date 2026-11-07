
use super::*;

#[test]
fn form_round_trip() {
    let src = r#"
[block]
type = "x"
name = "X"
group = "Тест"
includes = ["A.h", "B.h"]

[[input]]
name = "k"
label = "Коэффициент"
type = "float"
default = 2.5

[[output]]
name = "q"
type = "bool"

[[param]]
name = "t"
type = "duration"
default = "1s"
min = 0
"#;
    let def: BlockDef = toml::from_str(src).unwrap();
    let block = LibraryBlock {
        def: def.clone(),
        dir: Default::default(),
        inline_templates: [(ESP32_CPP_TEMPLATE, "// @loop\n".to_owned())].into(),
    };
    let form = Form::from_block(&block);
    assert_eq!(form.includes, "A.h, B.h");
    assert_eq!(form.inputs[0].label, "Коэффициент");
    assert_eq!(form.to_def(), Ok(def));
    assert_eq!(form.template, "// @loop\n");
}

fn hx_wizard() -> LibraryWizard {
    use crab_core::native_wizard::{WizAction, WizArg, WizOutput, WizParam};
    LibraryWizard {
        lib: "GyverHX711".into(),
        header: "GyverHX711.h".into(),
        class: "GyverHX711".into(),
        params: vec![
            WizParam {
                name: "data".into(),
                ty: WizType::Pin,
            },
            WizParam {
                name: "clock".into(),
                ty: WizType::Pin,
            },
        ],
        args: vec![WizArg::Param("data".into()), WizArg::Param("clock".into())],
        init: vec![],
        outputs: vec![WizOutput {
            name: "value".into(),
            ty: WizType::Int,
            expr: "read()".into(),
            when: "available()".into(),
        }],
        actions: vec![WizAction {
            input: "tare".into(),
            call: "tare()".into(),
        }],
    }
}

#[test]
fn the_wizard_makes_and_renames_rows() {
    let mut form = Form::new_block("hx".into(), String::new());
    form.inputs.clear();
    form.outputs.clear();
    form.wizard = Some(hx_wizard());
    sync_wizard(&mut form);
    let names = |f: &Form| {
        (
            f.params.iter().map(|r| r.name.clone()).collect::<Vec<_>>(),
            f.inputs.iter().map(|r| r.name.clone()).collect::<Vec<_>>(),
            f.outputs.iter().map(|r| r.name.clone()).collect::<Vec<_>>(),
        )
    };
    assert_eq!(
        names(&form),
        (
            vec!["data".to_owned(), "clock".to_owned()],
            vec!["tare".to_owned()],
            vec!["value".to_owned()]
        )
    );
    assert_eq!(form.params[0].kind, ParamKind::Pin);
    let def = form.to_def().unwrap();
    crab_core::native::parse_native(&form.native, &def).unwrap();
    assert_eq!(LibraryWizard::parse(&form.native), form.wizard);

    // A caption written on the "Порты" tab survives a rename in the wizard.
    form.outputs[0].label = "Вес".into();
    let w = form.wizard.as_mut().unwrap();
    w.outputs[0].name = "weight".into();
    w.params.pop();
    w.args.pop();
    sync_wizard(&mut form);
    assert_eq!(form.outputs[0].name, "weight");
    assert_eq!(form.outputs[0].label, "Вес");
    assert_eq!(form.params.len(), 1);

    // A wizard with a mistake leaves the rows and the code alone.
    let before = (form.params.clone(), form.native.clone());
    form.wizard.as_mut().unwrap().class = String::new();
    sync_wizard(&mut form);
    assert_eq!((form.params.clone(), form.native.clone()), before);
}

#[test]
fn form_errors() {
    let mut form = Form::new_block("ok".into(), String::new());
    assert!(form.to_def().is_ok());
    form.inputs[0].default = "maybe".into();
    assert!(form.to_def().unwrap_err().contains("maybe"));
    form.inputs[0].default = "да".into();
    assert_eq!(
        form.to_def().unwrap().inputs[0].default,
        Some(ParamValue::Bool(true))
    );
    form.kind = VAR_GET.into();
    assert!(form.to_def().is_err());
    form.kind = "ok".into();
    form.outputs[0].name = "in".into();
    assert!(form.to_def().is_err());
}

#[test]
fn insert_at_cursor_replaces_selection() {
    use egui::text::{CCursor, CCursorRange};
    let ctx = egui::Context::default();
    let id = Id::unique("test_template");
    let mut text = "аб".to_owned();
    insert_at_cursor(&ctx, id, &mut text, "{{ id }}");
    assert_eq!(text, "аб{{ id }}");

    let mut state = egui::text_edit::TextEditState::default();
    let range = CCursorRange::two(CCursor::new(1), CCursor::new(2));
    state.cursor.set_char_range(Some(range));
    state.store(&ctx, id);
    let mut text = "абв".to_owned();
    insert_at_cursor(&ctx, id, &mut text, "X");
    assert_eq!(text, "аXв");
    // The cursor moved after the inserted text.
    insert_at_cursor(&ctx, id, &mut text, "Y");
    assert_eq!(text, "аXYв");
}

#[test]
fn docs_formatting_wraps_selection() {
    // "абв" with "б" selected.
    let (text, from, to) = format_selection("абв", 1, 2, &Fmt::Bold);
    assert_eq!((text.as_str(), from, to), ("а**б**в", 3, 4));
    // No selection: a placeholder is inserted and selected.
    let (text, from, to) = format_selection("", 0, 0, &Fmt::Code);
    assert_eq!((text.as_str(), from, to), ("`текст`", 1, 6));
    let (text, from, to) = format_selection("см. ", 4, 4, &Fmt::Link);
    assert_eq!(text, "см. [текст](https://)");
    assert_eq!((from, to), (5, 10));
    let (text, from, to) = format_selection("сайт", 0, 4, &Fmt::Link);
    assert_eq!(text, "[сайт](https://)");
    assert_eq!((from, to), (7, 15));
}

#[test]
fn docs_formatting_prefixes_lines() {
    let (text, ..) = format_selection(
        "один
два
три",
        2,
        7,
        &Fmt::Bullets,
    );
    assert_eq!(
        text,
        "- один
- два
три"
    );
    // Again: the prefix goes away.
    let (text, ..) = format_selection(&text, 0, 10, &Fmt::Bullets);
    assert_eq!(
        text,
        "один
два
три"
    );
    let (text, ..) = format_selection(
        "а
б",
        0,
        3,
        &Fmt::Numbered,
    );
    assert_eq!(
        text,
        "1. а
2. б"
    );
    // A heading level replaces another one and toggles off on the same.
    let (text, ..) = format_selection("# Тема", 0, 0, &Fmt::Heading(2));
    assert_eq!(text, "## Тема");
    let (text, ..) = format_selection(&text, 0, 0, &Fmt::Heading(2));
    assert_eq!(text, "Тема");
}

#[test]
fn docs_blocks_stand_on_own_lines() {
    let (text, ..) = format_selection("текст", 5, 5, &Fmt::Image("images/a.png".into()));
    assert_eq!(
        text,
        "текст
![описание](images/a.png)
"
    );
    let (text, from, to) = format_selection("", 0, 0, &Fmt::CodeBlock);
    assert_eq!(
        (text.as_str(), from, to),
        (
            "```
код
```
",
            4,
            7
        )
    );
}

#[test]
fn docs_pictures_are_copied_and_pruned() {
    let dir = std::env::temp_dir().join(format!("crab_docs_img_{}", std::process::id()));
    let block = dir.join("block");
    fs::create_dir_all(&block).unwrap();
    let pic = dir.join("Схема 1.png");
    fs::write(&pic, "png").unwrap();
    fs::write(dir.join("old.png"), "png").unwrap();

    let mut attach = Attach::default();
    let target = attach.pictures.add(pic.clone());
    assert_eq!(target, "images/1.png");
    assert_eq!(attach.pictures.path(&target), Some(pic.clone()));
    // The same file again gets another name.
    assert_eq!(attach.pictures.add(pic), "images/1_2.png");

    let docs = format!(
        "![схема]({target})
"
    );
    attach.apply(&block, &docs).unwrap();
    let root = block.join(crab_core::blockfile::IMAGES_DIR);
    assert!(root.join("1.png").is_file());
    assert!(!root.join("1_2.png").exists());
    assert_eq!(attach.pictures.path(&target), Some(root.join("1.png")));

    // A picture the docs no longer use is removed with the folder.
    attach.apply(&block, "без картинок").unwrap();
    assert!(!root.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn attached_library_adds_its_header() {
    let dir = std::env::temp_dir().join(format!("crab_lib_header_{}", std::process::id()));
    let lib = dir.join("DHT sensor library");
    fs::create_dir_all(lib.join("src")).unwrap();
    fs::write(lib.join("src/DHT.h"), "").unwrap();
    fs::write(lib.join("src/DHT_U.h"), "").unwrap();
    assert_eq!(
        library_header(&lib, "dht"),
        Some("DHT.h".to_owned()),
        "header named like the library wins"
    );
    assert_eq!(library_header(&lib, "other"), None);
    fs::remove_dir_all(&dir).unwrap();
    assert_eq!(with_include("Wire.h", "DHT.h"), "Wire.h, DHT.h");
    assert_eq!(with_include("DHT.h", "DHT.h"), "DHT.h");
    assert_eq!(without_include("Wire.h, DHT.h", "DHT.h"), "Wire.h");
}

#[test]
fn rename_follows_in_template() {
    let t =
        "{{ output.out }} = {{ input.in }} + {{ input.in2 }}; {{ input[\"in\"] }} {{ param.in }}";
    assert_eq!(
            rename_in_template(t, "input", "in", "вход"),
            "{{ output.out }} = {{ input.вход }} + {{ input.in2 }}; {{ input[\"вход\"] }} {{ param.in }}"
        );
    let mut form = Form::new_block("demo".into(), String::new());
    form.inputs[0].name = "вход".into();
    form.template = rename_in_template(&form.template, "input", "in", "вход");
    let def = form.to_def().unwrap();
    assert!(crab_codegen::preview_block(&def, &form.template).is_ok());
}

#[test]
fn new_block_generates() {
    let form = Form::new_block("demo".into(), String::new());
    let def = form.to_def().unwrap();
    let code = crab_codegen::preview_block(&def, &form.template).unwrap();
    assert!(
        code.contains("demo_1_out = false;") || code.contains("demo_1_out"),
        "{code}"
    );
}

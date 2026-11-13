use super::*;

#[test]
fn form_round_trip() {
    let src = r#"
[block]
type = "x"
name = "X"
group = "Тест"

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
    };
    let form = Form::from_block(&block);
    assert_eq!(form.inputs[0].label, "Коэффициент");
    assert_eq!(form.to_def(), Ok(def));
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

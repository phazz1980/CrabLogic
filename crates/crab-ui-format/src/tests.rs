use crate::*;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

fn w(kind: WidgetKind, x: u8, y: u8, input: Option<u16>) -> Widget {
    Widget {
        kind,
        x,
        y,
        w: 40,
        h: 8,
        font: 8,
        input,
    }
}

fn item(parent: Option<u16>, kind: MenuItemKind, cap: &str, var: Option<u16>) -> MenuItem {
    MenuItem {
        parent,
        kind,
        caption: cap.to_string(),
        var_slot: var,
    }
}

fn sample() -> Config {
    let mut c = Config::new(DisplayKind::Ssd1306_128x64);
    c.screens.push(Screen {
        widgets: vec![
            w(
                WidgetKind::Text {
                    text: "Давление".to_string(),
                },
                0,
                0,
                None,
            ),
            w(
                WidgetKind::Value {
                    format: 0,
                    decimals: 1,
                    unit: "бар".to_string(),
                },
                0,
                16,
                Some(0),
            ),
            w(
                WidgetKind::Bar {
                    min: 0.0,
                    max: 10.0,
                },
                0,
                40,
                Some(0),
            ),
        ],
    });
    c.screens.push(Screen {
        widgets: vec![
            w(WidgetKind::Led, 100, 0, Some(1)),
            w(WidgetKind::Icon { id: 3 }, 110, 0, None),
        ],
    });
    c.menu = Some(Menu {
        timeout_s: 30,
        font: 8,
        hints: false,
        items: vec![
            item(None, MenuItemKind::Submenu, "Настройки", None),
            item(Some(0), MenuItemKind::Submenu, "Насос", None),
            item(
                Some(1),
                MenuItemKind::Number {
                    min: 0.0,
                    max: 10.0,
                    step: 0.5,
                    decimals: 1,
                    unit: "бар".to_string(),
                },
                "Уставка",
                Some(0),
            ),
            item(Some(1), MenuItemKind::Bool, "Включён", Some(1)),
            item(
                Some(0),
                MenuItemKind::Choice {
                    choices: vec!["Авто".to_string(), "Ручной".to_string()],
                },
                "Режим",
                Some(2),
            ),
            item(None, MenuItemKind::Action, "Сброс", None),
            item(None, MenuItemKind::View, "Температура", Some(3)),
        ],
    });
    c
}

#[test]
fn round_trip() {
    let c = sample();
    let bytes = c.encode();
    let v = ConfigView::parse(&bytes).unwrap();
    assert_eq!(v.display, c.display);
    assert_eq!(v.screen_count(), 2);

    for (sv, s) in v.screens().zip(&c.screens) {
        assert_eq!(sv.len(), s.widgets.len());
        let got: Vec<WidgetView> = sv.widgets().collect();
        assert_eq!(got.len(), s.widgets.len());
        for (g, e) in got.iter().zip(&s.widgets) {
            assert_eq!(
                (g.x, g.y, g.w, g.h, g.font, g.input),
                (e.x, e.y, e.w, e.h, e.font, e.input)
            );
            match (&g.kind, &e.kind) {
                (WidgetKindView::Text { text }, WidgetKind::Text { text: t }) => {
                    assert_eq!(*text, t)
                }
                (
                    WidgetKindView::Value {
                        format,
                        decimals,
                        unit,
                    },
                    WidgetKind::Value {
                        format: f,
                        decimals: d,
                        unit: u,
                    },
                ) => assert_eq!((*format, *decimals, *unit), (*f, *d, u.as_str())),
                (WidgetKindView::Bar { min, max }, WidgetKind::Bar { min: a, max: b }) => {
                    assert_eq!((*min, *max), (*a, *b))
                }
                (WidgetKindView::Led, WidgetKind::Led) => {}
                (WidgetKindView::Icon { id }, WidgetKind::Icon { id: i }) => assert_eq!(id, i),
                _ => panic!("kind mismatch"),
            }
        }
    }

    let m = v.menu.unwrap();
    let em = c.menu.as_ref().unwrap();
    assert_eq!(m.timeout_s, 30);
    assert_eq!(m.len(), em.items.len());
    for (g, e) in m.items().zip(&em.items) {
        assert_eq!(g.parent, e.parent);
        assert_eq!(g.var_slot, e.var_slot);
        assert_eq!(g.caption, e.caption);
        match (&g.kind, &e.kind) {
            (ItemKindView::Submenu, MenuItemKind::Submenu)
            | (ItemKindView::Bool, MenuItemKind::Bool)
            | (ItemKindView::Action, MenuItemKind::Action)
            | (ItemKindView::View, MenuItemKind::View) => {}
            (
                ItemKindView::Number {
                    min,
                    max,
                    step,
                    decimals,
                    unit,
                },
                MenuItemKind::Number {
                    min: a,
                    max: b,
                    step: s,
                    decimals: d,
                    unit: u,
                },
            ) => {
                assert_eq!(
                    (*min, *max, *step, *decimals, *unit),
                    (*a, *b, *s, *d, u.as_str())
                )
            }
            (ItemKindView::Choice { choices }, MenuItemKind::Choice { choices: c }) => {
                let got: Vec<&str> = choices.clone().collect();
                let exp: Vec<&str> = c.iter().map(String::as_str).collect();
                assert_eq!(got, exp);
            }
            _ => panic!("item kind mismatch"),
        }
    }
    // Three levels deep: "Уставка" -> "Насос" -> "Настройки".
    let items: Vec<ItemView> = m.items().collect();
    assert_eq!(items[2].parent, Some(1));
    assert_eq!(items[1].parent, Some(0));
    assert_eq!(items[0].parent, None);
}

#[test]
fn no_menu_and_empty() {
    let c = Config::new(DisplayKind::Lcd1602);
    let b = c.encode();
    let v = ConfigView::parse(&b).unwrap();
    assert!(v.menu.is_none());
    assert_eq!(v.screens().count(), 0);
}

#[test]
fn every_prefix_fails() {
    let bytes = sample().encode();
    for n in 0..bytes.len() {
        assert!(ConfigView::parse(&bytes[..n]).is_err(), "prefix {n} parsed");
    }
    assert!(ConfigView::parse(&bytes).is_ok());
}

#[test]
fn trailing_fails() {
    let mut b = sample().encode();
    b.push(0);
    assert_eq!(ConfigView::parse(&b).err(), Some(FormatError::Trailing));
}

#[test]
fn corruption_never_panics() {
    let bytes = sample().encode();
    for i in 0..bytes.len() {
        for v in [0u8, 1, 0x7F, 0x80, 0xC0, 0xFF] {
            let mut b = bytes.clone();
            b[i] = v;
            if let Ok(view) = ConfigView::parse(&b) {
                // A validated view must be fully iterable.
                for s in view.screens() {
                    let _ = s.widgets().count();
                }
                if let Some(m) = view.menu {
                    for it in m.items() {
                        if let ItemKindView::Choice { choices } = it.kind {
                            let _ = choices.count();
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn wrong_version() {
    let mut c = sample();
    c.version = 3;
    assert_eq!(
        ConfigView::parse(&c.encode()).err(),
        Some(FormatError::BadVersion)
    );
}

#[test]
fn bad_parent() {
    let mut c = sample();
    c.menu.as_mut().unwrap().items[0].parent = Some(0);
    assert_eq!(
        ConfigView::parse(&c.encode()).err(),
        Some(FormatError::BadParent)
    );
}

#[test]
fn typical_size() {
    let n = sample().encode().len();
    std::println!("encoded size of the sample config: {n} bytes");
    assert!(n < 2048);
}

#[test]
fn menu_style_round_trips() {
    let c = sample();
    let bytes = c.encode();
    let v = ConfigView::parse(&bytes).unwrap().menu.unwrap();
    assert_eq!((v.font, v.hints), (8, false));
}

#[test]
fn version_one_menu_reads_with_default_style() {
    let mut c = sample();
    c.version = 1;
    let bytes = c.encode();
    let v = ConfigView::parse(&bytes).unwrap().menu.unwrap();
    assert_eq!((v.timeout_s, v.font, v.hints), (30, 16, true));
    assert_eq!(v.len(), c.menu.as_ref().unwrap().items.len());
}

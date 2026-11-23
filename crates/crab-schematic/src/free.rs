//! Free sheets: drawing sheets that the generator does not fill. What is put on them by hand is
//! the whole drawing: the devices and the wires drawn between their terminals, the same way as on
//! the power and the control sheets (`specs/panel-free-sheets`).

use crab_core::{split_part, Project, SheetId, MAX_SHEETS};

use crate::canvas::Canvas;
use crate::control::{draw_hand, generated_on_control, Hand, Pins, Refs};
use crate::doc::{Page, Sheet};
use crate::lang::Lang;
use crate::nets::Nets;
use crate::xref::Xrefs;

/// How many free sheets the layout asks for: up to the last one that has a caption or a device
/// put on it by hand. Free sheets are numbered `free:0`, `free:1`, … with no gap.
pub(crate) fn count(project: &Project) -> usize {
    let layout = &project.panel_layout;
    let placed = layout
        .placed
        .iter()
        .filter(|(key, _)| project.panel.device(split_part(key).0).is_some())
        .filter_map(|(_, at)| match at.sheet {
            SheetId::Free(k) => Some(k),
            _ => None,
        });
    let captioned = layout
        .captions
        .keys()
        .filter_map(|key| match key.parse::<SheetId>() {
            Ok(SheetId::Free(k)) => Some(k),
            _ => None,
        });
    placed
        .chain(captioned)
        .map(|k| k.saturating_add(1))
        .max()
        .unwrap_or(0)
        .min(MAX_SHEETS)
}

/// The free sheets `free:0`, `free:1`, … (`count` of them), the first one with the document number
/// `first`. `contacts` are the places of the devices the generator draws on the power sheets,
/// `coils` the places of the coils and of the auxiliary contacts (the references under them).
/// Returns the sheets and the places of the coils and auxiliary contacts put on them by hand.
#[allow(clippy::too_many_arguments)]
pub(crate) fn free_sheets(
    project: &Project,
    nets: &Nets,
    lang: Lang,
    first: usize,
    count: usize,
    contacts: &Xrefs,
    coils: &Xrefs,
    page: &Page,
) -> (Vec<Sheet>, Xrefs) {
    let mut generated = generated_on_control(&project.panel, nets);
    generated.extend(contacts.keys().cloned());
    let hand = Hand {
        panel: &project.panel,
        layout: &project.panel_layout,
        nets,
        page,
        refs: Refs {
            power: contacts,
            coils,
        },
        generated: &generated,
    };
    let mut sheets = Vec::new();
    let mut places = Xrefs::new();
    for index in 0..count {
        let id = SheetId::Free(index);
        let mut c = Canvas::new();
        c.use_labels(&project.panel_layout);
        c.set_sheet(&format!("f{index}"));
        let mut pins = Pins::new();
        draw_hand(
            &mut c,
            &hand,
            id,
            index,
            first + index,
            &mut pins,
            &mut places,
        );
        // The default caption tells the sheets apart in the contents: `Схема 1`, `Схема 2`, …
        let default = format!("{} {}", lang.pick("Схема", "Diagram"), index + 1);
        let title = project
            .panel_layout
            .caption(id)
            .map_or(default, str::to_owned);
        sheets.push(Sheet::new(Some(id), title, c));
    }
    (sheets, places)
}

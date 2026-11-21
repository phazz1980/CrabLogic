//! Electrical documentation of a project's panel (`specs/electrical-schematics`).
//!
//! The panel (`panel.toml`) lists devices; here the wires between them are worked out and
//! numbered ([`nets`]), laid out on A3 (or A4) sheets ([`power`]) and written as Typst markup ([`doc`]),
//! which `crab-pdf` turns into a PDF.

mod canvas;
pub mod control;
pub mod doc;
mod error;
mod lang;
pub mod nets;
pub mod power;
mod symbols;
pub mod tables;
pub mod title;
pub mod xref;

use crab_core::{Library, Project};

pub use canvas::{Align, Op, Rgb, EARTH, INK, LOW, WEAK};
pub use doc::{Document, Format, Page, Sheet};
pub use error::Error;
pub use lang::Lang;
pub use nets::{build_nets, BoardTerminal, Nets, Pin, Wire, WireClass, BOARD, EXTERNAL};
pub use tables::{device_kind_name, Table, Tables};
pub use xref::{Xref, Xrefs};

/// The documents of a project: the sheets and the wires with their numbers. Run
/// [`crab_core::check_panel`] first. The numbers are not stored in the project: call
/// [`Nets::store`] and save it to keep them.
pub fn build_document(
    project: &Project,
    lib: &Library,
    lang: Lang,
) -> Result<(Document, Nets), Error> {
    build_document_in(project, lib, lang, Format::default())
}

/// [`build_document`] on sheets of the given format.
pub fn build_document_in(
    project: &Project,
    lib: &Library,
    lang: Lang,
    format: Format,
) -> Result<(Document, Nets), Error> {
    let page = Page::new(format);
    let nets = build_nets(project, lib);
    let panel = &project.panel;
    // The title sheet is number 1, the power sheets follow it, then the control sheets.
    // Two passes: the first one places the devices (the number of sheets and the zones do not
    // depend on the captions), the second one writes the references to the places of the other part.
    let none = Xrefs::new();
    let (power, power_at) = power::power_sheets(panel, &nets, lang, 2, &none, &page)?;
    let first_control = power.len() + 2;
    let (_, coil_at) = control::control_sheets(project, &nets, lang, first_control, &none, &page);
    let (power, _) = power::power_sheets(panel, &nets, lang, 2, &coil_at, &page)?;
    let (control, _) =
        control::control_sheets(project, &nets, lang, first_control, &power_at, &page);
    let mut sheets = power;
    sheets.extend(control);
    // The tables come last; the sheets are only for the panel that has something to draw.
    let tables = tables::build_tables(project, &nets, lang);
    if !sheets.is_empty() {
        for table in [&tables.terminals, &tables.wires, &tables.bom] {
            sheets.extend(tables::table_sheets(table, &page));
        }
    }
    if !sheets.is_empty() {
        let titles: Vec<&str> = sheets.iter().map(|s| s.title.as_str()).collect();
        let title = title::title_sheet(project, &nets, &titles, lang, &page);
        sheets.insert(0, title);
    }
    let doc = Document {
        name: project.display_name(),
        page,
        sheets,
        power_at,
        coil_at,
        tables,
    };
    Ok((doc, nets))
}

/// The Typst markup of the documentation of a project.
pub fn source(project: &Project, lib: &Library, lang: Lang) -> Result<String, Error> {
    source_in(project, lib, lang, Format::default())
}

/// [`source`] on sheets of the given format.
pub fn source_in(
    project: &Project,
    lib: &Library,
    lang: Lang,
    format: Format,
) -> Result<String, Error> {
    let (doc, _) = build_document_in(project, lib, lang, format)?;
    Ok(doc::typst_source(&doc, lang))
}

/// The PDF of the documentation of a project (needs the `crablogic-pdf` helper).
pub fn render_pdf(project: &Project, lib: &Library, lang: Lang) -> Result<Vec<u8>, Error> {
    render_pdf_in(project, lib, lang, Format::default())
}

/// [`render_pdf`] on sheets of the given format.
pub fn render_pdf_in(
    project: &Project,
    lib: &Library,
    lang: Lang,
    format: Format,
) -> Result<Vec<u8>, Error> {
    Ok(crab_pdf::compile(source_in(project, lib, lang, format)?)?)
}

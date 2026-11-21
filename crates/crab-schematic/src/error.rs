/// Why the documentation could not be made.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Pdf(#[from] crab_pdf::PdfError),
}

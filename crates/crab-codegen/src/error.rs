use crab_core::{Diagnostic, PortRef};

/// Code generation errors. Messages are user-facing, hence in Russian.
#[derive(Debug, thiserror::Error)]
pub enum GenError {
    /// The schema did not pass the check; diagnostics include warnings too.
    #[error("в схеме есть ошибки, генерация невозможна")]
    Schema(Vec<Diagnostic>),

    #[error("выходы {a} и {b} дают одно имя переменной «{name}»; переименуйте блок")]
    NameClash {
        name: String,
        a: PortRef,
        b: PortRef,
    },
}

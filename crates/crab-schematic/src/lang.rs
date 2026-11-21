//! Language of the captions on the sheets.

/// Language of the documentation; switchable for each build (`crablogic panel --lang`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    Ru,
    En,
}

impl Lang {
    /// The caption in this language.
    pub fn pick(self, ru: &'static str, en: &'static str) -> &'static str {
        match self {
            Lang::Ru => ru,
            Lang::En => en,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::Ru => "ru",
            Lang::En => "en",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        match code {
            "ru" => Some(Lang::Ru),
            "en" => Some(Lang::En),
            _ => None,
        }
    }
}

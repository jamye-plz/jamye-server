//! Framework-free moderation rules: report vocabulary, the report snapshot bounds, and the
//! chat-message content filter (decision R8: mask, never reject).

use std::{error::Error, fmt};

/// Replacement for every masked term occurrence.
pub const MASK: &str = "***";
/// Embedded default list; one term per line, `#` comments and blank lines ignored.
pub const DEFAULT_TERMS: &str = include_str!("default_terms.txt");
pub const MAX_FILTER_TERMS: usize = 500;
pub const MAX_FILTER_TERM_CHARS: usize = 64;
/// A one-character term would mask that character in every chat message.
pub const MIN_FILTER_TERM_CHARS: usize = 2;
/// Longest message text stored in a report snapshot.
pub const MAX_SNAPSHOT_TEXT_CHARS: usize = 4000;
/// Fixed report rate limit (assumption A23): 20 reports per hour per reporter.
pub const REPORT_RATE_LIMIT: u32 = 20;
pub const REPORT_RATE_WINDOW_SECONDS: u64 = 3600;
/// Upper bound on `JAMYE_OPERATOR_ACCOUNT_IDS`.
pub const MAX_OPERATOR_ACCOUNTS: usize = 10;
pub const MAX_SUSPENSION_REASON_CHARS: usize = 500;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportReason {
    Spam,
    Harassment,
    SexualContent,
    Violence,
    Hate,
    Illegal,
    Other,
}

impl ReportReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spam => "spam",
            Self::Harassment => "harassment",
            Self::SexualContent => "sexual_content",
            Self::Violence => "violence",
            Self::Hate => "hate",
            Self::Illegal => "illegal",
            Self::Other => "other",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "spam" => Some(Self::Spam),
            "harassment" => Some(Self::Harassment),
            "sexual_content" => Some(Self::SexualContent),
            "violence" => Some(Self::Violence),
            "hate" => Some(Self::Hate),
            "illegal" => Some(Self::Illegal),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportStatus {
    Open,
    Actioned,
    Dismissed,
}

impl ReportStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Actioned => "actioned",
            Self::Dismissed => "dismissed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "open" => Some(Self::Open),
            "actioned" => Some(Self::Actioned),
            "dismissed" => Some(Self::Dismissed),
            _ => None,
        }
    }
}

/// Bound the stored snapshot text to `MAX_SNAPSHOT_TEXT_CHARS` characters.
pub fn bounded_snapshot_text(text: &str) -> String {
    text.chars().take(MAX_SNAPSHOT_TEXT_CHARS).collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentFilterError {
    EmptyList,
    TooManyTerms,
    TermTooLong,
    TermTooShort,
    ControlCharacter,
}

impl fmt::Display for ContentFilterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyList => "the content filter list has no terms",
            Self::TooManyTerms => "the content filter list has too many terms",
            Self::TermTooLong => "a content filter term is too long",
            Self::TermTooShort => "a content filter term is shorter than 2 characters",
            Self::ControlCharacter => "a content filter term contains a control character",
        })
    }
}

impl Error for ContentFilterError {}

/// Case-insensitive substring masker. A body without a listed term is returned unchanged,
/// byte for byte. Matching is left to right and prefers the longest term at a position.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ContentFilter {
    /// Folded terms, longest first.
    terms: Vec<Vec<char>>,
}

impl ContentFilter {
    /// A filter without terms: every body passes through unchanged (tests, unconfigured callers).
    pub fn disabled() -> Self {
        Self::default()
    }

    /// The embedded default list.
    pub fn embedded() -> Self {
        // The embedded list is a compile-time asset covered by a unit test; the fallback keeps
        // a corrupted asset from taking the server down.
        Self::from_lines(DEFAULT_TERMS).unwrap_or_default()
    }

    /// Parse a one-term-per-line list. Blank lines and `#` comment lines are skipped.
    pub fn from_lines(source: &str) -> Result<Self, ContentFilterError> {
        Self::from_terms(
            source
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#')),
        )
    }

    pub fn from_terms<'a>(
        terms: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, ContentFilterError> {
        let mut folded: Vec<Vec<char>> = Vec::new();
        for term in terms {
            let term = term.trim();
            if term.is_empty() {
                continue;
            }
            if term.chars().any(char::is_control) {
                return Err(ContentFilterError::ControlCharacter);
            }
            let length = term.chars().count();
            if length > MAX_FILTER_TERM_CHARS {
                return Err(ContentFilterError::TermTooLong);
            }
            if length < MIN_FILTER_TERM_CHARS {
                return Err(ContentFilterError::TermTooShort);
            }
            let term = term.chars().map(fold).collect::<Vec<_>>();
            if !folded.contains(&term) {
                folded.push(term);
            }
            if folded.len() > MAX_FILTER_TERMS {
                return Err(ContentFilterError::TooManyTerms);
            }
        }
        if folded.is_empty() {
            return Err(ContentFilterError::EmptyList);
        }
        folded.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
        Ok(Self { terms: folded })
    }

    pub fn term_count(&self) -> usize {
        self.terms.len()
    }

    /// Replace every listed term occurrence with `***`.
    pub fn mask(&self, text: &str) -> String {
        if self.terms.is_empty() || text.is_empty() {
            return text.to_owned();
        }
        let original = text.chars().collect::<Vec<_>>();
        let folded = original.iter().copied().map(fold).collect::<Vec<_>>();
        let mut output = String::new();
        let mut masked_any = false;
        let mut index = 0;
        while index < original.len() {
            let matched = self
                .terms
                .iter()
                .find(|term| folded[index..].starts_with(term))
                .map(Vec::len);
            match matched {
                Some(length) => {
                    output.push_str(MASK);
                    masked_any = true;
                    index += length;
                }
                None => {
                    output.push(original[index]);
                    index += 1;
                }
            }
        }
        if masked_any { output } else { text.to_owned() }
    }

    /// Mask an optional message body; `None` stays `None`.
    pub fn mask_body(&self, body: Option<String>) -> Option<String> {
        body.map(|body| self.mask(&body))
    }
}

/// Single-char lowercase fold; characters whose lowercase form is not one char stay as is so
/// the folded and original sequences always have the same length.
fn fold(character: char) -> char {
    let mut lower = character.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(single), None) => single,
        _ => character,
    }
}

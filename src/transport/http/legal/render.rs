//! Startup-time rendering of the public legal pages: shared layout, placeholder
//! substitution with HTML escaping, and the optional address block.

use std::{error::Error, fmt};

use axum::body::Bytes;

use crate::config::legal::LegalConfig;

const ADDRESS_START: &str = "<!-- address:start -->";
const ADDRESS_END: &str = "<!-- address:end -->";
const PLACEHOLDER_OPEN: &str = "{{";
const PLACEHOLDER_CLOSE: &str = "}}";

const LAYOUT_HEAD: &str = r#"<!doctype html>
<html lang="ko">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>"#;
const LAYOUT_STYLE: &str = r#" - 잼얘좀</title>
<style>
:root { color-scheme: light dark; }
body { margin: 0; background: Canvas; color: CanvasText; font: 16px/1.7 system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Noto Sans KR", "Apple SD Gothic Neo", sans-serif; }
main { max-width: 720px; margin: 0 auto; padding: 24px 16px 48px; overflow-wrap: anywhere; }
h1 { font-size: 1.5rem; line-height: 1.3; margin: 0 0 16px; }
h2 { font-size: 1.25rem; line-height: 1.4; margin: 32px 0 8px; }
h3 { font-size: 1.05rem; margin: 20px 0 6px; }
p, ul, ol { margin: 8px 0; }
li { margin: 4px 0; }
a { color: LinkText; }
table { border-collapse: collapse; width: 100%; }
th, td { border: 1px solid color-mix(in srgb, CanvasText 25%, transparent); padding: 6px 8px; text-align: left; vertical-align: top; }
</style>
</head>
<body>
<main>
"#;
const LAYOUT_TAIL: &str = "</main>\n</body>\n</html>\n";

/// One of the four public legal documents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegalPage {
    Privacy,
    Terms,
    AccountDeletion,
    Support,
}

impl LegalPage {
    const fn name(self) -> &'static str {
        match self {
            Self::Privacy => "privacy",
            Self::Terms => "terms",
            Self::AccountDeletion => "account-deletion",
            Self::Support => "support",
        }
    }

    const fn title(self) -> &'static str {
        match self {
            Self::Privacy => "개인정보 처리방침",
            Self::Terms => "이용약관",
            Self::AccountDeletion => "계정 삭제 안내",
            Self::Support => "문의하기",
        }
    }

    const fn fragment(self) -> &'static str {
        match self {
            Self::Privacy => include_str!("../../../../docs/legal/privacy.ko.html"),
            Self::Terms => include_str!("../../../../docs/legal/terms.ko.html"),
            Self::AccountDeletion => {
                include_str!("../../../../docs/legal/account-deletion.ko.html")
            }
            Self::Support => include_str!("../../../../docs/legal/support.ko.html"),
        }
    }
}

/// Pre-rendered legal documents; requests only clone reference-counted bytes.
#[derive(Clone)]
pub struct LegalPages {
    privacy: Bytes,
    terms: Bytes,
    account_deletion: Bytes,
    support: Bytes,
}

impl LegalPages {
    /// Renders all four pages once. Any unresolved placeholder is an error.
    pub fn render(config: &LegalConfig) -> Result<Self, LegalRenderError> {
        Ok(Self {
            privacy: render_page(LegalPage::Privacy, config)?,
            terms: render_page(LegalPage::Terms, config)?,
            account_deletion: render_page(LegalPage::AccountDeletion, config)?,
            support: render_page(LegalPage::Support, config)?,
        })
    }

    pub fn page(&self, page: LegalPage) -> Bytes {
        match page {
            LegalPage::Privacy => self.privacy.clone(),
            LegalPage::Terms => self.terms.clone(),
            LegalPage::AccountDeletion => self.account_deletion.clone(),
            LegalPage::Support => self.support.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LegalRenderErrorKind {
    UnresolvedPlaceholder(&'static str),
    UnterminatedPlaceholder,
    MalformedAddressBlock,
    ResidualDelimiter,
}

/// A startup error for a legal page template. It names the page and the placeholder,
/// never an operator value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LegalRenderError {
    page: &'static str,
    kind: LegalRenderErrorKind,
}

impl LegalRenderError {
    fn new(page: &'static str, kind: LegalRenderErrorKind) -> Self {
        Self { page, kind }
    }
}

impl fmt::Display for LegalRenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let page = self.page;
        match self.kind {
            LegalRenderErrorKind::UnresolvedPlaceholder(name) => {
                write!(
                    formatter,
                    "legal page {page} has an unresolved placeholder {name:?}"
                )
            }
            LegalRenderErrorKind::UnterminatedPlaceholder => {
                write!(
                    formatter,
                    "legal page {page} has an unterminated placeholder"
                )
            }
            LegalRenderErrorKind::MalformedAddressBlock => {
                write!(formatter, "legal page {page} has a malformed address block")
            }
            LegalRenderErrorKind::ResidualDelimiter => {
                write!(
                    formatter,
                    "legal page {page} still contains a template delimiter"
                )
            }
        }
    }
}

impl Error for LegalRenderError {}

fn render_page(page: LegalPage, config: &LegalConfig) -> Result<Bytes, LegalRenderError> {
    let body = render_body(page.name(), page.fragment(), config)?;
    let mut html = String::with_capacity(body.len() + 2048);
    html.push_str(LAYOUT_HEAD);
    html.push_str(page.title());
    html.push_str(LAYOUT_STYLE);
    html.push_str(&body);
    html.push_str(LAYOUT_TAIL);
    Ok(Bytes::from(html))
}

/// Substitutes `{{OPERATOR_NAME}}`, `{{CONTACT_EMAIL}}` and `{{OPERATOR_ADDRESS}}` with
/// HTML-escaped values. The block between `<!-- address:start -->` and
/// `<!-- address:end -->` is dropped when no address is configured. Any other placeholder,
/// an unbalanced marker, or a leftover `{{` is an error.
pub fn render_body(
    page: &'static str,
    fragment: &'static str,
    config: &LegalConfig,
) -> Result<String, LegalRenderError> {
    let segments = address_segments(page, fragment, config.operator_address().is_some())?;
    let mut output = String::with_capacity(fragment.len() + 256);
    for segment in segments {
        substitute(page, segment, config, &mut output)?;
    }
    if output.contains(PLACEHOLDER_OPEN) {
        return Err(LegalRenderError::new(
            page,
            LegalRenderErrorKind::ResidualDelimiter,
        ));
    }
    Ok(output)
}

fn address_segments(
    page: &'static str,
    fragment: &'static str,
    keep_address: bool,
) -> Result<Vec<&'static str>, LegalRenderError> {
    let malformed = LegalRenderError::new(page, LegalRenderErrorKind::MalformedAddressBlock);
    let mut segments = Vec::new();
    let mut rest = fragment;
    while let Some((before, after_start)) = rest.split_once(ADDRESS_START) {
        segments.push(before);
        let Some((inner, after_end)) = after_start.split_once(ADDRESS_END) else {
            return Err(malformed);
        };
        if inner.contains(ADDRESS_START) {
            return Err(malformed);
        }
        if keep_address {
            segments.push(inner);
        }
        rest = after_end;
    }
    if rest.contains(ADDRESS_END) {
        return Err(malformed);
    }
    segments.push(rest);
    Ok(segments)
}

fn substitute(
    page: &'static str,
    segment: &'static str,
    config: &LegalConfig,
    output: &mut String,
) -> Result<(), LegalRenderError> {
    let mut rest = segment;
    while let Some((before, after_open)) = rest.split_once(PLACEHOLDER_OPEN) {
        output.push_str(before);
        let Some((name, after_close)) = after_open.split_once(PLACEHOLDER_CLOSE) else {
            return Err(LegalRenderError::new(
                page,
                LegalRenderErrorKind::UnterminatedPlaceholder,
            ));
        };
        let value = match name {
            "OPERATOR_NAME" => Some(config.operator_name()),
            "CONTACT_EMAIL" => Some(config.contact_email()),
            "OPERATOR_ADDRESS" => config.operator_address(),
            _ => None,
        };
        let Some(value) = value else {
            return Err(LegalRenderError::new(
                page,
                LegalRenderErrorKind::UnresolvedPlaceholder(name),
            ));
        };
        push_escaped(output, value);
        rest = after_close;
    }
    output.push_str(rest);
    Ok(())
}

fn push_escaped(output: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            other => output.push(other),
        }
    }
}

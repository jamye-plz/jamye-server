//! R8 content filter: embedded default list, list parsing, case-insensitive masking and
//! byte-identical pass-through for bodies without a listed term.

use jamye_server::domain::moderation::{
    ContentFilter, ContentFilterError, DEFAULT_TERMS, MASK, MAX_FILTER_TERM_CHARS,
};

use crate::TestResult;

#[test]
fn the_embedded_list_is_short_and_loads() -> TestResult {
    let filter = ContentFilter::from_lines(DEFAULT_TERMS)?;
    assert!(filter.term_count() >= 5, "default list is too small");
    assert!(filter.term_count() <= 30, "default list is not short");
    assert_eq!(ContentFilter::embedded(), filter);
    Ok(())
}

#[test]
fn every_occurrence_of_a_listed_term_is_replaced_with_three_asterisks() -> TestResult {
    let filter = ContentFilter::from_terms(["badterm"])?;
    assert_eq!(MASK, "***");
    assert_eq!(filter.mask("badterm"), "***");
    assert_eq!(filter.mask("a badterm b badterm c"), "a *** b *** c");
    assert_eq!(filter.mask("badtermbadterm"), "******");
    assert_eq!(filter.mask("prefixbadtermsuffix"), "prefix***suffix");
    Ok(())
}

#[test]
fn matching_is_case_insensitive_and_keeps_the_surrounding_case() -> TestResult {
    let filter = ContentFilter::from_terms(["badterm"])?;
    assert_eq!(filter.mask("BADTERM"), "***");
    assert_eq!(filter.mask("BadTerm and bAdTeRm"), "*** and ***");
    let korean = ContentFilter::from_terms(["쪽바리"])?;
    assert_eq!(korean.mask("그 쪽바리 말이야"), "그 *** 말이야");
    Ok(())
}

#[test]
fn a_body_without_a_listed_term_is_returned_byte_for_byte() -> TestResult {
    let filter = ContentFilter::from_lines(DEFAULT_TERMS)?;
    for body in [
        "",
        "hello world",
        "  leading and trailing  \n\ttabs\r\n",
        "ordinary profanity: damn, hell, shit, fuck, 씨발, 병신",
        "emoji 🙂 and 한글 and ÅÄÖ İstanbul ß",
        "새로운 주제를 올렸어요: [제목](/groups/1/topics/2/chat)",
        "Niger and Nigeria are countries",
        "spice rack, spicy food, a towel on a head, wet back of the car",
    ] {
        let masked = filter.mask(body);
        assert_eq!(masked, body, "body without a listed term changed");
        assert_eq!(masked.as_bytes(), body.as_bytes());
    }
    Ok(())
}

#[test]
fn the_longest_term_wins_at_a_position() -> TestResult {
    let filter = ContentFilter::from_terms(["bad", "badterm"])?;
    assert_eq!(filter.mask("badterm"), "***");
    assert_eq!(filter.mask("badly"), "***ly");
    Ok(())
}

#[test]
fn mask_body_keeps_none_and_masks_some() -> TestResult {
    let filter = ContentFilter::from_terms(["badterm"])?;
    assert_eq!(filter.mask_body(None), None);
    assert_eq!(
        filter.mask_body(Some("x badterm".to_owned())),
        Some("x ***".to_owned())
    );
    Ok(())
}

#[test]
fn a_disabled_filter_changes_nothing() {
    let filter = ContentFilter::disabled();
    assert_eq!(filter.mask("badterm"), "badterm");
    assert_eq!(filter.term_count(), 0);
}

#[test]
fn list_parsing_skips_comments_and_blank_lines_and_rejects_bad_lists() -> TestResult {
    let filter = ContentFilter::from_lines("# comment\n\n  badterm  \nother\n#more\nBADTERM\n")?;
    assert_eq!(
        filter.term_count(),
        2,
        "duplicates differing by case collapse"
    );
    assert_eq!(
        ContentFilter::from_lines("# only a comment\n\n").err(),
        Some(ContentFilterError::EmptyList)
    );
    assert_eq!(
        ContentFilter::from_lines("").err(),
        Some(ContentFilterError::EmptyList)
    );
    let long = "x".repeat(MAX_FILTER_TERM_CHARS + 1);
    assert_eq!(
        ContentFilter::from_lines(&long).err(),
        Some(ContentFilterError::TermTooLong)
    );
    assert_eq!(
        ContentFilter::from_terms(["bad\u{7}term"]).err(),
        Some(ContentFilterError::ControlCharacter)
    );
    Ok(())
}

#[test]
fn terms_shorter_than_two_characters_are_rejected_and_the_default_list_is_unaffected() -> TestResult
{
    // A one-character term would mask that character everywhere in every chat message.
    for list in ["x", "ok\nx\nother", "# c\n  가  \nother"] {
        assert_eq!(
            ContentFilter::from_lines(list).err(),
            Some(ContentFilterError::TermTooShort),
            "list {list:?} was accepted"
        );
    }
    assert_eq!(
        ContentFilter::from_terms(["a"]).err(),
        Some(ContentFilterError::TermTooShort)
    );
    // Two characters is the minimum, in any script.
    assert_eq!(ContentFilter::from_terms(["ab", "가나"])?.term_count(), 2);
    // The embedded list stays valid (it is parsed with the same rules) and non-empty.
    let embedded = ContentFilter::from_lines(DEFAULT_TERMS)?;
    assert_eq!(embedded, ContentFilter::embedded());
    assert!(embedded.term_count() > 0);
    Ok(())
}

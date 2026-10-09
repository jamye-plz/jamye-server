//! `JAMYE_OPERATOR_ACCOUNT_IDS` and `JAMYE_CONTENT_FILTER_TERMS_FILE` startup validation.

use std::{env, fs};

use jamye_server::{
    config::moderation::{ModerationConfig, ModerationConfigInput},
    domain::moderation::ContentFilter,
};
use uuid::Uuid;

use crate::TestResult;

fn input(ids: Option<&str>, file: Option<&str>) -> ModerationConfigInput {
    ModerationConfigInput {
        operator_account_ids: ids.map(str::to_owned),
        content_filter_terms_file: file.map(str::to_owned),
    }
}

#[test]
fn unset_values_mean_no_alert_and_the_embedded_list() -> TestResult {
    for blank in [None, Some(""), Some("   ")] {
        let config = ModerationConfig::resolve(input(blank, blank))?;
        assert!(config.operator_account_ids().is_empty());
        assert_eq!(config.content_filter(), &ContentFilter::embedded());
        assert!(config.content_filter().term_count() > 0);
    }
    Ok(())
}

#[test]
fn operator_ids_parse_trim_and_deduplicate() -> TestResult {
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let config =
        ModerationConfig::resolve(input(Some(&format!(" {first} , {second},{first}")), None))?;
    assert_eq!(config.operator_account_ids(), [first, second]);
    Ok(())
}

#[test]
fn malformed_or_too_many_operator_ids_fail_startup_naming_the_key() -> TestResult {
    let valid = Uuid::new_v4();
    let eleven = (0..11)
        .map(|_| Uuid::new_v4().to_string())
        .collect::<Vec<_>>()
        .join(",");
    for bad in [
        "not-a-uuid".to_owned(),
        format!("{valid},oops"),
        format!("{valid},,{valid}"),
        format!("{valid},"),
        eleven,
    ] {
        let error = ModerationConfig::resolve(input(Some(&bad), None))
            .err()
            .ok_or("a malformed operator id list was accepted")?;
        assert_eq!(error.key(), "JAMYE_OPERATOR_ACCOUNT_IDS");
        assert!(!error.to_string().contains(&bad), "error echoed the value");
    }
    Ok(())
}

#[test]
fn a_terms_file_replaces_the_embedded_list() -> TestResult {
    let path = env::temp_dir().join(format!("jamye-filter-{}.txt", Uuid::new_v4().simple()));
    fs::write(&path, "# operator list\ncustomterm\n")?;
    let result = (|| -> TestResult {
        let config = ModerationConfig::resolve(input(
            None,
            Some(path.to_str().ok_or("temp path is not UTF-8")?),
        ))?;
        assert_eq!(config.content_filter().term_count(), 1);
        assert_eq!(config.content_filter().mask("a CustomTerm b"), "a *** b");
        // The embedded terms are gone once the operator supplies a list.
        assert_eq!(config.content_filter().mask("nigger"), "nigger");
        Ok(())
    })();
    fs::remove_file(&path)?;
    result
}

#[test]
fn an_unreadable_or_empty_terms_file_fails_startup_naming_the_key() -> TestResult {
    let missing = env::temp_dir().join(format!("jamye-filter-missing-{}", Uuid::new_v4().simple()));
    let empty = env::temp_dir().join(format!(
        "jamye-filter-empty-{}.txt",
        Uuid::new_v4().simple()
    ));
    fs::write(&empty, "# nothing here\n\n")?;
    let directory = env::temp_dir();
    let result = (|| -> TestResult {
        for path in [&missing, &empty, &directory] {
            let error = ModerationConfig::resolve(input(
                None,
                Some(path.to_str().ok_or("temp path is not UTF-8")?),
            ))
            .err()
            .ok_or("an unusable terms file was accepted")?;
            assert_eq!(error.key(), "JAMYE_CONTENT_FILTER_TERMS_FILE");
        }
        Ok(())
    })();
    fs::remove_file(&empty)?;
    result
}

#[test]
fn a_terms_file_with_a_term_shorter_than_two_characters_fails_startup_naming_the_key() -> TestResult
{
    let path = env::temp_dir().join(format!(
        "jamye-filter-short-{}.txt",
        Uuid::new_v4().simple()
    ));
    fs::write(&path, "# operator list\ncustomterm\nx\n")?;
    let result = (|| -> TestResult {
        let error = ModerationConfig::resolve(input(
            None,
            Some(path.to_str().ok_or("temp path is not UTF-8")?),
        ))
        .err()
        .ok_or("a terms file with a one-character term was accepted")?;
        assert_eq!(error.key(), "JAMYE_CONTENT_FILTER_TERMS_FILE");
        let message = error.to_string();
        assert!(message.contains("shorter than 2"), "unexpected: {message}");
        assert!(
            !message.contains("customterm"),
            "error echoed a list term: {message}"
        );
        Ok(())
    })();
    fs::remove_file(&path)?;
    result
}

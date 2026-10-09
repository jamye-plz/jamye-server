//! Human-readable and JSON rendering of admin command results. Reports expose ids, the reason,
//! timestamps and the stored snapshot only: no emails, tokens or account profile data. Message
//! text is user content, so it is escaped before it reaches an operator's terminal.

use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::ports::moderation::{ReportDetail, ReportTarget};

const LIST_TEXT_CHARS: usize = 200;

pub fn timestamp(value: OffsetDateTime) -> String {
    value
        .format(&Rfc3339)
        .unwrap_or_else(|_| value.unix_timestamp().to_string())
}

/// Escapes control and bidirectional-format characters (including newlines) and truncates to
/// `max_chars` characters, appending `...` when text was cut.
pub fn terminal_safe(text: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for (count, character) in text.chars().enumerate() {
        if count == max_chars {
            output.push_str("...");
            break;
        }
        match character {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            other if other.is_control() || is_bidi_format(other) => {
                output.push_str(&format!("\\u{{{:04x}}}", u32::from(other)));
            }
            other => output.push(other),
        }
    }
    output
}

fn is_bidi_format(character: char) -> bool {
    matches!(
        character,
        '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

fn snapshot_text(report: &ReportDetail) -> Option<&str> {
    report
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.get("text"))
        .and_then(Value::as_str)
}

fn target_line(report: &ReportDetail) -> String {
    match report.target {
        ReportTarget::Message(message_id) => {
            format!("message {message_id} (group {})", report.group_id)
        }
        ReportTarget::User(user_id) => format!("user {user_id} (group {})", report.group_id),
    }
}

pub fn report_list_text(reports: &[ReportDetail]) -> String {
    if reports.is_empty() {
        return "no reports\n".to_owned();
    }
    let mut output = String::new();
    for report in reports {
        output.push_str(&format!(
            "{} {} {} {}\n  target: {}\n  reporter: {}\n",
            report.id,
            report.status.as_str(),
            report.reason.as_str(),
            timestamp(report.created_at),
            target_line(report),
            report.reporter_id,
        ));
        if let Some(text) = snapshot_text(report) {
            output.push_str(&format!(
                "  text: \"{}\"\n",
                terminal_safe(text, LIST_TEXT_CHARS)
            ));
        }
    }
    output.push_str(&format!("{} report(s)\n", reports.len()));
    output
}

pub fn report_text(report: &ReportDetail) -> String {
    let mut output = format!(
        "report: {}\nstatus: {}\nreason: {}\ncreated_at: {}\nhandled_at: {}\ntarget: {}\nreporter: {}\n",
        report.id,
        report.status.as_str(),
        report.reason.as_str(),
        timestamp(report.created_at),
        report.handled_at.map_or_else(|| "-".to_owned(), timestamp),
        target_line(report),
        report.reporter_id,
    );
    if let Some(snapshot) = &report.snapshot {
        let content_type = snapshot
            .get("content_type")
            .and_then(Value::as_str)
            .unwrap_or("-");
        output.push_str(&format!("snapshot content_type: {content_type}\n"));
        match snapshot_text(report) {
            Some(text) => output.push_str(&format!(
                "snapshot text: \"{}\"\n",
                terminal_safe(text, usize::MAX)
            )),
            None => output.push_str("snapshot text: -\n"),
        }
        if let Some(media) = snapshot.get("media").and_then(Value::as_array) {
            output.push_str(&format!("snapshot media: {}\n", media.len()));
            for item in media {
                let kind = item.get("kind").and_then(Value::as_str).unwrap_or("-");
                let upload_id = item.get("upload_id").and_then(Value::as_str).unwrap_or("-");
                output.push_str(&format!(
                    "  media: {} upload {}\n",
                    terminal_safe(kind, 32),
                    terminal_safe(upload_id, 64)
                ));
            }
        }
    }
    output
}

pub fn report_json(report: &ReportDetail) -> Value {
    let (target_type, message_id, user_id): (&str, Option<Uuid>, Option<Uuid>) = match report.target
    {
        ReportTarget::Message(message_id) => ("message", Some(message_id), None),
        ReportTarget::User(user_id) => ("user", None, Some(user_id)),
    };
    json!({
        "id": report.id,
        "status": report.status.as_str(),
        "reason": report.reason.as_str(),
        "created_at": timestamp(report.created_at),
        "handled_at": report.handled_at.map(timestamp),
        "reporter_id": report.reporter_id,
        "group_id": report.group_id,
        "target_type": target_type,
        "message_id": message_id,
        "user_id": user_id,
        "snapshot": report.snapshot,
    })
}

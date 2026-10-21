use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use pons_ir::{
    ContentKind, Document, ErrorContent, Operation, Presence, ResponseHeader, TypeRef,
    media_type_matches, operation_path, validate_value,
};
use serde_json::Value;

use crate::live::Client;

mod plan;

use plan::{Plan, RequestPlan, declared_errors, matching_error, plan};

const DEFAULT_TIMEOUT_SECS: u64 = 10;

pub(crate) fn run(args: &[OsString]) -> Result<(), crate::CliError> {
    let options = Options::parse(args)?;
    let document = crate::read_document(&options.document)?;
    crate::validate(&document)?;
    let client = Client::new(&options.base_url, options.timeout);
    let outcomes = check(&document, &client);
    let summary = report(&outcomes);
    if summary.failed > 0 {
        return Err(crate::CliError::Failure(format!(
            "{} operation(s) did not match the contract",
            summary.failed
        )));
    }
    Ok(())
}

struct Options {
    document: PathBuf,
    base_url: String,
    timeout: Duration,
}

impl Options {
    fn parse(args: &[OsString]) -> Result<Self, crate::CliError> {
        let mut positionals = Vec::new();
        let mut base_url = None;
        let mut timeout = None;
        let mut index = 0;
        while index < args.len() {
            let Some(argument) = args[index].to_str() else {
                return Err(crate::CliError::Usage(format!(
                    "test argument {index} is not valid UTF-8"
                )));
            };
            match argument {
                "--base-url" if base_url.is_some() => {
                    return Err(crate::CliError::Usage(
                        "--base-url given more than once".into(),
                    ));
                }
                "--base-url" => {
                    index += 1;
                    let value = value_for("--base-url", args.get(index))?;
                    if !absolute_url(value) {
                        return Err(crate::CliError::Usage(format!(
                            "--base-url expects an absolute http or https URL, not {value:?}"
                        )));
                    }
                    base_url = Some(value.trim_end_matches('/').to_owned());
                }
                "--timeout" if timeout.is_some() => {
                    return Err(crate::CliError::Usage(
                        "--timeout given more than once".into(),
                    ));
                }
                "--timeout" => {
                    index += 1;
                    let value = value_for("--timeout", args.get(index))?;
                    let seconds = value.parse::<u64>().map_err(|_| {
                        crate::CliError::Usage(format!(
                            "--timeout expects whole seconds, not {value:?}"
                        ))
                    })?;
                    if seconds == 0 {
                        return Err(crate::CliError::Usage(
                            "--timeout expects a positive number of seconds".into(),
                        ));
                    }
                    timeout = Some(seconds);
                }
                other if other.starts_with('-') => {
                    return Err(crate::CliError::Usage(format!(
                        "unexpected test option {other:?}"
                    )));
                }
                _ => positionals.push(args[index].clone()),
            }
            index += 1;
        }
        let document = match positionals.as_slice() {
            [document] => PathBuf::from(document),
            [] => {
                return Err(crate::CliError::Usage(
                    "test expects an IR document and --base-url".into(),
                ));
            }
            _ => {
                return Err(crate::CliError::Usage(format!(
                    "test takes one document, got {}",
                    positionals.len()
                )));
            }
        };
        let Some(base_url) = base_url else {
            return Err(crate::CliError::Usage("test needs --base-url <url>".into()));
        };
        Ok(Self {
            document,
            base_url,
            timeout: Duration::from_secs(timeout.unwrap_or(DEFAULT_TIMEOUT_SECS)),
        })
    }
}

fn value_for<'a>(flag: &str, value: Option<&'a OsString>) -> Result<&'a str, crate::CliError> {
    let value = value.ok_or_else(|| crate::CliError::Usage(format!("{flag} requires a value")))?;
    let value = value
        .to_str()
        .ok_or_else(|| crate::CliError::Usage(format!("{flag} requires a UTF-8 value")))?;
    if value.starts_with('-') {
        return Err(crate::CliError::Usage(format!(
            "{flag} expects a value, not the flag {value:?}"
        )));
    }
    Ok(value)
}

fn absolute_url(value: &str) -> bool {
    let rest = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"));
    let Some(rest) = rest else {
        return false;
    };
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit('@')
        .next()
        .unwrap_or_default();
    !authority.is_empty() && !value.chars().any(char::is_whitespace)
}

enum Outcome {
    Passed {
        id: String,
        status: u16,
        invalid: Option<u16>,
        note: String,
    },
    Failed {
        id: String,
        problems: Vec<String>,
    },
    Skipped {
        id: String,
        reason: String,
    },
}

fn check(document: &Document, client: &Client) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    for operation in &document.operations {
        let id = operation_path(&operation.id);
        let plan = match plan(document, operation) {
            Ok(plan) => plan,
            Err(reason) => {
                outcomes.push(Outcome::Skipped { id, reason });
                continue;
            }
        };
        outcomes.push(execute(document, client, &plan, id));
    }
    outcomes
}

fn execute(document: &Document, client: &Client, plan: &Plan<'_>, id: String) -> Outcome {
    let reply = match client.send(&plan.valid.request()) {
        Ok(reply) => reply,
        Err(error) => {
            return Outcome::Failed {
                id,
                problems: vec![format!("transport: {error}")],
            };
        }
    };
    if reply.status != plan.status {
        return Outcome::Failed {
            id,
            problems: vec![format!(
                "expected status {}, found {}",
                plan.status, reply.status
            )],
        };
    }
    let problems = response_problems(
        document,
        &reply,
        plan.success.content.into(),
        plan.success.media_type.as_deref(),
        plan.success.body.as_ref(),
        &plan.success.headers,
    );
    if !problems.is_empty() {
        return Outcome::Failed { id, problems };
    }
    let invalid = match &plan.invalid {
        Some(request) => match check_invalid(document, client, plan.operation, request) {
            Ok(status) => Some(status),
            Err(problems) => {
                return Outcome::Failed { id, problems };
            }
        },
        None => None,
    };
    Outcome::Passed {
        id,
        status: reply.status,
        invalid,
        note: plan.invalid_note.clone(),
    }
}

fn check_invalid(
    document: &Document,
    client: &Client,
    operation: &Operation,
    request: &RequestPlan,
) -> Result<u16, Vec<String>> {
    let reply = client
        .send(&request.request())
        .map_err(|error| vec![format!("transport: {error}")])?;
    if (200..300).contains(&reply.status) {
        return Err(vec![format!(
            "the server accepted the invalid request with {}",
            reply.status
        )]);
    }
    let errors = declared_errors(document, operation);
    let Some(error) = matching_error(&errors, reply.status) else {
        return Err(vec![format!(
            "status {} is not a declared error",
            reply.status
        )]);
    };
    let problems = response_problems(
        document,
        &reply,
        error.content.into(),
        error.media_type.as_deref(),
        error.body.as_ref(),
        &error.headers,
    );
    if problems.is_empty() {
        Ok(reply.status)
    } else {
        Err(problems)
    }
}

#[derive(Clone, Copy)]
enum Expect {
    Json,
    ProblemJson,
    Text,
    Empty,
}

impl From<ContentKind> for Expect {
    fn from(content: ContentKind) -> Self {
        match content {
            ContentKind::Json => Self::Json,
            ContentKind::ProblemJson => Self::ProblemJson,
            ContentKind::Text => Self::Text,
            ContentKind::Form | ContentKind::Multipart | ContentKind::Bytes => Self::Text,
            ContentKind::Stream | ContentKind::Empty => Self::Empty,
        }
    }
}

impl From<ErrorContent> for Expect {
    fn from(content: ErrorContent) -> Self {
        match content {
            ErrorContent::Json => Self::Json,
            ErrorContent::ProblemJson => Self::ProblemJson,
            ErrorContent::Text => Self::Text,
            ErrorContent::Empty => Self::Empty,
        }
    }
}

fn response_problems(
    document: &Document,
    reply: &crate::live::Reply,
    expect: Expect,
    media_type: Option<&str>,
    body: Option<&TypeRef>,
    headers: &[ResponseHeader],
) -> Vec<String> {
    let mut problems = Vec::new();
    check_content_type(
        reply.content_type.as_deref(),
        expect,
        media_type,
        &mut problems,
    );
    match expect {
        Expect::Empty => {}
        Expect::Text => {
            if reply.body.trim().is_empty() {
                problems.push("expected a text body, found nothing".into());
            }
        }
        Expect::Json | Expect::ProblemJson => {
            match (body, serde_json::from_str::<Value>(&reply.body)) {
                (None, _) => problems.push("the document declares no response body type".into()),
                (Some(_), Err(error)) => {
                    problems.push(format!("response body is not JSON: {error}"));
                }
                (Some(body), Ok(value)) => {
                    if let Err(errors) =
                        validate_value(document, body, pons_ir::Direction::Output, &value)
                    {
                        problems.push(format!(
                            "response body does not match the contract: {}",
                            render_violations(&errors)
                        ));
                    }
                }
            }
        }
    }
    for header in headers {
        if header.presence != Presence::Required {
            continue;
        }
        if !reply
            .headers
            .contains_key(&header.wire.to_ascii_lowercase())
        {
            problems.push(format!(
                "required response header {} is missing",
                header.wire
            ));
        }
    }
    problems
}

fn check_content_type(
    found: Option<&str>,
    expect: Expect,
    media_type: Option<&str>,
    problems: &mut Vec<String>,
) {
    if matches!(expect, Expect::Empty) {
        return;
    }
    let found = found.unwrap_or_default();
    if let Some(media_type) = media_type {
        if !media_type_matches(found, media_type) {
            problems.push(format!(
                "expected content type {media_type}, found {:?}",
                found
            ));
        }
        return;
    }
    let matches = match expect {
        Expect::Json => json_media(found),
        Expect::ProblemJson => media_base(found) == "application/problem+json",
        Expect::Text => media_base(found).starts_with("text/"),
        Expect::Empty => true,
    };
    if !matches {
        let expected = match expect {
            Expect::Json => "a JSON content type",
            Expect::ProblemJson => "application/problem+json",
            Expect::Text => "a text content type",
            Expect::Empty => return,
        };
        problems.push(format!("expected {expected}, found {found:?}"));
    }
}

fn json_media(media_type: &str) -> bool {
    let base = media_base(media_type);
    base == "application/json" || base.ends_with("+json")
}

fn media_base(media_type: &str) -> String {
    media_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn json_content(content: ContentKind) -> bool {
    matches!(content, ContentKind::Json | ContentKind::ProblemJson)
}

fn content_name(content: ContentKind) -> &'static str {
    match content {
        ContentKind::Json => "JSON",
        ContentKind::Form => "form",
        ContentKind::Multipart => "multipart",
        ContentKind::Text => "text",
        ContentKind::Bytes => "raw byte",
        ContentKind::Stream => "stream",
        ContentKind::ProblemJson => "problem JSON",
        ContentKind::Empty => "empty",
    }
}

fn default_content_type(content: ContentKind) -> &'static str {
    match content {
        ContentKind::ProblemJson => "application/problem+json",
        _ => "application/json",
    }
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn header_text(value: &Value) -> Option<String> {
    let text = scalar_text(value)?;
    (!text
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\t')))
    .then_some(text)
}

fn percent_encode(segment: &str) -> String {
    let mut encoded = String::new();
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn join_path(parts: &[String]) -> String {
    let mut path = String::new();
    for part in parts {
        path.push('/');
        path.push_str(part);
    }
    path
}

fn render_violations(errors: &[pons_ir::Violation]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

struct Summary {
    passed: usize,
    failed: usize,
    skipped: usize,
}

fn report(outcomes: &[Outcome]) -> Summary {
    let mut summary = Summary {
        passed: 0,
        failed: 0,
        skipped: 0,
    };
    for outcome in outcomes {
        match outcome {
            Outcome::Passed {
                id,
                status,
                invalid,
                note,
            } => {
                summary.passed += 1;
                match invalid {
                    Some(invalid_status) => {
                        println!("ok   {id} {status} (invalid request -> {invalid_status})");
                    }
                    None => println!("ok   {id} {status} ({note})"),
                }
            }
            Outcome::Failed { id, problems } => {
                summary.failed += 1;
                println!("fail {id}");
                for problem in problems {
                    println!("     {problem}");
                }
            }
            Outcome::Skipped { id, reason } => {
                summary.skipped += 1;
                println!("skip {id}: {reason}");
            }
        }
    }
    println!(
        "\n{} passed, {} failed, {} skipped",
        summary.passed, summary.failed, summary.skipped
    );
    summary
}

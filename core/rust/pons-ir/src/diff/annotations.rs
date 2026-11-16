use crate::{Diagnostic, DiagnosticSeverity, Doc};

use super::{ChangeLevel, Diff, add};

pub(super) fn add_doc_change(
    diff: &mut Diff,
    path: &str,
    old: Option<&Doc>,
    new: Option<&Doc>,
    label: &str,
) {
    if old == new {
        return;
    }
    let (level, change) =
        if old.and_then(|doc| doc.deprecated) != new.and_then(|doc| doc.deprecated) {
            (ChangeLevel::Warning, "deprecation changed")
        } else {
            (ChangeLevel::Info, "documentation changed")
        };
    add(diff, level, path, format!("{label} {change}"));
}

pub(super) fn diff_diagnostics(
    diff: &mut Diff,
    path: &str,
    old: &[Diagnostic],
    new: &[Diagnostic],
) {
    let mut unmatched = old.iter().collect::<Vec<_>>();
    for diagnostic in new {
        let found = unmatched
            .iter()
            .position(|previous| previous.code == diagnostic.code);
        let Some(index) = found else {
            if diagnostic.severity == DiagnosticSeverity::Error {
                add(
                    diff,
                    ChangeLevel::Breaking,
                    path,
                    format!("error diagnostic added: {}", diagnostic.code),
                );
            }
            continue;
        };
        let previous = unmatched.remove(index);
        if previous.severity != diagnostic.severity {
            let escalated = severity_rank(diagnostic.severity) > severity_rank(previous.severity);

            let level = if diagnostic.severity == DiagnosticSeverity::Error {
                ChangeLevel::Breaking
            } else {
                ChangeLevel::NonBreaking
            };
            add(
                diff,
                level,
                path,
                format!(
                    "diagnostic severity {}: {}",
                    if escalated { "escalated" } else { "lowered" },
                    diagnostic.code
                ),
            );
            continue;
        }
        if previous.message != diagnostic.message {
            add(
                diff,
                ChangeLevel::NonBreaking,
                path,
                format!("diagnostic message changed: {}", diagnostic.code),
            );
        }
    }
    for previous in unmatched {
        if previous.severity == DiagnosticSeverity::Error {
            add(
                diff,
                ChangeLevel::NonBreaking,
                path,
                format!("error diagnostic removed: {}", previous.code),
            );
        }
    }
}

const fn severity_rank(severity: DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Info => 0,
        DiagnosticSeverity::Warning => 1,
        DiagnosticSeverity::Error => 2,
    }
}

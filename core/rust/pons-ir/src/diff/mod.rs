use std::collections::BTreeSet;

use crate::{Document, Info};

mod annotations;
mod auth;
mod operations;
mod paths;
mod types;
mod validity;

use annotations::diff_diagnostics;
use auth::diff_auth_schemes;
use operations::{diff_errors, diff_operations};
use types::diff_types;
pub use validity::Validity;
use validity::diff_validity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangeLevel {
    Info,
    NonBreaking,
    Warning,
    Breaking,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub level: ChangeLevel,
    pub path: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    pub changes: Vec<Change>,

    pub validity: Validity,
}

impl Diff {
    pub fn worst(&self) -> Option<ChangeLevel> {
        self.changes.iter().map(|change| change.level).max()
    }

    pub fn has_breaking_changes(&self) -> bool {
        self.worst() == Some(ChangeLevel::Breaking)
    }
}

impl Document {
    pub fn diff(&self, newer: &Self) -> Diff {
        let mut old = self.clone();
        let mut new = newer.clone();
        old.canonicalize();
        new.canonicalize();

        let mut diff = Diff::default();
        diff_validity(&mut diff, &old, &new);
        diff_diagnostics(
            &mut diff,
            "document.diagnostics",
            &old.diagnostics,
            &new.diagnostics,
        );
        if old.ext != new.ext {
            add(
                &mut diff,
                ChangeLevel::Warning,
                "ext",
                "document metadata changed",
            );
        }
        diff_info(&mut diff, &old.info, &new.info);

        let mut used = BTreeSet::new();
        diff_types(&mut diff, &mut used, &old, &new);
        diff_auth_schemes(&mut diff, &mut used, &old, &new);
        diff_errors(
            &mut diff,
            &mut used,
            "common_errors",
            &old.common_errors,
            &new.common_errors,
        );
        diff_operations(&mut diff, &mut used, &old, &new);
        diff
    }
}

fn diff_info(diff: &mut Diff, old: &Info, new: &Info) {
    if old.title != new.title {
        add(diff, ChangeLevel::Info, "info.title", "API title changed");
    }
    if old.version != new.version {
        add(
            diff,
            ChangeLevel::NonBreaking,
            "info.version",
            "API version changed",
        );
    }
    if old.description != new.description {
        add(
            diff,
            ChangeLevel::Info,
            "info.description",
            "API description changed",
        );
    }
    if old.servers != new.servers {
        add(
            diff,
            ChangeLevel::NonBreaking,
            "info.servers",
            "API servers changed",
        );
    }
    if old.ext != new.ext {
        add(
            diff,
            ChangeLevel::Warning,
            "info.ext",
            "document metadata changed",
        );
    }
}

fn add(diff: &mut Diff, level: ChangeLevel, path: impl Into<String>, detail: impl Into<String>) {
    diff.changes.push(Change {
        level,
        path: path.into(),
        detail: detail.into(),
    });
}

#[cfg(test)]
mod tests;

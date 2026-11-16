use std::collections::{BTreeMap, BTreeSet};

use crate::{AuthScheme, Document, OAuthFlow};

use super::paths::Path;
use super::{ChangeLevel, Diff, add};

pub(super) fn diff_auth_schemes(
    diff: &mut Diff,
    used: &mut BTreeSet<String>,
    old: &Document,
    new: &Document,
) {
    for id in old.auth_schemes.keys() {
        if !new.auth_schemes.contains_key(id) {
            add(
                diff,
                ChangeLevel::Breaking,
                Path::key("auth_schemes", id).claim(used),
                "authentication scheme removed",
            );
        }
    }
    for (id, scheme) in &new.auth_schemes {
        let owner = Path::key("auth_schemes", id);
        let Some(previous) = old.auth_schemes.get(id) else {
            add(
                diff,
                ChangeLevel::NonBreaking,
                owner.claim(used),
                "authentication scheme added",
            );
            continue;
        };
        let path = owner.claim(used);
        match (previous, scheme) {
            (AuthScheme::Bearer { format: old }, AuthScheme::Bearer { format: new }) => {
                if old != new {
                    add(
                        diff,
                        ChangeLevel::Info,
                        &path,
                        "bearer token format documentation changed",
                    );
                }
            }
            (
                AuthScheme::ApiKey {
                    location: old_location,
                    name: old_name,
                },
                AuthScheme::ApiKey { location, name },
            ) => {
                if old_location != location {
                    add(
                        diff,
                        ChangeLevel::Breaking,
                        &path,
                        "API key location changed",
                    );
                }
                if old_name != name {
                    add(diff, ChangeLevel::Breaking, &path, "API key name changed");
                }
            }
            (AuthScheme::CookieSession { cookie: old }, AuthScheme::CookieSession { cookie }) => {
                if old != cookie {
                    add(
                        diff,
                        ChangeLevel::Breaking,
                        &path,
                        "session cookie name changed",
                    );
                }
            }
            (AuthScheme::OAuth2 { flows: old }, AuthScheme::OAuth2 { flows }) => {
                diff_oauth_flows(diff, used, &owner, old, flows);
            }
            (
                AuthScheme::Custom {
                    description: old,
                    ext: old_ext,
                    ..
                },
                AuthScheme::Custom {
                    description, ext, ..
                },
            ) => {
                if old != description {
                    add(
                        diff,
                        ChangeLevel::Info,
                        &path,
                        "custom scheme description changed",
                    );
                }
                if old_ext != ext {
                    add(
                        diff,
                        ChangeLevel::Warning,
                        &path,
                        "custom scheme metadata changed",
                    );
                }
            }
            _ => add(
                diff,
                ChangeLevel::Breaking,
                &path,
                "authentication scheme kind changed",
            ),
        }
    }
}

fn diff_oauth_flows(
    diff: &mut Diff,
    used: &mut BTreeSet<String>,
    owner: &Path,
    old: &[OAuthFlow],
    new: &[OAuthFlow],
) {
    let old: BTreeMap<_, _> = old.iter().map(|flow| (flow.kind, flow)).collect();
    let new: BTreeMap<_, _> = new.iter().map(|flow| (flow.kind, flow)).collect();
    for kind in old.keys() {
        if !new.contains_key(kind) {
            add(
                diff,
                ChangeLevel::Breaking,
                owner.child("flows", &format!("{kind:?}")).claim(used),
                "OAuth2 flow removed",
            );
        }
    }
    for (kind, flow) in &new {
        let flow_path = owner.child("flows", &format!("{kind:?}")).claim(used);
        let Some(previous) = old.get(kind) else {
            add(
                diff,
                ChangeLevel::NonBreaking,
                &flow_path,
                "OAuth2 flow added",
            );
            continue;
        };
        if previous.authorization_url != flow.authorization_url {
            add(
                diff,
                ChangeLevel::Breaking,
                &flow_path,
                "OAuth2 authorization URL changed",
            );
        }
        if previous.token_url != flow.token_url {
            add(
                diff,
                ChangeLevel::Breaking,
                &flow_path,
                "OAuth2 token URL changed",
            );
        }
        if previous.scopes != flow.scopes {
            add(
                diff,
                ChangeLevel::Info,
                &flow_path,
                "OAuth2 scope documentation changed",
            );
        }
        if previous.ext != flow.ext {
            add(
                diff,
                ChangeLevel::Warning,
                &flow_path,
                "OAuth2 flow metadata changed",
            );
        }
    }
}

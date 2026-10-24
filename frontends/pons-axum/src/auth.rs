use std::collections::BTreeSet;
use std::marker::PhantomData;

use axum::extract::FromRequestParts;
use pons_ir::AuthRequirement;

use crate::describe::{DescribeRequest, OperationCx};

pub trait AuthSpec {
    fn apply(cx: &mut OperationCx);

    fn requirement(&self) -> AuthRequirement {
        let mut cx = OperationCx::default();
        Self::apply(&mut cx);
        cx.auth_requirement()
    }
}

impl OperationCx {
    pub fn require_auth(&mut self, scheme: impl Into<String>) {
        self.require_all([scheme]);
    }

    pub fn require_scopes<I, S>(&mut self, scheme: impl Into<String>, scopes: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let scheme = scheme.into();
        self.require_all([scheme.clone()]);
        let values = self.auth.scopes.entry(scheme).or_default();
        for scope in scopes {
            let scope = scope.into();
            if !values.contains(&scope) {
                values.push(scope);
            }
        }
    }

    pub fn require_all<I, S>(&mut self, schemes: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut unique = Vec::new();
        for scheme in schemes {
            let scheme = scheme.into();
            if !unique.contains(&scheme) {
                unique.push(scheme);
            }
        }
        if unique.is_empty() {
            return;
        }
        if self.auth.alternatives.is_empty() {
            self.auth.alternatives.push(unique);
        } else {
            self.auth.alternatives.iter_mut().for_each(|alternative| {
                for scheme in &unique {
                    if !alternative.contains(scheme) {
                        alternative.push(scheme.clone());
                    }
                }
            });
        }
    }

    pub fn require_any<I, S>(&mut self, schemes: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let schemes = schemes.into_iter().map(Into::into).collect::<Vec<_>>();
        if !schemes.is_empty() {
            self.auth.alternatives.push(schemes);
        }
    }

    pub fn optional_auth(&mut self) {
        self.auth.optional = true;
    }

    pub(crate) fn auth_requirement(&self) -> AuthRequirement {
        self.auth.clone()
    }
}

pub(crate) fn merge_auth_requirements(target: &mut AuthRequirement, source: &AuthRequirement) {
    for (scheme, scopes) in &source.scopes {
        let values = target.scopes.entry(scheme.clone()).or_default();
        for scope in scopes {
            if !values.contains(scope) {
                values.push(scope.clone());
            }
        }
    }
    if says_something(source) {
        target.optional &= source.optional;
    }
    let combined: Vec<BTreeSet<String>> = match (
        target.alternatives.is_empty(),
        source.alternatives.is_empty(),
    ) {
        (_, true) => target.alternatives.iter().map(|s| as_set(s)).collect(),
        (true, false) => source.alternatives.iter().map(|s| as_set(s)).collect(),
        (false, false) => target
            .alternatives
            .iter()
            .flat_map(|left| {
                source
                    .alternatives
                    .iter()
                    .map(move |right| as_set(left).union(&as_set(right)).cloned().collect())
            })
            .collect(),
    };
    target.alternatives = canonical_alternatives(combined);
}

fn as_set(schemes: &[String]) -> BTreeSet<String> {
    schemes.iter().cloned().collect()
}

fn says_something(source: &AuthRequirement) -> bool {
    !source.alternatives.is_empty() || !source.scopes.is_empty() || source.optional
}

fn canonical_alternatives(alternatives: Vec<BTreeSet<String>>) -> Vec<Vec<String>> {
    let mut sets = alternatives;
    sets.sort();
    sets.dedup();

    sets.iter()
        .enumerate()
        .filter(|(index, set)| {
            !sets
                .iter()
                .enumerate()
                .any(|(other, candidate)| other != *index && candidate.is_subset(set))
        })
        .map(|(_, set)| set.iter().cloned().collect())
        .collect()
}

pub struct SessionAuth;

impl AuthSpec for SessionAuth {
    fn apply(cx: &mut OperationCx) {
        cx.require_all(["session"]);
    }
}

pub struct Auth<T, P = SessionAuth>(pub T, pub PhantomData<fn() -> P>);

impl<T, P> Auth<T, P> {
    pub fn new(value: T) -> Self {
        Self(value, PhantomData)
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<S, T, P> FromRequestParts<S> for Auth<T, P>
where
    S: Send + Sync,
    T: FromRequestParts<S>,
{
    type Rejection = T::Rejection;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        T::from_request_parts(parts, state).await.map(Self::new)
    }
}

impl<T, P> DescribeRequest for Auth<T, P>
where
    T: DescribeRequest,
    P: AuthSpec,
{
    fn describe(cx: &mut OperationCx) {
        T::describe(cx);
        P::apply(cx);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn composes_auth_requirements() {
        let mut cx = OperationCx::default();
        cx.require_all(["session"]);
        cx.require_scopes("session", ["read", "write"]);
        cx.require_all(["tenant"]);
        cx.require_any(["api_key", "basic"]);
        cx.optional_auth();
        assert_eq!(
            cx.auth.alternatives,
            vec![
                vec![String::from("session"), String::from("tenant")],
                vec![String::from("api_key"), String::from("basic")],
            ]
        );
        assert!(cx.auth.optional);
        assert_eq!(cx.auth.scopes["session"], ["read", "write"]);
    }

    #[test]
    fn merges_auth_scopes() {
        let mut target = AuthRequirement {
            alternatives: vec![vec!["session".into()]],
            scopes: [("session".into(), vec!["read".into()])].into(),
            optional: false,
            ext: BTreeMap::new(),
        };
        let source = AuthRequirement {
            alternatives: vec![vec!["session".into()]],
            scopes: [("session".into(), vec!["read".into(), "write".into()])].into(),
            optional: false,
            ext: BTreeMap::new(),
        };
        merge_auth_requirements(&mut target, &source);
        assert_eq!(target.scopes["session"], ["read", "write"]);
    }

    fn requirement(alternatives: &[&[&str]], optional: bool) -> AuthRequirement {
        AuthRequirement {
            alternatives: alternatives
                .iter()
                .map(|schemes| schemes.iter().map(|scheme| (*scheme).to_owned()).collect())
                .collect(),
            scopes: BTreeMap::new(),
            optional,
            ext: BTreeMap::new(),
        }
    }

    #[test]
    fn merging_is_idempotent() {
        let any = requirement(&[&["session", "api_key"]], false);
        let mut stacked = any.clone();
        merge_auth_requirements(&mut stacked, &any);
        assert_eq!(
            stacked.alternatives,
            vec![vec!["api_key".to_owned(), "session".to_owned()]]
        );

        let mut once = any.clone();
        merge_auth_requirements(&mut once, &any);
        let mut twice = once.clone();
        merge_auth_requirements(&mut twice, &once);
        assert_eq!(once, twice);

        let alternative_groups = requirement(&[&["session"], &["api_key", "basic"]], false);
        let mut merged = alternative_groups.clone();
        merge_auth_requirements(&mut merged, &alternative_groups);
        let mut again = merged.clone();
        merge_auth_requirements(&mut again, &alternative_groups);
        assert_eq!(merged, again);
    }

    #[test]
    fn drops_redundant_superset_alternatives() {
        let mut target = requirement(&[&["session"], &["session", "tenant"]], false);
        merge_auth_requirements(&mut target, &requirement(&[&["session"]], false));
        assert_eq!(target.alternatives, vec![vec!["session".to_owned()]]);

        let mut independent = requirement(&[&["api_key", "session"], &["basic", "session"]], false);
        merge_auth_requirements(&mut independent, &requirement(&[&["session"]], false));
        assert_eq!(
            independent.alternatives,
            vec![
                vec!["api_key".to_owned(), "session".to_owned()],
                vec!["basic".to_owned(), "session".to_owned()],
            ]
        );
    }

    #[test]
    fn sorting_and_dedup_canonicalizes_alternatives() {
        let mut target = requirement(&[&["session", "session", "api_key"]], false);
        merge_auth_requirements(&mut target, &requirement(&[&["api_key", "session"]], false));
        assert_eq!(
            target.alternatives,
            vec![vec!["api_key".to_owned(), "session".to_owned()]]
        );
    }

    #[test]
    fn a_required_layer_overrides_an_optional_operation() {
        let mut optional_target = requirement(&[&["session"]], true);
        merge_auth_requirements(&mut optional_target, &requirement(&[&["api_key"]], false));
        assert!(!optional_target.optional);
        assert_eq!(
            optional_target.alternatives,
            vec![vec!["api_key".to_owned(), "session".to_owned()]]
        );

        let mut optional_source = requirement(&[&["session"]], false);
        merge_auth_requirements(
            &mut optional_source,
            &requirement(&[&["api_key", "basic"]], true),
        );
        assert!(!optional_source.optional);
        assert_eq!(
            optional_source.alternatives,
            vec![vec![
                "api_key".to_owned(),
                "basic".to_owned(),
                "session".to_owned()
            ]]
        );

        let mut both = requirement(&[&["session"]], true);
        merge_auth_requirements(&mut both, &requirement(&[&["api_key"]], true));
        assert!(both.optional);
        assert_eq!(
            both.alternatives,
            vec![vec!["api_key".to_owned(), "session".to_owned()]]
        );
    }

    #[test]
    fn an_empty_requirement_leaves_the_other_side_untouched() {
        let mut target = requirement(&[&["session", "api_key"]], false);
        merge_auth_requirements(&mut target, &AuthRequirement::default());
        assert_eq!(target, requirement(&[&["api_key", "session"]], false));

        let mut empty = AuthRequirement::default();
        merge_auth_requirements(&mut empty, &requirement(&[&["session"]], false));
        assert_eq!(empty.alternatives, vec![vec!["session".to_owned()]]);

        let mut optional = requirement(&[&["session"]], true);
        merge_auth_requirements(&mut optional, &AuthRequirement::default());
        assert!(
            optional.optional,
            "a requirement that names no scheme, no scope, and no optionality says nothing about optional auth"
        );
    }
}

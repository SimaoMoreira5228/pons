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
    if source.alternatives.is_empty() {
        return;
    }
    if target.alternatives.is_empty() {
        target.alternatives = source.alternatives.clone();
        target.optional = source.optional;
        return;
    }
    let existing = target.alternatives.clone();
    target.alternatives = existing
        .into_iter()
        .flat_map(|left| {
            source.alternatives.iter().map(move |right| {
                let mut combined = left.clone();
                right.iter().for_each(|scheme| {
                    if !combined.contains(scheme) {
                        combined.push(scheme.clone());
                    }
                });
                combined
            })
        })
        .collect();
    target.optional &= source.optional;
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
}

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

pub(crate) fn merge_auth_requirements(target: &mut AuthRequirement, source: &AuthRequirement) {
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

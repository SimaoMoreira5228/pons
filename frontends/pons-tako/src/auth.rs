use std::marker::PhantomData;

use http::request::Parts;
use pons_ir::AuthRequirement;
use tako_rs_core::extractors::FromRequestParts;

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

    pub fn optional(&mut self) {
        self.auth.optional = true;
    }

    pub(crate) fn auth_requirement(&self) -> AuthRequirement {
        self.auth.clone()
    }
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

impl<'a, T, P> FromRequestParts<'a> for Auth<T, P>
where
    T: FromRequestParts<'a> + Send + 'a,
    P: 'a,
{
    type Error = T::Error;

    async fn from_request_parts(parts: &'a mut Parts) -> Result<Self, Self::Error> {
        T::from_request_parts(parts).await.map(Self::new)
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

use std::marker::PhantomData;

use actix_web::FromRequest;
use futures_util::future::LocalBoxFuture;
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

impl<T, P> FromRequest for Auth<T, P>
where
    T: FromRequest,
    T::Future: 'static,
{
    type Error = T::Error;
    type Future = LocalBoxFuture<'static, Result<Self, Self::Error>>;

    fn from_request(
        req: &actix_web::HttpRequest,
        payload: &mut actix_web::dev::Payload,
    ) -> Self::Future {
        let future = T::from_request(req, payload);
        Box::pin(async move { future.await.map(Self::new) })
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
}

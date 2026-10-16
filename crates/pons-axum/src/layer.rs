use std::convert::Infallible;

use axum::extract::Request;
use axum::response::IntoResponse;
use tower_layer::Layer;
use tower_service::Service;

use crate::{AuthSpec, Router, auth};

pub struct DocumentedLayer<L, P> {
    layer: L,
    spec: P,
}

impl<L, P> DocumentedLayer<L, P> {
    pub fn new(layer: L, spec: P) -> Self {
        Self { layer, spec }
    }
}

impl<L, P> DocumentedLayer<L, P>
where
    P: AuthSpec,
{
    pub fn layer<S>(self, mut router: Router<S>) -> Router<S>
    where
        S: Clone + Send + Sync + 'static,
        L: Layer<axum::routing::Route> + Clone + Send + Sync + 'static,
        L::Service: Service<Request> + Clone + Send + Sync + 'static,
        <L::Service as Service<Request>>::Response: IntoResponse + 'static,
        <L::Service as Service<Request>>::Error: Into<Infallible> + 'static,
        <L::Service as Service<Request>>::Future: Send + 'static,
    {
        let DocumentedLayer { layer, spec } = self;
        let requirement = spec.requirement();
        for operation in &mut router.operations {
            auth::merge_auth_requirements(&mut operation.auth, &requirement);
        }
        router.inner = router.inner.layer(layer);
        router
    }
}

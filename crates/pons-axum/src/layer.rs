use std::convert::Infallible;

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
        L::Service: Service<axum::extract::Request> + Clone + Send + Sync + 'static,
        <L::Service as Service<axum::extract::Request>>::Response: IntoResponse + 'static,
        <L::Service as Service<axum::extract::Request>>::Error: Into<Infallible> + 'static,
        <L::Service as Service<axum::extract::Request>>::Future: Send + 'static,
    {
        let requirement = self.spec.requirement();
        router.inner = router.inner.layer(self.layer);
        router
            .operations
            .iter_mut()
            .for_each(|operation| auth::merge_auth_requirements(&mut operation.auth, &requirement));
        router
    }
}

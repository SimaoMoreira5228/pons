use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::extract::Request;
use axum::response::{IntoResponse, Response};
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
        let requirement = self.spec.requirement();
        let layer = self.layer;
        router
            .pending_layers
            .push(BoxedLayer(Arc::new(move |route| {
                BoxedService(Box::new(Erased(layer.clone().layer(route))))
            })));
        auth::merge_auth_requirements(&mut router.auth_requirement, &requirement);
        router
    }
}

type BoxedFuture = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

#[derive(Clone)]
pub(crate) struct BoxedLayer(Arc<dyn Fn(axum::routing::Route) -> BoxedService + Send + Sync>);

impl Layer<axum::routing::Route> for BoxedLayer {
    type Service = BoxedService;

    fn layer(&self, inner: axum::routing::Route) -> Self::Service {
        (self.0)(inner)
    }
}

pub struct BoxedService(Box<dyn CloneService + Send + Sync>);

impl Clone for BoxedService {
    fn clone(&self) -> Self {
        Self(self.0.clone_box())
    }
}

impl Service<Request> for BoxedService {
    type Response = Response;
    type Error = Infallible;
    type Future = BoxedFuture;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        self.0.call(request)
    }
}

pub trait CloneService:
    Service<Request, Response = Response, Error = Infallible, Future = BoxedFuture>
{
    fn clone_box(&self) -> Box<dyn CloneService + Send + Sync>;
}

impl<S> CloneService for S
where
    S: Service<Request, Response = Response, Error = Infallible, Future = BoxedFuture>
        + Clone
        + Send
        + Sync
        + 'static,
{
    fn clone_box(&self) -> Box<dyn CloneService + Send + Sync> {
        Box::new(self.clone())
    }
}

struct Erased<S>(S);

impl<S: Clone> Clone for Erased<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S> Service<Request> for Erased<S>
where
    S: Service<Request> + Send + 'static,
    S::Response: IntoResponse + 'static,
    S::Error: Into<Infallible> + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = Infallible;
    type Future = BoxedFuture;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let future = self.0.call(request);
        Box::pin(async move {
            future
                .await
                .map(IntoResponse::into_response)
                .map_err(Into::into)
        })
    }
}

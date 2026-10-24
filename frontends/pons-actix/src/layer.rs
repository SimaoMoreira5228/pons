use std::rc::Rc;

use actix_service::boxed::BoxService;
use actix_web::body::MessageBody;
use actix_web::dev::{ServiceRequest, ServiceResponse, Transform};
use actix_web::{Error, Route};

use crate::auth::AuthSpec;
use crate::router::Router;

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
    pub fn scope<S, B>(self, mut router: Router<S>) -> Router<S>
    where
        L: Transform<
                BoxService<ServiceRequest, ServiceResponse, Error>,
                ServiceRequest,
                Response = ServiceResponse<B>,
                Error = Error,
                InitError = (),
            > + 'static,
        B: MessageBody + 'static,
    {
        let requirement = self.spec.requirement();
        router.scope_current(requirement, LayerInstaller::new(self.layer));
        router
    }

    pub fn global<S, B>(self, mut router: Router<S>) -> Router<S>
    where
        L: Transform<
                BoxService<ServiceRequest, ServiceResponse, Error>,
                ServiceRequest,
                Response = ServiceResponse<B>,
                Error = Error,
                InitError = (),
            > + 'static,
        B: MessageBody + 'static,
    {
        let requirement = self.spec.requirement();
        router.add_global_layer(requirement, LayerInstaller::new(self.layer));
        router
    }
}

#[derive(Clone)]
pub(crate) struct LayerInstaller(Rc<dyn Fn(Route) -> Route>);

impl LayerInstaller {
    fn new<L, B>(layer: L) -> Self
    where
        L: Transform<
                BoxService<ServiceRequest, ServiceResponse, Error>,
                ServiceRequest,
                Response = ServiceResponse<B>,
                Error = Error,
                InitError = (),
            > + 'static,
        B: MessageBody + 'static,
    {
        let layer = Rc::new(layer);
        Self(Rc::new(move |route| route.wrap(Rc::clone(&layer))))
    }

    pub(crate) fn apply(&self, route: Route) -> Route {
        (self.0)(route)
    }

    pub(crate) fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

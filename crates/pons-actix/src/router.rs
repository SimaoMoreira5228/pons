use std::collections::BTreeMap;
use std::sync::Arc;

use actix_web::Route;
use actix_web::dev::{ServiceFactory, ServiceRequest};
use pons_core::{ErrorSpec, TypeCx};
use pons_ir::{
    AuthRequirement, AuthScheme, Diagnostic, Document, ErrorResponse, Info, TypeDef, TypeRef,
    ValidationError,
};

use crate::auth::merge_auth_requirements;
use crate::layer::LayerInstaller;
use crate::operation::build;
use crate::operation_cx::push_diagnostic;
use crate::{
    declare_degraded_headers, prune_unreferenced_types, resolve_request_body, reuse_inherited_slots,
};

pub(crate) struct ResourceEntry {
    pub(crate) path: String,
    pub(crate) routes: Vec<Route>,
    pub(crate) layers: Vec<LayerInstaller>,
}

pub struct Router<S = ()> {
    pub(crate) info: Info,
    pub(crate) types: BTreeMap<String, TypeDef>,
    pub(crate) carriers: Vec<TypeRef>,
    pub(crate) auth_schemes: BTreeMap<String, AuthScheme>,
    pub(crate) common_errors: Vec<ErrorResponse>,
    pub(crate) operations: Vec<pons_ir::Operation>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) resources: Vec<ResourceEntry>,
    pub(crate) global_layers: Vec<LayerInstaller>,
    pub(crate) global_auth: Vec<AuthRequirement>,
    pub(crate) state: Option<S>,
}

impl<S> Router<S> {
    pub fn new(info: Info) -> Self {
        Self {
            info,
            types: BTreeMap::new(),
            carriers: Vec::new(),
            auth_schemes: BTreeMap::new(),
            common_errors: Vec::new(),
            operations: Vec::new(),
            diagnostics: Vec::new(),
            resources: Vec::new(),
            global_layers: Vec::new(),
            global_auth: Vec::new(),
            state: None,
        }
    }

    pub fn route(mut self, path: &str, route: crate::routing::MethodRoute) -> Self {
        if self
            .resources
            .iter()
            .any(|entry| entry.path == path && !entry.layers.is_empty())
        {
            push_diagnostic(
                &mut self.diagnostics,
                "PONS-E102",
                format!(
                    "the route {path} is registered after a documented layer wrapped it, so the new route cannot stay outside that layer; register it before the layer"
                ),
            );
            return self;
        }
        for description in route.descriptions {
            let mut built = build(path, description);
            resolve_request_body(&mut built.operation);
            self.types.extend(built.types);
            self.carriers.extend(built.carriers);
            self.operations.push(built.operation);
        }
        match self.resources.iter_mut().find(|entry| entry.path == path) {
            Some(entry) => entry.routes.extend(route.routes),
            None => self.resources.push(ResourceEntry {
                path: path.to_owned(),
                routes: route.routes,
                layers: Vec::new(),
            }),
        }
        self
    }

    pub fn auth_scheme(mut self, id: impl Into<String>, scheme: AuthScheme) -> Self {
        self.insert_auth_scheme(id.into(), scheme);
        self
    }

    pub fn common_error<E: ErrorSpec>(mut self) -> Self {
        let mut types = TypeCx::new();
        self.common_errors.extend(E::errors(&mut types));
        let (definitions, diagnostics) = types.into_parts();
        self.types.extend(definitions);
        self.diagnostics.extend(diagnostics);
        self
    }

    pub fn with_state<S2>(self, state: S2) -> Router<S2> {
        Router {
            info: self.info,
            types: self.types,
            carriers: self.carriers,
            auth_schemes: self.auth_schemes,
            common_errors: self.common_errors,
            operations: self.operations,
            diagnostics: self.diagnostics,
            resources: self.resources,
            global_layers: self.global_layers,
            global_auth: self.global_auth,
            state: Some(state),
        }
    }

    pub fn document(&self) -> Document {
        let mut operations = self.operations.clone();
        self.settle(&mut operations);
        let mut document = Document {
            ir_version: pons_ir::IR_VERSION.to_owned(),
            info: self.info.clone(),
            types: self.types.clone(),
            auth_schemes: self.auth_schemes.clone(),
            common_errors: self.common_errors.clone(),
            operations,
            ext: BTreeMap::new(),
            diagnostics: self.diagnostics.clone(),
        };
        prune_unreferenced_types(&mut document, &self.carriers);
        document
    }

    pub fn try_lower_document(
        self,
        lower: impl FnOnce(Document) -> Result<Document, Vec<ValidationError>>,
    ) -> Result<Document, Vec<ValidationError>> {
        let mut document = lower(self.into_document_value())?;
        document.canonicalize();
        document.validate()?;
        Ok(document)
    }

    pub fn into_document(self) -> Result<Document, Vec<ValidationError>> {
        crate::checked(self.into_document_value())
    }

    pub(crate) fn into_document_value(mut self) -> Document {
        let mut operations = std::mem::take(&mut self.operations);
        self.settle(&mut operations);
        let mut document = Document {
            ir_version: pons_ir::IR_VERSION.to_owned(),
            info: self.info,
            types: self.types,
            auth_schemes: self.auth_schemes,
            common_errors: self.common_errors,
            operations,
            ext: BTreeMap::new(),
            diagnostics: self.diagnostics,
        };
        prune_unreferenced_types(&mut document, &self.carriers);
        document
    }

    fn settle(&self, operations: &mut [pons_ir::Operation]) {
        operations.iter_mut().for_each(|operation| {
            for requirement in &self.global_auth {
                merge_auth_requirements(&mut operation.auth, requirement);
            }
        });
        reuse_inherited_slots(&self.common_errors, operations);
        declare_degraded_headers(&self.common_errors, operations);
    }

    pub(crate) fn insert_auth_scheme(&mut self, id: String, scheme: AuthScheme) {
        match self.auth_schemes.get(&id) {
            Some(existing) if existing == &scheme => {}
            Some(_) => push_diagnostic(
                &mut self.diagnostics,
                "PONS-E102",
                format!(
                    "the auth scheme {id} is declared twice with different definitions; one scheme id names one scheme"
                ),
            ),
            None => {
                self.auth_schemes.insert(id, scheme);
            }
        }
    }

    pub(crate) fn scope_current(&mut self, requirement: AuthRequirement, layer: LayerInstaller) {
        self.operations
            .iter_mut()
            .for_each(|operation| merge_auth_requirements(&mut operation.auth, &requirement));
        self.resources
            .iter_mut()
            .for_each(|resource| resource.layers.push(layer.clone()));
    }

    pub(crate) fn add_global_layer(&mut self, requirement: AuthRequirement, layer: LayerInstaller) {
        self.global_auth.push(requirement);
        self.global_layers.push(layer);
    }
}

impl<S: 'static> Router<S> {
    pub fn into_actix(
        mut self,
    ) -> Result<
        actix_web::App<
            impl ServiceFactory<
                ServiceRequest,
                Config = (),
                Response = actix_web::dev::ServiceResponse,
                Error = actix_web::Error,
                InitError = (),
            > + 'static,
        >,
        Vec<ValidationError>,
    > {
        let state = self.state.take();
        let resources = std::mem::take(&mut self.resources);
        let global_layers = std::mem::take(&mut self.global_layers);
        let document = crate::checked(self.into_document_value())?;
        let mut app = actix_web::App::new();
        for entry in resources {
            app = app.service(entry.into_resource(&global_layers));
        }
        let app = app.app_data(actix_web::web::Data::new(Arc::new(document)));
        Ok(match state {
            Some(state) => app.app_data(actix_web::web::Data::new(state)),
            None => app,
        })
    }
}

impl ResourceEntry {
    fn into_resource(
        self,
        global_layers: &[LayerInstaller],
    ) -> impl actix_web::dev::HttpServiceFactory + 'static {
        let mut resource = actix_web::web::resource(&self.path);
        for mut route in self.routes {
            for layer in self.layers.iter().chain(global_layers) {
                route = layer.apply(route);
            }
            resource = resource.route(route);
        }
        resource
    }
}

impl<S> Router<S> {
    pub(crate) fn push_resource(&mut self, resource: ResourceEntry) {
        match self
            .resources
            .iter_mut()
            .find(|entry| entry.path == resource.path)
        {
            Some(existing) if existing.layers.is_empty() && resource.layers.is_empty() => {
                existing.routes.extend(resource.routes);
            }
            Some(existing) if same_layers(&existing.layers, &resource.layers) => {
                existing.routes.extend(resource.routes);
            }
            Some(_) => push_diagnostic(
                &mut self.diagnostics,
                "PONS-E102",
                format!(
                    "the route {} is registered in two routers with different documented layers; one resource cannot serve both",
                    resource.path
                ),
            ),
            None => self.resources.push(resource),
        }
    }
}

fn same_layers(left: &[LayerInstaller], right: &[LayerInstaller]) -> bool {
    left.len() == right.len() && left.iter().zip(right).all(|(left, right)| left.same(right))
}

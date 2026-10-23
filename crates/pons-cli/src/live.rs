use std::collections::BTreeMap;

use std::time::Duration;

use pons_ir::Method;
use ureq::Agent;

pub(crate) struct Client {
    agent: Agent,
    base_url: String,
}

impl Client {
    pub(crate) fn new(base_url: &str, timeout: Duration) -> Self {
        let config = Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .build();
        Self {
            agent: Agent::new_with_config(config),
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
    }

    pub(crate) fn send(&self, request: &Request<'_>) -> Result<Reply, String> {
        let url = format!("{}{}", self.base_url, request.path);
        let response = match request.method {
            Method::Get => self
                .agent
                .get(&url)
                .query_pairs(request.query.iter().cloned())
                .headers(request)
                .call(),
            Method::Delete => self
                .agent
                .delete(&url)
                .query_pairs(request.query.iter().cloned())
                .headers(request)
                .call(),
            Method::Head => self
                .agent
                .head(&url)
                .query_pairs(request.query.iter().cloned())
                .headers(request)
                .call(),
            Method::Options => self
                .agent
                .options(&url)
                .query_pairs(request.query.iter().cloned())
                .headers(request)
                .call(),
            Method::Post | Method::Put | Method::Patch => {
                let builder = match request.method {
                    Method::Post => self.agent.post(&url),
                    Method::Put => self.agent.put(&url),
                    _ => self.agent.patch(&url),
                };
                builder
                    .query_pairs(request.query.iter().cloned())
                    .headers(request)
                    .send_body(request.body.as_ref())
            }
        }
        .map_err(|error| error.to_string())?;

        let mut response = response;
        let status = response.status().as_u16();
        let mut headers: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (name, value) in response.headers() {
            if let Ok(value) = value.to_str() {
                headers
                    .entry(name.as_str().to_ascii_lowercase())
                    .or_default()
                    .push(value.to_owned());
            }
        }
        let content_type = headers
            .get("content-type")
            .and_then(|values| values.first())
            .cloned();
        let body = response
            .body_mut()
            .with_config()
            .limit(10 * 1024 * 1024)
            .read_to_string()
            .map_err(|error| error.to_string())?;
        Ok(Reply {
            status,
            content_type,
            headers,
            body,
        })
    }
}

pub(crate) struct Request<'a> {
    pub(crate) method: Method,
    pub(crate) path: &'a str,
    pub(crate) query: &'a [(String, String)],
    pub(crate) headers: &'a [(String, String)],
    pub(crate) body: Option<Body<'a>>,
}

pub(crate) struct Body<'a> {
    pub(crate) content_type: &'a str,
    pub(crate) text: &'a str,
}

pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) content_type: Option<String>,
    pub(crate) headers: BTreeMap<String, Vec<String>>,
    pub(crate) body: String,
}

trait RequestBuilderExt {
    fn headers(self, request: &Request<'_>) -> Self;
}

impl<B> RequestBuilderExt for ureq::RequestBuilder<B> {
    fn headers(mut self, request: &Request<'_>) -> Self {
        for (name, value) in request.headers {
            self = self.header(name, value);
        }
        self
    }
}

trait SendBodyExt {
    fn send_body(
        self,
        body: Option<&Body<'_>>,
    ) -> Result<ureq::http::Response<ureq::Body>, ureq::Error>;
}

impl SendBodyExt for ureq::RequestBuilder<ureq::typestate::WithoutBody> {
    fn send_body(
        self,
        body: Option<&Body<'_>>,
    ) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
        match body {
            Some(body) => self
                .header("Content-Type", body.content_type)
                .force_send_body()
                .send(body.text),
            None => self.call(),
        }
    }
}

impl SendBodyExt for ureq::RequestBuilder<ureq::typestate::WithBody> {
    fn send_body(
        self,
        body: Option<&Body<'_>>,
    ) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
        match body {
            Some(body) => self
                .header("Content-Type", body.content_type)
                .send(body.text),
            None => self.send(()),
        }
    }
}

#![forbid(unsafe_code)]
//! Synchronous, bounded TypeSafe Choice adapter. No retries or provider escalation.
//! RustDecision owns numerical validation, typed mapping and acceptance policy.
mod config;
mod strict_json;
mod wire;
pub use config::{Config, StateEncoding};
use reqwest::{blocking::Client, header};
pub use rust_decision;
use rust_decision::{
    Backend, BackendEvent, BackendFailure, BackendRequest, Capabilities, Observation, Policy,
    Report, Request,
};
use std::{fmt, io::Read};

/// Safe error categories. Display/Debug never include request, body, URL or key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Configuration,
    InvalidRequest,
    RequestTooLarge,
    ResponseTooLarge,
    Transport,
    TimedOut,
    Http(u16),
    MalformedResponse,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Jev adapter error: {self:?}")
    }
}
impl std::error::Error for Error {}
impl Error {
    fn backend(self) -> BackendFailure {
        match self {
            Self::TimedOut | Self::Http(408 | 504) => BackendFailure::TimedOut,
            Self::Http(401 | 403) => BackendFailure::Authentication,
            Self::InvalidRequest | Self::RequestTooLarge | Self::Http(422) => {
                BackendFailure::UnsupportedCapability
            }
            Self::ResponseTooLarge | Self::MalformedResponse => BackendFailure::MalformedResponse,
            _ => BackendFailure::Transport,
        }
    }
}
/// Unknown token counts remain None rather than manufactured zeroes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}
/// Adapter-owned provenance; requested and returned model IDs are distinct.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub provider_label: String,
    pub requested_model: String,
    pub returned_model: String,
    pub typesafe_request_id: Option<String>,
    pub gateway_request_id: Option<String>,
    pub usage: Option<Usage>,
    pub adapter_version: &'static str,
}
/// One operation's core decision and adapter evidence. Never stale from a prior call.
#[derive(Clone, Debug)]
pub struct JevReport<T> {
    pub core: Report<T>,
    pub metadata: Option<Metadata>,
    pub adapter_error: Option<Error>,
}
/// A blocking HTTP backend. Use on a synchronous thread, outside async runtimes.
pub struct JevClient {
    config: Config,
    endpoint: reqwest::Url,
    http: Client,
    metadata: Option<Metadata>,
    error: Option<Error>,
}
impl fmt::Debug for JevClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JevClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}
impl JevClient {
    /// Validate configuration and build TLS transport without making a request.
    pub fn new(config: Config) -> Result<Self, Error> {
        let endpoint = config.validate()?;
        let http = Client::builder()
            .timeout(config.timeout)
            .connect_timeout(config.connect_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .user_agent(concat!("rust-jev/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| Error::Configuration)?;
        Ok(Self {
            config,
            endpoint,
            http,
            metadata: None,
            error: None,
        })
    }
    /// Return a snapshot tied to this operation, including when core validation
    /// prevents invocation. Observations do not interrupt blocking HTTP; transport
    /// timeout bounds HTTP operations and the core rechecks observations on return.
    pub fn decide<T: Clone>(
        &mut self,
        request: &Request<T>,
        policy: &Policy,
        observe: impl FnMut() -> Observation,
    ) -> JevReport<T> {
        self.metadata = None;
        self.error = None;
        let core = rust_decision::decide(request, policy, self, observe);
        JevReport {
            core,
            metadata: self.metadata.take(),
            adapter_error: self.error.take(),
        }
    }
    fn send(&mut self, r: &BackendRequest) -> Result<BackendEvent, Error> {
        let raw_bytes = r
            .context
            .len()
            .checked_add(r.instructions.len())
            .and_then(|n| n.checked_add(r.question_id.len()))
            .and_then(|n| {
                r.options.iter().try_fold(n, |n, (id, d)| {
                    n.checked_add(id.len())?.checked_add(d.len())
                })
            })
            .ok_or(Error::RequestTooLarge)?;
        if raw_bytes > self.config.max_request_bytes {
            return Err(Error::RequestTooLarge);
        }
        let body = wire::request(
            &self.config.model,
            r,
            matches!(self.config.state_encoding, StateEncoding::Json),
        )?;
        if body.len() > self.config.max_request_bytes {
            return Err(Error::RequestTooLarge);
        }
        let response = self
            .http
            .post(self.endpoint.clone())
            // Blocking client timeout also applies per read. A request timeout
            // additionally sets the inner transport's total header/body timer.
            .timeout(self.config.timeout)
            .header(header::AUTHORIZATION, self.config.auth())
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json")
            .body(body)
            .send()
            .map_err(|e| {
                if e.is_timeout() {
                    Error::TimedOut
                } else {
                    Error::Transport
                }
            })?;
        if !response.status().is_success() {
            return Err(Error::Http(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|n| n > self.config.max_response_bytes as u64)
        {
            return Err(Error::ResponseTooLarge);
        }
        let id = |name| {
            response
                .headers()
                .get(name)
                .and_then(|h| h.to_str().ok())
                .filter(|s| config::label(s))
                .map(String::from)
        };
        let typesafe_id = id("x-typesafe-request-id");
        let gateway_id = id("x-request-id");
        let mut body = vec![];
        response
            .take(self.config.max_response_bytes as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.get_ref()
                        .and_then(|e| e.downcast_ref::<reqwest::Error>())
                        .is_some_and(reqwest::Error::is_timeout)
                {
                    Error::TimedOut
                } else {
                    Error::Transport
                }
            })?;
        if body.len() > self.config.max_response_bytes {
            return Err(Error::ResponseTooLarge);
        }
        let (prediction, metadata) = wire::response(
            &body,
            r,
            &self.config.model,
            &self.config.provider_label,
            typesafe_id,
            gateway_id,
        )?;
        self.metadata = Some(metadata);
        Ok(BackendEvent::Prediction(prediction))
    }
}
impl Backend for JevClient {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            text_choice: true,
            max_text_bytes: self.config.max_request_bytes,
            max_options: 255,
        }
    }
    fn invoke(&mut self, request: &BackendRequest) -> BackendEvent {
        self.metadata = None;
        self.error = None;
        match self.send(request) {
            Ok(event) => event,
            Err(error) => {
                self.error = Some(error);
                BackendEvent::Failed(error.backend())
            }
        }
    }
}

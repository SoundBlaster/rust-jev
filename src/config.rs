use crate::Error;
use reqwest::{Url, header::HeaderValue};
use std::{fmt, time::Duration};

/// Explicit interpretation of the core's text context; never auto-detect JSON.
#[derive(Clone, Copy, Debug, Default)]
pub enum StateEncoding {
    #[default]
    Text,
    Json,
}
/// Explicit HTTP configuration. Credentials and URLs are redacted in Debug.
pub struct Config {
    pub endpoint: String,
    pub model: String,
    pub provider_label: String,
    pub timeout: Duration,
    pub connect_timeout: Duration,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub state_encoding: StateEncoding,
    auth: HeaderValue,
    local_http: bool,
}
impl Config {
    pub fn new(api_key: &str, model: impl Into<String>) -> Result<Self, Error> {
        if api_key.is_empty() || !api_key.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(Error::Configuration);
        }
        let mut auth = HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|_| Error::Configuration)?;
        auth.set_sensitive(true);
        Ok(Self {
            endpoint: "https://api.typesafe.ai/v1/systemone".into(),
            model: model.into(),
            provider_label: "typesafe".into(),
            timeout: Duration::from_secs(10),
            connect_timeout: Duration::from_secs(5),
            max_request_bytes: 1024 * 1024,
            max_response_bytes: 1024 * 1024,
            state_encoding: StateEncoding::Text,
            auth,
            local_http: false,
        })
    }
    /// Explicit HTTP exception for literal loopback addresses only, used by mock tests.
    pub fn allow_loopback_http(mut self) -> Self {
        self.local_http = true;
        self
    }
    pub(crate) fn validate(&self) -> Result<Url, Error> {
        if self.endpoint.chars().any(char::is_whitespace)
            || !label(&self.model)
            || !label(&self.provider_label)
            || self.timeout.is_zero()
            || self.connect_timeout.is_zero()
            || self.timeout > Duration::from_secs(300)
            || self.connect_timeout > self.timeout
            || self.max_request_bytes == 0
            || self.max_response_bytes == 0
            || self.max_request_bytes > 16 * 1024 * 1024
            || self.max_response_bytes > 16 * 1024 * 1024
        {
            return Err(Error::Configuration);
        }
        let url = Url::parse(&self.endpoint).map_err(|_| Error::Configuration)?;
        let loopback = url
            .host_str()
            .and_then(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .ok()
            })
            .is_some_and(|ip| ip.is_loopback());
        if url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !(url.scheme() == "https" || (url.scheme() == "http" && self.local_http && loopback))
        {
            return Err(Error::Configuration);
        }
        Ok(url)
    }
    pub(crate) fn auth(&self) -> HeaderValue {
        self.auth.clone()
    }
}
pub(crate) fn label(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 512 && !s.chars().any(char::is_control)
}
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("credentials", &"[redacted]")
            .field("endpoint", &"[redacted]")
            .finish_non_exhaustive()
    }
}

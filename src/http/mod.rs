#[cfg(test)]
mod tests;

use std::{io::Read, net::SocketAddr, str::FromStr, sync::RwLock};

use http::{Request, Response};
use ureq::{typestate::WithoutBody, RequestBuilder};

use crate::{digest::Digest, EventHandler, Reference};

const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

#[derive(thiserror::Error, Debug)]
pub enum HttpError {
    #[error("{0}")]
    HttpMessage(#[from] Box<http::Error>),

    #[error("{0}")]
    Client(#[from] Box<ureq::Error>),

    #[error("Missing authentication tokens.")]
    MissingTokens,

    #[error("Invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

impl From<ureq::Error> for HttpError {
    fn from(value: ureq::Error) -> Self {
        HttpError::Client(Box::new(value))
    }
}

impl From<http::header::InvalidHeaderValue> for HttpError {
    fn from(value: http::header::InvalidHeaderValue) -> Self {
        HttpError::HttpMessage(Box::new(value.into()))
    }
}

impl From<http::Error> for HttpError {
    fn from(value: http::Error) -> Self {
        HttpError::HttpMessage(Box::new(value))
    }
}

pub(super) struct Client<'a, E> {
    agent: ureq::Agent,
    event_handler: &'a E,
    auth_token: RwLock<Option<String>>,
    host: String,
}

impl<'a, E> Client<'a, E>
where
    E: EventHandler,
{
    /// Create a new HTTP client to the registry/image in `reference`.
    ///
    /// It tries to guess the URI scheme for the registry:
    ///
    /// * If it is a loopback IP (like `127.0.0.1`), or if the port
    ///   is `:80`, it uses `http://`.
    /// * In any other case, it uses `https://`.
    pub fn new(reference: &Reference, event_handler: &'a E) -> Self {
        let host = format!(
            "{}{}/v2/{}",
            guess_scheme(reference.registry),
            reference.registry,
            reference.repository
        );

        let agent = ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .http_status_as_error(false)
            .build()
            .into();

        Client {
            agent,
            event_handler,
            auth_token: Default::default(),
            host,
        }
    }

    /// Send a `GET` request to the registry.
    ///
    /// The path must not include the `v2/$image` prefix.
    pub fn get(&self, path: &str, accept: Option<&str>) -> Result<Response<ureq::Body>, HttpError> {
        let url = format!("{}/{}", self.host, path);
        let mut request = Request::get(&url);
        if let Some(accept) = accept {
            request = request.header("Accept", accept);
        }

        self.send(request.body(())?)
    }

    /// Send a `GET` request to download a blob.
    pub fn download_blob(&self, blob: &Digest) -> Result<impl Read, HttpError> {
        let response = self.get(&format!("blobs/{}", blob.source()), None)?;
        Ok(blob.wrap_reader(response.into_body().into_reader()))
    }

    /// Send a request to the registry.
    ///
    /// If it responds with a `401` error, get the token from the
    /// URL in the `WWW-Authenticate` header.
    fn send(&self, mut request: Request<()>) -> Result<Response<ureq::Body>, HttpError> {
        self.event_handler.registry_request(request.uri());

        let auth_token = self.auth_token.read().unwrap();
        if let Some(auth) = auth_token.as_deref() {
            request.headers_mut().append("Authorization", auth.parse()?);
            return Ok(self.agent.run(request)?);
        }

        drop(auth_token);
        let mut auth_token = self.auth_token.write().unwrap();

        // Try a request with no token.

        let response = match self.agent.run(request.clone()) {
            Ok(r) if r.status().as_u16() == 401 => r,
            Ok(r) => return Ok(r),
            Err(e) => return Err(e.into()),
        };

        // Request a token if the response from the 401 includes the
        // WWW-Authenticate header.
        //
        // The response from the `realm` URL must include either
        // `token` or `access_token`.

        let Some(auth_request) = response
            .headers()
            .get("www-authenticate")
            .and_then(|auth| build_auth_request(self.agent.clone(), auth.as_bytes()))
        else {
            return Err(ureq::Error::StatusCode(401).into());
        };

        if let Some(uri) = auth_request.uri_ref() {
            self.event_handler.registry_auth(uri);
        }

        #[derive(serde::Deserialize, Debug)]
        struct Tokens {
            token: Option<String>,
            access_token: Option<String>,
        }

        let mut token =
            match serde_json::from_reader(auth_request.call()?.into_body().into_reader())? {
                Tokens { token: Some(t), .. } => t,
                Tokens {
                    access_token: Some(t),
                    ..
                } => t,
                _ => return Err(HttpError::MissingTokens),
            };

        token.insert_str(0, "Bearer ");
        *auth_token = Some(token);
        drop(auth_token);

        // Repeat the request, now that we have a token.
        self.send(request)
    }
}

fn guess_scheme(registry: &str) -> &'static str {
    const HTTP: &str = "http://";
    const HTTPS: &str = "https://";

    if registry.ends_with(":80") {
        return HTTP;
    }

    if let Ok(address) = SocketAddr::from_str(registry) {
        let loopback = match address {
            SocketAddr::V4(v4) => v4.ip().is_loopback(),
            SocketAddr::V6(v6) => v6.ip().is_loopback(),
        };

        return if loopback { HTTP } else { HTTPS };
    }

    HTTPS
}

/// Parse a `WWW-Authenticate` header and build the request to
/// get the authentication token.
///
/// Return `None` if the header can't be parsed.
///
/// See <https://distribution.github.io/distribution/spec/auth/token/>
/// for more details.
fn build_auth_request(agent: ureq::Agent, auth_spec: &[u8]) -> Option<RequestBuilder<WithoutBody>> {
    let mut request = None;
    let mut pending_params = vec![];
    let mut tail = std::str::from_utf8(auth_spec).ok()?;

    // The first token must be `Bearer`
    tail = tail.strip_prefix("Bearer ")?;

    loop {
        let (key, value) = tail.split_once('=')?;
        let key = key.trim_ascii();
        let (value, after) = value.strip_prefix('"')?.split_once('"')?;

        if key == "realm" {
            request = Some(
                pending_params
                    .drain(..)
                    .fold(agent.get(value), |r, (k, v)| r.query(k, v)),
            );
        } else {
            match request.take() {
                Some(r) => request = Some(r.query(key, value)),
                None => pending_params.push((key, value)),
            }
        }

        tail = match after.trim_ascii_start() {
            "" => return request,
            t => t.strip_prefix(',')?,
        };
    }
}

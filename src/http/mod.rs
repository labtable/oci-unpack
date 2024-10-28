#[cfg(test)]
mod tests;

use std::{io::Read, net::SocketAddr, str::FromStr, sync::RwLock};

use crate::{digest::Digest, EventHandler, Reference};

const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

#[derive(thiserror::Error, Debug)]
pub enum HttpError {
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

pub(super) struct Client<'a, E> {
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

        Client {
            event_handler,
            auth_token: Default::default(),
            host,
        }
    }

    /// Send a `GET` request to the registry.
    ///
    /// The path must not include the `v2/$image` prefix.
    pub fn get(&self, path: &str, accept: Option<&str>) -> Result<ureq::Response, HttpError> {
        let url = format!("{}/{}", self.host, path);
        let mut request = ureq::get(&url);
        if let Some(accept) = accept {
            request = request.set("Accept", accept);
        }

        self.send(request)
    }

    /// Send a `GET` request to download a blob.
    pub fn download_blob(&self, blob: &Digest) -> Result<impl Read, HttpError> {
        let response = self.get(&format!("blobs/{}", blob.source()), None)?;
        Ok(blob.wrap_reader(response.into_reader()))
    }

    /// Send a request to the registry.
    ///
    /// If it responds with a `401` error, get the token from the
    /// URL in the `WWW-Authenticate` header.
    fn send(&self, request: ureq::Request) -> Result<ureq::Response, HttpError> {
        let request = request.set("User-Agent", USER_AGENT);

        self.event_handler.registry_request(request.url());

        let auth_token = self.auth_token.read().unwrap();
        if let Some(auth) = auth_token.as_deref() {
            return Ok(request.set("Authorization", auth).call()?);
        }

        drop(auth_token);
        let mut auth_token = self.auth_token.write().unwrap();

        // Try a request with no token.

        let response = match request.clone().call() {
            Ok(r) => return Ok(r),
            Err(ureq::Error::Status(401, r)) => r,
            Err(e) => return Err(e.into()),
        };

        // Request a token if the response from the 401 includes the
        // WWW-Authenticate header.
        //
        // The response from the `realm` URL must include either
        // `token` or `access_token`.

        let Some(auth_request) = response
            .header("www-authenticate")
            .and_then(build_auth_request)
        else {
            return Err(ureq::Error::Status(401, response).into());
        };

        self.event_handler.registry_auth(auth_request.url());

        #[derive(serde::Deserialize, Debug)]
        struct Tokens {
            token: Option<String>,
            access_token: Option<String>,
        }

        let mut token = match serde_json::from_reader(auth_request.call()?.into_reader())? {
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
fn build_auth_request(auth_spec: &str) -> Option<ureq::Request> {
    let mut request = None;
    let mut pending_params = vec![];
    let mut tail = auth_spec;

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
                    .fold(ureq::get(value), |r, (k, v)| r.query(k, v)),
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

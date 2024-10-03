use std::{net::SocketAddr, str::FromStr};

use crate::EventHandler;

use super::DownloadError;

const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

pub(super) struct Client<E> {
    event_handler: E,
    auth_token: Option<String>,
    host: String,
}

impl<E> Client<E>
where
    E: EventHandler,
{
    /// Create a new HTTP client to the host for `registry`.
    ///
    /// It tries to guess the URI scheme for the registry:
    ///
    /// * If it is a loopback IP (like `127.0.0.1`), or if the port
    ///   is `:80`, it uses `http://`.
    /// * In any other case, it uses `https://`.
    pub fn new(registry: &str, event_handler: E) -> Self {
        Client {
            event_handler,
            auth_token: None,
            host: format!("{}/{}", guess_scheme(registry), registry),
        }
    }

    /// Send a `GET` request to the registry.
    pub fn get(
        &mut self,
        path: &str,
        accept: Option<&str>,
    ) -> Result<ureq::Response, DownloadError> {
        let mut request = ureq::get(&format!("{}/{}", self.host, path));
        if let Some(accept) = accept {
            request = request.set("Accept", accept);
        }

        self.send(request)
    }

    /// Send a request to the registry.
    ///
    /// If it responds with a `401` error, get the token from the
    /// URL in the `WWW-Authenticate` header.
    fn send(&mut self, request: ureq::Request) -> Result<ureq::Response, DownloadError> {
        let request = request.set("User-Agent", USER_AGENT);

        if let Some(auth) = &self.auth_token {
            return Ok(request.set("Authorization", auth).call()?);
        }

        // Try a request with no token.
        self.event_handler.registry_request(request.url());

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
            _ => return Err(DownloadError::MissingTokens),
        };

        token.insert_str(0, "Bearer ");
        self.auth_token = Some(token);

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

#[test]
fn request_token_after_unauthorized() {
    use tiny_http::{Header, Response};

    struct VoidHandler;

    impl EventHandler for VoidHandler {}

    let server_port = super::tests::http_server(|port, req| {
        const SERVICE: &str = "registry.docker.io";
        const SCOPE: &str = "repository:foo/bar:pull";

        // Use the `url` crate to parse the request query.
        let base_url = url::Url::parse("http://0").ok();
        let url_parser = url::Url::options().base_url(base_url.as_ref());

        let req_url = url_parser.parse(req.url()).unwrap();

        let authorization = req
            .headers()
            .iter()
            .find(|h| h.field.equiv("authorization"))
            .map(|h| h.value.to_string());

        let response = match (req_url.path(), authorization) {
            ("/token", None) => {
                // Verify the query.
                for (k, v) in req_url.query_pairs() {
                    if !((k == "service" && v == SERVICE) || (k == "scope" && v == SCOPE)) {
                        panic!("Invalid query: {k:?} = {v:?}")
                    }
                }

                let json = r#"
                    {
                      "access_token": "00AA11BB",
                      "expires_in": "X",
                      "issued_at": "Y",
                      "token": "00AA11BB"
                    }

                "#;
                Response::from_string(json)
            }

            ("/test", None) => {
                let auth = format!(
                    r#"Bearer realm="http://127.1:{port}/token",service="{SERVICE}",scope="{SCOPE}""#
                );

                Response::from_data(vec![])
                    .with_status_code(401)
                    .with_header(Header::from_bytes("WWW-Authenticate", auth).unwrap())
            }

            ("/test", Some(auth)) => Response::from_string(format!("token={auth}")),

            _ => Response::from_string("Not Found").with_status_code(404),
        };

        req.respond(response).expect("Send response");

        true
    });

    let mut client = Client::new(&format!("127.0.0.1:{server_port}"), VoidHandler);

    // Send a regular request.
    //
    // The client must start the authentication process after
    // receiving a 401.

    let response = client.get("test", None).expect("GET /test");
    assert!(matches!(
        response.into_string().as_deref(),
        Ok("token=Bearer 00AA11BB")
    ));
}

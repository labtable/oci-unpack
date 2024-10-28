use tiny_http::{Request, Server};

use crate::{EventHandler, Reference};

/// Start a HTTP server in a random port.
///
/// Request are handled in `handler`. The server is stopped when the
/// function returns `false`
///
/// Returns the port number of the server.
pub(super) fn test_http_server<F>(mut handler: F) -> u16
where
    F: FnMut(u16, Request) -> bool,
    F: Send + 'static,
{
    let server = Server::http("127.1:0").expect("start HTTP server");
    let port = server.server_addr().to_ip().unwrap().port();

    std::thread::spawn(move || {
        let timeout = std::time::Duration::from_secs(60);
        while let Ok(Some(request)) = server.recv_timeout(timeout) {
            if !handler(port, request) {
                break;
            }
        }
    });

    port
}

#[test]
fn request_token_after_unauthorized() {
    use tiny_http::{Header, Response};

    struct VoidHandler;

    impl EventHandler for VoidHandler {}

    let server_port = test_http_server(|port, req| {
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

            ("/v2/abc/def/test", None) => {
                let auth = format!(
                    r#"Bearer realm="http://127.1:{port}/token",service="{SERVICE}",scope="{SCOPE}""#
                );

                Response::from_data(vec![])
                    .with_status_code(401)
                    .with_header(Header::from_bytes("WWW-Authenticate", auth).unwrap())
            }

            ("/v2/abc/def/test", Some(auth)) => Response::from_string(format!("token={auth}")),

            _ => Response::from_string("Not Found").with_status_code(404),
        };

        req.respond(response).expect("Send response");

        true
    });

    let reference = format!("127.0.0.1:{server_port}/abc/def");
    let reference = Reference::try_from(reference.as_str()).unwrap();
    let client = crate::http::Client::new(&reference, &VoidHandler);

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

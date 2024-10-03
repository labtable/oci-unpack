use std::time::Duration;

use tiny_http::{Request, Server};

/// Start a HTTP server in a random port.
///
/// Request are handled in `handler`. The server is stopped when the
/// function returns `false`
///
/// Returns the port number of the server.
pub(super) fn http_server<F>(mut handler: F) -> u16
where
    F: FnMut(u16, Request) -> bool,
    F: Send + 'static,
{
    let server = Server::http("127.1:0").expect("start HTTP server");
    let port = server.server_addr().to_ip().unwrap().port();

    std::thread::spawn(move || {
        let timeout = Duration::from_secs(60);
        while let Ok(Some(request)) = server.recv_timeout(timeout) {
            if !handler(port, request) {
                break;
            }
        }
    });

    port
}

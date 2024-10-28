use std::time::Duration;

use oci_unpack::MediaType;
use tiny_http::{Header, Request, Response, Server};

use super::blobs::Blob;

pub const ARCH: &str = "ARCH";

pub const OS: &str = "OS";

/// Start a registry server in a random port.
///
/// Returns the port number of the server.
pub fn start_registry(
    repository: &'static str,
    tag: &'static str,
    config: Blob,
    layers: Vec<Blob>,
) -> u16 {
    let server = Server::http("127.1:0").expect("start registry server");
    let port = server.server_addr().to_ip().unwrap().port();

    let registry = Registry::new(server, repository, tag, config, layers);

    std::thread::spawn(move || registry.run());

    port
}

struct Registry {
    server: Server,
    manifest_path: String,
    blobs_prefix: String,
    config: Blob,
    layers: Vec<Blob>,
}

impl Registry {
    fn new(
        server: Server,
        repository: &'static str,
        tag: &'static str,
        config: Blob,
        layers: Vec<Blob>,
    ) -> Registry {
        Registry {
            server,
            manifest_path: format!("/v2/{repository}/manifests/{tag}"),
            blobs_prefix: format!("/v2/{repository}/blobs/sha256:"),
            config,
            layers,
        }
    }

    fn run(mut self) {
        let timeout = Duration::from_secs(30);

        while let Ok(Some(request)) = self.server.recv_timeout(timeout) {
            self.handle(request);
        }
    }

    fn handle(&mut self, request: Request) {
        if request.method() != &tiny_http::Method::Get {
            return;
        }

        let url = request.url();

        // Manifest
        if url == self.manifest_path {
            self.manifest(request);
            return;
        }

        // Blobs
        if let Some(digest) = url.strip_prefix(&self.blobs_prefix) {
            if let Some(blob) = self.find_blob(digest) {
                Self::send_body(request, blob.media_type, blob.data.clone());
            }
        }
    }

    fn manifest(&self, request: Request) {
        #[derive(serde::Serialize, Debug)]
        struct Image<'a> {
            config: &'a Blob,
            layers: &'a [Blob],
        }

        Self::send_json(
            request,
            MediaType::OciManifestV1,
            Image {
                config: &self.config,
                layers: &self.layers,
            },
        );
    }

    fn find_blob(&self, digest: &str) -> Option<&Blob> {
        if self.config.digest == digest {
            return Some(&self.config);
        }

        self.layers.iter().find(|l| l.digest == digest)
    }

    fn send_json(request: Request, media_type: MediaType, body: impl serde::Serialize) {
        let body = serde_json::to_vec(&body).expect("Serialize JSON");
        Self::send_body(request, media_type, body);
    }

    fn send_body(request: Request, media_type: MediaType, body: impl Into<Vec<u8>>) {
        let response = Response::from_data(body)
            .with_status_code(200)
            .with_header(Header::from_bytes("Content-Type", media_type.as_str()).unwrap());

        request.respond(response).expect("Send response");
    }
}

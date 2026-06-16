// SPDX-License-Identifier: Apache-2.0
//! HTTP-over-UDS client used by the CLI subcommands. Wraps hyper 1.x
//! handshake on top of a tokio [`UnixStream`].

use std::path::{Path, PathBuf};

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::client::conn::http1;
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;

use crate::error::{AnvilDaemonError, Result};

pub struct Client {
    socket: PathBuf,
}

impl Client {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
        }
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub async fn get(&self, path: &str) -> Result<Vec<u8>> {
        self.send("GET", path, Vec::new()).await
    }

    pub async fn post(&self, path: &str, body: Vec<u8>) -> Result<Vec<u8>> {
        self.send("POST", path, body).await
    }

    pub async fn put(&self, path: &str, body: Vec<u8>) -> Result<Vec<u8>> {
        self.send("PUT", path, body).await
    }

    /// Send `body` to `path` using `method` and return the response body
    /// when the status is 2xx. Non-2xx responses are turned into
    /// [`AnvilDaemonError::HttpStatus`] so callers can surface a clean
    /// error to the user.
    async fn send(&self, method: &str, path: &str, body: Vec<u8>) -> Result<Vec<u8>> {
        let stream = UnixStream::connect(&self.socket).await.map_err(|source| {
            AnvilDaemonError::SocketConnect {
                socket: self.socket.clone(),
                source,
            }
        })?;
        let io = TokioIo::new(stream);
        let (mut sender, conn) = http1::handshake::<_, Full<Bytes>>(io).await?;
        // Drive the connection in the background; ignore its error since
        // any meaningful failure surfaces via send_request below.
        tokio::spawn(async move {
            if let Err(err) = conn.await {
                tracing::debug!(?err, "client connection closed");
            }
        });

        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "anvil.local")
            .header(
                "user-agent",
                concat!("anvil-cli/", env!("CARGO_PKG_VERSION")),
            );
        if !body.is_empty() {
            builder = builder.header("content-type", "application/json");
        }
        let req = builder.body(Full::new(Bytes::from(body)))?;

        let resp = sender.send_request(req).await?;
        let status = resp.status();
        let bytes = resp.into_body().collect().await?.to_bytes().to_vec();
        if status == StatusCode::OK || status == StatusCode::CREATED {
            Ok(bytes)
        } else {
            Err(AnvilDaemonError::HttpStatus {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&bytes).into_owned(),
            })
        }
    }
}

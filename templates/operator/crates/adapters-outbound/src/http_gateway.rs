//! The {{product}} API client — **the part to replace** with the real one
//! (a generated client, or hand-written calls against the real API).
//!
//! It speaks a placeholder REST shape, bearer-token authenticated:
//!
//! - `GET    {url}/api/health`          → 2xx when up;
//! - `GET    {url}/api/objects/{name}`  → `{"id", "name", "description"}`, or 404;
//! - `PUT    {url}/api/objects/{name}`  → create, answers `{"id": "…"}`;
//! - `PATCH  {url}/api/objects/{name}`  → update of the fields sent only;
//! - `DELETE {url}/api/objects/{name}`  → 2xx, or 404 when already gone.
//!
//! What must survive the rewrite is the mapping of every failure onto a
//! [`GatewayError`]: it decides the reason code the CR shows.

use std::time::Duration;

use application::ports::{GatewayError, ObjectPatch, Remote, RemoteGateway, RemoteObject};
use reqwest::{Client, RequestBuilder, Response, StatusCode, Url};
use serde::{Deserialize, Serialize};

/// Per-request timeout: a hung server must not hold a reconcile forever.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

pub struct HttpGateway {
    base: Url,
    token: String,
    client: Client,
}

#[derive(Serialize)]
struct ObjectBody<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
}

#[derive(Serialize)]
struct PatchBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
}

#[derive(Deserialize)]
struct Created {
    id: String,
}

#[derive(Deserialize)]
struct Read {
    id: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
}

impl HttpGateway {
    /// `url`: the instance's `spec.url`. `Err` when it can't serve as a
    /// base URL.
    pub fn new(url: &str, token: impl Into<String>, client: Client) -> Result<Self, String> {
        let base = Url::parse(url).map_err(|e| format!("url {url:?}: {e}"))?;
        if base.cannot_be_a_base() || !matches!(base.scheme(), "http" | "https") {
            return Err(format!("url {url:?}: not an http(s) base URL"));
        }
        Ok(Self {
            base,
            token: token.into(),
            client,
        })
    }

    /// The default HTTP client: rustls, platform trust store, timeout.
    pub fn default_client() -> Result<Client, String> {
        Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| format!("building the HTTP client: {e}"))
    }

    /// `{url}/<segments…>`, each segment percent-encoded.
    #[expect(
        clippy::expect_used,
        reason = "`new` only accepts an http(s) URL that can be a base"
    )]
    fn url(&self, segments: &[&str]) -> Url {
        let mut url = self.base.clone();
        url.path_segments_mut()
            // Checked in `new`.
            .expect("an http(s) URL is a base")
            .pop_if_empty()
            .extend(segments);
        url
    }

    async fn send(&self, request: RequestBuilder) -> Result<Response, GatewayError> {
        let response = request
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| GatewayError::Unreachable(e.to_string()))?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.text().await.unwrap_or_default();
        let why = format!("{status}: {}", body.chars().take(200).collect::<String>());
        Err(match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => GatewayError::Unauthorized(why),
            StatusCode::NOT_FOUND => GatewayError::NotFound(why),
            StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT => GatewayError::Unreachable(why),
            _ => GatewayError::Api(why),
        })
    }
}

#[async_trait::async_trait]
impl RemoteGateway for HttpGateway {
    async fn ping(&self) -> Result<(), GatewayError> {
        self.send(self.client.get(self.url(&["api", "health"])))
            .await
            .map(|_| ())
    }

    async fn read(&self, name: &str) -> Result<Option<Remote>, GatewayError> {
        let response = match self
            .send(self.client.get(self.url(&["api", "objects", name])))
            .await
        {
            Ok(response) => response,
            Err(GatewayError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        let read = response
            .json::<Read>()
            .await
            .map_err(|e| GatewayError::Api(format!("unexpected answer: {e}")))?;
        Ok(Some(Remote {
            id: read.id,
            object: RemoteObject {
                name: read.name,
                description: read.description,
            },
        }))
    }

    async fn update(&self, name: &str, patch: &ObjectPatch) -> Result<(), GatewayError> {
        let body = PatchBody {
            description: patch.description.as_deref(),
        };
        self.send(
            self.client
                .patch(self.url(&["api", "objects", name]))
                .json(&body),
        )
        .await
        .map(|_| ())
    }

    async fn create(&self, object: &RemoteObject) -> Result<String, GatewayError> {
        let body = ObjectBody {
            name: &object.name,
            description: object.description.as_deref(),
        };
        let response = self
            .send(
                self.client
                    .put(self.url(&["api", "objects", &object.name]))
                    .json(&body),
            )
            .await?;
        response
            .json::<Created>()
            .await
            .map(|c| c.id)
            .map_err(|e| GatewayError::Api(format!("unexpected answer: {e}")))
    }

    async fn delete(&self, name: &str) -> Result<(), GatewayError> {
        self.send(self.client.delete(self.url(&["api", "objects", name])))
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gateway(url: &str) -> Result<HttpGateway, String> {
        HttpGateway::new(url, "t", Client::new())
    }

    #[test]
    fn paths_hang_off_the_base_url() {
        let gw = gateway("https://example.com/sub/").unwrap();
        assert_eq!(
            gw.url(&["api", "objects", "a b"]).as_str(),
            "https://example.com/sub/api/objects/a%20b"
        );
        let gw = gateway("https://example.com").unwrap();
        assert_eq!(
            gw.url(&["api", "health"]).as_str(),
            "https://example.com/api/health"
        );
    }

    #[test]
    fn only_http_base_urls_are_accepted() {
        assert!(gateway("not a url").is_err());
        assert!(gateway("mailto:someone@example.com").is_err());
        assert!(gateway("ftp://example.com").is_err());
    }
}

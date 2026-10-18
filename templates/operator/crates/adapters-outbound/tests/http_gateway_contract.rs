//! Contract test of `HttpGateway` against a scripted server (`wiremock`):
//! the requests it sends and how each answer maps onto a `GatewayError`.
//! Rewrite it along with the gateway when moving to the real API.

use adapters_outbound::HttpGateway;
use application::ports::{GatewayError, ObjectPatch, RemoteGateway, RemoteObject};
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "s3cr3t";

async fn setup() -> (MockServer, HttpGateway) {
    let server = MockServer::start().await;
    let gateway =
        HttpGateway::new(&server.uri(), TOKEN, HttpGateway::default_client().unwrap()).unwrap();
    (server, gateway)
}

fn object() -> RemoteObject {
    RemoteObject {
        name: "docs".into(),
        description: Some("example".into()),
    }
}

#[tokio::test]
async fn ping_sends_the_bearer_token() {
    let (server, gateway) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/health"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    gateway.ping().await.unwrap();
}

#[tokio::test]
async fn read_returns_the_object_or_none() {
    let (server, gateway) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/objects/docs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"id": "42", "name": "docs", "description": "example"}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/objects/gone"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let read = gateway.read("docs").await.unwrap().unwrap();
    assert_eq!((read.id.as_str(), read.object), ("42", object()));
    assert_eq!(gateway.read("gone").await.unwrap(), None);
}

#[tokio::test]
async fn update_sends_only_the_fields_of_the_patch() {
    let (server, gateway) = setup().await;
    Mock::given(method("PATCH"))
        .and(path("/api/objects/docs"))
        .and(body_json(serde_json::json!({"description": "example"})))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let patch = ObjectPatch {
        description: Some("example".into()),
    };
    gateway.update("docs", &patch).await.unwrap();
}

#[tokio::test]
async fn create_puts_the_object_and_returns_its_id() {
    let (server, gateway) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/api/objects/docs"))
        .and(body_json(
            serde_json::json!({"name": "docs", "description": "example"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "42"})))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(gateway.create(&object()).await.unwrap(), "42");
}

#[tokio::test]
async fn delete_reports_an_object_already_gone() {
    let (server, gateway) = setup().await;
    Mock::given(method("DELETE"))
        .and(path("/api/objects/docs"))
        .respond_with(ResponseTemplate::new(204))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/objects/docs"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    gateway.delete("docs").await.unwrap();
    assert!(matches!(
        gateway.delete("docs").await,
        Err(GatewayError::NotFound(_))
    ));
}

#[tokio::test]
async fn failures_map_onto_gateway_errors() {
    let (server, gateway) = setup().await;
    for (status, check) in [
        (
            401,
            (|e| matches!(e, GatewayError::Unauthorized(_))) as fn(&GatewayError) -> bool,
        ),
        (403, |e| matches!(e, GatewayError::Unauthorized(_))),
        (503, |e| matches!(e, GatewayError::Unreachable(_))),
        (500, |e| matches!(e, GatewayError::Api(_))),
    ] {
        server.reset().await;
        Mock::given(method("GET"))
            .and(path("/api/health"))
            .respond_with(ResponseTemplate::new(status).set_body_string("nope"))
            .mount(&server)
            .await;
        let err = gateway.ping().await.unwrap_err();
        assert!(check(&err), "{status}: {err:?}");
    }
}

#[tokio::test]
async fn an_unexpected_answer_is_an_api_error() {
    let (server, gateway) = setup().await;
    Mock::given(method("PUT"))
        .and(path("/api/objects/docs"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;
    assert!(matches!(
        gateway.create(&object()).await,
        Err(GatewayError::Api(_))
    ));
}

#[tokio::test]
async fn a_closed_port_is_unreachable() {
    // A port just released: nothing listens on it any more.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let gateway = HttpGateway::new(
        &format!("http://127.0.0.1:{port}"),
        TOKEN,
        HttpGateway::default_client().unwrap(),
    )
    .unwrap();
    assert!(matches!(
        gateway.ping().await,
        Err(GatewayError::Unreachable(_))
    ));
}

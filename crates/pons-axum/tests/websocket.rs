use std::collections::BTreeMap;

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket};
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use pons_axum::{DescribeResponse, Info, Router, WebSocketUpgrade, routing::get};
use pons_core::Schema;
use serde::{Deserialize, Serialize};
use tower::ServiceExt;

#[allow(dead_code)]
#[derive(Deserialize, Schema)]
struct ClientMessage {
    text: String,
}

#[allow(dead_code)]
#[derive(Serialize, Schema)]
struct ServerMessage {
    text: String,
}

async fn echo(
    upgrade: WebSocketUpgrade<ClientMessage, ServerMessage>,
) -> impl IntoResponse + DescribeResponse {
    upgrade.on_upgrade(|mut socket: WebSocket| async move {
        while let Some(Ok(Message::Text(text))) = socket.recv().await {
            if socket.send(Message::Text(text)).await.is_err() {
                break;
            }
        }
    })
}

fn info() -> Info {
    Info {
        title: "WebSocket".into(),
        version: "1".into(),
        description: None,
        servers: Vec::new(),
        ext: BTreeMap::new(),
    }
}

fn router() -> Router {
    Router::<()>::new(info()).route("/ws", get(echo))
}

#[test]
fn a_websocket_operation_describes_both_message_directions() {
    let document = router()
        .into_document()
        .expect("the websocket operation describes a contract");
    let operation = &document.operations[0];
    let stream = operation
        .stream
        .as_ref()
        .expect("the operation publishes a stream");
    let pons_ir::StreamKind::Websocket { client, server } = &stream.kind else {
        panic!("the operation is a websocket stream");
    };
    let pons_ir::TypeRef::Named(client) = client else {
        panic!("the client message is named");
    };
    let pons_ir::TypeRef::Named(server) = server else {
        panic!("the server message is named");
    };
    let client = document
        .types
        .get(&client.id)
        .expect("the client message is described");
    let server = document
        .types
        .get(&server.id)
        .expect("the server message is described");
    assert_eq!(client.name, ["client", "message"]);
    assert_eq!(client.direction, pons_ir::Direction::Input);
    assert_eq!(server.name, ["server", "message"]);
    assert_eq!(server.direction, pons_ir::Direction::Output);
    assert_eq!(
        operation.responses[0].status,
        pons_ir::ResponseStatus::Exact(101)
    );
    assert_eq!(operation.responses[0].content, pons_ir::ContentKind::Empty);
    assert!(operation.responses[0].body.is_none());
}

#[tokio::test]
async fn a_plain_http_request_is_refused_before_upgrading() {
    let app = router().into_axum().expect("the router validates");
    let response = app
        .oneshot(Request::builder().uri("/ws").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_upgrade_switches_protocols_and_streams_messages() {
    let app = router().into_axum().expect("the router validates");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a local listener");
    let address = listener.local_addr().expect("a local address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("the server runs");
    });

    let (mut socket, response) = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
        .await
        .expect("the websocket connects");
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);

    socket
        .send(tokio_tungstenite::tungstenite::Message::text("hello"))
        .await
        .expect("the client sends a message");
    let message = socket
        .next()
        .await
        .expect("the server answers")
        .expect("the server answer is a message");
    assert_eq!(
        message,
        tokio_tungstenite::tungstenite::Message::text("hello")
    );

    server.abort();
}

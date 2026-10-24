use std::collections::BTreeMap;

use actix_web::http::StatusCode;
use actix_web::test as actix_test;
use actix_web::test::TestRequest;
use futures_util::StreamExt;
use pons_actix::{Info, Router, WebSocket, WebSocketUpgrade, routing::get_with};
use pons_core::Schema;
use serde::{Deserialize, Serialize};

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
) -> WebSocket<ClientMessage, ServerMessage> {
    upgrade.on_upgrade(|mut session, mut messages| async move {
        while let Some(Ok(message)) = messages.next().await {
            if let actix_ws::Message::Text(text) = message
                && session.text(text).await.is_err()
            {
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
    Router::<()>::new(info()).route(
        "/ws",
        get_with(echo, pons_actix::OperationMeta::new("ws.echo")),
    )
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
}

#[actix_web::test]
async fn a_plain_http_request_is_refused_before_upgrading() {
    let app = actix_test::init_service(router().into_actix().expect("the router validates")).await;
    let response = actix_test::call_service(&app, TestRequest::get().uri("/ws").to_request()).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#![cfg(feature = "websocket")]

use std::collections::BTreeMap;

use pons_core::Schema;
use pons_ir::{ContentKind, Direction, ResponseStatus, StreamKind, TypeRef};
use pons_tako::{Info, PonsWebSocket, Router};
use serde::{Deserialize, Serialize};
use tako::types::Request;

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct ClientMessage {
    text: String,
}

#[derive(Serialize, Schema)]
struct ServerMessage {
    text: String,
}

async fn echo(request: Request) -> PonsWebSocket<ClientMessage, ServerMessage> {
    PonsWebSocket::new(request, |mut socket| async move {
        while let Some(Ok(ClientMessage { text })) = socket.recv().await {
            if socket.send(ServerMessage { text }).await.is_err() {
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

#[test]
fn a_websocket_operation_describes_both_message_directions() {
    let mut router = Router::new(info());
    router.get("/ws", echo);
    let document = router
        .into_document()
        .expect("the websocket operation describes a contract");
    document.validate().expect("the document is valid");

    let operation = &document.operations[0];
    let stream = operation
        .stream
        .as_ref()
        .expect("the operation publishes a stream");
    let StreamKind::Websocket { client, server } = &stream.kind else {
        panic!("the operation is a websocket stream");
    };
    let TypeRef::Named(client) = client else {
        panic!("the client message is named");
    };
    let TypeRef::Named(server) = server else {
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
    assert_eq!(client.direction, Direction::Input);
    assert_eq!(server.name, ["server", "message"]);
    assert_eq!(server.direction, Direction::Output);

    assert_eq!(operation.responses.len(), 1);
    assert_eq!(operation.responses[0].status, ResponseStatus::Exact(101));
    assert_eq!(operation.responses[0].content, ContentKind::Empty);
    assert!(operation.responses[0].body.is_none());
}

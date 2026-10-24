use std::marker::PhantomData;

use axum::response::sse::Event;
use futures_util::{Stream, StreamExt};
use pons_core::Schema;
use serde::Serialize;

pub trait SseEvent: Schema + Serialize {
    const NAME: &'static str;
}

pub struct Sse<T, S> {
    stream: S,
    event: PhantomData<fn() -> T>,
}

impl<T, S> Sse<T, S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            event: PhantomData,
        }
    }
}

impl<T, S> axum::response::IntoResponse for Sse<T, S>
where
    T: SseEvent + Send + 'static,
    S: Stream<Item = T> + Send + 'static,
{
    fn into_response(self) -> axum::response::Response {
        let events = self.stream.map(|data| {
            Event::default()
                .event(T::NAME)
                .json_data(data)
                .map_err(std::io::Error::other)
        });
        axum::response::Sse::new(events).into_response()
    }
}

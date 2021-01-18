use std::pin::Pin;
use std::task::{self, Poll};

use destream::en::{self, IntoStream};
use futures::ready;
use futures::stream::{Fuse, FusedStream, Stream, StreamExt, TryStreamExt};
use pin_project::pin_project;

use crate::constants::*;

use super::{Encoder, JSONStream};
use futures::task::Context;

#[pin_project]
struct JSONMapEntryStream<'en> {
    #[pin]
    key: Fuse<JSONStream<'en>>,

    #[pin]
    value: Fuse<JSONStream<'en>>,
}

impl<'en> JSONMapEntryStream<'en> {
    fn new<K: IntoStream<'en>, V: IntoStream<'en>>(key: K, value: V) -> Result<Self, super::Error> {
        let key = key.into_stream(Encoder)?;
        let value = value.into_stream(Encoder)?;

        Ok(Self {
            key: key.fuse(),
            value: value.fuse(),
        })
    }
}

impl<'en> Stream for JSONMapEntryStream<'en> {
    type Item = Result<Vec<u8>, super::Error>;

    fn poll_next(self: Pin<&mut Self>, cxt: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        let result = if !this.key.is_terminated() {
            match ready!(this.key.as_mut().poll_next(cxt)) {
                Some(result) => Some(result),
                None => Some(Ok(vec![COLON])),
            }
        } else if !this.value.is_terminated() {
            match ready!(this.value.as_mut().poll_next(cxt)) {
                Some(result) => Some(result),
                None => None,
            }
        } else {
            None
        };

        Poll::Ready(result)
    }
}

impl<'en> FusedStream for JSONMapEntryStream<'en> {
    fn is_terminated(&self) -> bool {
        self.key.is_terminated() && self.value.is_terminated()
    }
}

#[pin_project]
struct JSONEncodingStream<
    I: Stream<Item = Result<Vec<u8>, super::Error>>,
    S: Stream<Item = Result<I, super::Error>>,
> {
    #[pin]
    source: Fuse<S>,

    next: Option<Pin<Box<I>>>,

    started: bool,
    finished: bool,

    start: u8,
    end: u8,
}

impl<
        I: Stream<Item = Result<Vec<u8>, super::Error>>,
        S: Stream<Item = Result<I, super::Error>>,
    > Stream for JSONEncodingStream<I, S>
{
    type Item = Result<Vec<u8>, super::Error>;

    fn poll_next(self: Pin<&mut Self>, cxt: &mut task::Context) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        Poll::Ready(loop {
            match this.next {
                Some(next) => match ready!(next.as_mut().poll_next(cxt)) {
                    Some(result) => break Some(result),
                    None => *this.next = None,
                },
                None => match ready!(this.source.as_mut().poll_next(cxt)) {
                    Some(Ok(next)) => {
                        *this.next = Some(Box::pin(next));

                        if *this.started {
                            break Some(Ok(vec![COMMA]));
                        } else {
                            *this.started = true;
                            break Some(Ok(vec![*this.start]));
                        }
                    }
                    Some(Err(cause)) => break Some(Err(en::Error::custom(cause))),
                    None if !*this.started => {
                        *this.started = true;
                        break Some(Ok(vec![*this.start]));
                    }
                    None if !*this.finished => {
                        *this.finished = true;
                        break Some(Ok(vec![*this.end]));
                    }
                    None => break None,
                },
            }
        })
    }
}

impl<
        I: Stream<Item = Result<Vec<u8>, super::Error>>,
        S: Stream<Item = Result<I, super::Error>>,
    > FusedStream for JSONEncodingStream<I, S>
{
    fn is_terminated(&self) -> bool {
        self.finished
    }
}

pub fn encode_list<'en, I: IntoStream<'en>, S: Stream<Item = Result<I, super::Error>> + 'en>(
    seq: S,
) -> impl Stream<Item = Result<Vec<u8>, super::Error>> + 'en {
    let source = seq
        .map(|result| result.and_then(|element| element.into_stream(Encoder)))
        .map_err(en::Error::custom);

    JSONEncodingStream {
        source: source.fuse(),
        next: None,
        started: false,
        finished: false,
        start: LIST_BEGIN,
        end: LIST_END,
    }
}

pub fn encode_map<
    'en,
    K: IntoStream<'en>,
    V: IntoStream<'en>,
    S: Stream<Item = Result<(K, V), super::Error>> + 'en,
>(
    seq: S,
) -> impl Stream<Item = Result<Vec<u8>, super::Error>> + 'en {
    let source = seq
        .map(|result| result.and_then(|(key, value)| JSONMapEntryStream::new(key, value)))
        .map_err(en::Error::custom);

    JSONEncodingStream {
        source: source.fuse(),
        next: None,
        started: false,
        finished: false,
        start: MAP_BEGIN,
        end: MAP_END,
    }
}

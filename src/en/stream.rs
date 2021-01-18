use std::fmt;
use std::pin::Pin;
use std::task::{self, Poll};

use destream::en::{self, IntoStream};
use futures::ready;
use futures::stream::{Fuse, FusedStream, Stream, StreamExt};
use pin_project::pin_project;

use crate::constants::*;

use super::JSONStream;
use futures::task::Context;

#[pin_project]
pub struct JSONListStream<
    'en,
    E: fmt::Display,
    I: IntoStream<'en> + 'en,
    S: Stream<Item = Result<I, E>> + 'en,
> {
    #[pin]
    source: Fuse<S>,

    next: Option<Result<JSONStream<'en>, super::Error>>,

    finished: bool,
    started: bool,
}

impl<'en, E: fmt::Display, I: IntoStream<'en>, S: Stream<Item = Result<I, E>> + 'en> Stream
    for JSONListStream<'en, E, I, S>
{
    type Item = Result<Vec<u8>, super::Error>;

    fn poll_next(self: Pin<&mut Self>, cxt: &mut task::Context) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        Poll::Ready(loop {
            match this.next {
                Some(Ok(next)) => match ready!(next.as_mut().poll_next(cxt)) {
                    Some(result) => break Some(result),
                    None => *this.next = None,
                },
                Some(Err(cause)) => {
                    let result = Err(en::Error::custom(cause));
                    *this.next = None;
                    break Some(result);
                }
                None => match ready!(this.source.as_mut().poll_next(cxt)) {
                    Some(Ok(value)) => {
                        *this.next = Some(value.into_stream(super::Encoder));

                        if *this.started {
                            break Some(Ok(vec![COMMA]));
                        } else {
                            *this.started = true;
                            break Some(Ok(vec![LIST_BEGIN]));
                        }
                    }
                    Some(Err(cause)) => break Some(Err(en::Error::custom(cause))),
                    None if !*this.started => {
                        *this.started = true;
                        break Some(Ok(vec![LIST_BEGIN]));
                    }
                    None if !*this.finished => {
                        *this.finished = true;
                        break Some(Ok(vec![LIST_END]));
                    }
                    None => break None,
                },
            }
        })
    }
}

impl<'en, E: fmt::Display, I: IntoStream<'en>, S: Stream<Item = Result<I, E>> + 'en> From<S>
    for JSONListStream<'en, E, I, S>
{
    fn from(s: S) -> JSONListStream<'en, E, I, S> {
        JSONListStream {
            source: s.fuse(),
            next: None,
            finished: false,
            started: false,
        }
    }
}

impl<'en, E: fmt::Display, I: IntoStream<'en>, S: Stream<Item = Result<I, E>> + 'en> FusedStream
    for JSONListStream<'en, E, I, S>
{
    fn is_terminated(&self) -> bool {
        self.finished
    }
}

#[pin_project]
struct JSONMapEntryStream<'en> {
    started: bool,

    #[pin]
    key: Fuse<JSONStream<'en>>,

    #[pin]
    value: Fuse<JSONStream<'en>>,
}

impl<'en> JSONMapEntryStream<'en> {
    fn new<K: IntoStream<'en>, V: IntoStream<'en>>(key: K, value: V) -> Result<Self, super::Error> {
        let key = key.into_stream(super::Encoder)?;
        let value = value.into_stream(super::Encoder)?;

        Ok(Self {
            started: false,
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
                Some(result) if !*this.started => {
                    *this.started = true;

                    if let Ok(chunk) = result {
                        if !chunk.starts_with(&[QUOTE]) {
                            Some(Err(en::Error::custom("JSON map key must be a string")))
                        } else {
                            Some(Ok(chunk))
                        }
                    } else {
                        Some(result)
                    }
                }
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
pub struct JSONMapStream<
    'en,
    E: fmt::Display,
    K: IntoStream<'en> + 'en,
    V: IntoStream<'en> + 'en,
    S: Stream<Item = Result<(K, V), E>> + 'en,
> {
    #[pin]
    source: Fuse<S>,

    next: Option<Pin<Box<JSONMapEntryStream<'en>>>>,

    finished: bool,
    started: bool,
}

impl<
        'en,
        E: fmt::Display,
        K: IntoStream<'en>,
        V: IntoStream<'en>,
        S: Stream<Item = Result<(K, V), E>> + 'en,
    > Stream for JSONMapStream<'en, E, K, V, S>
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
                    Some(Ok((key, value))) => {
                        match JSONMapEntryStream::new(key, value) {
                            Ok(next) => *this.next = Some(Box::pin(next)),
                            Err(cause) => break Some(Err(cause)),
                        }

                        if *this.started {
                            break Some(Ok(vec![COMMA]));
                        } else {
                            *this.started = true;
                            break Some(Ok(vec![MAP_BEGIN]));
                        }
                    }
                    Some(Err(cause)) => break Some(Err(en::Error::custom(cause))),
                    None if !*this.started => {
                        *this.started = true;
                        break Some(Ok(vec![MAP_BEGIN]));
                    }
                    None if !*this.finished => {
                        *this.finished = true;
                        break Some(Ok(vec![MAP_END]));
                    }
                    None => break None,
                },
            }
        })
    }
}

impl<
        'en,
        E: fmt::Display,
        K: IntoStream<'en>,
        V: IntoStream<'en>,
        S: Stream<Item = Result<(K, V), E>> + 'en,
    > From<S> for JSONMapStream<'en, E, K, V, S>
{
    fn from(s: S) -> JSONMapStream<'en, E, K, V, S> {
        JSONMapStream {
            source: s.fuse(),
            next: None,
            finished: false,
            started: false,
        }
    }
}

impl<
        'en,
        E: fmt::Display,
        K: IntoStream<'en>,
        V: IntoStream<'en>,
        S: Stream<Item = Result<(K, V), E>> + 'en,
    > FusedStream for JSONMapStream<'en, E, K, V, S>
{
    fn is_terminated(&self) -> bool {
        self.finished
    }
}

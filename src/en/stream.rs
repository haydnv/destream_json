use std::fmt;
use std::mem;
use std::pin::Pin;
use std::task::{self, Poll};

use destream::en::{self, ToStream};
use futures::ready;
use futures::stream::{Fuse, Stream, StreamExt};
use pin_project::pin_project;

use crate::constants::*;

use super::JSONStream;

#[pin_project]
pub struct JSONListStream<
    'en,
    E: fmt::Display,
    I: ToStream<'en> + 'en,
    S: Stream<Item = Result<I, E>> + 'en,
> {
    #[pin]
    source: Fuse<S>,

    started: bool,
    next: Option<Result<JSONStream<'en>, super::Error>>,
}

impl<'en, E: fmt::Display, I: ToStream<'en>, S: Stream<Item = Result<I, E>> + 'en> Stream
    for JSONListStream<'en, E, I, S>
{
    type Item = Result<Vec<u8>, super::Error>;

    fn poll_next(self: Pin<&mut Self>, cxt: &mut task::Context) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        Poll::Ready(loop {
            if let Some(Ok(item_stream)) = this.next {
                match ready!(item_stream.as_mut().poll_next(cxt)) {
                    Some(result) => break Some(result),
                    None => *this.next = None,
                }
            } else if let Some(Err(cause)) = this.next {
                let result = Err(en::Error::custom(cause));
                *this.next = None;
                break Some(result);
            }

            match ready!(this.source.as_mut().poll_next(cxt)) {
                Some(Ok(value)) => {
                    *this.started = true;

                    let mut next = Some(value.into_stream(super::Encoder));
                    mem::swap(this.next, &mut next);
                }
                Some(Err(cause)) => break Some(Err(en::Error::custom(cause))),
                None if !*this.started => {
                    *this.started = true;
                    break Some(Ok(LIST_EMPTY.to_vec()));
                }
                None => break None,
            }
        })
    }
}

impl<'en, E: fmt::Display, I: ToStream<'en>, S: Stream<Item = Result<I, E>> + 'en> From<S>
    for JSONListStream<'en, E, I, S>
{
    fn from(s: S) -> JSONListStream<'en, E, I, S> {
        JSONListStream {
            source: s.fuse(),
            started: false,
            next: None,
        }
    }
}

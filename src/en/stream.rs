use std::fmt;
use std::pin::Pin;
use std::task::{self, Poll};

use destream::en::{self, IntoStream};
use futures::ready;
use futures::stream::{Fuse, FusedStream, Stream, StreamExt};
use pin_project::pin_project;

use crate::constants::*;

use super::JSONStream;

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

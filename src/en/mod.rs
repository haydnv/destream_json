//! Serialize a Rust data structure into JSON data.

use std::collections::VecDeque;
use std::fmt;
use std::mem;
use std::pin::Pin;

use destream::en::{self, IntoStream};
use futures::future;
use futures::stream::{Stream, StreamExt, TryStreamExt};

use crate::constants::*;

mod stream;

pub type JSONStream<'en> = Pin<Box<dyn Stream<Item = Result<Vec<u8>, Error>> + Send + Unpin + 'en>>;

/// An error encountered while encoding a stream.
pub struct Error {
    message: String,
}

impl en::Error for Error {
    fn custom<I: fmt::Display>(info: I) -> Self {
        let message = info.to_string();
        Self { message }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.message)
    }
}

struct MapEncoder<'en> {
    pending_key: Option<JSONStream<'en>>,
    entries: VecDeque<(JSONStream<'en>, JSONStream<'en>)>,
}

impl<'en> MapEncoder<'en> {
    #[inline]
    fn new(size_hint: Option<usize>) -> Self {
        let entries = if let Some(len) = size_hint {
            VecDeque::with_capacity(len)
        } else {
            VecDeque::new()
        };

        Self {
            pending_key: None,
            entries,
        }
    }
}

impl<'en> en::EncodeMap<'en> for MapEncoder<'en> {
    type Ok = JSONStream<'en>;
    type Error = Error;

    #[inline]
    fn encode_key<T: IntoStream<'en> + 'en>(&mut self, key: T) -> Result<(), Self::Error> {
        if self.pending_key.is_none() {
            self.pending_key = Some(key.into_stream(Encoder)?);
            Ok(())
        } else {
            Err(en::Error::custom(
                "You must call encode_value before calling encode_key again",
            ))
        }
    }

    #[inline]
    fn encode_value<T: IntoStream<'en> + 'en>(&mut self, value: T) -> Result<(), Self::Error> {
        if self.pending_key.is_none() {
            return Err(en::Error::custom(
                "You must call encode_key before encode_value",
            ));
        }

        let value = value.into_stream(Encoder)?;

        let mut key = None;
        mem::swap(&mut self.pending_key, &mut key);

        self.entries.push_back((key.unwrap(), value));
        Ok(())
    }

    fn end(mut self) -> Result<Self::Ok, Self::Error> {
        if self.pending_key.is_some() {
            return Err(en::Error::custom(
                "You must call encode_value after calling encode_key",
            ));
        }

        let mut encoded = delimiter(MAP_BEGIN);

        while let Some((key, value)) = self.entries.pop_front() {
            encoded = Box::pin(encoded.chain(key).chain(delimiter(COLON)).chain(value));

            if !self.entries.is_empty() {
                encoded = Box::pin(encoded.chain(delimiter(COMMA)));
            }
        }

        encoded = Box::pin(encoded.chain(delimiter(MAP_END)));
        Ok(encoded)
    }
}

struct SequenceEncoder<'en> {
    items: VecDeque<JSONStream<'en>>,
}

impl<'en> SequenceEncoder<'en> {
    #[inline]
    fn new(size_hint: Option<usize>) -> Self {
        let items = if let Some(len) = size_hint {
            VecDeque::with_capacity(len)
        } else {
            VecDeque::new()
        };

        Self { items }
    }

    #[inline]
    fn push(&mut self, value: JSONStream<'en>) {
        self.items.push_back(value);
    }

    fn encode(mut self) -> Result<JSONStream<'en>, Error> {
        let mut encoded = delimiter(LIST_BEGIN);

        while let Some(item) = self.items.pop_front() {
            encoded = Box::pin(encoded.chain(item));

            if !self.items.is_empty() {
                encoded = Box::pin(encoded.chain(delimiter(COMMA)));
            }
        }

        encoded = Box::pin(encoded.chain(delimiter(LIST_END)));
        Ok(encoded)
    }
}

impl<'en> en::EncodeSeq<'en> for SequenceEncoder<'en> {
    type Ok = JSONStream<'en>;
    type Error = Error;

    #[inline]
    fn encode_element<T: IntoStream<'en> + 'en>(&mut self, value: T) -> Result<(), Self::Error> {
        let encoded = value.into_stream(Encoder)?;
        self.push(encoded);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.encode()
    }
}

impl<'en> en::EncodeTuple<'en> for SequenceEncoder<'en> {
    type Ok = JSONStream<'en>;
    type Error = Error;

    #[inline]
    fn encode_element<T: IntoStream<'en> + 'en>(&mut self, value: T) -> Result<(), Self::Error> {
        let encoded = value.into_stream(Encoder)?;
        self.push(encoded);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.encode()
    }
}

struct Encoder;

impl<'en> en::Encoder<'en> for Encoder {
    type Ok = JSONStream<'en>;
    type Error = Error;
    type EncodeMap = MapEncoder<'en>;
    type EncodeSeq = SequenceEncoder<'en>;
    type EncodeTuple = SequenceEncoder<'en>;

    #[inline]
    fn encode_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    #[inline]
    fn encode_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    #[inline]
    fn encode_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    #[inline]
    fn encode_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    #[inline]
    fn encode_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    #[inline]
    fn encode_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    #[inline]
    fn encode_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    #[inline]
    fn encode_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    #[inline]
    fn encode_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    #[inline]
    fn encode_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    #[inline]
    fn encode_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    #[inline]
    fn encode_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(format!("\"{}\"", v)))
    }

    #[inline]
    fn encode_none(self) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt("null"))
    }

    #[inline]
    fn encode_some<T: IntoStream<'en> + 'en>(self, value: T) -> Result<Self::Ok, Self::Error> {
        value.into_stream(self)
    }

    #[inline]
    fn encode_unit(self) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt("null"))
    }

    #[inline]
    fn encode_map(self, size_hint: Option<usize>) -> Result<Self::EncodeMap, Self::Error> {
        Ok(MapEncoder::new(size_hint))
    }

    #[inline]
    fn encode_map_stream<
        K: IntoStream<'en> + 'en,
        V: IntoStream<'en> + 'en,
        S: Stream<Item = Result<(K, V), Self::Error>> + Send + Unpin + 'en,
    >(
        self,
        map: S,
    ) -> Result<Self::Ok, Self::Error> {
        Ok(Box::pin(stream::encode_map(map)))
    }

    #[inline]
    fn encode_seq(self, size_hint: Option<usize>) -> Result<Self::EncodeSeq, Self::Error> {
        Ok(SequenceEncoder::new(size_hint))
    }

    #[inline]
    fn encode_seq_stream<
        T: IntoStream<'en> + 'en,
        S: Stream<Item = Result<T, Self::Error>> + Send + Unpin + 'en,
    >(
        self,
        seq: S,
    ) -> Result<Self::Ok, Self::Error> {
        Ok(Box::pin(stream::encode_list(seq)))
    }

    #[inline]
    fn encode_tuple(self, len: usize) -> Result<Self::EncodeTuple, Self::Error> {
        Ok(SequenceEncoder::new(Some(len)))
    }
}

#[inline]
fn encode_fmt<'en, T: fmt::Display>(value: T) -> JSONStream<'en> {
    let encoded = value.to_string().as_bytes().to_vec();
    Box::pin(futures::stream::once(future::ready(Ok(encoded))))
}

#[inline]
fn delimiter<'en>(byte: u8) -> JSONStream<'en> {
    let encoded = futures::stream::once(future::ready(Ok(vec![byte])));
    Box::pin(encoded)
}

/// Given an encodable value, return an encoded stream.
pub fn encode<'en, T: IntoStream<'en> + 'en>(
    value: T,
) -> Result<impl Stream<Item = Result<Vec<u8>, Error>> + Send + Unpin + 'en, Error> {
    value.into_stream(Encoder)
}

/// Given a stream of encodable key-value pairs, return a streaming JSON object.
pub fn encode_map<
    'en,
    K: IntoStream<'en> + 'en,
    V: IntoStream<'en> + 'en,
    S: Stream<Item = (K, V)> + Send + Unpin + 'en,
>(
    seq: S,
) -> impl Stream<Item = Result<Vec<u8>, Error>> + Send + Unpin + 'en {
    stream::encode_map(seq.map(Result::<(K, V), Error>::Ok))
}

/// Given a stream of encodable key-value pairs, return a streaming JSON list.
pub fn try_encode_map<
    'en,
    E: fmt::Display + 'en,
    K: IntoStream<'en> + 'en,
    V: IntoStream<'en> + 'en,
    S: Stream<Item = Result<(K, V), E>> + Send + Unpin + 'en,
>(
    seq: S,
) -> impl Stream<Item = Result<Vec<u8>, Error>> + Send + Unpin + 'en {
    Box::pin(stream::encode_map(seq.map_err(en::Error::custom)))
}

/// Given a stream of encodable elements, return a streaming JSON list.
pub fn encode_seq<'en, T: IntoStream<'en> + 'en, S: Stream<Item = T> + Send + Unpin + 'en>(
    seq: S,
) -> impl Stream<Item = Result<Vec<u8>, Error>> + Send + Unpin + 'en {
    stream::encode_list(seq.map(Result::<T, Error>::Ok))
}

/// Given a stream of encodable elements, return a streaming JSON list.
pub fn try_encode_seq<
    'en,
    E: fmt::Display + 'en,
    T: IntoStream<'en> + 'en,
    S: Stream<Item = Result<T, E>> + Send + Unpin + 'en,
>(
    seq: S,
) -> impl Stream<Item = Result<Vec<u8>, Error>> + Send + Unpin + 'en {
    stream::encode_list(seq.map_err(en::Error::custom))
}

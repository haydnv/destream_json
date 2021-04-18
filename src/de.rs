//! Decode a JSON stream to a Rust data structure.

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use async_trait::async_trait;
use bytes::{BufMut, Bytes};
use destream::{de, FromStream, Visitor};
use futures::stream::{Fuse, FusedStream, Stream, StreamExt, TryStreamExt};

#[cfg(feature = "tokio-io")]
use tokio::io::{AsyncRead, AsyncReadExt, BufReader};

use crate::constants::*;

#[async_trait]
pub trait Read: Send + Unpin {
    async fn next(&mut self) -> Option<Result<Bytes, Error>>;

    fn is_terminated(&self) -> bool;
}

pub struct SourceStream<S> {
    source: Fuse<S>,
}

#[async_trait]
impl<S: Stream<Item = Result<Bytes, Error>> + Send + Unpin> Read for SourceStream<S> {
    async fn next(&mut self) -> Option<Result<Bytes, Error>> {
        self.source.next().await
    }

    fn is_terminated(&self) -> bool {
        self.source.is_terminated()
    }
}

impl<S: Stream> From<S> for SourceStream<S> {
    fn from(source: S) -> Self {
        Self {
            source: source.fuse(),
        }
    }
}

#[cfg(feature = "tokio-io")]
pub struct SourceReader<R: AsyncRead> {
    reader: BufReader<R>,
    terminated: bool,
}

#[cfg(feature = "tokio-io")]
#[async_trait]
impl<R: AsyncRead + Send + Unpin> Read for SourceReader<R> {
    async fn next(&mut self) -> Option<Result<Bytes, Error>> {
        let mut chunk = Vec::new();
        match self.reader.read_buf(&mut chunk).await {
            Ok(0) => {
                self.terminated = true;
                Some(Ok(chunk.into()))
            }
            Ok(size) => {
                debug_assert_eq!(chunk.len(), size);
                Some(Ok(chunk.into()))
            }
            Err(cause) => Some(Err(de::Error::custom(format!("io error: {}", cause)))),
        }
    }

    fn is_terminated(&self) -> bool {
        self.terminated
    }
}

#[cfg(feature = "tokio-io")]
impl<R: AsyncRead> From<R> for SourceReader<R> {
    fn from(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
            terminated: false,
        }
    }
}

/// An error encountered while decoding a JSON stream.
pub struct Error {
    message: String,
}

impl Error {
    fn invalid_utf8<I: fmt::Display>(info: I) -> Self {
        de::Error::custom(format!("invalid UTF-8: {}", info))
    }

    fn unexpected_end() -> Self {
        de::Error::custom("unexpected end of stream")
    }
}

impl std::error::Error for Error {}

impl de::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self {
            message: msg.to_string(),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.message, f)
    }
}

struct MapAccess<'a, S> {
    decoder: &'a mut Decoder<S>,
    size_hint: Option<usize>,
    done: bool,
}

impl<'a, S: Read + 'a> MapAccess<'a, S> {
    async fn new(
        decoder: &'a mut Decoder<S>,
        size_hint: Option<usize>,
    ) -> Result<MapAccess<'a, S>, Error> {
        decoder.expect_whitespace().await?;
        decoder.expect_delimiter(MAP_BEGIN).await?;
        decoder.expect_whitespace().await?;

        let done = decoder.maybe_delimiter(MAP_END).await?;

        Ok(MapAccess {
            decoder,
            size_hint,
            done,
        })
    }
}

#[async_trait]
impl<'a, S: Read + 'a> de::MapAccess for MapAccess<'a, S> {
    type Error = Error;

    async fn next_key<K: FromStream>(&mut self, context: K::Context) -> Result<Option<K>, Error> {
        if self.done {
            return Ok(None);
        }

        self.decoder.expect_whitespace().await?;
        let key = K::from_stream(context, self.decoder).await?;

        self.decoder.expect_whitespace().await?;
        self.decoder.expect_delimiter(COLON).await?;
        self.decoder.expect_whitespace().await?;

        Ok(Some(key))
    }

    async fn next_value<V: FromStream>(&mut self, context: V::Context) -> Result<V, Error> {
        let value = V::from_stream(context, self.decoder).await?;
        self.decoder.expect_whitespace().await?;

        if self.decoder.maybe_delimiter(MAP_END).await? {
            self.done = true;
        } else {
            self.decoder.expect_delimiter(COMMA).await?;
        }

        Ok(value)
    }

    fn size_hint(&self) -> Option<usize> {
        self.size_hint
    }
}

struct SeqAccess<'a, S> {
    decoder: &'a mut Decoder<S>,
    size_hint: Option<usize>,
    done: bool,
}

impl<'a, S: Read + 'a> SeqAccess<'a, S> {
    async fn new(
        decoder: &'a mut Decoder<S>,
        size_hint: Option<usize>,
    ) -> Result<SeqAccess<'a, S>, Error> {
        decoder.expect_whitespace().await?;
        decoder.expect_delimiter(LIST_BEGIN).await?;
        decoder.expect_whitespace().await?;

        let done = decoder.maybe_delimiter(LIST_END).await?;

        Ok(SeqAccess {
            decoder,
            size_hint,
            done,
        })
    }
}

#[async_trait]
impl<'a, S: Read + 'a> de::SeqAccess for SeqAccess<'a, S> {
    type Error = Error;

    async fn next_element<T: FromStream>(
        &mut self,
        context: T::Context,
    ) -> Result<Option<T>, Self::Error> {
        if self.done {
            return Ok(None);
        }

        self.decoder.expect_whitespace().await?;
        let value = T::from_stream(context, self.decoder).await?;
        self.decoder.expect_whitespace().await?;

        if self.decoder.maybe_delimiter(LIST_END).await? {
            self.done = true;
        } else {
            self.decoder.expect_delimiter(COMMA).await?;
        }

        Ok(Some(value))
    }

    fn size_hint(&self) -> Option<usize> {
        self.size_hint
    }
}

/// A structure that decodes Rust values from a JSON stream.
pub struct Decoder<S> {
    source: S,
    buffer: Vec<u8>,
    numeric: HashSet<u8>,
}

#[cfg(feature = "tokio-io")]
impl<A: AsyncRead> Decoder<A>
where
    SourceReader<A>: Read,
{
    pub fn from_reader(reader: A) -> Decoder<SourceReader<A>> {
        Decoder {
            source: SourceReader::from(reader),
            buffer: Vec::new(),
            numeric: NUMERIC.iter().cloned().collect(),
        }
    }
}

impl<S: Stream> Decoder<SourceStream<S>>
where
    SourceStream<S>: Read,
{
    pub fn from_stream(stream: S) -> Decoder<SourceStream<S>> {
        Decoder {
            source: SourceStream::from(stream),
            buffer: Vec::new(),
            numeric: NUMERIC.iter().cloned().collect(),
        }
    }
}

impl<S: Read> Decoder<S> {
    async fn buffer(&mut self) -> Result<(), Error> {
        if let Some(data) = self.source.next().await {
            self.buffer.extend(data?);
        }

        Ok(())
    }

    async fn buffer_string(&mut self) -> Result<Vec<u8>, Error> {
        self.expect_delimiter(QUOTE).await?;

        let mut i = 0;
        let mut escaped = false;
        loop {
            while i >= self.buffer.len() && !self.source.is_terminated() {
                self.buffer().await?;
            }

            if i < self.buffer.len() && &self.buffer[i..i + 1] == QUOTE && !escaped {
                break;
            } else if self.source.is_terminated() {
                return Err(Error::unexpected_end());
            }

            if escaped {
                escaped = false;
            } else if self.buffer[i] == ESCAPE[0] {
                escaped = true;
            }

            i += 1;
        }

        let mut s = Vec::with_capacity(i);
        let mut escape = false;
        for byte in self.buffer.drain(0..i) {
            let as_slice = std::slice::from_ref(&byte);
            if escape {
                s.put_u8(byte);
                escape = false;
            } else if as_slice == ESCAPE {
                escape = true;
            } else {
                s.put_u8(byte);
            }
        }

        self.buffer.remove(0); // process the end quote
        self.buffer.shrink_to_fit();
        Ok(s)
    }

    async fn buffer_while<F: Fn(u8) -> bool>(&mut self, cond: F) -> Result<usize, Error> {
        let mut i = 0;
        loop {
            while i >= self.buffer.len() && !self.source.is_terminated() {
                self.buffer().await?;
            }

            if i < self.buffer.len() && cond(self.buffer[i]) {
                i += 1;
            } else if self.source.is_terminated() {
                return Ok(i);
            } else {
                break;
            }
        }

        Ok(i)
    }

    async fn decode_number<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Error> {
        let mut i = 0;
        loop {
            if self.buffer[i] == DECIMAL[0] {
                return de::Decoder::decode_f64(self, visitor).await;
            } else if !self.numeric.contains(&self.buffer[i]) {
                return de::Decoder::decode_i64(self, visitor).await;
            }

            i += 1;
            while i >= self.buffer.len() && !self.source.is_terminated() {
                self.buffer().await?;
            }

            if self.source.is_terminated() {
                return de::Decoder::decode_i64(self, visitor).await;
            }
        }
    }

    async fn expect_delimiter(&mut self, delimiter: &'static [u8]) -> Result<(), Error> {
        while self.buffer.is_empty() && !self.source.is_terminated() {
            self.buffer().await?;
        }

        if self.buffer.is_empty() {
            return Err(Error::unexpected_end());
        }

        if &self.buffer[0..1] == delimiter {
            self.buffer.remove(0);
            Ok(())
        } else {
            Err(de::Error::invalid_value(
                self.buffer[0] as char,
                &format!("{}", String::from_utf8(delimiter.to_vec()).unwrap()),
            ))
        }
    }

    async fn expect_whitespace(&mut self) -> Result<(), Error> {
        let i = self.buffer_while(|b| (b as char).is_whitespace()).await?;
        self.buffer.drain(..i);
        Ok(())
    }

    async fn ignore_value(&mut self) -> Result<(), Error> {
        self.expect_whitespace().await?;

        while self.buffer.is_empty() && !self.source.is_terminated() {
            self.buffer().await?;
        }

        if self.buffer.is_empty() {
            Ok(())
        } else {
            if self.buffer.starts_with(QUOTE) {
                self.parse_string().await?;
            } else if self.numeric.contains(&self.buffer[0]) {
                self.parse_number::<f64>().await?;
            } else if self.buffer[0] == b'n' {
                self.parse_unit().await?;
            } else {
                self.parse_bool().await?;
            }

            Ok(())
        }
    }

    async fn maybe_delimiter(&mut self, delimiter: &'static [u8]) -> Result<bool, Error> {
        while self.buffer.is_empty() && !self.source.is_terminated() {
            self.buffer().await?;
        }

        if self.buffer.is_empty() {
            Ok(false)
        } else if self.buffer.starts_with(delimiter) {
            self.buffer.remove(0);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn parse_bool(&mut self) -> Result<bool, Error> {
        self.expect_whitespace().await?;

        while self.buffer.len() < 4 && !self.source.is_terminated() {
            self.buffer().await?;
        }

        if self.buffer.is_empty() {
            return Err(Error::unexpected_end());
        } else if self.buffer.starts_with(TRUE) {
            self.buffer.drain(0..4);
            return Ok(true);
        }

        while self.buffer.len() < 5 && !self.source.is_terminated() {
            self.buffer().await?;
        }

        if self.buffer.is_empty() {
            return Err(Error::unexpected_end());
        } else if self.buffer.starts_with(FALSE) {
            self.buffer.drain(0..5);
            return Ok(false);
        }

        let i = Ord::min(self.buffer.len(), 5);
        let unknown = String::from_utf8(self.buffer[..i].to_vec()).map_err(Error::invalid_utf8)?;
        Err(de::Error::invalid_value(unknown, &"a boolean"))
    }

    async fn parse_number<N: FromStr>(&mut self) -> Result<N, Error>
    where
        <N as FromStr>::Err: fmt::Display,
    {
        self.expect_whitespace().await?;

        let numeric = self.numeric.clone();
        let i = self.buffer_while(|b| numeric.contains(&b)).await?;
        let n = String::from_utf8(self.buffer[0..i].to_vec()).map_err(Error::invalid_utf8)?;

        match n.parse() {
            Ok(number) => {
                self.buffer.drain(..i);
                Ok(number)
            }
            Err(cause) => Err(de::Error::invalid_value(cause, &std::any::type_name::<N>())),
        }
    }

    async fn parse_string(&mut self) -> Result<String, Error> {
        let s = self.buffer_string().await?;
        String::from_utf8(s).map_err(Error::invalid_utf8)
    }

    async fn parse_unit(&mut self) -> Result<(), Error> {
        self.expect_whitespace().await?;

        while self.buffer.len() < 4 && !self.source.is_terminated() {
            self.buffer().await?;
        }

        if self.buffer.starts_with(NULL) {
            self.buffer.drain(..NULL.len());
            Ok(())
        } else {
            let i = Ord::min(self.buffer.len(), 5);
            let as_str =
                String::from_utf8(self.buffer[..i].to_vec()).map_err(Error::invalid_utf8)?;

            Err(de::Error::invalid_type(as_str, &"null"))
        }
    }
}

#[async_trait]
impl<S: Read> de::Decoder for Decoder<S> {
    type Error = Error;

    async fn decode_any<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        self.expect_whitespace().await?;

        while self.buffer.is_empty() && !self.source.is_terminated() {
            self.buffer().await?;
        }

        if self.buffer.is_empty() {
            Err(Error::unexpected_end())
        } else if self.buffer.starts_with(QUOTE) {
            self.decode_string(visitor).await
        } else if self.buffer.starts_with(LIST_BEGIN) {
            self.decode_seq(visitor).await
        } else if self.buffer.starts_with(MAP_BEGIN) {
            self.decode_map(visitor).await
        } else if self.numeric.contains(&self.buffer[0]) {
            self.decode_number(visitor).await
        } else if self.buffer.len() >= 5 && self.buffer.starts_with(FALSE) {
            self.decode_bool(visitor).await
        } else if self.buffer.len() >= 4 && self.buffer.starts_with(TRUE) {
            self.decode_bool(visitor).await
        } else if self.buffer.len() >= 4 && self.buffer.starts_with(NULL) {
            self.decode_option(visitor).await
        } else {
            while self.buffer.len() < 4 && !self.source.is_terminated() {
                self.buffer().await?;
            }

            if self.buffer.is_empty() {
                Err(Error::unexpected_end())
            } else if self.buffer.starts_with(TRUE) {
                self.decode_bool(visitor).await
            } else if self.buffer.starts_with(NULL) {
                self.decode_option(visitor).await
            } else {
                while self.buffer.len() < 5 && !self.source.is_terminated() {
                    self.buffer().await?;
                }

                if self.buffer.is_empty() {
                    Err(Error::unexpected_end())
                } else if self.buffer.starts_with(FALSE) {
                    self.decode_bool(visitor).await
                } else {
                    let i = Ord::min(self.buffer.len(), 5);
                    let s = String::from_utf8(self.buffer[0..i].to_vec())
                        .map_err(Error::invalid_utf8)?;

                    Err(de::Error::invalid_value(
                        s,
                        &std::any::type_name::<V::Value>(),
                    ))
                }
            }
        }
    }

    async fn decode_bool<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let b = self.parse_bool().await?;
        visitor.visit_bool(b)
    }

    async fn decode_i8<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let i = self.parse_number().await?;
        visitor.visit_i8(i)
    }

    async fn decode_i16<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let i = self.parse_number().await?;
        visitor.visit_i16(i)
    }

    async fn decode_i32<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let i = self.parse_number().await?;
        visitor.visit_i32(i)
    }

    async fn decode_i64<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let i = self.parse_number().await?;
        visitor.visit_i64(i)
    }

    async fn decode_u8<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let u = self.parse_number().await?;
        visitor.visit_u8(u)
    }

    async fn decode_u16<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let u = self.parse_number().await?;
        visitor.visit_u16(u)
    }

    async fn decode_u32<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let u = self.parse_number().await?;
        visitor.visit_u32(u)
    }

    async fn decode_u64<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let u = self.parse_number().await?;
        visitor.visit_u64(u)
    }

    async fn decode_f32<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let f = self.parse_number().await?;
        visitor.visit_f32(f)
    }

    async fn decode_f64<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let f = self.parse_number().await?;
        visitor.visit_f64(f)
    }

    async fn decode_string<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        self.expect_whitespace().await?;

        let s = self.parse_string().await?;
        visitor.visit_string(s)
    }

    async fn decode_byte_buf<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let encoded = self.parse_string().await?;
        let decoded = base64::decode(encoded).map_err(de::Error::custom)?;
        visitor.visit_byte_buf(decoded)
    }

    async fn decode_option<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        self.expect_whitespace().await?;

        while self.buffer.len() < 4 && !self.source.is_terminated() {
            self.buffer().await?;
        }

        if self.buffer.starts_with(NULL) {
            self.buffer.drain(0..4);
            visitor.visit_none()
        } else {
            visitor.visit_some(self).await
        }
    }

    async fn decode_seq<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let access = SeqAccess::new(self, None).await?;
        visitor.visit_seq(access).await
    }

    async fn decode_unit<V: Visitor>(
        &mut self,
        visitor: V,
    ) -> Result<<V as Visitor>::Value, Self::Error> {
        self.parse_unit().await?;
        visitor.visit_unit()
    }

    async fn decode_tuple<V: Visitor>(
        &mut self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        let access = SeqAccess::new(self, Some(len)).await?;
        visitor.visit_seq(access).await
    }

    async fn decode_map<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let access = MapAccess::new(self, None).await?;
        visitor.visit_map(access).await
    }

    async fn decode_ignored_any<V: Visitor>(
        &mut self,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.ignore_value().await?;
        visitor.visit_unit()
    }
}

impl<S: Read> From<S> for Decoder<S> {
    fn from(source: S) -> Self {
        Self {
            source,
            buffer: vec![],
            numeric: NUMERIC.iter().cloned().collect(),
        }
    }
}

/// Decode the given JSON-encoded stream of bytes into an instance of `T` using the given context.
pub async fn decode<S: Stream<Item = Bytes> + Send + Unpin, T: FromStream>(
    context: T::Context,
    source: S,
) -> Result<T, Error> {
    let source = source.map(Result::<Bytes, Error>::Ok);
    T::from_stream(context, &mut Decoder::from(SourceStream::from(source))).await
}

/// Decode the given JSON-encoded stream of bytes into an instance of `T` using the given context.
pub async fn try_decode<
    E: fmt::Display,
    S: Stream<Item = Result<Bytes, E>> + Send + Unpin,
    T: FromStream,
>(
    context: T::Context,
    source: S,
) -> Result<T, Error> {
    let mut decoder = Decoder::from_stream(source.map_err(|e| de::Error::custom(e)));
    T::from_stream(context, &mut decoder).await
}

/// Decode the given JSON-encoded stream of bytes into an instance of `T` using the given context.
#[cfg(feature = "tokio-io")]
/// Decode the given JSON-encoded stream of bytes into an instance of `T` using the given context.
pub async fn read_from<S: AsyncReadExt + Send + Unpin, T: FromStream>(
    context: T::Context,
    source: S,
) -> Result<T, Error> {
    T::from_stream(context, &mut Decoder::from(SourceReader::from(source))).await
}

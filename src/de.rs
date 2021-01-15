//! Decode a JSON stream to a Rust data structure.

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use async_trait::async_trait;
use destream::{de, FromStream, Visitor};
use futures::stream::{Fuse, Stream, StreamExt};

use crate::constants::*;

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
}

impl<'a, S: Stream<Item = Vec<u8>> + Send + Unpin + 'a> MapAccess<'a, S> {
    async fn new(
        decoder: &'a mut Decoder<S>,
        size_hint: Option<usize>,
    ) -> Result<MapAccess<'a, S>, Error> {
        decoder.expect_whitespace().await;
        decoder.expect_byte(MAP_BEGIN).await?;

        Ok(MapAccess { decoder, size_hint })
    }
}

#[async_trait]
impl<'a, S: Stream<Item = Vec<u8>> + Send + Unpin + 'a> de::MapAccess for MapAccess<'a, S> {
    type Error = Error;

    async fn next_key<K: FromStream>(&mut self) -> Result<Option<K>, Error> {
        self.decoder.expect_whitespace().await;

        if self.decoder.maybe_byte(MAP_END).await? {
            return Ok(None);
        }

        let key = K::from_stream(self.decoder).await?;
        Ok(Some(key))
    }

    async fn next_value<V: FromStream>(&mut self) -> Result<V, Error> {
        self.decoder.expect_whitespace().await;
        self.decoder.expect_byte(COLON).await?;
        self.decoder.expect_whitespace().await;

        let value = V::from_stream(self.decoder).await?;

        self.decoder.expect_comma_or(MAP_END).await?;

        Ok(value)
    }

    async fn next_entry<K: FromStream, V: FromStream>(&mut self) -> Result<Option<(K, V)>, Error> {
        if let Some(key) = self.next_key().await? {
            let value = self.next_value().await?;
            Ok(Some((key, value)))
        } else {
            Ok(None)
        }
    }

    fn size_hint(&self) -> Option<usize> {
        self.size_hint
    }
}

struct SeqAccess<'a, S> {
    decoder: &'a mut Decoder<S>,
    size_hint: Option<usize>,
}

impl<'a, S: Stream<Item = Vec<u8>> + Send + Unpin + 'a> SeqAccess<'a, S> {
    async fn new(
        decoder: &'a mut Decoder<S>,
        size_hint: Option<usize>,
    ) -> Result<SeqAccess<'a, S>, Error> {
        decoder.expect_whitespace().await;
        decoder.expect_byte(LIST_BEGIN).await?;

        Ok(SeqAccess { decoder, size_hint })
    }
}

#[async_trait]
impl<'a, S: Stream<Item = Vec<u8>> + Send + Unpin + 'a> de::SeqAccess for SeqAccess<'a, S> {
    type Error = Error;

    async fn next_element<T: FromStream>(&mut self) -> Result<Option<T>, Self::Error> {
        self.decoder.expect_whitespace().await;

        if self.decoder.maybe_byte(LIST_END).await? {
            return Ok(None);
        }

        let value = T::from_stream(self.decoder).await?;
        self.decoder.expect_comma_or(LIST_END).await?;
        Ok(Some(value))
    }

    fn size_hint(&self) -> Option<usize> {
        self.size_hint
    }
}

/// A structure that decodes Rust values from a JSON stream.
pub struct Decoder<S> {
    source: Fuse<S>,
    buffer: Vec<u8>,
    numeric: HashSet<u8>,
}

impl<S: Stream<Item = Vec<u8>> + Send + Unpin> Decoder<S> {
    async fn buffer(&mut self) {
        if let Some(data) = self.source.next().await {
            self.buffer.extend(data);
        }
    }

    async fn buffer_string(&mut self) -> Result<Vec<u8>, Error> {
        self.expect_byte(QUOTE).await?;

        let mut i = 0;
        loop {
            while i >= self.buffer.len() && !self.source.is_done() {
                self.buffer().await;
            }

            if i < self.buffer.len()
                && self.buffer[i] == QUOTE
                && (i == 0 || self.buffer[i - 1] != ESCAPE)
            {
                break;
            } else if self.source.is_done() {
                return Err(Error::unexpected_end());
            } else {
                i += 1;
            }
        }

        let s = self.buffer.drain(0..i).collect();
        self.buffer.remove(0); // process the end quote
        self.buffer.shrink_to_fit();
        Ok(s)
    }

    async fn buffer_while<F: Fn(u8) -> bool>(&mut self, cond: F) -> Vec<u8> {
        let mut i = 0;
        loop {
            while i >= self.buffer.len() && !self.source.is_done() {
                self.buffer().await;
            }

            if i < self.buffer.len() && cond(self.buffer[i]) {
                i += 1;
            } else if self.source.is_done() {
                return self.buffer.drain(..).collect();
            } else {
                break;
            }
        }

        let buffered = self.buffer.drain(0..i).collect();
        self.buffer.shrink_to_fit();
        buffered
    }

    async fn decode_number<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Error> {
        let mut i = 0;
        loop {
            if self.buffer[i] == DECIMAL {
                return de::Decoder::decode_f64(self, visitor).await;
            } else if !self.numeric.contains(&self.buffer[i]) {
                return de::Decoder::decode_i64(self, visitor).await;
            }

            i += 1;
            while i >= self.buffer.len() && !self.source.is_done() {
                self.buffer().await;
            }

            if self.source.is_done() {
                return de::Decoder::decode_i64(self, visitor).await;
            }
        }
    }

    async fn expect_byte(&mut self, byte: u8) -> Result<(), Error> {
        while self.buffer.is_empty() && !self.source.is_done() {
            self.buffer().await;
        }

        if self.buffer.is_empty() {
            return Err(Error::unexpected_end());
        }

        let next_char = self.buffer.remove(0);
        if next_char == byte {
            Ok(())
        } else {
            Err(de::Error::invalid_value(
                next_char as char,
                &format!("{}", (byte as char)),
            ))
        }
    }

    async fn expect_comma_or(&mut self, byte: u8) -> Result<(), Error> {
        self.expect_whitespace().await;

        while self.buffer.is_empty() && !self.source.is_done() {
            self.buffer().await;
        }

        if self.buffer.is_empty() {
            Err(Error::unexpected_end())
        } else if self.buffer[0] == byte {
            Ok(())
        } else {
            self.expect_byte(COMMA).await
        }
    }

    async fn expect_whitespace(&mut self) {
        self.buffer_while(|b| (b as char).is_whitespace()).await;
    }

    async fn ignore_value(&mut self) -> Result<(), Error> {
        self.expect_whitespace().await;

        while self.buffer.is_empty() && !self.source.is_done() {
            self.buffer().await;
        }

        if self.buffer.is_empty() {
            Ok(())
        } else {
            if self.buffer[0] == QUOTE {
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

    async fn maybe_byte(&mut self, byte: u8) -> Result<bool, Error> {
        while self.buffer.is_empty() && !self.source.is_done() {
            self.buffer().await;
        }

        if self.buffer.is_empty() {
            Ok(false)
        } else if self.buffer[0] == byte {
            self.buffer.remove(0);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn parse_bool(&mut self) -> Result<bool, Error> {
        self.expect_whitespace().await;

        while self.buffer.len() < 4 && !self.source.is_done() {
            self.buffer().await;
        }

        if self.buffer.is_empty() {
            return Err(Error::unexpected_end());
        } else if self.buffer.starts_with(TRUE) {
            self.buffer.drain(0..4);
            return Ok(true);
        }

        while self.buffer.len() < 5 && !self.source.is_done() {
            self.buffer().await;
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
        self.expect_whitespace().await;

        let numeric = self.numeric.clone();
        let n = self.buffer_while(|b| numeric.contains(&b)).await;
        let n = String::from_utf8(n).map_err(Error::invalid_utf8)?;

        n.parse()
            .map_err(|e| de::Error::invalid_value(e, &std::any::type_name::<N>()))
    }

    async fn parse_string(&mut self) -> Result<String, Error> {
        let s = self.buffer_string().await?;
        String::from_utf8(s).map_err(Error::invalid_utf8)
    }

    async fn parse_unit(&mut self) -> Result<(), Error> {
        self.expect_whitespace().await;

        while self.buffer.len() < 4 && !self.source.is_done() {
            self.buffer().await;
        }

        if self.buffer.starts_with(NULL) {
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
impl<S: Stream<Item = Vec<u8>> + Send + Unpin> de::Decoder for Decoder<S> {
    type Error = Error;

    async fn decode_any<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        self.expect_whitespace().await;

        while self.buffer.is_empty() && !self.source.is_done() {
            self.buffer().await;
        }

        if self.buffer.is_empty() {
            Err(Error::unexpected_end())
        } else if self.buffer[0] == QUOTE {
            self.decode_string(visitor).await
        } else if self.buffer[0] == MAP_BEGIN {
            self.decode_map(visitor).await
        } else if self.buffer[0] == LIST_BEGIN {
            self.decode_seq(visitor).await
        } else if self.numeric.contains(&self.buffer[0]) {
            self.decode_number(visitor).await
        } else if self.buffer.len() >= 5 && self.buffer.starts_with(FALSE) {
            self.decode_bool(visitor).await
        } else if self.buffer.len() >= 4 && self.buffer.starts_with(TRUE) {
            self.decode_bool(visitor).await
        } else if self.buffer.len() >= 4 && self.buffer.starts_with(NULL) {
            self.decode_option(visitor).await
        } else {
            while self.buffer.len() < 4 && !self.source.is_done() {
                self.buffer().await;
            }

            if self.buffer.is_empty() {
                Err(Error::unexpected_end())
            } else if self.buffer.starts_with(TRUE) {
                self.decode_bool(visitor).await
            } else if self.buffer.starts_with(NULL) {
                self.decode_option(visitor).await
            } else {
                while self.buffer.len() < 5 && !self.source.is_done() {
                    self.buffer().await;
                }

                if self.buffer.is_empty() {
                    Err(Error::unexpected_end())
                } else if self.buffer.starts_with(FALSE) {
                    self.decode_bool(visitor).await
                } else {
                    let s = String::from_utf8(self.buffer[0..5].to_vec())
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
        self.expect_whitespace().await;

        let s = self.parse_string().await?;
        visitor.visit_string(s)
    }

    async fn decode_byte_buf<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let encoded = self.parse_string().await?;
        let decoded = base64::decode(encoded).map_err(de::Error::custom)?;
        visitor.visit_byte_buf(decoded)
    }

    async fn decode_option<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        self.expect_whitespace().await;

        while self.buffer.len() < 4 && !self.source.is_done() {
            self.buffer().await;
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

    async fn decode_identifier<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        self.decode_string(visitor).await
    }

    async fn decode_ignored_any<V: Visitor>(
        &mut self,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.ignore_value().await?;
        visitor.visit_unit()
    }
}

impl<S: Stream> From<S> for Decoder<S> {
    fn from(source: S) -> Self {
        Self {
            source: source.fuse(),
            buffer: vec![],
            numeric: NUMERIC.iter().cloned().collect(),
        }
    }
}

/// Decode the given JSON-encoded stream of bytes into an instance of `T`.
pub async fn from_stream<S: Stream<Item = Vec<u8>> + Send + Unpin, T: FromStream>(
    source: S,
) -> Result<T, Error> {
    T::from_stream(&mut Decoder::from(source)).await
}

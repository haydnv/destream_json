//! Library for decoding and encoding JSON streams.

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use async_trait::async_trait;
use destream::{de, FromStream, Visitor};
use futures::stream::{Stream, StreamExt};

const COLON: u8 = b':';
const COMMA: u8 = b',';
const DECIMAL: u8 = b'.';
const ESCAPE: u8 = b'\\';
const FALSE: &[u8] = b"false";
const TRUE: &[u8] = b"true";
const LIST_BEGIN: u8 = b'[';
const LIST_END: u8 = b']';
const NULL: &[u8] = b"null";
const MAP_BEGIN: u8 = b'{';
const MAP_END: u8 = b'}';
const NUMERIC: [u8; 15] = [
    b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'0', b'-', b'e', b'E', DECIMAL,
];
const QUOTE: u8 = b'"';

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
        decoder.expect_byte(MAP_BEGIN).await?;

        Ok(MapAccess { decoder, size_hint })
    }
}

#[async_trait]
impl<'a, S: Stream<Item = Vec<u8>> + Send + Unpin + 'a> de::MapAccess for MapAccess<'a, S> {
    type Error = Error;

    async fn next_key<K: FromStream>(&mut self) -> Result<Option<K>, Error> {
        self.decoder.expect_whitespace().await?;

        if self.decoder.maybe_byte(MAP_END).await? {
            return Ok(None);
        }

        let key = K::from_stream(self.decoder).await?;
        Ok(Some(key))
    }

    async fn next_value<V: FromStream>(&mut self) -> Result<V, Error> {
        self.decoder.expect_whitespace().await?;
        self.decoder.expect_byte(COLON).await?;
        self.decoder.expect_whitespace().await?;

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
        decoder.expect_byte(LIST_BEGIN).await?;

        Ok(SeqAccess { decoder, size_hint })
    }
}

#[async_trait]
impl<'a, S: Stream<Item = Vec<u8>> + Send + Unpin + 'a> de::SeqAccess for SeqAccess<'a, S> {
    type Error = Error;

    async fn next_element<T: FromStream>(&mut self) -> Result<Option<T>, Self::Error> {
        self.decoder.expect_whitespace().await?;

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

pub struct Decoder<S> {
    source: S,
    buffer: Vec<u8>,
    numeric: HashSet<u8>,
}

impl<S: Stream<Item = Vec<u8>> + Send + Unpin> Decoder<S> {
    async fn buffer(&mut self) -> Result<(), Error> {
        if let Some(data) = self.source.next().await {
            self.buffer.extend(data);
            Ok(())
        } else {
            Err(Error::unexpected_end())
        }
    }

    async fn buffer_string(&mut self) -> Result<Vec<u8>, Error> {
        self.expect_byte(QUOTE).await?;

        let mut i = 0;
        loop {
            while self.buffer.is_empty() {
                self.buffer().await?;
            }

            if self.buffer[i] == QUOTE && (i == 0 || self.buffer[i - 1] != ESCAPE) {
                break;
            } else {
                i += 1;
            }
        }

        let s = self.buffer.drain(0..i).collect();
        self.buffer.remove(0);
        Ok(s)
    }

    async fn buffer_while<F: Fn(u8) -> bool>(&mut self, cond: F) -> Result<Vec<u8>, Error> {
        let mut i = 0;
        loop {
            while self.buffer.is_empty() {
                self.buffer().await?;
            }

            if cond(self.buffer[i]) {
                i += 1;
            } else {
                break;
            }
        }

        Ok(self.buffer.drain(0..i).collect())
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
            while i >= self.buffer.len() {
                self.buffer().await?;
            }
        }
    }

    async fn expect_byte(&mut self, byte: u8) -> Result<(), Error> {
        while self.buffer.is_empty() {
            self.buffer().await?;
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
        self.expect_whitespace().await?;

        while self.buffer.is_empty() {
            self.buffer().await?;
        }

        if self.buffer[0] != byte {
            self.expect_byte(COMMA).await?;
        }

        Ok(())
    }

    async fn expect_whitespace(&mut self) -> Result<(), Error> {
        self.buffer_while(|b| (b as char).is_whitespace()).await?;
        Ok(())
    }

    async fn maybe_byte(&mut self, byte: u8) -> Result<bool, Error> {
        while self.buffer.is_empty() {
            self.buffer().await?;
        }

        if self.buffer[0] == byte {
            self.buffer.remove(0);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn parse_number<N: FromStr>(&mut self) -> Result<N, Error>
    where
        <N as FromStr>::Err: fmt::Display,
    {
        let numeric = self.numeric.clone();
        let n = self.buffer_while(|b| numeric.contains(&b)).await?;
        let n = String::from_utf8(n).map_err(Error::invalid_utf8)?;

        n.parse()
            .map_err(|e| de::Error::invalid_value(e, &std::any::type_name::<N>()))
    }

    async fn parse_string(&mut self) -> Result<String, Error> {
        let s = self.buffer_string().await?;
        String::from_utf8(s).map_err(Error::invalid_utf8)
    }
}

#[async_trait]
impl<S: Stream<Item = Vec<u8>> + Send + Unpin> de::Decoder for Decoder<S> {
    type Error = Error;

    async fn decode_any<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        while self.buffer.is_empty() {
            self.buffer().await?;
        }

        if self.buffer[0] == QUOTE {
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
            while self.buffer.len() < 4 {
                self.buffer().await?;
            }

            if self.buffer.starts_with(TRUE) {
                self.decode_bool(visitor).await
            } else if self.buffer.starts_with(NULL) {
                self.decode_option(visitor).await
            } else {
                while self.buffer.len() < 5 {
                    self.buffer().await?;
                }

                if self.buffer.starts_with(FALSE) {
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
        while self.buffer.len() < 4 {
            self.buffer().await?;
        }

        if self.buffer.starts_with(TRUE) {
            self.buffer.drain(0..4);
            return visitor.visit_bool(true);
        }

        while self.buffer.len() < 5 {
            self.buffer().await?;
        }

        if self.buffer.starts_with(FALSE) {
            self.buffer.drain(0..5);
            return visitor.visit_bool(false);
        }

        let unknown = String::from_utf8(self.buffer[0..5].to_vec()).map_err(Error::invalid_utf8)?;

        Err(de::Error::invalid_value(unknown, &"a boolean"))
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
        let s = self.parse_string().await?;
        visitor.visit_string(s)
    }

    async fn decode_byte_buf<V: Visitor>(&mut self, _visitor: V) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }

    async fn decode_option<V: Visitor>(&mut self, _visitor: V) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }

    async fn decode_seq<V: Visitor>(&mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let access = SeqAccess::new(self, None).await?;
        visitor.visit_seq(access).await
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

    async fn decode_identifier<V: Visitor>(
        &mut self,
        _visitor: V,
    ) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }

    async fn decode_ignored_any<V: Visitor>(
        &mut self,
        _visitor: V,
    ) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }
}

impl<S> From<S> for Decoder<S> {
    fn from(source: S) -> Self {
        Self {
            source,
            buffer: vec![],
            numeric: NUMERIC.iter().cloned().collect(),
        }
    }
}

//! Library for decoding and encoding JSON streams.

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use async_trait::async_trait;
use destream::{de, Visitor};
use futures::{Stream, StreamExt};

const DECIMAL: u8 = b'.';
const FALSE: &[u8] = b"false";
const TRUE: &[u8] = b"true";
const LIST_BEGIN: u8 = b'[';
const NULL: &[u8] = b"null";
const MAP_BEGIN: u8 = b'{';
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

    async fn decode_number<V: Visitor>(mut self, visitor: V) -> Result<V::Value, Error> {
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

    async fn parse_int<I: FromStr>(&mut self) -> Result<I, Error>
    where
        <I as FromStr>::Err: fmt::Display,
    {
        let mut i = 0;
        loop {
            while i == self.buffer.len() {
                self.buffer().await?;
            }

            if self.numeric.contains(&self.buffer[i]) && self.buffer[i] != DECIMAL {
                i += 1;
            } else {
                break;
            }
        }

        let n =
            String::from_utf8(self.buffer.drain(0..i).collect()).map_err(Error::invalid_utf8)?;

        n.parse()
            .map_err(|e| de::Error::invalid_value(e, "8-bit integer"))
    }
}

#[async_trait]
impl<S: Stream<Item = Vec<u8>> + Send + Unpin> de::Decoder for Decoder<S> {
    type Error = Error;

    async fn decode_any<V: Visitor>(mut self, visitor: V) -> Result<V::Value, Self::Error> {
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
                    let s = String::from_utf8(self.buffer.into()).map_err(Error::invalid_utf8)?;
                    Err(de::Error::invalid_value(
                        s,
                        std::any::type_name::<V::Value>(),
                    ))
                }
            }
        }
    }

    async fn decode_bool<V: Visitor>(mut self, visitor: V) -> Result<V::Value, Self::Error> {
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

        let unknown =
            String::from_utf8(self.buffer.drain(0..5).collect()).map_err(Error::invalid_utf8)?;

        Err(de::Error::invalid_value(unknown, "a boolean"))
    }

    async fn decode_i8<V: Visitor>(mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let i = self.parse_int().await?;
        visitor.visit_i8(i)
    }

    async fn decode_i16<V: Visitor>(mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let i = self.parse_int().await?;
        visitor.visit_i16(i)
    }

    async fn decode_i32<V: Visitor>(mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let i = self.parse_int().await?;
        visitor.visit_i32(i)
    }

    async fn decode_i64<V: Visitor>(mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let i = self.parse_int().await?;
        visitor.visit_i64(i)
    }

    async fn decode_u8<V: Visitor>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }

    async fn decode_u16<V: Visitor>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }

    async fn decode_u32<V: Visitor>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }

    async fn decode_u64<V: Visitor>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }

    async fn decode_f32<V: Visitor>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }

    async fn decode_f64<V: Visitor>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }

    async fn decode_char<V: Visitor>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }

    async fn decode_string<V: Visitor>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }

    async fn decode_byte_buf<V: Visitor>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }

    async fn decode_option<V: Visitor>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }

    async fn decode_seq<V: Visitor>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }

    async fn decode_tuple<V: Visitor>(
        self,
        _len: usize,
        _visitor: V,
    ) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }

    async fn decode_map<V: Visitor>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }

    async fn decode_identifier<V: Visitor>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }

    async fn decode_ignored_any<V: Visitor>(self, _visitor: V) -> Result<V::Value, Self::Error> {
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

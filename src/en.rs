//! Serialize a Rust data structure into JSON data.

use std::fmt;
use std::pin::Pin;

use destream::en::{self, ToStream};
use futures::future;
use futures::stream::{self, Stream, StreamExt, TryStreamExt};

pub type JSONStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>, Error>>>>;

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

struct EncodeMap;

impl en::EncodeMap for EncodeMap {
    type Ok = JSONStream;
    type Error = Error;

    fn encode_key<T: ToStream + ?Sized>(&mut self, _key: &T) -> Result<(), Self::Error> {
        unimplemented!()
    }

    fn encode_value<T: ToStream + ?Sized>(&mut self, _value: &T) -> Result<(), Self::Error> {
        unimplemented!()
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        unimplemented!()
    }
}

struct EncodeSeq;

impl en::EncodeSeq for EncodeSeq {
    type Ok = JSONStream;
    type Error = Error;

    fn encode_element<T: ToStream + ?Sized>(&mut self, _value: &T) -> Result<(), Self::Error> {
        unimplemented!()
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        unimplemented!()
    }
}

struct EncodeStruct;

impl en::EncodeStruct for EncodeStruct {
    type Ok = JSONStream;
    type Error = Error;

    fn encode_field<T: ToStream + ?Sized>(
        &mut self,
        _key: &'static str,
        _value: &T,
    ) -> Result<(), Self::Error> {
        unimplemented!()
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        unimplemented!()
    }
}

struct EncodeTuple;

impl en::EncodeTuple for EncodeTuple {
    type Ok = JSONStream;
    type Error = Error;

    fn encode_element<T: ToStream + ?Sized>(&mut self, _value: &T) -> Result<(), Self::Error> {
        unimplemented!()
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        unimplemented!()
    }
}

struct Encoder;

impl en::Encoder for Encoder {
    type Ok = JSONStream;
    type Error = Error;
    type EncodeMap = EncodeMap;
    type EncodeSeq = EncodeSeq;
    type EncodeStruct = EncodeStruct;
    type EncodeTuple = EncodeTuple;

    fn encode_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    fn encode_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    fn encode_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    fn encode_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    fn encode_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    fn encode_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    fn encode_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    fn encode_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    fn encode_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    fn encode_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    fn encode_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    fn encode_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt(v))
    }

    fn encode_none(self) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt("null"))
    }

    fn encode_some<T: ToStream + ?Sized>(self, _value: &T) -> Result<Self::Ok, Self::Error> {
        unimplemented!()
    }

    fn encode_unit(self) -> Result<Self::Ok, Self::Error> {
        Ok(encode_fmt("null"))
    }

    fn encode_seq(self, _len: Option<usize>) -> Result<Self::EncodeSeq, Self::Error> {
        unimplemented!()
    }

    fn encode_tuple(self, _len: usize) -> Result<Self::EncodeTuple, Self::Error> {
        unimplemented!()
    }

    fn encode_map(self, _len: Option<usize>) -> Result<Self::EncodeMap, Self::Error> {
        unimplemented!()
    }

    fn encode_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::EncodeStruct, Self::Error> {
        unimplemented!()
    }
}

pub fn encode_stream<T: ToStream, S: Stream<Item = T>>(
    source: S,
) -> impl Stream<Item = Result<Vec<u8>, Error>> {
    source.map(|item| item.to_stream(Encoder)).try_flatten()
}

fn encode_fmt<T: fmt::Display>(value: T) -> JSONStream {
    let encoded = value.to_string().as_bytes().to_vec();
    Box::pin(stream::once(future::ready(Ok(encoded))))
}

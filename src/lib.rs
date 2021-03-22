//! Library for decoding and encoding JSON streams.
//!
//! Example:
//! ```
//! # use futures::executor::block_on;
//! let expected = ("one".to_string(), 2.0, vec![3, 4]);
//! let stream = destream_json::encode(&expected).unwrap();
//! let actual = block_on(destream_json::try_decode((), stream)).unwrap();
//! assert_eq!(expected, actual);
//! ```
//!
//! Deviations from the [JSON spec](https://www.json.org/):
//!  - `destream_json` will not error out if asked to decode or encode a non-string key in a JSON
//!    object (i.e., it supports a superset of the official JSON spec). This may cause issues
//!    when using another JSON library to decode a stream encoded by `destream_json`. This can be
//!    mitigated by simply not using non-string object keys, or adding an explicit check at encoding
//!    time.

mod constants;
pub mod de;
pub mod en;

pub use de::{decode, try_decode};
pub use en::{encode, encode_map, encode_seq};

#[cfg(tokio_io)]
pub use de::read_from;

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::fmt;
    use std::iter::FromIterator;
    use std::marker::PhantomData;

    use async_trait::async_trait;
    use bytes::Bytes;
    use destream::de::{self, FromStream, Visitor};
    use destream::en::IntoStream;
    use futures::future;
    use futures::stream::{self, Stream, StreamExt, TryStreamExt};

    use super::de::*;
    use super::en::*;

    async fn test_decode<T: FromStream<Context = ()> + PartialEq + fmt::Debug>(
        encoded: &str,
        expected: T,
    ) {
        for i in 1..encoded.len() {
            let source = stream::iter(encoded.as_bytes().into_iter().cloned()).chunks(i);
            let actual: T = decode((), source).await.unwrap();
            assert_eq!(expected, actual)
        }
    }

    async fn test_encode<'en, S: Stream<Item = Result<Vec<u8>, super::en::Error>> + 'en>(
        encoded_stream: S,
        expected: &str,
    ) {
        let encoded = encoded_stream
            .try_fold(vec![], |mut buffer, chunk| {
                buffer.extend(chunk);
                future::ready(Ok(buffer))
            })
            .await
            .unwrap();

        assert_eq!(expected, String::from_utf8(encoded).unwrap());
    }

    async fn test_encode_value<'en, T: IntoStream<'en> + PartialEq + fmt::Debug + 'en>(
        value: T,
        expected: &str,
    ) {
        test_encode(encode(value).unwrap(), expected).await;
    }

    async fn test_encode_list<
        'en,
        T: IntoStream<'en> + 'en,
        S: Stream<Item = T> + Send + Unpin + 'en,
    >(
        seq: S,
        expected: &str,
    ) {
        test_encode(encode_seq(seq), expected).await;
    }

    async fn test_encode_map<
        'en,
        K: IntoStream<'en> + 'en,
        V: IntoStream<'en> + 'en,
        S: Stream<Item = (K, V)> + Send + Unpin + 'en,
    >(
        map: S,
        expected: &str,
    ) {
        test_encode(encode_map(map), expected).await;
    }

    #[tokio::test]
    async fn test_json_primitives() {
        test_decode("true", true).await;
        test_decode("false", false).await;

        test_encode_value(true, "true").await;
        test_encode_value(false, "false").await;

        test_decode("1", 1u8).await;
        test_decode(" 2 ", 2u16).await;
        test_decode("4658 ", 4658_u32).await;
        test_decode(&2u64.pow(63).to_string(), 2u64.pow(63)).await;

        test_encode_value(1u8, "1").await;
        test_encode_value(2u16, "2").await;
        test_encode_value(4658_u32, "4658").await;
        test_encode_value(2u64.pow(63), &2u64.pow(63).to_string()).await;

        test_decode("-1", -1i8).await;
        test_decode("\t\n-32", -32i16).await;
        test_decode("53\t", 53i32).await;
        test_decode(&(-2i64).pow(63).to_string(), (-2i64).pow(63)).await;

        test_encode_value(-1i8, "-1").await;
        test_encode_value(-32i16, "-32").await;
        test_encode_value(53i32, "53").await;
        test_encode_value((-2i64).pow(63), &(-2i64).pow(63).to_string()).await;

        test_decode("2e2", 2e2_f32).await;
        test_decode("-2e-3", -2e-3_f64).await;
        test_decode("3.14", 3.14_f32).await;
        test_decode("-1.414e4", -1.414e4_f64).await;

        test_encode_value(2e2_f32, "200").await;
        test_encode_value(-2e3, "-2000").await;
        test_encode_value(3.14_f32, "3.14").await;
        test_encode_value(-1.414e4_f64, "-14140").await;

        test_decode("\t\r\n\" hello world \"", " hello world ".to_string()).await;
        test_decode("\"one \\\" two\"", "one \\\" two".to_string()).await;

        test_encode_value("hello world", "\"hello world\"").await;
        test_encode_value("one \\\" two", "\"one \\\" two\"").await;
    }

    #[tokio::test]
    async fn test_bytes() {
        struct BytesVisitor;
        impl Visitor for BytesVisitor {
            type Value = Vec<u8>;

            fn expecting() -> &'static str {
                "a byte buffer"
            }

            fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
                Ok(v)
            }
        }

        let utf8_str = "मकर संक्रान्ति";

        let encoded = encode(Bytes::from(utf8_str.as_bytes())).unwrap();
        let decoded: Bytes = try_decode((), encoded).await.unwrap();

        assert_eq!(utf8_str, std::str::from_utf8(&decoded).unwrap());
    }

    #[tokio::test]
    async fn test_seq() {
        test_encode_list(stream::empty::<u8>(), "[]").await;

        test_decode("[1, 2, 3]", vec![1, 2, 3]).await;
        test_encode_value(&[1u8, 2u8, 3u8], "[1,2,3]").await;

        test_encode_list(stream::iter(&[1u8, 2u8, 3u8]), "[1,2,3]").await;
        test_encode_list(
            stream::iter(vec![vec![1, 2, 3], vec![], vec![4]]),
            "[[1,2,3],[],[4]]",
        )
        .await;

        test_decode(
            "\t[\r\n\rtrue,\r\n\t-1,\r\n\t\"hello world. \"\r\n]",
            (true, -1i16, "hello world. ".to_string()),
        )
        .await;
        test_encode_value(
            (true, -1i16, "hello world. "),
            "[true,-1,\"hello world. \"]",
        )
        .await;
        test_encode_list(
            stream::iter(vec!["hello ", "\tworld"]),
            "[\"hello \",\"\tworld\"]",
        )
        .await;

        test_decode(" [ 1.23, 4e3, -3.45]\n", [1.23, 4e3, -3.45]).await;
        test_encode_value(&[1.23, 4e3, -3.45], "[1.23,4000,-3.45]").await;

        test_decode(
            "[\"one\", \"two\", \"three\"]",
            HashSet::<String>::from_iter(vec!["one", "two", "three"].into_iter().map(String::from)),
        )
        .await;
        test_encode_value(&["one", "two", "three"], "[\"one\",\"two\",\"three\"]").await;
    }

    #[tokio::test]
    async fn test_map() {
        let mut map = HashMap::<String, bool>::from_iter(vec![
            ("k1".to_string(), true),
            ("k2".to_string(), false),
        ]);

        test_decode("\r\n\t{ \"k1\":\ttrue  , \"k2\": false\r\n}", map.clone()).await;

        map.remove("k2");
        test_encode_value(map.clone(), "{\"k1\":true}").await;
        test_encode_map(stream::iter(map), "{\"k1\":true}").await;

        let map = BTreeMap::<i8, Option<bool>>::from_iter(vec![(-1, Some(true)), (2, None)]);

        test_decode("\r\n\t{ -1:\ttrue, 2:null}", map.clone()).await;
        test_encode_value(map.clone(), "{-1:true,2:null}").await;
        test_encode_map(stream::iter(map), "{-1:true,2:null}").await;
    }

    #[tokio::test]
    async fn test_err() {
        #[derive(Debug, Default, Eq, PartialEq)]
        struct TestMap;

        #[async_trait]
        impl FromStream for TestMap {
            type Context = ();

            async fn from_stream<D: de::Decoder>(_: (), decoder: &mut D) -> Result<Self, D::Error> {
                decoder.decode_map(TestVisitor::<Self>::default()).await
            }
        }

        #[derive(Debug, Default, Eq, PartialEq)]
        struct TestSeq;

        #[async_trait]
        impl FromStream for TestSeq {
            type Context = ();

            async fn from_stream<D: de::Decoder>(_: (), decoder: &mut D) -> Result<Self, D::Error> {
                decoder.decode_seq(TestVisitor::<Self>::default()).await
            }
        }

        #[derive(Default)]
        struct TestVisitor<T> {
            phantom: PhantomData<T>,
        }

        #[async_trait]
        impl<T: Default + Send> de::Visitor for TestVisitor<T> {
            type Value = T;

            fn expecting() -> &'static str {
                "a Test struct"
            }

            async fn visit_map<A: de::MapAccess>(self, mut access: A) -> Result<T, A::Error> {
                let _key = access.next_key::<String>(()).await?;

                assert!(access.next_value::<String>(()).await.is_err());
                assert!(access.next_value::<Vec<i64>>(()).await.is_ok());

                Ok(T::default())
            }

            async fn visit_seq<A: de::SeqAccess>(self, mut access: A) -> Result<T, A::Error> {
                assert!(access.next_element::<String>(()).await.is_err());
                assert!(access.next_element::<Vec<i64>>(()).await.is_err());
                assert!(access.next_element::<i64>(()).await.is_ok());

                Ok(T::default())
            }
        }

        let encoded = "{\"k1\": [1, 2, 3]}";
        let source = stream::iter(encoded.as_bytes().into_iter().cloned()).chunks(5);
        let actual: TestMap = decode((), source).await.unwrap();
        assert_eq!(actual, TestMap);

        let encoded = "\t[ 1,2, 3]";
        let source = stream::iter(encoded.as_bytes().into_iter().cloned()).chunks(2);
        let actual: TestSeq = decode((), source).await.unwrap();
        assert_eq!(actual, TestSeq);
    }

    #[cfg(tokio_io)]
    #[tokio::test]
    async fn test_async_read() {
        use std::io::Cursor;

        let encoded = "[\"hello\", 1, {}]";
        let cursor = Cursor::new(encoded.as_bytes());
        let decoded: (String, i64, HashMap<String, bool>) = read_from((), cursor).await.unwrap();

        assert_eq!(
            decoded,
            ("hello".to_string(), 1i64, HashMap::<String, bool>::new())
        );
    }
}

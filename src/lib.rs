//! Library for decoding and encoding JSON streams.

mod constants;
pub mod de;
pub mod en;

pub use de::decode;
pub use en::encode;

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::fmt;
    use std::iter::FromIterator;

    use destream::de::{self, FromStream, Visitor};
    use destream::en::ToStream;
    use futures::future;
    use futures::stream::{self, StreamExt, TryStreamExt};

    use super::de::*;
    use super::en::*;

    async fn test_decode<T: FromStream + PartialEq + fmt::Debug>(encoded: &str, expected: T) {
        for i in 1..encoded.len() {
            let source = stream::iter(encoded.as_bytes().into_iter().cloned()).chunks(i);
            let actual: T = decode(source).await.unwrap();
            assert_eq!(expected, actual)
        }
    }

    async fn test_encode<'en, T: ToStream<'en> + PartialEq + fmt::Debug + 'en>(
        value: T,
        expected: &str,
    ) {
        let encoded = encode(value)
            .unwrap()
            .try_fold(vec![], |mut buffer, chunk| {
                buffer.extend(chunk);
                future::ready(Ok(buffer))
            })
            .await
            .unwrap();

        assert_eq!(expected, String::from_utf8(encoded).unwrap());
    }

    #[tokio::test]
    async fn test_json_primitives() {
        test_decode("true", true).await;
        test_decode("false", false).await;

        test_encode(true, "true").await;
        test_encode(false, "false").await;

        test_decode("1", 1u8).await;
        test_decode(" 2 ", 2u16).await;
        test_decode("4658 ", 4658_u32).await;
        test_decode(&2u64.pow(63).to_string(), 2u64.pow(63)).await;

        test_encode(1u8, "1").await;
        test_encode(2u16, "2").await;
        test_encode(4658_u32, "4658").await;
        test_encode(2u64.pow(63), &2u64.pow(63).to_string()).await;

        test_decode("-1", -1i8).await;
        test_decode("\t\n-32", -32i16).await;
        test_decode("53\t", 53i32).await;
        test_decode(&(-2i64).pow(63).to_string(), (-2i64).pow(63)).await;

        test_encode(-1i8, "-1").await;
        test_encode(-32i16, "-32").await;
        test_encode(53i32, "53").await;
        test_encode((-2i64).pow(63), &(-2i64).pow(63).to_string()).await;

        test_decode("2e2", 2e2_f32).await;
        test_decode("-2e-3", -2e-3_f64).await;
        test_decode("3.14", 3.14_f32).await;
        test_decode("-1.414e4", -1.414e4_f64).await;

        test_encode(2e2_f32, "200").await;
        test_encode(-2e3, "-2000").await;
        test_encode(3.14_f32, "3.14").await;
        test_encode(-1.414e4_f64, "-14140").await;

        test_decode("\t\r\n\" hello world \"", " hello world ".to_string()).await;
        test_decode("\"one \\\" two\"", "one \\\" two".to_string()).await;

        test_encode("hello world", "\"hello world\"").await;
        test_encode("one \\\" two", "\"one \\\" two\"").await;
    }

    #[tokio::test]
    async fn test_bytes() {
        struct BytesVisitor;
        impl Visitor for BytesVisitor {
            type Value = Vec<u8>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a byte buffer")
            }

            fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
                Ok(v)
            }
        }

        let expected = "मकर संक्रान्ति";
        let mut decoder = Decoder::from(stream::once(future::ready(
            format!("\"{}\"", base64::encode(expected.as_bytes()))
                .as_bytes()
                .to_vec(),
        )));

        let actual = de::Decoder::decode_byte_buf(&mut decoder, BytesVisitor)
            .await
            .unwrap();

        assert_eq!(expected.as_bytes().to_vec(), actual);
    }

    #[tokio::test]
    async fn test_seq() {
        test_decode("[1, 2, 3]", vec![1, 2, 3]).await;
        test_encode(&[1u8, 2u8, 3u8], "[1,2,3]").await;

        test_decode(
            "\t[\r\n\rtrue,\r\n\t-1,\r\n\t\"hello world. \"\r\n]",
            (true, -1i16, "hello world. ".to_string()),
        )
        .await;
        test_encode(
            (true, -1i16, "hello world. "),
            "[true,-1,\"hello world. \"]",
        )
        .await;

        test_decode(" [ 1.23, 4e3, -3.45]\n", [1.23, 4e3, -3.45]).await;
        test_encode(&[1.23, 4e3, -3.45], "[1.23,4000,-3.45]").await;

        test_decode(
            "[\"one\", \"two\", \"three\"]",
            HashSet::<String>::from_iter(vec!["one", "two", "three"].into_iter().map(String::from)),
        )
        .await;
        test_encode(&["one", "two", "three"], "[\"one\",\"two\",\"three\"]").await;
    }

    #[tokio::test]
    async fn test_map() {
        let mut map = HashMap::<String, bool>::from_iter(vec![
            ("k1".to_string(), true),
            ("k2".to_string(), false),
        ]);

        test_decode("\r\n\t{ \"k1\":\ttrue, \"k2\":false}", map.clone()).await;

        map.remove("k2");
        test_encode(map, "{\"k1\":true}").await;

        let map = BTreeMap::<i32, Option<bool>>::from_iter(vec![(-1, Some(true)), (2, None)]);
        test_decode("\r\n\t{ -1:\ttrue, 2:null}", map.clone()).await;
        test_encode(map, "{-1:true,2:null}").await;
    }
}

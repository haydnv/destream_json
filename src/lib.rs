//! Library for decoding and encoding JSON streams.

pub mod de;

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::fmt;
    use std::iter::FromIterator;

    use destream::de::{self, FromStream, Visitor};
    use futures::future;
    use futures::stream::{self, StreamExt};

    use super::de::*;

    async fn run_test<T: FromStream + PartialEq + fmt::Debug>(encoded: &str, expected: T) {
        for i in 1..encoded.len() {
            let source = stream::iter(encoded.as_bytes().into_iter().cloned()).chunks(i);
            let actual: T = from_stream(source).await.unwrap();
            assert_eq!(expected, actual)
        }
    }

    #[tokio::test]
    async fn test_json_primitives() {
        run_test("true", true).await;
        run_test("false", false).await;

        run_test("1", 1u8).await;
        run_test(" 2 ", 2u16).await;
        run_test("4658 ", 4658_u32).await;
        run_test(&2u64.pow(63).to_string(), 2u64.pow(63)).await;

        run_test("-1", -1i8).await;
        run_test("\t\n-32", -32i16).await;
        run_test("53\t", 53i32).await;
        run_test(&(-2i64).pow(63).to_string(), (-2i64).pow(63)).await;

        run_test("2e2", 2e2_f32).await;
        run_test("-2e-3", -2e-3_f64).await;
        run_test("3.14", 3.14_f32).await;
        run_test("-1.414e4", -1.414e4_f64).await;

        run_test("\"one \\\" two\"", "one \\\" two".to_string()).await;

        run_test("\t\r\n\" hello world \"", " hello world ".to_string()).await;
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
        run_test("[1, 2, 3]", vec![1, 2, 3]).await;

        run_test(
            "\t[\r\n\rtrue,\r\n\t-1,\r\n\t\"hello world. \"\r\n]",
            (true, -1i16, "hello world. ".to_string()),
        )
        .await;

        run_test(" [ 1.23, 4e3, -3.45]\n", [1.23, 4e3, -3.45]).await;

        run_test(
            "[\"one\", \"two\", \"three\"]",
            HashSet::<String>::from_iter(vec!["one", "two", "three"].into_iter().map(String::from)),
        )
        .await;
    }

    #[tokio::test]
    async fn test_map() {
        run_test(
            "\r\n\t{ \"k1\":\ttrue, \"k2\":false}",
            HashMap::<String, bool>::from_iter(vec![
                ("k1".to_string(), true),
                ("k2".to_string(), false),
            ]),
        )
        .await;

        run_test(
            "\r\n\t{ -1:\ttrue, 2:null}",
            BTreeMap::<i32, Option<bool>>::from_iter(vec![(-1, Some(true)), (2, None)]),
        )
        .await;
    }
}

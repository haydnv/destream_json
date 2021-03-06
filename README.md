# destream_json
Rust library for encoding and decoding JSON streams

Example:
```rust
let expected = ("one".to_string(), 2.0, vec![3, 4]);
let stream = destream_json::encode(&expected).unwrap();
let actual = destream_json::try_decode((), stream).await;
assert_eq!(expected, actual);
```

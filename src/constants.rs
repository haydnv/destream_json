pub const COLON: u8 = b':';
pub const COMMA: u8 = b',';
pub const DECIMAL: u8 = b'.';
pub const ESCAPE: u8 = b'\\';
pub const FALSE: &[u8] = b"false";
pub const TRUE: &[u8] = b"true";
pub const LIST_BEGIN: u8 = b'[';
pub const LIST_END: u8 = b']';
pub const NULL: &[u8] = b"null";
pub const MAP_BEGIN: u8 = b'{';
pub const MAP_END: u8 = b'}';
pub const NUMERIC: [u8; 15] = [
    b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'0', b'-', b'e', b'E', DECIMAL,
];
pub const QUOTE: u8 = b'"';

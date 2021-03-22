pub const COLON: &'static [u8] = b":";
pub const COMMA: &'static [u8] = b",";
pub const DECIMAL: &'static [u8] = b".";
pub const ESCAPE: &'static [u8] = b"\\";
pub const FALSE: &[u8] = b"false";
pub const TRUE: &[u8] = b"true";
pub const LIST_BEGIN: &'static [u8] = b"[";
pub const LIST_END: &'static [u8] = b"]";
pub const NULL: &[u8] = b"null";
pub const MAP_BEGIN: &'static [u8] = b"{";
pub const MAP_END: &'static [u8] = b"}";
pub const NUMERIC: [u8; 15] = [
    b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'0', b'-', b'e', b'E', b'.',
];
pub const QUOTE: &'static [u8] = b"\"";

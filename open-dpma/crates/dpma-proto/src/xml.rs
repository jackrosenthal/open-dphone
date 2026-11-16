//! XML text helpers.

use crate::cbuf::c_bytes;

/// The entity for a character `phone_utils_xml_escape` replaces.
fn entity(c: u8) -> Option<&'static [u8]> {
    match c {
        b'"' => Some(b"&quot;"),
        b'&' => Some(b"&amp;"),
        b'\'' => Some(b"&apos;"),
        b'<' => Some(b"&lt;"),
        b'>' => Some(b"&gt;"),
        _ => None,
    }
}

/// `phone_utils_xml_escape(in, out, size)` (0x1731a0) into a buffer of
/// `size` bytes: `"`, `&`, `'`, `<`, and `>` become entities.  When an
/// entity does not fit (it must be shorter than the space left, which
/// keeps one byte free), the character is copied unescaped instead; when
/// not even that fits, the output ends.  The input is a C string (it ends
/// at a NUL).  Returns the output without its terminating NUL.
pub fn escape_bounded(input: &[u8], size: usize) -> Vec<u8> {
    let mut out = Vec::new();
    if size == 0 {
        // The C code would write before its buffer; nothing sensible to do.
        return out;
    }
    let mut left = size - 1;
    for &c in c_bytes(input) {
        match entity(c) {
            Some(e) if e.len() < left => {
                out.extend_from_slice(e);
                left -= e.len();
            }
            _ => {
                if left < 1 {
                    break;
                }
                out.push(c);
                left -= 1;
            }
        }
    }
    out
}

/// `phone_utils_xml_escape` with unlimited room.
pub fn escape(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for &c in s {
        match entity(c) {
            Some(e) => out.extend_from_slice(e),
            None => out.push(c),
        }
    }
    out
}

/// Does `s`, which follows an `&`, begin with the rest of a reference: one
/// of the five predefined entities, or a character reference (`#38;`,
/// `#x26;`)?
fn continues_reference(s: &[u8]) -> bool {
    if [&b"amp;"[..], b"lt;", b"gt;", b"quot;", b"apos;"]
        .iter()
        .any(|name| s.starts_with(name))
    {
        return true;
    }
    let (digits, is_digit): (&[u8], fn(&u8) -> bool) = match s {
        [b'#', b'x', rest @ ..] => (rest, u8::is_ascii_hexdigit),
        [b'#', rest @ ..] => (rest, u8::is_ascii_digit),
        _ => return false,
    };
    let n = digits.iter().take_while(|c| is_digit(c)).count();
    n > 0 && digits.get(n) == Some(&b';')
}

/// `s` as the value of a double-quoted attribute (or as element text),
/// changed only where a document would not be well formed with the bytes
/// as they are: `"` and `<` become entities, and so does an `&` that
/// begins no reference.  A reference (`&amp;`, `&#38;`) stays, so a value
/// that was written escaped comes out as written, and `>` and `'`, which
/// may stand in such a value, stay too.  The input is a C string (it ends
/// at a NUL).
pub fn attr_value(s: &[u8]) -> Vec<u8> {
    let s = c_bytes(s);
    let mut out = Vec::with_capacity(s.len());
    for (i, &c) in s.iter().enumerate() {
        match c {
            b'"' => out.extend_from_slice(b"&quot;"),
            b'<' => out.extend_from_slice(b"&lt;"),
            b'&' if !continues_reference(&s[i + 1..]) => out.extend_from_slice(b"&amp;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribute_values() {
        // Nothing to change: the bytes as they are.
        for same in [
            &b"America/Denver"[..],
            b"",
            b"it's 5 > 4",
            b"R&amp;D &lt;lab&gt; &quot;x&quot; &apos;y&apos;",
            b"&#38; &#x26; &#x3C;",
            "Zürich".as_bytes(),
            b"Jos\xe9",
        ] {
            assert_eq!(attr_value(same), same);
        }
        assert_eq!(attr_value(b"ACME <Corp>"), b"ACME &lt;Corp>");
        assert_eq!(attr_value(b"say \"hi\""), b"say &quot;hi&quot;");
        assert_eq!(attr_value(b"R&D"), b"R&amp;D");
        assert_eq!(attr_value(b"http://x/a?b=1&c=2"), b"http://x/a?b=1&amp;c=2");
        // An `&` that only looks like the start of a reference.
        for (raw, escaped) in [
            (&b"&"[..], &b"&amp;"[..]),
            (b"&amp", b"&amp;amp"),
            (b"&nbsp;", b"&amp;nbsp;"),
            (b"&#;", b"&amp;#;"),
            (b"&#x;", b"&amp;#x;"),
            (b"&#12a;", b"&amp;#12a;"),
            (b"&#xZ;", b"&amp;#xZ;"),
            (b"a && b", b"a &amp;&amp; b"),
        ] {
            assert_eq!(attr_value(raw), escaped);
        }
        // A C string.
        assert_eq!(attr_value(b"a<b\0c<d"), b"a&lt;b");
    }

    #[test]
    fn escapes() {
        assert_eq!(escape(b"a&b<c>d\"e'f"), b"a&amp;b&lt;c&gt;d&quot;e&apos;f");
        assert_eq!(
            escape("plain ünïcödé".as_bytes()),
            "plain ünïcödé".as_bytes()
        );
        assert_eq!(escape(b"Jos\xe9 <\xff>"), b"Jos\xe9 &lt;\xff&gt;");
        assert_eq!(escape(b""), b"");
    }

    #[test]
    fn bounded() {
        let s = b"a&b<c>d\"e'f";
        assert_eq!(escape_bounded(s, 1024), escape(s));
        // size 1: nothing fits.
        assert_eq!(escape_bounded(b"abc", 1), b"");
        assert_eq!(escape_bounded(b"abc", 3), b"ab");
        // "&amp;" (5) needs more than 5 bytes left, i.e., size >= 7.
        assert_eq!(escape_bounded(b"&", 6), b"&");
        assert_eq!(escape_bounded(b"&", 7), b"&amp;");
        assert_eq!(escape_bounded(b"x&", 7), b"x&");
        // "&quot;" and "&apos;" (6) need size >= 8; "&lt;" and "&gt;" (4)
        // need size >= 6.
        assert_eq!(escape_bounded(b"\"'", 8), b"&quot;'");
        assert_eq!(escape_bounded(b"<>", 6), b"&lt;>");
        assert_eq!(escape_bounded(b"<>", 10), b"&lt;&gt;");
        // A NUL ends the input.
        assert_eq!(escape_bounded(b"ab\0cd", 100), b"ab");
        assert_eq!(escape_bounded(b"ab", 0), b"");
    }
}

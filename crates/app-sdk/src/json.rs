//! A small JSON value, parser, and writer, so apps need no heavier
//! dependency for PBX requests.

use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    /// Numbers are kept as their text (the PBX sends most as strings).
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

static NULL: Json = Json::Null;

impl Json {
    pub fn parse(text: &str) -> Option<Json> {
        let mut p = Parser {
            b: text.as_bytes(),
            i: 0,
            depth: 0,
        };
        let v = p.value()?;
        p.ws();
        (p.i == p.b.len()).then_some(v)
    }

    /// Object member, or `Null`.
    pub fn get(&self, key: &str) -> &Json {
        match self {
            Json::Obj(m) => m
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v)
                .unwrap_or(&NULL),
            _ => &NULL,
        }
    }

    /// Follows a path of object keys.
    pub fn path(&self, keys: &[&str]) -> &Json {
        keys.iter().fold(self, |v, k| v.get(k))
    }

    /// Text of a string, number, or boolean; "" otherwise.
    pub fn text(&self) -> String {
        match self {
            Json::Str(s) | Json::Num(s) => s.clone(),
            Json::Bool(b) => b.to_string(),
            _ => String::new(),
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) | Json::Num(s) => Some(s),
            _ => None,
        }
    }

    /// Integer value of a number or a numeric string.
    pub fn int(&self) -> Option<i64> {
        match self {
            Json::Str(s) | Json::Num(s) => {
                let s = s.trim();
                s.parse::<i64>()
                    .ok()
                    .or_else(|| s.split('.').next()?.parse().ok())
            }
            Json::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }

    /// Truthiness the way the PBX writes it: `true`, `1`, `"1"`, `"yes"`.
    pub fn truthy(&self) -> bool {
        match self {
            Json::Bool(b) => *b,
            Json::Str(s) | Json::Num(s) => {
                matches!(s.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
            }
            _ => false,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }

    /// The elements of an array; a single non-null value counts as a list
    /// of one (the PBX collapses one-element lists in some replies).
    pub fn items(&self) -> Vec<&Json> {
        match self {
            Json::Arr(a) => a.iter().collect(),
            Json::Null => Vec::new(),
            other => vec![other],
        }
    }

    pub fn obj<const N: usize>(members: [(&str, Json); N]) -> Json {
        Json::Obj(
            members
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }

    pub fn str(s: impl Into<String>) -> Json {
        Json::Str(s.into())
    }

    pub fn num(n: i64) -> Json {
        Json::Num(n.to_string())
    }

    pub fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => out.push_str(n),
            Json::Str(s) => write_str(s, out),
            Json::Arr(a) => {
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Json::Obj(m) => {
                out.push('{');
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_str(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

impl std::fmt::Display for Json {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = String::new();
        self.write(&mut s);
        f.write_str(&s)
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    depth: u32,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn lit(&mut self, s: &str, v: Json) -> Option<Json> {
        if self.b[self.i..].starts_with(s.as_bytes()) {
            self.i += s.len();
            Some(v)
        } else {
            None
        }
    }

    fn value(&mut self) -> Option<Json> {
        self.ws();
        match *self.b.get(self.i)? {
            b'n' => self.lit("null", Json::Null),
            b't' => self.lit("true", Json::Bool(true)),
            b'f' => self.lit("false", Json::Bool(false)),
            b'"' => self.string().map(Json::Str),
            b'[' => {
                self.nest()?;
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                } else {
                    loop {
                        a.push(self.value()?);
                        self.ws();
                        match self.b.get(self.i)? {
                            b',' => self.i += 1,
                            b']' => {
                                self.i += 1;
                                break;
                            }
                            _ => return None,
                        }
                    }
                }
                self.depth -= 1;
                Some(Json::Arr(a))
            }
            b'{' => {
                self.nest()?;
                self.i += 1;
                let mut m = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                } else {
                    loop {
                        self.ws();
                        let k = self.string()?;
                        self.ws();
                        if self.b.get(self.i) != Some(&b':') {
                            return None;
                        }
                        self.i += 1;
                        m.push((k, self.value()?));
                        self.ws();
                        match self.b.get(self.i)? {
                            b',' => self.i += 1,
                            b'}' => {
                                self.i += 1;
                                break;
                            }
                            _ => return None,
                        }
                    }
                }
                self.depth -= 1;
                Some(Json::Obj(m))
            }
            b'-' | b'0'..=b'9' => {
                let start = self.i;
                while self.i < self.b.len()
                    && matches!(
                        self.b[self.i],
                        b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
                    )
                {
                    self.i += 1;
                }
                Some(Json::Num(
                    String::from_utf8_lossy(&self.b[start..self.i]).into_owned(),
                ))
            }
            _ => None,
        }
    }

    fn nest(&mut self) -> Option<()> {
        self.depth += 1;
        (self.depth <= 64).then_some(())
    }

    fn hex4(&mut self) -> Option<u32> {
        let s = std::str::from_utf8(self.b.get(self.i..self.i + 4)?).ok()?;
        self.i += 4;
        u32::from_str_radix(s, 16).ok()
    }

    fn string(&mut self) -> Option<String> {
        if self.b.get(self.i) != Some(&b'"') {
            return None;
        }
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let c = *self.b.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = *self.b.get(self.i)?;
                    self.i += 1;
                    let ch = match e {
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xd800..0xdc00).contains(&cp)
                                && self.b.get(self.i..self.i + 2) == Some(b"\\u")
                            {
                                self.i += 2;
                                let lo = self.hex4()?;
                                cp = 0x10000
                                    + ((cp - 0xd800) << 10)
                                    + (lo.wrapping_sub(0xdc00) & 0x3ff);
                            }
                            char::from_u32(cp).unwrap_or('\u{fffd}')
                        }
                        other => other as char,
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                c => out.push(c),
            }
        }
        String::from_utf8(out).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let t = r#"{"a":[1,"two",{"b":null}],"c":true,"d":"x\nyé"}"#;
        let v = Json::parse(t).unwrap();
        assert_eq!(v.path(&["a"]).items().len(), 3);
        assert_eq!(v.get("a").items()[0].int(), Some(1));
        assert!(v.get("c").truthy());
        assert_eq!(v.get("d").text(), "x\nyé");
        assert_eq!(Json::parse(&v.to_string()).unwrap(), v);
    }

    #[test]
    fn rejects_garbage() {
        assert!(Json::parse("{").is_none());
        assert!(Json::parse("[1,]").is_none());
        assert!(Json::parse("1 2").is_none());
    }

    #[test]
    fn single_item_is_a_list() {
        let v = Json::parse(r#"{"x":{"y":"1"}}"#).unwrap();
        assert_eq!(v.get("x").items().len(), 1);
        assert_eq!(v.get("missing").items().len(), 0);
    }
}

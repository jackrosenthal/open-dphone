//! A small mutable XML tree with a serializer that matches the stock
//! firmware's TinyXML output byte for byte.
//!
//! Processing instructions and the XML declaration are dropped on parse.
//! Comments inside elements are kept (the stock merge copies them with the
//! subtree they sit in).  Attribute order is preserved.

use std::fmt;

/// A child of an [`Element`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Element(Element),
    Text(String),
    /// A comment inside an element.  The stock merge carries these along
    /// when it copies a subtree, so they are kept for byte-identical output.
    Comment(String),
}

/// An XML element: name, ordered attributes, and children.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

/// Why a document could not be parsed.
#[derive(Debug)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "XML parse error: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

impl Element {
    pub fn new(name: &str) -> Self {
        Element { name: name.to_owned(), attrs: Vec::new(), children: Vec::new() }
    }

    /// Builder: add or replace an attribute.
    pub fn with_attr(mut self, name: &str, value: &str) -> Self {
        self.set_attr(name, value);
        self
    }

    /// Builder: append a child element.
    pub fn with_child(mut self, child: Element) -> Self {
        self.children.push(Node::Element(child));
        self
    }

    /// Parse a document and return its root element.
    pub fn parse(text: &str) -> Result<Element, ParseError> {
        let opts = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
        let doc =
            roxmltree::Document::parse_with_options(text, opts).map_err(|e| ParseError(e.to_string()))?;
        Ok(convert(doc.root_element()))
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Set an attribute.  An existing attribute keeps its position; a new one
    /// goes to the end (TinyXML `SetAttribute`).
    pub fn set_attr(&mut self, name: &str, value: &str) {
        match self.attrs.iter_mut().find(|(k, _)| k == name) {
            Some((_, v)) => *v = value.to_owned(),
            None => self.attrs.push((name.to_owned(), value.to_owned())),
        }
    }

    pub fn remove_attr(&mut self, name: &str) {
        self.attrs.retain(|(k, _)| k != name);
    }

    /// Child elements, in order.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    pub fn elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    /// Child elements with the given name.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.elements().filter(move |e| e.name == name)
    }

    /// First child element with the given name.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|e| e.name == name)
    }

    pub fn child_mut(&mut self, name: &str) -> Option<&mut Element> {
        self.elements_mut().find(|e| e.name == name)
    }

    /// First child element with the given name, created at the end if absent.
    pub fn child_or_insert(&mut self, name: &str) -> &mut Element {
        if self.child(name).is_none() {
            self.children.push(Node::Element(Element::new(name)));
        }
        self.child_mut(name).unwrap()
    }

    /// First child element with the given name and `id` attribute.
    pub fn child_with_id(&self, name: &str, id: &str) -> Option<&Element> {
        self.elements().find(|e| e.name == name && e.attr("id") == Some(id))
    }

    pub fn child_with_id_mut(&mut self, name: &str, id: &str) -> Option<&mut Element> {
        self.elements_mut().find(|e| e.name == name && e.attr("id") == Some(id))
    }

    /// Remove every child element for which `pred` is true.
    pub fn remove_children(&mut self, mut pred: impl FnMut(&Element) -> bool) {
        self.children.retain(|n| match n {
            Node::Element(e) => !pred(e),
            _ => true,
        });
    }

    /// Text of the first child if it is a text node (TinyXML `GetText`).
    pub fn text(&self) -> Option<&str> {
        match self.children.first() {
            Some(Node::Text(t)) => Some(t),
            _ => None,
        }
    }

    /// Serialize as a document in the stock TinyXML layout: four-space
    /// indentation, `<a />` for empty elements, and a trailing newline.
    pub fn to_document(&self) -> String {
        let mut out = String::new();
        self.print(&mut out, 0);
        out.push('\n');
        out
    }

    fn print(&self, out: &mut String, depth: usize) {
        for _ in 0..depth {
            out.push_str("    ");
        }
        out.push('<');
        out.push_str(&self.name);
        for (k, v) in &self.attrs {
            out.push(' ');
            encode(k, out);
            // TinyXML picks the quote from the raw value, then encodes it.
            let q = if v.contains('"') { '\'' } else { '"' };
            out.push('=');
            out.push(q);
            encode(v, out);
            out.push(q);
        }
        match self.children.as_slice() {
            [] => out.push_str(" />"),
            [Node::Text(t)] => {
                out.push('>');
                encode(t, out);
                out.push_str("</");
                out.push_str(&self.name);
                out.push('>');
            }
            children => {
                out.push('>');
                for c in children {
                    match c {
                        Node::Element(e) => {
                            out.push('\n');
                            e.print(out, depth + 1);
                        }
                        Node::Text(t) => encode(t, out),
                        Node::Comment(t) => {
                            out.push('\n');
                            for _ in 0..=depth {
                                out.push_str("    ");
                            }
                            out.push_str("<!--");
                            out.push_str(t);
                            out.push_str("-->");
                        }
                    }
                }
                out.push('\n');
                for _ in 0..depth {
                    out.push_str("    ");
                }
                out.push_str("</");
                out.push_str(&self.name);
                out.push('>');
            }
        }
    }
}

fn convert(n: roxmltree::Node<'_, '_>) -> Element {
    let mut e = Element::new(n.tag_name().name());
    for a in n.attributes() {
        e.attrs.push((a.name().to_owned(), a.value().to_owned()));
    }
    for c in n.children() {
        if c.is_element() {
            e.children.push(Node::Element(convert(c)));
        } else if c.is_comment() {
            e.children.push(Node::Comment(c.text().unwrap_or("").to_owned()));
        } else if c.is_text() {
            // TinyXML condenses white space in text nodes.
            let t = c.text().unwrap_or("");
            let t: Vec<&str> = t.split_whitespace().collect();
            if !t.is_empty() {
                e.children.push(Node::Text(t.join(" ")));
            }
        }
    }
    e
}

/// TinyXML `EncodeString`.
fn encode(s: &str, out: &mut String) {
    let b = s.as_bytes();
    let mut i = 0;
    for (pos, ch) in s.char_indices() {
        if pos < i {
            continue;
        }
        match ch {
            '&' if b.get(pos + 1) == Some(&b'#') && b.get(pos + 2) == Some(&b'x') => {
                // A hexadecimal character reference passes through unchanged.
                match s[pos..].find(';') {
                    Some(end) => {
                        out.push_str(&s[pos..=pos + end]);
                        i = pos + end + 1;
                    }
                    None => out.push_str("&amp;"),
                }
            }
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 32 => {
                out.push_str(&format!("&#x{:02X};", c as u32));
            }
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_matches_tinyxml() {
        let e = Element::parse(
            "<?xml version=\"1.0\"?><!-- c --><config><!-- i --><setting id=\"a\" value=\"x&amp;y&lt;&quot;&apos;é\"/><s id=\"t\">text</s></config>",
        )
        .unwrap();
        assert_eq!(
            e.to_document(),
            "<config>\n    <!-- i -->\n    <setting id=\"a\" value='x&amp;y&lt;&quot;&apos;é' />\n    <s id=\"t\">text</s>\n</config>\n"
        );
        assert_eq!(Element::new("config").to_document(), "<config />\n");
    }
}

//! Contact list documents (`/nvdata/contact_lists/*`), the `<phonebooks>` /
//! `<contacts>` / `<contact>` shape the stock phone and DPMA servers use.

use crate::xml::Element;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Number {
    pub dial: String,
    pub label: String,
    pub primary: bool,
    pub raw: Element,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contact {
    pub id: String,
    pub first_name: String,
    pub last_name: String,
    pub organization: String,
    /// For example, `sip`.
    pub contact_type: String,
    /// Extension to subscribe to for presence, if any.
    pub subscribe_to: Option<String>,
    pub numbers: Vec<Number>,
    /// The contact element, for attributes not modeled here.
    pub raw: Element,
}

impl Contact {
    /// The number marked primary, else the first.
    pub fn primary_number(&self) -> Option<&Number> {
        self.numbers.iter().find(|n| n.primary).or(self.numbers.first())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactGroup {
    pub id: String,
    pub group_name: String,
    pub contacts: Vec<Contact>,
}

fn attr(e: &Element, name: &str) -> String {
    e.attr(name).unwrap_or("").to_owned()
}

fn group(e: &Element) -> ContactGroup {
    ContactGroup {
        id: attr(e, "id"),
        group_name: attr(e, "group_name"),
        contacts: e
            .children_named("contact")
            .map(|c| Contact {
                id: attr(c, "id"),
                first_name: attr(c, "first_name"),
                last_name: attr(c, "last_name"),
                organization: attr(c, "organization"),
                contact_type: attr(c, "contact_type"),
                subscribe_to: c.attr("subscribe_to").filter(|v| !v.is_empty()).map(str::to_owned),
                numbers: c
                    .child("numbers")
                    .map(|n| {
                        n.children_named("number")
                            .map(|n| Number {
                                dial: attr(n, "dial"),
                                label: attr(n, "label"),
                                primary: n.attr("primary") == Some("1"),
                                raw: n.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                raw: c.clone(),
            })
            .collect(),
    }
}

/// Parse a contact document.  The root may be `<phonebooks>` holding
/// `<contacts>` groups, or a single `<contacts>` group.
pub fn parse_contacts(text: &str) -> Result<Vec<ContactGroup>, crate::xml::ParseError> {
    let root = Element::parse(text)?;
    Ok(if root.name == "contacts" {
        vec![group(&root)]
    } else {
        root.children_named("contacts").map(group).collect()
    })
}

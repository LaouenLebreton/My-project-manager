//! A small element tree: MSPDI is element-only (no attributes matter), and the importer
//! needs to know which elements it did not use, so it can report them.

use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Debug, Default)]
pub struct Element {
    pub name: String,
    pub text: String,
    pub children: Vec<Element>,
}

impl Element {
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Trimmed text of a child element, if present and not empty.
    pub fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.trim()).filter(|t| !t.is_empty())
    }

    /// Exact text of a child element, untrimmed (names and notes keep their spaces).
    pub fn raw_of(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.as_str())
    }

    pub fn int_of(&self, name: &str) -> Option<i64> {
        self.text_of(name).and_then(|t| t.parse::<f64>().ok()).map(|v| v.round() as i64)
    }

    pub fn flag(&self, name: &str) -> bool {
        matches!(self.text_of(name), Some("1") | Some("true"))
    }

    /// True if the element carries information: non-default text or any children.
    pub fn has_content(&self) -> bool {
        !self.children.is_empty() || !matches!(self.text.trim(), "" | "0" | "0.0" | "-1" | "PT0H0M0S")
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct XmlError(pub String);

/// Parses a document into its root element. Namespaces prefixes are dropped (MSPDI uses a
/// default namespace).
pub fn parse(input: &str) -> Result<Element, XmlError> {
    let mut reader = Reader::from_str(input);
    let mut stack: Vec<Element> = vec![Element::default()];
    loop {
        let event = reader.read_event().map_err(|e| XmlError(format!("not valid XML: {e}")))?;
        match event {
            Event::Start(e) => stack.push(Element { name: local(e.name().as_ref()), ..Default::default() }),
            Event::Empty(e) => {
                let el = Element { name: local(e.name().as_ref()), ..Default::default() };
                stack.last_mut().expect("root").children.push(el);
            }
            Event::End(_) => {
                let el = stack.pop().expect("balanced");
                stack.last_mut().ok_or_else(|| XmlError("unbalanced XML".into()))?.children.push(el);
            }
            Event::Text(t) => stack.last_mut().expect("root").text.push_str(&t.xml10_content()),
            Event::CData(c) => stack.last_mut().expect("root").text.push_str(&c.xml10_content()),
            Event::GeneralRef(r) => {
                let name: &str = &r;
                let resolved = quick_xml::escape::unescape(&format!("&{name};"))
                    .map_err(|e| XmlError(format!("unknown entity &{name};: {e}")))?
                    .into_owned();
                stack.last_mut().expect("root").text.push_str(&resolved);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut doc = stack.pop().ok_or_else(|| XmlError("empty document".into()))?;
    if !stack.is_empty() {
        return Err(XmlError("the XML ends before all elements are closed".into()));
    }
    doc.children.pop().filter(|_| doc.children.is_empty()).ok_or_else(|| XmlError("expected one root element".into()))
}

fn local(qname: &str) -> String {
    qname.rsplit(':').next().unwrap_or(qname).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_entities_and_cdata() {
        let root = parse("<?xml version=\"1.0\"?><Project xmlns=\"x\"><Name>R&amp;D &#8211; 1</Name><Notes><![CDATA[a<b]]></Notes><Empty/></Project>").unwrap();
        assert_eq!(root.name, "Project");
        assert_eq!(root.text_of("Name"), Some("R&D – 1"));
        assert_eq!(root.text_of("Notes"), Some("a<b"));
        assert!(root.child("Empty").is_some());
    }

    #[test]
    fn rejects_broken_xml() {
        assert!(parse("<Project><Name>x</Project>").is_err());
        assert!(parse("not xml at all").is_err());
    }
}

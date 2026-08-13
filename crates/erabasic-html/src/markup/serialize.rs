use super::{HtmlDocument, HtmlNode};

#[must_use]
pub fn serialize_document(document: &HtmlDocument) -> String {
    fn node(output: &mut String, item: &HtmlNode) {
        match item {
            HtmlNode::Text { text, .. } => output.push_str(&super::super::escape(text)),
            HtmlNode::Element {
                kind,
                attributes,
                children,
                ..
            } => {
                output.push('<');
                output.push_str(kind.tag_name());
                for attribute in attributes {
                    output.push(' ');
                    output.push_str(&attribute.name);
                    output.push_str("='");
                    output.push_str(&super::super::escape(&attribute.value));
                    output.push('\'');
                }
                output.push('>');
                for child in children {
                    node(output, child);
                }
                if !kind.is_void() {
                    output.push_str("</");
                    output.push_str(kind.tag_name());
                    output.push('>');
                }
            }
        }
    }
    let mut output = String::new();
    for item in &document.nodes {
        node(&mut output, item);
    }
    output
}

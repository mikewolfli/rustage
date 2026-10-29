// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Reading JUnit XML back into test results (roadmap task T21.11).
//!
//! The quality centre already writes JUnit XML. Writing a report nobody can read back makes the
//! export a screenshot rather than an interchange format, so this module imports the same shape it
//! writes:
//!
//! * **A test's state is read, not assumed.** `<failure>` and `<error>` are failed, `<skipped>` is
//!   skipped, and a `<testcase>` with no child is passed. JUnit has no "not run" state, so an absent
//!   test is simply absent — importing never invents a `NotRun` row and never turns a missing test
//!   into a pass.
//! * **A malformed file is refused with its line.** A name that is not quoted, an unclosed tag, a
//!   closing tag that does not match, a test case without a name and a non-numeric `time` are each
//!   refused with the line they occur on, rather than being skipped silently.
//! * **The environment is checked, not ignored.** Our export writes `revision` and `target` into a
//!   `<properties>` block. When those are present, [`JUnitImport::matches`] refuses an import into a
//!   different revision or target; when they are absent the file is accepted and the absence is
//!   reported, because a file with no environment is not evidence for a specific one.
//!
//! The parser is deliberately small: elements, attributes, entities and nesting, nothing else. It
//! does not resolve DTDs or CDATA sections, and text content is ignored — the importer only needs
//! the tags and attributes the exporter writes.

use std::collections::BTreeMap;

use crate::app::quality::report::{TestStatus, Tests};

/// One imported JUnit file.
#[derive(Clone, PartialEq, Debug)]
pub struct JUnitImport {
    /// The test results it carried.
    pub tests: Tests,
    /// The `<properties>` it declared, by name.
    pub properties: BTreeMap<String, String>,
}

impl JUnitImport {
    /// Reads one JUnit XML document.
    pub fn from_xml(text: &str) -> Result<Self, String> {
        let root = parse_document(text)?;
        if root.name != "testsuite" && root.name != "testsuites" {
            return Err(format!(
                "the document root is <{}>, not <testsuite> or <testsuites>",
                root.name
            ));
        }
        let mut tests = Tests::new();
        let mut properties = BTreeMap::new();
        collect(&root, &mut tests, &mut properties)?;
        Ok(Self { tests, properties })
    }

    /// Refuses an import into a different revision or target than the file declares.
    ///
    /// A file that declares neither is accepted: [`Self::environment`] says so, and the caller can
    /// decide whether that is enough. A declared value that disagrees is never overwritten by the
    /// caller's, and never silently accepted.
    pub fn matches(&self, revision: u64, target: &str) -> Result<(), String> {
        if let Some(declared) = self.properties.get("revision")
            && declared != &revision.to_string()
        {
            return Err(format!(
                "the file is for revision {declared}, not {revision}"
            ));
        }
        if let Some(declared) = self.properties.get("target")
            && declared != target
        {
            return Err(format!(
                "the file is for target {declared:?}, not {target:?}"
            ));
        }
        Ok(())
    }

    /// What the file says about its environment.
    pub fn environment(&self) -> String {
        let mut parts = Vec::new();
        for name in ["revision", "target"] {
            match self.properties.get(name) {
                Some(value) => parts.push(format!("{name} {value}")),
                None => parts.push(format!("{name} not declared")),
            }
        }
        parts.join(", ")
    }
}

/// One XML element.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Node {
    name: String,
    attributes: BTreeMap<String, String>,
    children: Vec<Node>,
}

/// Walks the tree under `node`, recording test cases and properties.
fn collect(
    node: &Node,
    tests: &mut Tests,
    properties: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    match node.name.as_str() {
        "testcase" => {
            let name = node.attributes.get("name").ok_or_else(|| {
                "a <testcase> has no name attribute, so its result cannot be attributed".to_owned()
            })?;
            let seconds = match node.attributes.get("time") {
                Some(value) => Some(value.parse::<f64>().map_err(|_| {
                    format!("test case {name:?} has time {value:?}, which is not a number")
                })?),
                None => None,
            };
            let status = if node
                .children
                .iter()
                .any(|child| child.name == "failure" || child.name == "error")
            {
                TestStatus::Failed
            } else if node.children.iter().any(|child| child.name == "skipped") {
                TestStatus::Skipped
            } else {
                TestStatus::Passed
            };
            tests.record(name.clone(), status, seconds);
            return Ok(());
        }
        "property" => {
            if let (Some(name), Some(value)) =
                (node.attributes.get("name"), node.attributes.get("value"))
            {
                properties.insert(name.clone(), value.clone());
            }
            return Ok(());
        }
        _ => {}
    }
    for child in &node.children {
        collect(child, tests, properties)?;
    }
    Ok(())
}

/// Parses a whole document, returning its root element.
fn parse_document(text: &str) -> Result<Node, String> {
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    let bytes: Vec<char> = text.chars().collect();
    let mut index = 0;
    let mut line = 1;

    while index < bytes.len() {
        let character = bytes[index];
        if character == '\n' {
            line += 1;
            index += 1;
            continue;
        }
        if character != '<' {
            index += 1;
            continue;
        }
        // Skip the XML declaration, comments and DOCTYPEs: none of them carries a result.
        if bytes[index..].starts_with(&['<', '?']) {
            index = skip_until(&bytes, index, '>', line)? + 1;
            continue;
        }
        if bytes[index..].starts_with(&['<', '!']) {
            if bytes[index..].starts_with(&['<', '!', '-', '-']) {
                let end = find(&bytes, index, &['-', '-', '>'])
                    .ok_or_else(|| format!("line {line}: an XML comment is not closed"))?;
                line += bytes[index..end].iter().filter(|c| **c == '\n').count();
                index = end + 3;
            } else {
                index = skip_until(&bytes, index, '>', line)? + 1;
            }
            continue;
        }
        let Some(close) = find(&bytes, index, &['>']) else {
            return Err(format!("line {line}: a tag is not closed"));
        };
        let body: String = bytes[index + 1..close].iter().collect();
        line += body.chars().filter(|c| *c == '\n').count();
        let relative_line = line - body.chars().filter(|c| *c == '\n').count();
        if let Some(rest) = body.strip_prefix('/') {
            let name = rest.trim();
            let Some(open) = stack.pop() else {
                return Err(format!("line {relative_line}: </{name}> closes nothing"));
            };
            if open.name != name {
                return Err(format!(
                    "line {relative_line}: </{name}> closes <{}>",
                    open.name
                ));
            }
            let finished = match stack.last_mut() {
                Some(parent) => {
                    parent.children.push(open);
                    index = close + 1;
                    continue;
                }
                None => open,
            };
            if root.is_some() {
                return Err(format!(
                    "line {relative_line}: a second root element <{}>",
                    finished.name
                ));
            }
            root = Some(finished);
            index = close + 1;
            continue;
        }

        let self_closing = body.ends_with('/');
        let content = if self_closing {
            &body[..body.len() - 1]
        } else {
            &body[..]
        };
        let node = parse_element(content, relative_line)?;
        if self_closing {
            match stack.last_mut() {
                Some(parent) => parent.children.push(node),
                None => {
                    if root.is_some() {
                        return Err(format!(
                            "line {relative_line}: a second root element <{}>",
                            node.name
                        ));
                    }
                    root = Some(node);
                }
            }
        } else {
            stack.push(node);
        }
        index = close + 1;
    }

    if let Some(open) = stack.last() {
        return Err(format!("line {line}: <{}> is never closed", open.name));
    }
    root.ok_or_else(|| "the document has no element".to_owned())
}

/// Parses an element's name and attributes.
fn parse_element(content: &str, line: usize) -> Result<Node, String> {
    let mut characters = content.char_indices();
    let name_end = characters
        .by_ref()
        .find(|(_, character)| character.is_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(content.len());
    let name = &content[..name_end];
    if name.is_empty() {
        return Err(format!("line {line}: a tag has no name"));
    }
    let mut attributes = BTreeMap::new();
    let mut rest = &content[name_end..];
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let Some(equals) = rest.find('=') else {
            return Err(format!(
                "line {line}: <{name}> has {rest:?}, which is not name=\"value\""
            ));
        };
        let attribute = rest[..equals].trim();
        if attribute.is_empty() {
            return Err(format!(
                "line {line}: <{name}> has an attribute with no name"
            ));
        }
        let after = rest[equals + 1..].trim_start();
        let Some(quote) = after.chars().next() else {
            return Err(format!(
                "line {line}: <{name}> attribute {attribute} has no value"
            ));
        };
        if quote != '"' && quote != '\'' {
            return Err(format!(
                "line {line}: <{name}> attribute {attribute} is not quoted"
            ));
        }
        let value_start = 1;
        let Some(value_end) = after[value_start..].find(quote) else {
            return Err(format!(
                "line {line}: <{name}> attribute {attribute} is not closed"
            ));
        };
        let value = unescape(&after[value_start..value_start + value_end], line)?;
        attributes.insert(attribute.to_owned(), value);
        rest = &after[value_start + value_end + 1..];
    }
    Ok(Node {
        name: name.to_owned(),
        attributes,
        children: Vec::new(),
    })
}

/// Advances past the next `character`, or refuses.
fn skip_until(bytes: &[char], from: usize, character: char, line: usize) -> Result<usize, String> {
    bytes[from..]
        .iter()
        .position(|c| *c == character)
        .map(|offset| from + offset)
        .ok_or_else(|| format!("line {line}: the declaration is not closed"))
}

/// Finds a character sequence, returning the index it starts at.
fn find(bytes: &[char], from: usize, needle: &[char]) -> Option<usize> {
    bytes[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

/// Decodes the five predefined XML entities.
fn unescape(text: &str, line: usize) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find(';') else {
            return Err(format!(
                "line {line}: an entity in an attribute is not closed"
            ));
        };
        let entity = &after[..end];
        out.push(match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            other => {
                return Err(format!(
                    "line {line}: the entity &{other}; is not one of the five XML entities"
                ));
            }
        });
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::JUnitImport;
    use crate::app::quality::report::{TestStatus, Tests};

    #[test]
    fn the_export_reads_back_with_the_same_statuses_and_times() {
        let mut tests = Tests::new();
        tests.record("a successful test", TestStatus::Passed, Some(0.25));
        tests.record("a failing test <one>", TestStatus::Failed, None);
        tests.record("a skipped test", TestStatus::Skipped, None);
        tests.record("a test that did not run", TestStatus::NotRun, None);

        let imported = JUnitImport::from_xml(&tests.junit()).expect("our own export");
        let names: Vec<&str> = imported
            .tests
            .results()
            .iter()
            .map(|result| result.name.as_str())
            .collect();
        // JUnit has no "not run" state, so the not-run test is absent rather than passed.
        assert_eq!(
            names,
            vec![
                "a failing test <one>",
                "a skipped test",
                "a successful test"
            ]
        );
        let status_of = |name: &str| {
            imported
                .tests
                .results()
                .into_iter()
                .find(|result| result.name == name)
                .map(|result| result.status)
        };
        assert_eq!(status_of("a successful test"), Some(TestStatus::Passed));
        assert_eq!(status_of("a failing test <one>"), Some(TestStatus::Failed));
        assert_eq!(status_of("a skipped test"), Some(TestStatus::Skipped));
    }

    #[test]
    fn an_error_element_is_a_failure_and_entities_are_decoded() {
        let xml = r#"<?xml version="1.0"?>
<testsuite name="s" tests="2">
  <testcase name="reads &amp; writes" time="1.5"><error message="boom"/></testcase>
  <testcase name="plain"/>
</testsuite>"#;
        let imported = JUnitImport::from_xml(xml).expect("a document");
        let results = imported.tests.results();
        let read = results
            .iter()
            .find(|result| result.name == "reads & writes")
            .expect("the entity was decoded");
        assert_eq!(read.status, TestStatus::Failed);
        assert_eq!(read.seconds, Some(1.5));
        assert_eq!(
            results
                .iter()
                .find(|result| result.name == "plain")
                .map(|result| result.status),
            Some(TestStatus::Passed)
        );
    }

    #[test]
    fn a_property_block_carries_the_environment_and_a_mismatch_is_refused() {
        let xml = r#"<testsuite name="s">
  <properties>
    <property name="revision" value="7"/>
    <property name="target" value="desktop"/>
  </properties>
  <testcase name="ok"/>
</testsuite>"#;
        let imported = JUnitImport::from_xml(xml).expect("a document");
        assert_eq!(imported.properties["revision"], "7");
        imported
            .matches(7, "desktop")
            .expect("the same environment");
        assert!(imported.matches(8, "desktop").is_err());
        assert!(imported.matches(7, "tablet").is_err());
        assert!(imported.environment().contains("revision 7"));
    }

    #[test]
    fn a_file_without_an_environment_says_so_rather_than_claiming_one() {
        let imported =
            JUnitImport::from_xml("<testsuite><testcase name=\"ok\"/></testsuite>").expect("a doc");
        assert!(imported.matches(7, "desktop").is_ok());
        assert!(imported.environment().contains("revision not declared"));
    }

    #[test]
    fn malformed_documents_are_refused_with_their_line() {
        for (xml, expected) in [
            ("<testsuite>\n  <testcase name=x/>\n</testsuite>", "line 2"),
            (
                "<testsuite>\n  <testcase name=\"a\">\n</testsuite>",
                "closes <testcase>",
            ),
            ("<testsuite>\n  <testcase/>\n</testsuite>", "no name"),
            (
                "<testsuite>\n  <testcase name=\"a\" time=\"soon\"/>\n</testsuite>",
                "not a number",
            ),
            ("<testsuite></testsuite><testsuite/>", "second root"),
            ("<suite/>", "document root"),
            ("<testsuite>", "never closed"),
        ] {
            let error = JUnitImport::from_xml(xml).expect_err("a malformed document");
            assert!(error.contains(expected), "{xml:?}: {error}");
        }
    }
}

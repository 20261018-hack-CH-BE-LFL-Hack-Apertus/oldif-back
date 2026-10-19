use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Word {
    pub text: String,
    pub bbox: [f64; 4],
    pub confidence: Option<f64>,
    pub line_id: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Line {
    pub id: String,
    pub text: String,
    pub bbox: [f64; 4],
}
#[derive(Serialize)]
pub struct Geometry {
    pub width: f64,
    pub height: f64,
    pub words: Vec<Word>,
    pub lines: Vec<Line>,
    pub text: String,
}
pub fn bbox_xml(text: &str) -> Result<Geometry> {
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        nodes_limit: 100_000,
        entity_resolver: None,
    };
    let doc = roxmltree::Document::parse_with_options(text, options)
        .map_err(|_| Error::bad("Invalid bounded page geometry"))?;
    let page = doc
        .descendants()
        .find(|node| node.has_tag_name("page"))
        .ok_or_else(|| Error::bad("Page geometry is absent"))?;
    let width = attribute(page, "width")?;
    let height = attribute(page, "height")?;
    let mut words = vec![];
    for (index, line) in page
        .descendants()
        .filter(|n| n.has_tag_name("line"))
        .enumerate()
    {
        for word in line.descendants().filter(|n| n.has_tag_name("word")) {
            words.push(Word {
                text: word.text().unwrap_or("").into(),
                bbox: [
                    attribute(word, "xMin")?,
                    attribute(word, "yMin")?,
                    attribute(word, "xMax")?,
                    attribute(word, "yMax")?,
                ],
                confidence: None,
                line_id: format!("line-{index:05}"),
            });
        }
    }
    geometry(width, height, words)
}
fn attribute(node: roxmltree::Node<'_, '_>, key: &str) -> Result<f64> {
    node.attribute(key)
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|n| n.is_finite() && *n >= 0.0 && *n < 100_000.0)
        .ok_or_else(|| Error::bad("Invalid page geometry coordinate"))
}
pub fn tsv(text: &str) -> Result<Geometry> {
    let (mut width, mut height) = (0.0, 0.0);
    let mut words = vec![];
    for row in text.lines().skip(1) {
        let columns = row.splitn(12, '\t').collect::<Vec<_>>();
        if columns.len() != 12 {
            continue;
        }
        let value = |i: usize| {
            columns[i]
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && *v >= 0.0 && *v < 100_000.0)
                .ok_or_else(|| Error::bad("Invalid OCR geometry"))
        };
        if columns[0] == "1" {
            width = value(8)?;
            height = value(9)?;
        }
        if columns[0] != "5" || columns[11].trim().is_empty() {
            continue;
        }
        let (x, y, w, h) = (value(6)?, value(7)?, value(8)?, value(9)?);
        let confidence = columns[10]
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 100.0);
        words.push(Word {
            text: columns[11].into(),
            bbox: [x, y, x + w, y + h],
            confidence,
            line_id: format!("{}-{}-{}", columns[2], columns[3], columns[4]),
        });
    }
    geometry(width, height, words)
}
fn geometry(width: f64, height: f64, words: Vec<Word>) -> Result<Geometry> {
    if width <= 0.0 || height <= 0.0 || words.len() > 20_000 {
        return Err(Error::bad("Page geometry exceeds its bounds"));
    }
    let mut lines: Vec<Line> = vec![];
    for word in &words {
        if word.bbox[2] < word.bbox[0]
            || word.bbox[3] < word.bbox[1]
            || word.bbox[2] > width + 1.0
            || word.bbox[3] > height + 1.0
        {
            return Err(Error::bad("Word geometry lies outside its page"));
        }
        if let Some(line) = lines.last_mut().filter(|line| line.id == word.line_id) {
            line.text.push(' ');
            line.text.push_str(&word.text);
            line.bbox = union(line.bbox, word.bbox);
        } else {
            lines.push(Line {
                id: word.line_id.clone(),
                text: word.text.clone(),
                bbox: word.bbox,
            });
        }
    }
    let text = lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Geometry {
        width,
        height,
        words,
        lines,
        text,
    })
}
pub fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geometry_preserves_unicode_and_rejects_external_entity_resolution() {
        let xml = r#"<doc><page width="100" height="100"><line><word xMin="1" yMin="2" xMax="20" yMax="12">läsen &amp; écrire</word></line></page></doc>"#;
        let g = bbox_xml(xml).unwrap();
        assert_eq!(g.words[0].text, "läsen & écrire");
        assert_eq!(g.words[0].bbox, [1.0, 2.0, 20.0, 12.0]);
        let external = r#"<!DOCTYPE doc [<!ENTITY x SYSTEM "file:///etc/passwd">]><doc><page width="100" height="100"><line><word xMin="1" yMin="2" xMax="20" yMax="12">&x;</word></line></page></doc>"#;
        assert!(bbox_xml(external).is_err());
    }
    #[test]
    fn ocr_words_keep_confidence_and_physical_coordinates() {
        let text = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n1\t1\t0\t0\t0\t0\t0\t0\t100\t100\t-1\t\n5\t1\t1\t1\t1\t1\t10\t20\t30\t10\t91.5\tläsen";
        let g = tsv(text).unwrap();
        assert_eq!(g.text, "läsen");
        assert_eq!(g.words[0].confidence, Some(91.5));
        assert_eq!(g.words[0].bbox, [10.0, 20.0, 40.0, 30.0]);
    }
}

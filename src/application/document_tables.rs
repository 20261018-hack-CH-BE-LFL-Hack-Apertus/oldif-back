use crate::infrastructure::document_geometry::{Geometry, Word, union};
use serde_json::{Value, json};

pub fn candidates(geometry: &Geometry) -> Vec<Value> {
    let mut tables = vec![];
    let mut rows: Vec<Vec<String>> = vec![];
    let mut box_all = [0.0; 4];
    let mut anchors: Vec<f64> = vec![];
    for words in physical_rows(geometry) {
        let bbox = words
            .iter()
            .fold(words[0].bbox, |bbox, word| union(bbox, word.bbox));
        let mut cells: Vec<String> = vec![];
        let mut starts = vec![];
        let mut last_x = 0.0;
        for word in words {
            let gap = word.bbox[0] - last_x;
            if cells.is_empty() || gap > geometry.width * 0.035 {
                cells.push(word.text.clone());
                starts.push(word.bbox[0]);
            } else {
                let last = cells.last_mut().unwrap();
                last.push(' ');
                last.push_str(&word.text);
            }
            last_x = word.bbox[2];
        }
        let aligned = cells.len() >= 2
            && cells.len() == anchors.len()
            && starts
                .iter()
                .zip(&anchors)
                .all(|(a, b)| (a - b).abs() < geometry.width * 0.025);
        if !aligned {
            flush(&mut tables, &mut rows, box_all);
            anchors = starts;
            box_all = bbox;
        }
        if cells.len() >= 2 {
            box_all = union(box_all, bbox);
            rows.push(cells);
        } else {
            anchors.clear();
        }
    }
    flush(&mut tables, &mut rows, box_all);
    tables
}
fn physical_rows(geometry: &Geometry) -> Vec<Vec<&Word>> {
    let mut words = geometry.words.iter().collect::<Vec<_>>();
    words.sort_by(|a, b| {
        a.bbox[1]
            .total_cmp(&b.bbox[1])
            .then(a.bbox[0].total_cmp(&b.bbox[0]))
    });
    let mut rows: Vec<Vec<&Word>> = vec![];
    for word in words {
        let aligned = rows.last().is_some_and(|row| {
            let first = row[0];
            let tolerance = (first.bbox[3] - first.bbox[1]).max(word.bbox[3] - word.bbox[1]) * 0.6;
            ((first.bbox[1] + first.bbox[3] - word.bbox[1] - word.bbox[3]) / 2.0).abs() <= tolerance
        });
        if aligned {
            rows.last_mut().unwrap().push(word);
        } else {
            rows.push(vec![word]);
        }
    }
    for row in &mut rows {
        row.sort_by(|a, b| a.bbox[0].total_cmp(&b.bbox[0]));
    }
    rows
}

fn flush(tables: &mut Vec<Value>, rows: &mut Vec<Vec<String>>, bbox: [f64; 4]) {
    if rows.len() >= 3 && tables.len() < 30 {
        tables.push(json!({"bbox":bbox,"row_count":rows.len(),"column_count":rows[0].len(),"rows":rows,"method":"aligned-text-gaps","status":"unverified","semantic_table_claim":false}));
    }
    rows.clear();
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::document_geometry::bbox_xml;
    #[test]
    fn repeated_columns_are_candidates_and_prose_is_not_a_verified_table() {
        let mut xml = String::from("<doc><page width=\"200\" height=\"200\">");
        for y in [10, 30, 50] {
            xml.push_str(&format!("<line><word xMin=\"10\" yMin=\"{y}\" xMax=\"30\" yMax=\"{}\">name</word></line><line><word xMin=\"120\" yMin=\"{y}\" xMax=\"145\" yMax=\"{}\">value</word></line>",y+10,y+10));
        }
        xml.push_str("</page></doc>");
        let tables = candidates(&bbox_xml(&xml).unwrap());
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0]["row_count"], 3);
        assert_eq!(tables[0]["status"], "unverified");
    }
}

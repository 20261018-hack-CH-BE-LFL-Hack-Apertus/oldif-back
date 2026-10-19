use crate::{
    application::documents,
    domain::tabular_cell,
    error::{Error, Result},
    state::App,
};
use serde_json::{Value, json};

pub async fn export(
    app: &App,
    owner: &str,
    id: &str,
    format: &str,
) -> Result<(String, &'static str)> {
    let detail = documents::detail(app, owner, id).await?;
    serialize(&detail, format)
}
pub fn serialize(detail: &Value, format: &str) -> Result<(String, &'static str)> {
    let document = &detail["document"];
    let pages = detail["pages"].as_array().ok_or_else(Error::unavailable)?;
    let manifest = json!({"schema":"oldif-document-dataset/1.0","document_id":document["id"],"document_revision":document["version"],"original_sha256":document["original_sha256"],"byte_count":document["byte_count"],"page_count":pages.len(),"licence":document["licence"],"review":document["review"],"pages":pages.iter().map(|p|json!({"source_id":p["id"],"page":p["page"],"text_sha256":p["text_sha256"],"original_sha256":p["original_sha256"]})).collect::<Vec<_>>(),"transformations":document["transformation"],"extraction":document["extraction"],"ocr_performed":false,"human_quality_metric":"NOT_RUN","training_readiness":"requires rights, privacy, bias and human quality review"});
    match format {
        "manifest" => Ok((serde_json::to_string_pretty(&manifest)?, "application/json")),
        "json" => Ok((
            serde_json::to_string_pretty(
                &json!({"manifest":manifest,"document":document,"pages":pages,"revisions":detail["revisions"]}),
            )?,
            "application/json",
        )),
        "csv" | "tsv" => tabular(document, pages, format),
        _ => Err(Error::bad("Choose json, csv, tsv, or manifest")),
    }
}
fn tabular(document: &Value, pages: &[Value], format: &str) -> Result<(String, &'static str)> {
    let headers = [
        "document_id",
        "document_revision",
        "source_id",
        "page",
        "title",
        "text_json",
        "text_sha256",
        "original_sha256",
        "licence",
        "review_decision",
        "labels_json",
        "flags_json",
        "annotations_json",
    ];
    let delimiter = if format == "csv" { "," } else { "\t" };
    let mut lines = vec![headers.join(delimiter)];
    for p in pages {
        let annotations = document["annotations"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|a| a["page"] == p["page"])
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let fields = vec![
            document["id"].clone(),
            document["version"].clone(),
            p["id"].clone(),
            p["page"].clone(),
            document["title"].clone(),
            json!(p["text"].to_string()),
            p["text_sha256"].clone(),
            p["original_sha256"].clone(),
            document["licence"].clone(),
            document["review"]["decision"].clone(),
            json!(document["labels"].to_string()),
            json!(p["flags"].to_string()),
            json!(serde_json::to_string(&annotations)?),
        ];
        lines.push(
            fields
                .iter()
                .map(|v| cell(v, format))
                .collect::<Vec<_>>()
                .join(delimiter),
        );
    }
    let text = lines.join("\n") + "\n";
    Ok((
        text,
        if format == "csv" {
            "text/csv; charset=utf-8"
        } else {
            "text/tab-separated-values; charset=utf-8"
        },
    ))
}
fn cell(value: &Value, format: &str) -> String {
    let text = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    let safe = if format == "tsv" {
        tabular_cell(&text)
    } else if text.trim_start().starts_with(['=', '+', '-', '@']) {
        format!("'{text}")
    } else {
        text
    };
    format!("\"{}\"", safe.replace('"', "\"\""))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exports_preserve_page_hash_review_and_spreadsheet_safety() {
        let detail = json!({"document":{"id":"doc","version":3,"title":"=formula","original_sha256":"abc","licence":"private","review":{"decision":"needs_review"},"labels":[],"annotations":[]},"pages":[{"id":"doc-p0001","page":1,"text":"original\ntext","text_sha256":"def","original_sha256":"abc","flags":{"empty_text":false}}],"revisions":[]});
        let csv = serialize(&detail, "csv").unwrap().0;
        assert!(csv.contains("\"'=formula\""));
        assert!(csv.contains("needs_review"));
        let manifest: Value =
            serde_json::from_str(&serialize(&detail, "manifest").unwrap().0).unwrap();
        assert_eq!(manifest["pages"][0]["page"], 1);
        assert_eq!(manifest["pages"][0]["text_sha256"], "def");
        assert_eq!(manifest["human_quality_metric"], "NOT_RUN");
    }
}

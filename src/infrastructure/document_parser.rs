use super::document_process::bounded;
use crate::{
    config::DocumentConfig,
    error::{Error, Result},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

pub struct Parsed {
    pub pages: Vec<String>,
    pub metadata: Value,
}
pub fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
pub fn media(filename: &str, content_type: &str, bytes: &[u8]) -> Result<&'static str> {
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match (ext.as_str(), content_type.split(';').next().unwrap_or("")) {
        ("pdf", "application/pdf") if bytes.starts_with(b"%PDF-") => Ok("application/pdf"),
        ("txt", "text/plain") if !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok() => {
            Ok("text/plain")
        }
        ("json", "application/json") if serde_json::from_slice::<Value>(bytes).is_ok() => {
            Ok("application/json")
        }
        _ => Err(Error::bad("Upload a matching PDF, UTF-8 TXT, or JSON file")),
    }
}
pub async fn parse(config: &DocumentConfig, media: &str, bytes: &[u8]) -> Result<Parsed> {
    if bytes.is_empty() || bytes.len() > config.max_bytes {
        return Err(Error::bad(
            "Document exceeds the configured upload byte limit",
        ));
    }
    let parsed = match media {
        "application/pdf" => pdf(config, bytes).await?,
        "text/plain" => Parsed {
            pages: std::str::from_utf8(bytes)
                .map_err(|_| Error::bad("Invalid UTF-8"))?
                .split('\u{c}')
                .map(str::to_owned)
                .collect(),
            metadata: json!({"extractor":"utf8-text","version":"1"}),
        },
        "application/json" => json_pages(bytes)?,
        _ => return Err(Error::bad("Unsupported document format")),
    };
    validate_pages(config, &parsed.pages)?;
    Ok(parsed)
}
fn json_pages(bytes: &[u8]) -> Result<Parsed> {
    let input: Value = serde_json::from_slice(bytes)?;
    let rows = input
        .get("pages")
        .or_else(|| input.get("records"))
        .and_then(Value::as_array)
        .ok_or_else(|| Error::bad("JSON needs a pages or records array containing text"))?;
    let pages = rows
        .iter()
        .map(|row| {
            row["text"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| Error::bad("Every JSON page requires text"))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Parsed {
        pages,
        metadata: json!({"extractor":"json-text-array","version":"1","pagination":"array_order"}),
    })
}
fn validate_pages(config: &DocumentConfig, pages: &[String]) -> Result<()> {
    if pages.is_empty()
        || pages.len() > config.max_pages
        || pages.iter().map(String::len).sum::<usize>() > config.max_text_bytes
        || pages
            .iter()
            .any(|page| page.len() > 1_000_000 || page.contains('\0'))
    {
        return Err(Error::bad("Extracted document exceeds page or text limits"));
    }
    Ok(())
}
pub(crate) struct Workspace(pub(crate) PathBuf);
impl Workspace {
    pub(crate) async fn new(bytes: &[u8]) -> Result<Self> {
        let dir = std::env::temp_dir().join(format!("oldif-document-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir(&dir)
            .await
            .map_err(|_| Error::unavailable())?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| Error::unavailable())?;
        let workspace = Self(dir);
        tokio::fs::write(workspace.0.join("input.pdf"), bytes)
            .await
            .map_err(|_| Error::unavailable())?;
        Ok(workspace)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
async fn pdf(config: &DocumentConfig, bytes: &[u8]) -> Result<Parsed> {
    let workspace = Workspace::new(bytes).await?;
    let info = bounded(config, &workspace.0, "pdfinfo", &["input.pdf"], 65_536).await?;
    let info = String::from_utf8_lossy(&info);
    let count = info
        .lines()
        .find_map(|line| line.strip_prefix("Pages:"))
        .and_then(|s| s.trim().parse::<usize>().ok())
        .ok_or_else(|| Error::bad("PDF page count could not be determined"))?;
    if count == 0
        || count > config.max_pages
        || info
            .lines()
            .any(|line| line.starts_with("Encrypted:") && line.contains("yes"))
    {
        return Err(Error::bad(
            "Encrypted PDFs or documents above the page limit are not accepted",
        ));
    }
    let output = bounded(
        config,
        &workspace.0,
        "pdftotext",
        &["-layout", "-enc", "UTF-8", "input.pdf", "-"],
        config.max_text_bytes,
    )
    .await?;
    let text = String::from_utf8(output)
        .map_err(|_| Error::bad("PDF extractor returned invalid UTF-8"))?;
    let mut pages = text.split('\u{c}').map(str::to_owned).collect::<Vec<_>>();
    if pages.last().is_some_and(String::is_empty) {
        pages.pop();
    }
    if pages.len() != count {
        return Err(Error::bad(
            "PDF physical page count and extracted pagination disagree",
        ));
    }
    let mut metadata =
        json!({"extractor":"poppler-pdftotext","mode":"layout","ocr_performed":false});
    for (label, key) in [
        ("Title:", "pdf_title"),
        ("Author:", "pdf_author"),
        ("CreationDate:", "pdf_creation_date"),
    ] {
        if let Some(value) = info.lines().find_map(|line| line.strip_prefix(label)) {
            metadata[key] = json!(value.trim().chars().take(500).collect::<String>());
        }
    }
    Ok(Parsed { pages, metadata })
}
pub async fn preview(config: &DocumentConfig, bytes: &[u8], page: usize) -> Result<Vec<u8>> {
    let workspace = Workspace::new(bytes).await?;
    let page = page.to_string();
    let image = bounded(
        config,
        &workspace.0,
        "pdftoppm",
        &[
            "-f",
            &page,
            "-l",
            &page,
            "-singlefile",
            "-scale-to",
            "1400",
            "-png",
            "input.pdf",
        ],
        8_388_608,
    )
    .await?;
    if !image.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(Error::bad("PDF preview could not be rendered"));
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_type_spoofing_and_binary_text_are_rejected() {
        assert!(media("x.pdf", "application/pdf", b"not a pdf").is_err());
        assert!(media("x.txt", "text/plain", b"\0content").is_err());
        assert!(media("x.exe", "text/plain", b"text").is_err());
        assert!(media("x.txt", "text/plain", "läsen".as_bytes()).is_ok());
    }
    #[test]
    fn json_import_never_fetches_urls_or_infers_missing_text() {
        assert!(json_pages(br#"{"pages":[{"url":"https://example.org"}]}"#).is_err());
        assert_eq!(
            json_pages(br#"{"pages":[{"text":"first"},{"text":""}]}"#)
                .unwrap()
                .pages
                .len(),
            2
        );
    }
}

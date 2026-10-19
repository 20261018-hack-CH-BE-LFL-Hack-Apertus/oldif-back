use super::{
    document_geometry::{self, Geometry},
    document_parser::{Workspace, preview},
    document_process::{bounded, execute},
};
use crate::{
    config::DocumentConfig,
    error::{Error, Result},
};
use serde_json::{Value, json};

pub struct Extracted {
    pub geometry: Geometry,
    pub metadata: Value,
}
pub async fn extract(
    config: &DocumentConfig,
    bytes: &[u8],
    page: usize,
    ocr: bool,
    language: &str,
) -> Result<Extracted> {
    let workspace = Workspace::new(bytes).await?;
    if ocr {
        return recognize(config, &workspace, bytes, page, language).await;
    }
    let number = page.to_string();
    let xml = bounded(
        config,
        &workspace.0,
        "pdftotext",
        &[
            "-f",
            &number,
            "-l",
            &number,
            "-bbox-layout",
            "-enc",
            "UTF-8",
            "input.pdf",
            "-",
        ],
        4_194_304,
    )
    .await?;
    let text = String::from_utf8(xml).map_err(|_| Error::bad("Invalid page geometry encoding"))?;
    Ok(Extracted {
        geometry: document_geometry::bbox_xml(&text)?,
        metadata: json!({"method":"poppler-bbox-layout","coordinate_system":"pdf_points","language":null,"ocr_performed":false}),
    })
}
async fn recognize(
    config: &DocumentConfig,
    workspace: &Workspace,
    bytes: &[u8],
    page: usize,
    language: &str,
) -> Result<Extracted> {
    if !["eng", "deu", "fra", "ita"].contains(&language) {
        return Err(Error::bad(
            "Choose an installed OCR language: eng, deu, fra, or ita",
        ));
    }
    let dir = workspace.0.join("ocr");
    tokio::fs::create_dir(&dir)
        .await
        .map_err(|_| Error::unavailable())?;
    let image = preview(config, bytes, page).await?;
    tokio::fs::write(dir.join("page.png"), image)
        .await
        .map_err(|_| Error::unavailable())?;
    let langs = execute(config, &dir, &config.tesseract, &["--list-langs"], 65536).await?;
    if !String::from_utf8_lossy(&langs)
        .lines()
        .any(|line| line.trim() == language)
    {
        return Err(Error::bad("Selected OCR language is not installed"));
    }
    let version = execute(config, &dir, &config.tesseract, &["--version"], 65536).await?;
    let version = String::from_utf8_lossy(&version)
        .lines()
        .next()
        .unwrap_or("unknown")
        .to_owned();
    let output = execute(
        config,
        &dir,
        &config.tesseract,
        &["page.png", "stdout", "-l", language, "--psm", "3", "tsv"],
        4_194_304,
    )
    .await?;
    let text = String::from_utf8(output).map_err(|_| Error::bad("OCR returned invalid UTF-8"))?;
    Ok(Extracted {
        geometry: document_geometry::tsv(&text)?,
        metadata: json!({"method":"tesseract-ocr","engine_version":version,"coordinate_system":"image_pixels","language":language,"ocr_performed":true,"segmentation_mode":3,"raster_max_dimension":1400,"handwriting_status":"unknown","review_status":"unverified"}),
    })
}

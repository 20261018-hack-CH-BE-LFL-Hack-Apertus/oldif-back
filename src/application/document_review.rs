use crate::{
    application::{auth::User, documents, runs},
    error::{Error, Result},
    infrastructure::store::Write,
    state::App,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Annotation {
    pub page: usize,
    pub labels: Vec<String>,
    pub bias: String,
    pub missingness: String,
    pub handwriting_status: String,
    pub legibility_status: String,
    pub note: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub expected_version: i64,
    pub decision: String,
    pub reason: String,
    pub title: Option<String>,
    pub labels: Option<Vec<String>>,
    pub metadata: Option<BTreeMap<String, String>>,
    pub annotations: Option<Vec<Annotation>>,
}
pub async fn review(app: &App, owner: &str, id: &str, input: Review) -> Result<Value> {
    let document = documents::get(app, owner, id).await?;
    if input.expected_version != document.version {
        return Err(Error(
            axum::http::StatusCode::CONFLICT,
            "Reload the current document revision before reviewing".into(),
        ));
    }
    let data = revise(document.data.clone(), owner, &input)?;
    let review = json!({"document_id":id,"actor":owner,"decision":input.decision,"reason":input.reason,"previous_version":document.version,"resulting_version":document.version+1,"changes":{"title":data["title"],"labels":data["labels"],"metadata":data["metadata"],"annotations":data["annotations"]},"original_modified":false});
    let rows = app
        .store
        .batch(vec![
            Write::update(&document, data),
            Write::new(owner, "review", &uuid::Uuid::new_v4().to_string(), review),
        ])
        .await?;
    Ok(json!({"document":rows[0].public(),"revision":rows[1].public()}))
}
fn revise(mut data: Value, owner: &str, input: &Review) -> Result<Value> {
    if !["accepted", "rejected", "needs_review"].contains(&input.decision.as_str())
        || input.reason.trim().is_empty()
        || input.reason.len() > 4000
    {
        return Err(Error::bad(
            "Choose a review decision and provide a bounded reason",
        ));
    }
    if let Some(title) = &input.title {
        if title.trim().is_empty() || title.len() > 500 {
            return Err(Error::bad("Title must contain 1 to 500 bytes"));
        }
        data["title"] = json!(title);
        data["title_method"] = json!("human_review");
    }
    if let Some(values) = &input.labels {
        labels(values)?;
        data["labels"] = json!(values);
    }
    if let Some(metadata) = &input.metadata {
        if metadata.len() > 30
            || metadata
                .iter()
                .any(|(k, v)| k.is_empty() || k.len() > 80 || v.len() > 1000)
        {
            return Err(Error::bad("Metadata exceeds its field limits"));
        }
        data["metadata"] = json!(metadata);
    }
    if let Some(annotations) = &input.annotations {
        data["annotations"] = annotations_value(
            annotations,
            data["page_count"].as_u64().unwrap_or(0) as usize,
        )?;
    }
    data["review"] =
        json!({"decision":input.decision,"reason":input.reason,"actor":owner,"human_review":true});
    Ok(data)
}
fn labels(values: &[String]) -> Result<()> {
    if values.len() > 30 || values.iter().any(|v| v.trim().is_empty() || v.len() > 100) {
        return Err(Error::bad("Labels exceed the allowed count or length"));
    }
    Ok(())
}
fn annotations_value(values: &[Annotation], page_count: usize) -> Result<Value> {
    let mut seen = HashSet::new();
    let mut result = vec![];
    for a in values {
        if a.page == 0
            || a.page > page_count
            || !seen.insert(a.page)
            || a.bias.len() > 2000
            || a.missingness.len() > 2000
            || a.note.len() > 4000
            || !["unknown", "handwritten", "printed", "mixed"]
                .contains(&a.handwriting_status.as_str())
            || !["unknown", "legible", "partially_legible", "illegible"]
                .contains(&a.legibility_status.as_str())
        {
            return Err(Error::bad("Invalid or duplicate page annotation"));
        }
        labels(&a.labels)?;
        result.push(json!({"page":a.page,"labels":a.labels,"bias":a.bias,"missingness":a.missingness,"handwriting_status":a.handwriting_status,"legibility_status":a.legibility_status,"note":a.note,"method":"human_annotation"}));
    }
    Ok(json!(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enrichment {
    pub model: Option<String>,
    pub language: Option<String>,
    pub provider_processing_confirmed: bool,
    pub page_numbers: Option<Vec<usize>>,
    pub source_ids: Option<Vec<String>>,
}
pub async fn enrich(app: &App, user: &User, id: &str, input: Enrichment) -> Result<Value> {
    if !input.provider_processing_confirmed {
        return Err(Error::bad(
            "Confirm that selected page text may be sent to the model provider",
        ));
    }
    let document = documents::get(app, &user.id, id).await?;
    if document.data["status"] != "ready" {
        return Err(Error::bad("Document is not ready"));
    }
    let pages = documents::pages(app, &user.id, id).await?;
    let selection = if let Some(ids) = input.source_ids {
        if ids.is_empty() || ids.len() > 6 {
            return Err(Error::bad("Choose one to six exact document sources"));
        }
        crate::application::sources::select(
            app,
            &user.id,
            "document enrichment",
            &[id.into()],
            &ids,
        )
        .await?
    } else {
        selected_pages(&pages, input.page_numbers.as_deref())?
    };
    if selection
        .iter()
        .map(|page| page["text"].as_str().unwrap_or("").len())
        .sum::<usize>()
        > 80000
    {
        return Err(Error::bad(
            "Selected pages exceed the bounded model context; choose fewer or smaller pages",
        ));
    }
    let ids = selection
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let model = input.model.unwrap_or_else(|| "apertus-v1-5-8b".into());
    let language = input.language.unwrap_or_else(|| "en".into());
    let message="From the selected document pages, identify a concise title and only directly attested dates, organizations, or people. Quote the exact source passages. Distinguish document claims from verified events. State missing metadata and potential selection or transcription bias. Do not invent fields, infer outcomes, or treat the title as evidence. This is a metadata proposal requiring human review.".to_owned();
    let started = runs::start(
        app,
        user,
        runs::Input {
            message,
            model,
            language,
            session_id: None,
            dataset_ids: vec![id.into()],
            source_ids: ids,
            provider_processing_confirmed: true,
        },
    )
    .await?;
    let latest = documents::get(app, &user.id, id).await?;
    let mut data = latest.data.clone();
    data["enrichments"].as_array_mut().ok_or_else(Error::unavailable)?.push(json!({"run_id":started["run_id"],"session_id":started["session_id"],"page_numbers":selection.iter().map(|p|p["page"].clone()).collect::<Vec<_>>(),"state":"pending_human_review","applied":false}));
    app.store.put(Write::update(&latest, data)).await?;
    Ok(started)
}
fn selected_pages(pages: &[Value], requested: Option<&[usize]>) -> Result<Vec<Value>> {
    let numbers = requested.map(|n| n.to_vec()).unwrap_or_else(|| {
        pages
            .iter()
            .find(|p| !p["text"].as_str().unwrap_or("").trim().is_empty())
            .map(|p| vec![p["page"].as_u64().unwrap() as usize])
            .unwrap_or_default()
    });
    if numbers.is_empty()
        || numbers.len() > 6
        || numbers.iter().collect::<HashSet<_>>().len() != numbers.len()
    {
        return Err(Error::bad(
            "Choose between one and six distinct nonempty pages",
        ));
    }
    numbers
        .iter()
        .map(|n| {
            pages
                .iter()
                .find(|p| p["page"] == *n && !p["text"].as_str().unwrap_or("").trim().is_empty())
                .cloned()
                .ok_or_else(|| Error::bad("Selected page is missing or contains no extracted text"))
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn human_revisions_preserve_original_and_omit_unspecified_fields() {
        let source = json!({"original_sha256":"original","title":"Extracted","labels":["archive"],"page_count":2,"metadata":{},"annotations":[]});
        let input:Review=serde_json::from_value(json!({"expected_version":2,"decision":"accepted","reason":"Compared to original","title":"Corrected"})).unwrap();
        let changed = revise(source, "owner", &input).unwrap();
        assert_eq!(changed["original_sha256"], "original");
        assert_eq!(changed["labels"], json!(["archive"]));
        assert_eq!(changed["title"], "Corrected");
    }
    #[test]
    fn enrichment_rejects_missing_empty_or_excess_pages() {
        let pages = vec![
            json!({"page":1,"text":""}),
            json!({"page":2,"text":"facts"}),
        ];
        assert_eq!(selected_pages(&pages, None).unwrap()[0]["page"], 2);
        assert!(selected_pages(&pages, Some(&[1])).is_err());
        assert!(selected_pages(&pages, Some(&[2, 2])).is_err());
    }
}

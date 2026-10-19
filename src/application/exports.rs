use crate::{
    application::auth::hash,
    domain::tabular_cell,
    error::{Error, Result},
    infrastructure::store::Write,
    state::App,
};
use serde_json::{Value, json};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewInput {pub decision:String,pub reason:String,pub expected_version:i64}
pub async fn review(app: &App, owner: &str, run_id: &str, input: ReviewInput) -> Result<Value> {
    let run = app.store.get(owner, "run", run_id).await?;
    let decision = input.decision.as_str();
    let reason = input.reason.trim();
    if !["accepted", "rejected"].contains(&decision) || reason.is_empty() || reason.len() > 4000 {
        return Err(Error::bad("A review decision and reason are required"));
    }
    if input.expected_version != run.version {
        return Err(Error(
            axum::http::StatusCode::CONFLICT,
            "Reload the current run revision before reviewing".into(),
        ));
    }
    if run.data["status"] != "complete" {
        return Err(Error::bad("Only completed runs can be reviewed"));
    }
    let mut data = run.data.clone();
    let normalized=json!({"decision":decision,"reason":reason,"actor":owner});
    let previous=if run.data["review"].is_object(){json!({"decision":run.data["review"]["decision"],"reason":run.data["review"]["reason"],"actor":run.data["review"]["actor"]})}else{Value::Null};
    let transform=super::provenance::transformation("human_research_review",reason,owner,run_id,json!({"kind":"run","id":run_id,"version":run.version}),previous,normalized.clone());
    data["review"]=normalized;data["review"]["transformation"]=transform.clone();
    let review_id = Uuid::new_v4().to_string();
    let records=app.store.batch(vec![Write::update(&run,data),Write::new(owner,"review",&review_id,json!({"run_id":run_id,"decision":decision,"reason":reason,"actor":owner,"previous_version":run.version,"resulting_version":run.version+1,"transformation":transform}))]).await?;
    Ok(records[0].public())
}
pub async fn accepted(app: &App, owner: &str, session: Option<&str>) -> Result<Vec<Value>> {
    let mut runs = app.store.list(owner, "run").await?;
    runs.retain(|r| {
        r.data["review"]["decision"] == "accepted"
            && session.is_none_or(|s| r.data["session_id"] == s)
    });
    runs.sort_by(|a, b| a.id.cmp(&b.id));
    for run in &runs {
        if let Some(sources)=run.data["sources"].as_array() {super::document_quarantine::authorize_sources(app,owner,sources,"private_export").await?;}
    }
    let rows=runs.iter().map(|r|json!({"schema_version":"1.0","run_id":r.id,"revision":r.version,"result":r.data["result"],"review":r.data["review"],"model":r.data["model"]})).collect::<Vec<_>>();
    let bytes = serde_json::to_string(&rows)?;
    let id = hash(&bytes);
    if app.store.get(owner, "export", &id).await.is_err() {
        app.store
            .put(Write::new(
                owner,
                "export",
                &id,
                json!({"sha256":id,"items":rows,"count":rows.len()}),
            ))
            .await?;
    }
    Ok(rows)
}
pub enum ExportKind {
    Sources,
    Accepted,
}
pub fn serialize(rows: &[Value], format: &str, kind: ExportKind) -> Result<(String, &'static str)> {
    match format {
        "json" => Ok((serde_json::to_string_pretty(rows)?, "application/json")),
        "jsonl" => Ok((
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
            "application/x-ndjson",
        )),
        "tsv" | "csv" => {
            let mut lines = vec![match kind {
                ExportKind::Sources => "id\ttitle\ttext\tsource_url\tsha256\tlicence",
                ExportKind::Accepted => "run_id\trevision\tmodel\tanswer\tcitations_json\treview_decision\treview_reason\treview_actor\tevidence_only\tinterpretation_status\tverification_verdict",
            }.to_string()];
            if format == "csv" { lines[0] = lines[0].replace('\t', ","); }
            for row in rows {
                let cells = match kind {
                    ExportKind::Sources => ["id", "title", "text", "url", "sha256", "licence"]
                        .iter()
                        .map(|k| row[*k].as_str().unwrap_or("").to_owned())
                        .collect::<Vec<_>>(),
                    ExportKind::Accepted => vec![
                        row["run_id"].as_str().unwrap_or("").into(),
                        row["revision"].to_string(),
                        row["model"].as_str().unwrap_or("").into(),
                        row["result"]["answer"].as_str().unwrap_or("").into(),
                        row["result"]["citations"].to_string(),
                        row["review"]["decision"].as_str().unwrap_or("").into(),
                        row["review"]["reason"].as_str().unwrap_or("").into(),
                        row["review"]["actor"].as_str().unwrap_or("").into(),
                        row["result"]["evidence_only"]
                            .as_bool()
                            .map(|value| value.to_string())
                            .unwrap_or_default(),
                        row["result"]["interpretation_status"]
                            .as_str()
                            .unwrap_or("")
                            .into(),
                        row["result"]["verification"]["verdict"]
                            .as_str()
                            .unwrap_or("")
                            .into(),
                    ],
                };
                lines.push(
                    cells
                        .iter()
                        .map(|value| if format == "csv" { csv_cell(value) } else { tabular_cell(value) })
                        .collect::<Vec<_>>()
                        .join(if format == "csv" { "," } else { "\t" }),
                );
            }
            Ok((
                lines.join("\n") + "\n",
                if format == "csv" { "text/csv; charset=utf-8" } else { "text/tab-separated-values; charset=utf-8" },
            ))
        }
        _ => Err(Error::bad("Choose json, jsonl, csv, or tsv")),
    }
}
fn csv_cell(value: &str) -> String {
    let safe = if value.trim_start().starts_with(['=', '+', '-', '@']) { format!("'{value}") } else { value.to_owned() };
    format!("\"{}\"", safe.replace('"', "\"\""))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepted_tsv_preserves_review_and_citations() {
        let row = json!({"run_id":"run-1","revision":3,"model":"apertus-v1-5-8b","result":{"answer":"=a\tb\nc","citations":[{"source_id":"stapfer-2027","quote":"läsen"}]},"review":{"decision":"accepted","reason":"Checked against source","actor":"user-1"}});
        let (text, _) = serialize(&[row], "tsv", ExportKind::Accepted).unwrap();
        let lines = text.lines().collect::<Vec<_>>();
        let cells = lines[1].split('\t').collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert_eq!(cells.len(), 11);
        assert_eq!(&cells[..4], &["run-1", "3", "apertus-v1-5-8b", "'=a b c"]);
        assert_eq!(
            serde_json::from_str::<Value>(cells[4]).unwrap()[0]["source_id"],
            "stapfer-2027"
        );
        assert_eq!(
            &cells[5..8],
            &["accepted", "Checked against source", "user-1"]
        );
        assert_eq!(&cells[8..], &["", "", ""]);
    }
    #[test]
    fn empty_exports_keep_the_requested_columns() {
        assert!(
            serialize(&[], "tsv", ExportKind::Accepted)
                .unwrap()
                .0
                .starts_with("run_id\t")
        );
        assert_eq!(csv_cell("=formula\n\"quoted\""), "\"'=formula\n\"\"quoted\"\"\"");
        assert!(serialize(&[], "csv", ExportKind::Accepted).unwrap().0.starts_with("run_id,revision,model,"));
        assert!(
            serialize(&[], "tsv", ExportKind::Sources)
                .unwrap()
                .0
                .starts_with("id\ttitle\t")
        );
    }
    #[test]
    fn accepted_tsv_identifies_unverified_interpretations() {
        let row = json!({"run_id":"evidence-1","revision":4,"model":"apertus-v1-5-8b","result":{"answer":"Selected passages require human review.","citations":[{"source_id":"stapfer-2027","passage_id":"p048","quote":"läsen"}],"evidence_only":true,"interpretation_status":"needs_review","verification":{"verdict":"needs_revision"}},"review":{"decision":"accepted","reason":"Accept exact evidence only","actor":"reviewer"}});
        let (text, _) = serialize(&[row], "tsv", ExportKind::Accepted).unwrap();
        let mut lines = text.lines();
        let headers = lines.next().unwrap().split('\t').collect::<Vec<_>>();
        let cells = lines.next().unwrap().split('\t').collect::<Vec<_>>();
        assert_eq!(headers.len(), cells.len());
        for (name, expected) in [
            ("evidence_only", "true"),
            ("interpretation_status", "needs_review"),
            ("verification_verdict", "needs_revision"),
        ] {
            let index = headers.iter().position(|header| *header == name).unwrap();
            assert_eq!(cells[index], expected);
        }
    }
}
pub async fn dataset(app: &App, owner: &str, id: &str) -> Result<Vec<Value>> {
    let scope=match app.store.get(owner,"dataset",id).await {
        Ok(_)=>owner,
        Err(error) if error.0==axum::http::StatusCode::NOT_FOUND=>{app.store.get("public","dataset",id).await?;"public"},
        Err(error)=>return Err(error),
    };
    let mut rows = app.store.list(scope,"source").await?
        .into_iter()
        .filter(|r| r.data["dataset_id"] == id)
        .map(|r| r.public())
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    if rows.is_empty() {
        return Err(Error::missing());
    }
    Ok(rows)
}

pub async fn bundle_accepted(app:&App,owner:&str,session:Option<&str>,scope:&str)->Result<Value>{
    let mut runs=app.store.list(owner,"run").await?;runs.retain(|r|session.is_none_or(|id|r.data["session_id"]==id));runs.sort_by(|a,b|a.id.cmp(&b.id));
    let mut rows=vec![];let mut source_rows=vec![];let mut changes=vec![];let mut rejected=vec![];
    for run in runs {
        if run.data["review"]["decision"]!="accepted" {rejected.push(json!({"run_id":run.id,"reason":"REVIEW_NOT_ACCEPTED"}));continue;}
        let sources=run.data["sources"].as_array().cloned().unwrap_or_default();
        if scope=="public"&&!public_sources(app,owner,&sources).await? {rejected.push(json!({"run_id":run.id,"reason":"PUBLIC_RIGHTS_OR_SOURCE_REVIEW_UNRESOLVED"}));continue;}
        super::document_quarantine::authorize_sources(app,owner,&sources,if scope=="public"{"public_export"}else{"private_export"}).await?;
        let row=json!({"schema_version":"1.0","run_id":run.id,"revision":run.version,"result":run.data["result"],"review":run.data["review"],"model":run.data["model"]});
        rows.push(if scope=="public"{super::export_bundle::allowlist(&row,"research_result")}else{row});
        source_rows.extend(sources);
        changes.push(if scope=="public"{json!({"run_id":run.id,"revision":run.version,"decision":"accepted"})}else{json!({"run_id":run.id,"revision":run.version,"review":run.data["review"],"transformation":run.data["transformation"]})});
    }
    source_rows.sort_by(|a,b|a["id"].as_str().cmp(&b["id"].as_str()));source_rows.dedup_by(|a,b|a["id"]==b["id"]);
    super::export_bundle::build(&rows,"research_result",scope,super::export_bundle::source_manifest(&source_rows),json!(changes),json!(rejected))
}
pub async fn bundle_dataset(app:&App,owner:&str,id:&str,scope:&str)->Result<Value>{
    match app.store.get(owner,"document",id).await {
        Ok(_)=>return Ok(serde_json::from_str(&super::document_export::export(app,owner,id,"bundle",scope).await?.0)?),
        Err(error) if error.0==axum::http::StatusCode::NOT_FOUND=>{},Err(error)=>return Err(error),
    }
    let candidates=dataset(app,owner,id).await?;let mut rows=vec![];let mut rejected=vec![];
    for row in candidates {if scope=="public"&&!public_sources(app,owner,std::slice::from_ref(&row)).await? {rejected.push(json!({"source_id":row["id"],"reason":"PUBLIC_RIGHTS_OR_SOURCE_REVIEW_UNRESOLVED"}));}else{rows.push(if scope=="public"{super::export_bundle::allowlist(&row,"source")}else{row});}}
    let changes=json!(rows.iter().map(|r|json!({"source_id":r["id"],"normalization_provenance":super::provenance::normalization(r)})).collect::<Vec<_>>());
    super::export_bundle::build(&rows,"source",scope,super::export_bundle::source_manifest(&rows),if scope=="public"{json!([])}else{changes},json!(rejected))
}
async fn public_sources(app:&App,owner:&str,sources:&[Value])->Result<bool>{
    if sources.is_empty(){return Ok(false);}
    for source in sources {
        let Some(id)=source["document_id"].as_str() else{return Ok(false);};
        let document=super::documents::get(app,owner,id).await?;
        if document.data["status"]!="ready"||document.data["review"]["decision"]!="accepted"||document.data["review"]["human_review"]!=true||!document.data["acquisition"]["allowed_uses"].as_array().is_some_and(|uses|uses.iter().any(|u|u=="public_export")){return Ok(false);}
        if source["transcription_method"].as_str().is_some_and(|method|method=="tesseract-ocr")&&source["transcription_review_status"]!="accepted"{return Ok(false);}
    }
    Ok(true)
}

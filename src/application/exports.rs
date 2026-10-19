use crate::{
    application::{auth::hash, sources},
    domain::tabular_cell,
    error::{Error, Result},
    infrastructure::store::Write,
    state::App,
};
use serde_json::{Value, json};
use uuid::Uuid;

pub async fn review(app: &App, owner: &str, run_id: &str, input: Value) -> Result<Value> {
    let run = app.store.get(owner, "run", run_id).await?;
    let decision = input["decision"].as_str().unwrap_or("");
    let reason = input["reason"].as_str().unwrap_or("").trim();
    if !["accepted", "rejected"].contains(&decision) || reason.is_empty() || reason.len() > 4000 {
        return Err(Error::bad("A review decision and reason are required"));
    }
    if input["expected_version"].as_i64() != Some(run.version) {
        return Err(Error(
            axum::http::StatusCode::CONFLICT,
            "Reload the current run revision before reviewing".into(),
        ));
    }
    if run.data["status"] != "complete" {
        return Err(Error::bad("Only completed runs can be reviewed"));
    }
    let mut data = run.data.clone();
    data["review"] = json!({"decision":decision,"reason":reason,"actor":owner});
    let review_id = Uuid::new_v4().to_string();
    let records=app.store.batch(vec![Write::update(&run,data),Write::new(owner,"review",&review_id,json!({"run_id":run_id,"decision":decision,"reason":reason,"actor":owner,"previous_version":run.version,"resulting_version":run.version+1}))]).await?;
    Ok(records[0].public())
}
pub async fn accepted(app: &App, owner: &str, session: Option<&str>) -> Result<Vec<Value>> {
    let mut runs = app.store.list(owner, "run").await?;
    runs.retain(|r| {
        r.data["review"]["decision"] == "accepted"
            && session.is_none_or(|s| r.data["session_id"] == s)
    });
    runs.sort_by(|a, b| a.id.cmp(&b.id));
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
pub fn serialize(rows: &[Value], format: &str) -> Result<(String, &'static str)> {
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
        "tsv" => {
            let mut lines = vec!["id\ttitle\ttext\tsource_url\tsha256\tlicence".to_string()];
            for row in rows {
                lines.push(
                    ["id", "title", "text", "url", "sha256", "licence"]
                        .iter()
                        .map(|k| tabular_cell(row[*k].as_str().unwrap_or("")))
                        .collect::<Vec<_>>()
                        .join("\t"),
                );
            }
            Ok((
                lines.join("\n") + "\n",
                "text/tab-separated-values; charset=utf-8",
            ))
        }
        _ => Err(Error::bad("Choose json, jsonl, or tsv")),
    }
}
pub async fn dataset(app: &App, owner: &str, id: &str) -> Result<Vec<Value>> {
    let mut rows = sources::all(app, owner)
        .await?
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

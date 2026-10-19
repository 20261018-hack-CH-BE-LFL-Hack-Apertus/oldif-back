use crate::{
    application::{
        auth::{User, limit, now},
        sources,
    },
    domain,
    error::{Error, Result},
    infrastructure::store::Write,
    state::App,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
pub struct Input {
    pub message: String,
    #[serde(default = "default_model")]
    pub model: String,
    pub language: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub dataset_ids: Vec<String>,
    #[serde(default)]
    pub source_ids: Vec<String>,
    #[serde(default)]
    pub provider_processing_confirmed: bool,
}
pub fn default_model() -> String {
    "apertus-v1-5-70b".into()
}
pub async fn session(app: &App, user: &User, title: &str, language: &str) -> Result<Value> {
    if !domain::valid_locale(language) {
        return Err(Error::bad("Unsupported language"));
    }
    let id = Uuid::new_v4().to_string();
    let row=app.store.put(Write::new(&user.id,"conversation",&id,json!({"title":title.chars().take(100).collect::<String>(),"language":language,"updated_at":now()}))).await?;
    Ok(row.public())
}
pub async fn start(app: &App, user: &User, input: Input) -> Result<Value> {
    validate(&input)?;
    limit(app, &format!("run:{}", user.id), 15).await?;
    let permit = app.slots.clone().try_acquire_owned().map_err(|_| {
        Error(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "All research workers are busy; retry shortly".into(),
        )
    })?;
    let sid = if let Some(id) = &input.session_id {
        app.store.get(&user.id, "conversation", id).await?;
        id.clone()
    } else {
        session(app, user, &input.message, &input.language).await?["id"]
            .as_str()
            .ok_or_else(Error::unavailable)?
            .into()
    };
    let selected = sources::select(app, &user.id, &input.message, &input.dataset_ids, &input.source_ids).await?;
    if !input.provider_processing_confirmed && selected.iter().any(|source|source["document_id"].is_string()) {
        return Err(Error::bad("Confirm provider processing before researching private uploaded pages"));
    }
    super::document_quarantine::authorize_sources(app,&user.id,&selected,"provider_processing").await?;
    if selected.is_empty() {
        return Err(Error::bad(
            "No matching source was found. Name a school or an exact source identifier from the collection",
        ));
    }
    let history = history(app, &user.id, &sid).await?;
    let source_input = selected.iter().map(core_source).collect::<Vec<_>>();
    let created=app.core.run(json!({"question":input.message,"model":input.model,"locale":input.language,"sources":source_input,"session_id":sid,"user_id":user.id,"history":history})).await?;
    let id = created["run_id"]
        .as_str()
        .ok_or_else(Error::unavailable)?
        .to_owned();
    let run = json!({"run_id":id,"session_id":sid,"status":"running","model":input.model,"language":input.language,"question":input.message,"sources":selected,"started_at":now()});
    app.store
        .batch(vec![
            Write::new(&user.id, "run", &id, run),
            Write::new(
                &user.id,
                "message",
                &Uuid::new_v4().to_string(),
                json!({"session_id":sid,"role":"user","content":input.message,"run_id":id}),
            ),
        ])
        .await?;
    emit(app,&user.id,&sid,&id,"started",json!({"model":input.model,"retrieved_sources":selected.len(),"retrieval_method":selected[0]["retrieval_method"],"source_ids":selected.iter().map(|s|s["id"].clone()).collect::<Vec<_>>()})).await?;
    let next_app = app.clone();
    let owner = user.id.clone();
    let rid = id.clone();
    let conversation = sid.clone();
    tokio::spawn(async move {
        let _permit = permit;
        if let Err(error) = relay(&next_app, &owner, &conversation, &rid).await {
            let _ = failed(&next_app, &owner, &conversation, &rid, &error.1).await;
        }
    });
    Ok(json!({"run_id":id,"session_id":sid,"status":"running"}))
}
fn validate(input: &Input) -> Result<()> {
    if !domain::valid_question(&input.message)
        || !domain::valid_locale(&input.language)
        || !domain::valid_id(&input.model)
        || input.dataset_ids.len() > 20
        || input.source_ids.len() > 6
    {
        return Err(Error::bad(
            "Invalid message, model, language, or dataset selection",
        ));
    }
    Ok(())
}
fn core_source(row: &Value) -> Value {
    let text = row["text"].as_str().unwrap_or("");
    // The core receives complete records so exact-quote validation uses the original text.
    json!({"id":row["id"],"title":row["title"],"text":text,"url":row["url"],"page":row["page"],"sha256":row["sha256"],"text_sha256":row["text_sha256"],"record_hash_scope":row["record_hash_scope"],"licence":row["licence"],"transcription_method":row["transcription_method"],"transcription_review_status":row["transcription_review_status"],"file_page_index":row["file_page_index"],"printed_page_label":row["printed_page_label"],"printed_page_label_missing_reason":row["printed_page_label_missing_reason"]})
}
async fn history(app: &App, owner: &str, sid: &str) -> Result<Vec<Value>> {
    let rows = app.store.list(owner, "message").await?;
    let mut messages = rows
        .into_iter()
        .filter(|r| r.data["session_id"] == sid)
        .map(|r| json!({"role":r.data["role"],"content":r.data["content"]}))
        .collect::<Vec<_>>();
    if messages.len() > 10 {
        messages = messages.split_off(messages.len() - 10);
    }
    Ok(messages)
}
pub async fn emit(
    app: &App,
    owner: &str,
    sid: &str,
    run_id: &str,
    kind: &str,
    data: Value,
) -> Result<Value> {
    let _lock = app.event_lock.lock().await;
    let event = json!({"type":kind,"run_id":run_id,"session_id":sid,"sequence":0,"data":data,"created_at":now()});
    let committed = app
        .store
        .put(Write::new(
            owner,
            "event",
            &Uuid::new_v4().to_string(),
            event,
        ))
        .await?;
    let event = committed.data;
    let _ = app.events.send((owner.into(), event.clone()));
    Ok(event)
}
async fn relay(app: &App, owner: &str, sid: &str, id: &str) -> Result<()> {
    let mut stream = app.core.events(id, 0).await?;
    let mut terminal = false;
    while let Some(message) = stream.message().await? {
        let event: Value = serde_json::from_str(&message.json)?;
        let kind = event["kind"].as_str().unwrap_or("trace");
        if kind == "run.completed" {
            let status = event["data"]["status"].as_str().unwrap_or("failed");
            if status == "needs_review" || status == "complete" {
                finish(app, owner, sid, id, event["data"]["result"].clone()).await?;
            } else if status == "cancelled" {
                cancelled(app, owner, sid, id).await?;
            } else {
                failed(
                    app,
                    owner,
                    sid,
                    id,
                    event["data"]["result"]["error"]
                        .as_str()
                        .unwrap_or("Run failed"),
                )
                .await?;
            }
            terminal = true;
            break;
        }
        if matches!(kind, "done" | "complete" | "result") {
            finish(app, owner, sid, id, event["data"].clone()).await?;
            terminal = true;
            break;
        }
        if kind == "cancelled" {
            cancelled(app, owner, sid, id).await?;
            terminal = true;
            break;
        }
        if matches!(kind, "error" | "failed") {
            failed(
                app,
                owner,
                sid,
                id,
                event["data"]["message"].as_str().unwrap_or("Run failed"),
            )
            .await?;
            terminal = true;
            break;
        }
        let mut data = event["data"].clone();
        if !data.is_object() {
            data = json!({"value":data});
        }
        data["kind"] = json!(kind);
        data["agent"] = event["agent"].clone();
        data["core_sequence"] = event["sequence"].clone();
        emit(
            app,
            owner,
            sid,
            id,
            if kind == "delta" { "delta" } else { "trace" },
            data,
        )
        .await?;
    }
    if !terminal {
        return Err(Error::bad(
            "The model event stream ended before a result was committed",
        ));
    }
    Ok(())
}
async fn finish(app: &App, owner: &str, sid: &str, id: &str, mut result: Value) -> Result<()> {
    let row = app.store.get(owner, "run", id).await?;
    let mut data = row.data.clone();
    data["status"] = json!("complete");
    data["result"] = result.clone();
    data["finished_at"] = json!(now());
    result["review_version"] = json!(row.version + 1);
    let content = result["answer"].as_str().unwrap_or("");
    app.store.batch(vec![Write::update(&row,data),Write::new(owner,"message",&Uuid::new_v4().to_string(),json!({"session_id":sid,"role":"assistant","content":content,"result":result,"run_id":id}))]).await?;
    emit(app, owner, sid, id, "evidence", result.clone()).await?;
    emit(app, owner, sid, id, "done", result).await?;
    Ok(())
}
async fn failed(app: &App, owner: &str, sid: &str, id: &str, message: &str) -> Result<()> {
    let row = app.store.get(owner, "run", id).await?;
    let mut data = row.data.clone();
    data["status"] = json!("failed");
    data["error"] = json!(message);
    data["finished_at"] = json!(now());
    app.store.put(Write::update(&row, data)).await?;
    emit(app, owner, sid, id, "error", json!({"message":message})).await?;
    Ok(())
}
async fn cancelled(app: &App, owner: &str, sid: &str, id: &str) -> Result<()> {
    let row = app.store.get(owner, "run", id).await?;
    let mut data = row.data.clone();
    data["status"] = json!("cancelled");
    data["finished_at"] = json!(now());
    app.store.put(Write::update(&row, data)).await?;
    emit(
        app,
        owner,
        sid,
        id,
        "cancelled",
        json!({"message":"Run cancelled"}),
    )
    .await?;
    Ok(())
}
pub async fn control(
    app: &App,
    user: &User,
    id: &str,
    action: &str,
    message: &str,
) -> Result<Value> {
    let run = app.store.get(&user.id, "run", id).await?;
    if !["steer", "cancel"].contains(&action)
        || (action == "steer" && !domain::valid_question(message))
    {
        return Err(Error::bad("Invalid run action"));
    }
    let response = app
        .client
        .post(format!("{}/v1/runs/{id}/{action}", app.config.core_url))
        .bearer_auth(&app.config.token)
        .json(&json!({"message":message,"instruction":message}))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(Error::bad("This run no longer accepts that action"));
    }
    let sid = run.data["session_id"]
        .as_str()
        .ok_or_else(Error::unavailable)?;
    emit(
        app,
        &user.id,
        sid,
        id,
        "trace",
        json!({"kind":action,"agent":"user","message":message}),
    )
    .await?;
    Ok(json!({"status":action}))
}

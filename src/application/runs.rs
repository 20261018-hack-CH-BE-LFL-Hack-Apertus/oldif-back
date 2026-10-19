use crate::{
    application::{
        auth::{User, limit, now},
        run_requests, run_events,
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
#[serde(deny_unknown_fields)]
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
    #[serde(default)]
    pub idempotency_key: Option<String>,
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
pub async fn start(app:&App,user:&User,mut input:Input)->Result<Value> {
    if input.idempotency_key.is_none() {input.idempotency_key=Some(Uuid::new_v4().to_string());}
    validate(&input)?;
    limit(app,&format!("run:{}",user.id),15).await?;
    let job=run_requests::reserve(app,user,&input).await?;
    if let Some(run)=run_requests::existing_run(app,&user.id,&job).await? {
        if run.data["status"]!="running" {run_events::restore_terminal(app,&user.id,&run).await?;return Ok(response(&run,true));}
        if RelayGuard::active(&user.id,&run.id) {return Ok(response(&run,true));}
        let permit=worker(app)?;spawn_relay(app,user,&run,permit);return Ok(response(&run,true));
    }
    let permit=worker(app)?;
    let job=run_requests::prepare(app,user,job,&input).await?;
    let created=app.core.run(job.data["core_request"].clone()).await?;
    let id=created["run_id"].as_str().ok_or_else(Error::unavailable)?;
    let run=run_requests::commit(app,user,&job,id).await?;
    let selected=run.data["sources"].as_array().ok_or_else(Error::unavailable)?;
    let sid=run.data["session_id"].as_str().ok_or_else(Error::unavailable)?;
    run_events::emit_id(app,&user.id,sid,id,"started",json!({"model":input.model,"retrieved_sources":selected.len(),"retrieval_method":selected[0]["retrieval_method"],"source_ids":selected.iter().map(|row|row["id"].clone()).collect::<Vec<_>>()}),&format!("backend:{id}:started")).await?;
    if run.data["status"]=="running" {spawn_relay(app,user,&run,permit);}else{run_events::restore_terminal(app,&user.id,&run).await?;}
    Ok(response(&run,false))
}
fn response(run:&crate::infrastructure::store::Record,replay:bool)->Value {json!({"run_id":run.id,"session_id":run.data["session_id"],"status":run.data["status"],"idempotent_replay":replay})}
fn worker(app:&App)->Result<tokio::sync::OwnedSemaphorePermit> {
    app.slots.clone().try_acquire_owned().map_err(|_|Error(axum::http::StatusCode::TOO_MANY_REQUESTS,"All research workers are busy; retry shortly".into()))
}
static RELAYS:std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>>=std::sync::OnceLock::new();
struct RelayGuard(String);
impl RelayGuard {
    fn active(owner:&str,id:&str)->bool {RELAYS.get_or_init(Default::default).lock().unwrap().contains(&format!("{owner}:{id}"))}
    fn claim(owner:&str,id:&str)->Option<Self> {let key=format!("{owner}:{id}");let inserted=RELAYS.get_or_init(Default::default).lock().unwrap().insert(key.clone());if inserted{Some(Self(key))}else{None}}
}
impl Drop for RelayGuard {fn drop(&mut self){RELAYS.get_or_init(Default::default).lock().unwrap().remove(&self.0);}}
fn spawn_relay(app:&App,user:&User,run:&crate::infrastructure::store::Record,permit:tokio::sync::OwnedSemaphorePermit) {
    let Some(guard)=RelayGuard::claim(&user.id,&run.id) else{return;};
    let app=app.clone();let owner=user.id.clone();let id=run.id.clone();let sid=run.data["session_id"].as_str().unwrap_or("").to_owned();
    tokio::spawn(async move {
        let (_guard,_permit)=(guard,permit);
        if let Err(error)=relay(&app,&owner,&sid,&id).await {let _=run_events::stop(&app,&owner,&sid,&id,"failed",&error.1,&format!("backend:{id}:transport-error"),error.code()).await;}
    });
}
fn validate(input: &Input) -> Result<()> {
    if !domain::valid_question(&input.message)
        || !domain::valid_locale(&input.language)
        || !domain::valid_id(&input.model)
        || input.dataset_ids.len() > 20
        || input.source_ids.len() > 6
        || input.dataset_ids.iter().chain(input.source_ids.iter()).any(|id|!domain::valid_id(id))
        || input.session_id.as_ref().is_some_and(|id|!domain::valid_id(id))
        || input.idempotency_key.as_ref().is_none_or(|key|key.len()>128 || !domain::valid_id(key))
    {
        return Err(Error::bad(
            "Invalid message, model, language, or dataset selection",
        ));
    }
    Ok(())
}
pub(super) fn core_source(row: &Value) -> Value {
    let text = row["text"].as_str().unwrap_or("");
    // The core receives complete records so exact-quote validation uses the original text.
    json!({"id":row["id"],"title":row["title"],"text":text,"url":row["url"],"page":row["page"],"sha256":row["sha256"],"text_sha256":row["text_sha256"],"record_hash_scope":row["record_hash_scope"],"licence":row["licence"],"transcription_method":row["transcription_method"],"transcription_review_status":row["transcription_review_status"],"file_page_index":row["file_page_index"],"printed_page_label":row["printed_page_label"],"printed_page_label_missing_reason":row["printed_page_label_missing_reason"]})
}
pub(super) async fn history(app: &App, owner: &str, sid: &str) -> Result<Vec<Value>> {
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
pub use super::run_events::emit;
async fn relay(app:&App,owner:&str,sid:&str,id:&str)->Result<()> {
    let mut stream=app.core.events(id,0).await?;
    while let Some(message)=stream.message().await? {
        let event:Value=serde_json::from_str(&message.json)?;
        let sequence=event["sequence"].as_u64().ok_or_else(Error::unavailable)?;
        let canonical=format!("{id}:{sequence}");
        if event["event_id"].as_str().is_some_and(|value|value!=canonical) || event["run_id"].as_str().is_some_and(|value|value!=id) {return Err(Error::bad("Core event identity mismatch"));}
        let kind=event["kind"].as_str().unwrap_or("trace");
        if kind=="run.completed" {
            let status=event["data"]["status"].as_str().unwrap_or("failed");
            if matches!(status,"needs_review"|"complete") {run_events::finish(app,owner,sid,id,event["data"]["result"].clone(),&canonical).await?;}
            else {run_events::stop(app,owner,sid,id,if status=="cancelled"{"cancelled"}else{"failed"},event["data"]["result"]["error"].as_str().unwrap_or("Run ended without a result"),&canonical,event["data"]["result"]["code"].as_str().unwrap_or(if status=="cancelled"{"CANCELLED"}else{"UNAVAILABLE"})).await?;}
            return Ok(());
        }
        if matches!(kind,"done"|"complete"|"result") {run_events::finish(app,owner,sid,id,event["data"].clone(),&canonical).await?;return Ok(());}
        if matches!(kind,"error"|"failed"|"cancelled") {run_events::stop(app,owner,sid,id,if kind=="cancelled"{"cancelled"}else{"failed"},event["data"]["message"].as_str().unwrap_or("Run ended without a result"),&canonical,event["data"]["code"].as_str().unwrap_or(if kind=="cancelled"{"CANCELLED"}else{"UNAVAILABLE"})).await?;return Ok(());}
        let mut data=event["data"].clone();if !data.is_object(){data=json!({"value":data});}
        data["kind"]=json!(kind);data["agent"]=event["agent"].clone();data["core_sequence"]=json!(sequence);
        run_events::emit_id(app,owner,sid,id,if kind=="delta"{"delta"}else{"trace"},data,&canonical).await?;
    }
    Err(Error::bad("The model event stream ended before a result was committed"))
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
        .bearer_auth(&app.config.core_token)
        .json(&json!({"message":message}))
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

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[test]
    fn duplicate_relay_claim_returns_without_locking_again_on_drop() {
        let (send,receive)=std::sync::mpsc::channel();
        std::thread::spawn(move ||{let first=RelayGuard::claim("fixture","run");let duplicate=RelayGuard::claim("fixture","run");send.send(first.is_some()&&duplicate.is_none()).unwrap();});
        assert!(receive.recv_timeout(std::time::Duration::from_millis(250)).expect("duplicate claim blocked"));
    }
}

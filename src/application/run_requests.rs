use super::{auth::{self,User}, runs::{self,Input}, sources};
use crate::{error::{Error,Result}, infrastructure::store::{Record,Write}, state::App};
use axum::http::StatusCode;
use serde_json::json;

fn conflict()->Error {Error(StatusCode::CONFLICT,"IDEMPOTENCY_CONFLICT: retry the original research request without changing its input".into())}
fn fingerprint(input:&Input)->Result<String> {
    let mut value=serde_json::to_value(input)?;
    value.as_object_mut().unwrap().remove("session_id");
    Ok(auth::hash(&value.to_string()))
}
fn matches(record:&Record,input:&Input)->Result<()> {
    if record.data["fingerprint"]!=fingerprint(input)? || input.session_id.as_ref().is_some_and(|id|record.data["session_id"]!=*id) {return Err(conflict());}
    Ok(())
}
pub async fn reserve(app:&App,user:&User,input:&Input)->Result<Record> {
    let key=input.idempotency_key.as_deref().ok_or_else(Error::unavailable)?;
    let id=format!("research-{}",auth::hash(&json!([user.id,key]).to_string()));
    match app.store.get(&user.id,"job",&id).await {
        Ok(row)=>{matches(&row,input)?;return Ok(row);},
        Err(error) if error.0==StatusCode::NOT_FOUND=>{},
        Err(error)=>return Err(error),
    }
    if let Some(sid)=&input.session_id {app.store.get(&user.id,"conversation",sid).await?;}
    let sid=input.session_id.clone().unwrap_or_else(||format!("{id}-session"));
    let data=json!({"kind":"research-request","fingerprint":fingerprint(input)?,"session_id":sid,"input":input,"status":"preparing","created_at":auth::now()});
    match app.store.put(Write::new(&user.id,"job",&id,data)).await {
        Ok(row)=>Ok(row),
        Err(error) if error.0==StatusCode::CONFLICT=>{let row=app.store.get(&user.id,"job",&id).await?;matches(&row,input)?;Ok(row)},
        Err(error)=>Err(error),
    }
}
pub async fn prepare(app:&App,user:&User,job:Record,input:&Input)->Result<Record> {
    if job.data["core_request"].is_object() {
        let selected=job.data["sources"].as_array().ok_or_else(Error::unavailable)?;
        super::document_quarantine::authorize_sources(app,&user.id,selected,"provider_processing").await?;
        return Ok(job);
    }
    let sid=job.data["session_id"].as_str().ok_or_else(Error::unavailable)?;
    match app.store.get(&user.id,"conversation",sid).await {
        Ok(_)=>{},
        Err(error) if error.0==StatusCode::NOT_FOUND && input.session_id.is_none()=>{
            let write=Write::new(&user.id,"conversation",sid,json!({"title":input.message.chars().take(100).collect::<String>(),"language":input.language,"updated_at":auth::now()}));
            match app.store.put(write).await {Ok(_)=>{},Err(error) if error.0==StatusCode::CONFLICT=>{},Err(error)=>return Err(error)}
        },
        Err(error)=>return Err(error),
    }
    let selected=sources::select(app,&user.id,&input.message,&input.dataset_ids,&input.source_ids).await?;
    if !input.provider_processing_confirmed && selected.iter().any(|row|row["document_id"].is_string()) {return Err(Error::bad("Confirm provider processing before researching private uploaded pages"));}
    super::document_quarantine::authorize_sources(app,&user.id,&selected,"provider_processing").await?;
    if selected.is_empty() {return Err(Error::bad("No matching source was found. Name a school or an exact source identifier from the collection"));}
    let history=runs::history(app,&user.id,sid).await?;
    let request=json!({"question":input.message,"model":input.model,"locale":input.language,"sources":selected.iter().map(runs::core_source).collect::<Vec<_>>(),"session_id":sid,"user_id":user.id,"history":history,"idempotency_key":job.id});
    let mut data=job.data.clone();data["core_request"]=request;data["sources"]=json!(selected);data["status"]=json!("reserved");
    match app.store.put(Write::update(&job,data)).await {
        Ok(row)=>Ok(row),
        Err(error) if error.0==StatusCode::CONFLICT=>{
            let row=app.store.get(&user.id,"job",&job.id).await?;matches(&row,input)?;
            if !row.data["core_request"].is_object() {return Err(Error::unavailable());}Ok(row)
        },
        Err(error)=>Err(error),
    }
}
pub async fn existing_run(app:&App,owner:&str,job:&Record)->Result<Option<Record>> {
    let Some(id)=job.data["run_id"].as_str() else{return Ok(None);};
    Ok(Some(app.store.get(owner,"run",id).await?))
}
pub async fn commit(app:&App,user:&User,job:&Record,id:&str)->Result<Record> {
    if !crate::domain::valid_id(id) {return Err(Error::unavailable());}
    let input=&job.data["input"];let sid=&job.data["session_id"];
    let run=json!({"run_id":id,"request_id":job.id,"session_id":sid,"status":"running","model":input["model"],"language":input["language"],"question":input["message"],"sources":job.data["sources"],"started_at":auth::now()});
    let mut data=job.data.clone();data["run_id"]=json!(id);data["status"]=json!("started");
    let writes=vec![Write::new(&user.id,"run",id,run),Write::new(&user.id,"message",&format!("{id}-user"),json!({"session_id":sid,"role":"user","content":input["message"],"run_id":id})),Write::update(job,data)];
    match app.store.batch(writes).await {
        Ok(rows)=>rows.into_iter().next().ok_or_else(Error::unavailable),
        Err(error) if error.0==StatusCode::CONFLICT=>{
            let saved=app.store.get(&user.id,"job",&job.id).await?;
            if saved.data["run_id"]!=id {return Err(Error::unavailable());}
            app.store.get(&user.id,"run",id).await
        },
        Err(error)=>Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_identity_rejects_changed_content_but_accepts_original_session_ack() {
        let mut input:Input=serde_json::from_value(json!({"message":"Exact original question","language":"en","idempotency_key":"intent-1"})).unwrap();
        let row=Record{owner:"owner".into(),kind:"job".into(),id:"request".into(),version:1,created_at:0,data:json!({"fingerprint":fingerprint(&input).unwrap(),"session_id":"original"})};
        assert!(matches(&row,&input).is_ok());input.session_id=Some("original".into());assert!(matches(&row,&input).is_ok());
        input.message.push('!');assert_eq!(matches(&row,&input).unwrap_err().0,StatusCode::CONFLICT);
        input.message.pop();input.session_id=Some("other".into());assert!(matches(&row,&input).is_err());
    }
}

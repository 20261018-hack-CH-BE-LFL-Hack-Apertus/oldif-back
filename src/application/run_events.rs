use crate::{application::auth::now,error::{Error,Result},infrastructure::store::{Record,Write},state::App};
use serde_json::{Value,json};

pub fn public(record:Record)->Value {
    let mut value=record.data;
    if value["event_id"].is_null() {value["event_id"]=json!(format!("legacy:{}",record.id));}
    value
}
pub async fn emit(app:&App,owner:&str,sid:&str,run_id:&str,kind:&str,data:Value)->Result<Value> {
    let id=format!("backend:{}",uuid::Uuid::new_v4());
    emit_id(app,owner,sid,run_id,kind,data,&id).await
}
pub async fn emit_id(app:&App,owner:&str,sid:&str,run_id:&str,kind:&str,data:Value,id:&str)->Result<Value> {
    if !crate::domain::valid_id(id) {return Err(Error::bad("Invalid event identity"));}
    let _lock=app.event_lock.lock().await;
    match app.store.get(owner,"event",id).await {
        Ok(record)=>return Ok(public(record)),
        Err(error) if error.0==axum::http::StatusCode::NOT_FOUND=>{},
        Err(error)=>return Err(error),
    }
    let value=json!({"event_id":id,"type":kind,"run_id":run_id,"session_id":sid,"sequence":0,"data":data,"created_at":now()});
    let record=match app.store.put(Write::new(owner,"event",id,value)).await {
        Ok(record)=>record,
        Err(error) if error.0==axum::http::StatusCode::CONFLICT=>return Ok(public(app.store.get(owner,"event",id).await?)),
        Err(error)=>return Err(error),
    };
    let event=public(record);let _=app.events.send((owner.into(),event.clone()));Ok(event)
}
pub async fn finish(app:&App,owner:&str,sid:&str,id:&str,mut result:Value,terminal_id:&str)->Result<()> {
    let result={
        let _lock=app.event_lock.lock().await;
        let row=app.store.get(owner,"run",id).await?;
        if row.data["status"]=="complete" {let mut value=row.data["result"].clone();value["review_version"]=json!(row.version);value}
        else if row.data["status"]!="running" {return Ok(());}
        else {
            let mut data=row.data.clone();data["status"]=json!("complete");data["result"]=result.clone();data["finished_at"]=json!(now());data["terminal_event_id"]=json!(terminal_id);
            data["transformation"]=super::provenance::transformation("record_research_result","Core result retained for human review",owner,id,json!({"kind":"run","id":id,"version":row.version}),json!({"question":row.data["question"],"source_ids":row.data["sources"].as_array().map(|sources|sources.iter().map(|source|source["id"].clone()).collect::<Vec<_>>())}),json!({"value_kind":"record_reference","kind":"run","id":id,"version":row.version+1,"field":"result"}));
            result["review_version"]=json!(row.version+1);
            let content=result["answer"].as_str().unwrap_or("");
            app.store.batch(vec![Write::update(&row,data),Write::new(owner,"message",&format!("{id}-assistant"),json!({"session_id":sid,"role":"assistant","content":content,"result":result,"run_id":id}))]).await?;
            result
        }
    };
    emit_id(app,owner,sid,id,"evidence",result.clone(),&format!("backend:{id}:evidence")).await?;
    emit_id(app,owner,sid,id,"done",result,terminal_id).await?;Ok(())
}
pub async fn stop(app:&App,owner:&str,sid:&str,id:&str,status:&str,message:&str,terminal_id:&str)->Result<()> {
    {
        let _lock=app.event_lock.lock().await;
        let row=app.store.get(owner,"run",id).await?;
        if row.data["status"]!="running" && row.data["status"]!=status {return Ok(());}
        if row.data["status"]=="running" {
            let mut data=row.data.clone();data["status"]=json!(status);data["error"]=json!(message);data["finished_at"]=json!(now());data["terminal_event_id"]=json!(terminal_id);
            app.store.put(Write::update(&row,data)).await?;
        }
    }
    emit_id(app,owner,sid,id,if status=="cancelled"{"cancelled"}else{"error"},json!({"message":message}),terminal_id).await?;Ok(())
}
pub async fn restore_terminal(app:&App,owner:&str,row:&Record)->Result<()> {
    let sid=row.data["session_id"].as_str().ok_or_else(Error::unavailable)?;
    let fallback=format!("backend:{}:terminal",row.id);
    let terminal=row.data["terminal_event_id"].as_str().unwrap_or(&fallback);
    match row.data["status"].as_str() {
        Some("complete")=>finish(app,owner,sid,&row.id,row.data["result"].clone(),terminal).await,
        Some("failed"|"cancelled")=>stop(app,owner,sid,&row.id,row.data["status"].as_str().unwrap(),row.data["error"].as_str().unwrap_or("Run ended"),terminal).await,
        _=>Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_replay_identity_is_stable_without_rewriting_record() {
        let row=Record{owner:"owner".into(),kind:"event".into(),id:"old-event".into(),version:1,created_at:0,data:json!({"type":"trace","sequence":3})};
        assert_eq!(public(row.clone()),public(row.clone()));assert_eq!(public(row)["event_id"],"legacy:old-event");
    }
}

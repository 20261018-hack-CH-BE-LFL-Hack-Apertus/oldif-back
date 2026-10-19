use crate::{
    application::{
        auth::{self, User},
        runs,
    },
    error::{Error, Result},
    state::App,
};
use axum::{
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Connect {
    session_id: Option<String>,
    after_sequence: Option<u64>,
    last_sequence: Option<u64>,
}
impl Connect {
    fn cursor(&self)->Result<u64>{
        if self.last_sequence.zip(self.after_sequence).is_some_and(|(a,b)|a!=b){return Err(Error::bad("Conflicting last_sequence and after_sequence"));}
        Ok(self.last_sequence.or(self.after_sequence).unwrap_or(0))
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Steer {run_id:String,message:String}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cancel {run_id:String}
#[derive(Deserialize,serde::Serialize)]
#[serde(deny_unknown_fields)]
struct VoiceOffer {sdp:String,locale:String}
fn ancillary(input:&Value)->Result<()> {
    let mut body=input.clone();let kind=body.get("type").and_then(Value::as_str).ok_or_else(||Error::bad("Realtime type is required"))?.to_owned();
    body.as_object_mut().ok_or_else(||Error::bad("Expected command object"))?.remove("type");
    match kind.as_str(){
        "message"=>Ok(()),
        "subscribe"=>{let value:Connect=serde_json::from_value(body)?;value.cursor()?;if value.session_id.as_deref().is_none_or(|id|!crate::domain::valid_id(id)){return Err(Error::bad("Valid session_id is required"));}Ok(())},
        "steer"=>{let value:Steer=serde_json::from_value(body)?;if !crate::domain::valid_id(&value.run_id)||value.message.trim().is_empty()||value.message.len()>8000{return Err(Error::bad("Invalid steering command"));}Ok(())},
        "cancel"=>{let value:Cancel=serde_json::from_value(body)?;if !crate::domain::valid_id(&value.run_id){return Err(Error::bad("Invalid cancellation command"));}Ok(())},
        _=>Err(Error::bad("Unsupported realtime command")),
    }
}
pub async fn socket(
    State(app): State<App>,
    headers: HeaderMap,
    Query(query): Query<Connect>,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    query.cursor()?;
    auth::origin(&app, &headers)?;
    let user = auth::authenticate(&app, &headers).await?;
    if let Some(id) = &query.session_id {
        app.store.get(&user.id, "conversation", id).await?;
    }
    Ok(ws
        .max_message_size(32768)
        .max_frame_size(32768)
        .on_upgrade(move |socket| handle(socket, app, user, query, headers)))
}
async fn handle(socket: WebSocket, app: App, user: User, query: Connect, headers: HeaderMap) {
    let (mut tx, mut rx) = socket.split();
    let mut events = app.events.subscribe();
    let mut last = query.cursor().unwrap_or(0);
    let mut sid = query.session_id;
    if let Some(id) = &sid {
        if let Ok(rows) = replay(&app, &user, id, last).await {
            for event in rows {
                last = last.max(event["sequence"].as_u64().unwrap_or(0));
                if tx
                    .send(Message::Text(event.to_string().into()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
    }
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(20));
    loop {
        tokio::select! {
            incoming=rx.next()=>{
                let Some(Ok(message))=incoming else{break;};
                if let Message::Close(_)=message{break;}
                if let Message::Text(text)=message {
                    let input=match serde_json::from_str::<Value>(&text){Ok(value)=>value,Err(_)=>{let _=tx.send(Message::Text(json!({"type":"error","data":{"message":"Invalid JSON message","code":"INVALID_REQUEST"}}).to_string().into())).await;continue;}};
                    if let Err(e)=ancillary(&input) {let _=tx.send(Message::Text(json!({"type":"error","data":{"message":e.1,"code":e.code()}}).to_string().into())).await;continue;}
                    if input["type"]=="subscribe" {
                        if let Some(id)=input["session_id"].as_str() {
                            match replay(&app,&user,id,input["last_sequence"].as_u64().or_else(||input["after_sequence"].as_u64()).unwrap_or(0)).await {
                                Ok(rows)=>{sid=Some(id.into());last=input["last_sequence"].as_u64().or_else(||input["after_sequence"].as_u64()).unwrap_or(0);for event in rows{last=last.max(event["sequence"].as_u64().unwrap_or(0));if tx.send(Message::Text(event.to_string().into())).await.is_err(){return;}}},
                                Err(e)=>{let _=tx.send(Message::Text(json!({"type":"error","data":{"message":e.1,"code":e.code()}}).to_string().into())).await;}
                            }
                        }continue;
                    }
                    match dispatch(&app,&user,&sid,input).await {
                        Ok(created)=>{if let Some(id)=created["session_id"].as_str(){
                            sid=Some(id.into());
                            if created["idempotent_replay"]==true {
                                let ack=json!({"type":"run_reused","data":{"run_id":created["run_id"],"session_id":id,"status":created["status"]}});
                                if tx.send(Message::Text(ack.to_string().into())).await.is_err(){return;}
                                if let Ok(rows)=replay(&app,&user,id,0).await {for event in rows.into_iter().filter(|event|event["run_id"]==created["run_id"]) {last=last.max(event["sequence"].as_u64().unwrap_or(0));if tx.send(Message::Text(event.to_string().into())).await.is_err(){return;}}}
                            }
                        }},
                        Err(e)=>{if tx.send(Message::Text(json!({"type":"error","data":{"message":e.1,"code":e.code()}}).to_string().into())).await.is_err(){break;}}
                    }
                }
            },
            result=events.recv()=>match result {
                Ok((owner,event)) if owner==user.id && sid.as_deref()==event["session_id"].as_str()=>{
                    let sequence=event["sequence"].as_u64().unwrap_or(0);if sequence<=last{continue;}last=sequence;
                    if tx.send(Message::Text(event.to_string().into())).await.is_err(){break;}
                },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>{let _=tx.send(Message::Text(json!({"type":"resync_required","data":{"message":"Reconnect to restore committed events","code":"BACKPRESSURE"}}).to_string().into())).await;break;},
                Err(tokio::sync::broadcast::error::RecvError::Closed)=>break,
                _=>{}
            },
            _=heartbeat.tick()=>{if auth::authenticate(&app,&headers).await.is_err(){let _=tx.send(Message::Close(None)).await;break;}if tx.send(Message::Ping(Vec::new().into())).await.is_err(){break;}}
        }
    }
}
async fn replay(app: &App, user: &User, id: &str, after: u64) -> Result<Vec<Value>> {
    app.store.get(&user.id, "conversation", id).await?;
    let mut rows = app
        .store
        .list(&user.id, "event")
        .await?
        .into_iter()
        .map(crate::application::run_events::public)
        .filter(|r| r["session_id"] == id && r["sequence"].as_u64().unwrap_or(0) > after)
        .collect::<Vec<_>>();
    rows.sort_by_key(|r| r["sequence"].as_u64().unwrap_or(0));
    Ok(rows)
}
async fn dispatch(app: &App, user: &User, sid: &Option<String>, input: Value) -> Result<Value> {
    match input["type"].as_str().unwrap_or("") {
        "message" => {
            let mut input = input;
            input.as_object_mut().ok_or_else(||Error::bad("Expected a research command object"))?.remove("type");
            if input["session_id"].is_null(){input["session_id"]=json!(sid);}
            else if sid.as_ref().is_some_and(|id|input["session_id"]!=*id){return Err(Error::bad("Research command must match the subscribed session"));}
            if input["language"].is_null() {
                input["language"] = json!("en");
            }
            runs::start(app, user, serde_json::from_value(input)?).await
        }
        "steer" | "cancel" => {
            runs::control(
                app,
                user,
                input["run_id"].as_str().unwrap_or(""),
                input["type"].as_str().unwrap_or(""),
                input["message"].as_str().unwrap_or(""),
            )
            .await
        }
        _ => Err(Error::bad("Unsupported realtime command")),
    }
}
pub async fn voice(State(app): State<App>, headers: HeaderMap, offer: String) -> Result<Response> {
    let user = auth::authenticate(&app, &headers).await?;
    auth::limit(&app, &format!("voice:{}", user.id), 5).await?;
    let session = if offer.trim_start().starts_with('{') {
        serde_json::to_value(serde_json::from_str::<VoiceOffer>(&offer)?)?
    } else {
        json!({"sdp":offer,"locale":headers.get("x-oldif-locale").and_then(|v|v.to_str().ok()).unwrap_or("en")})
    };
    let sdp = session["sdp"].as_str().unwrap_or("");
    let locale = session["locale"].as_str().unwrap_or("en");
    if !sdp.starts_with("v=0") || sdp.len() > 128000 || !crate::domain::valid_locale(locale) {
        return Err(Error::bad("Invalid WebRTC offer or language"));
    }
    let response = app
        .client
        .post(format!("{}/v1/realtime/session", app.config.core_url))
        .bearer_auth(&app.config.core_token)
        .json(&session)
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(Error::bad(
            "Voice provider rejected the session; check its availability",
        ));
    }
    let answer = response.text().await?;
    let mut out = answer.into_response();
    out.headers_mut()
        .insert("content-type", HeaderValue::from_static("application/sdp"));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replay_aliases_reject_conflicts_and_unknown_commands() {
        for input in [json!({"last_sequence":5}),json!({"after_sequence":5}),json!({"last_sequence":5,"after_sequence":5})] {assert_eq!(serde_json::from_value::<Connect>(input).unwrap().cursor().unwrap(),5);}
        assert!(serde_json::from_value::<Connect>(json!({"last_sequence":5,"after_sequence":4})).unwrap().cursor().is_err());
        assert!(serde_json::from_value::<Connect>(json!({"last_sequence":5,"extra":true})).is_err());
        assert!(ancillary(&json!({"type":"subscribe","session_id":"session-1","last_sequence":5})).is_ok());
        for input in [json!({"type":"cancel","run_id":"run-1","message":"unexpected"}),json!({"type":"steer","run_id":"run-1","message":"ok","extra":true}),json!({"type":"subscribe","session_id":"session-1","last_sequence":5,"after_sequence":4})] {assert!(ancillary(&input).is_err());}
        assert!(serde_json::from_value::<VoiceOffer>(json!({"sdp":"v=0","locale":"en","provider_key":"forbidden"})).is_err());
    }
}

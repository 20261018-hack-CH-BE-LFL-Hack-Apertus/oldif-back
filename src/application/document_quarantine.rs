use super::{acquisition::Acquisition, auth, document_ingest::{self,Identity}, documents::{self,Metadata}, provenance};
use crate::{error::{Error,Result}, infrastructure::{document_parser::{self,Parsed},store::{Record,Write}},state::App};
use axum::http::StatusCode;
use base64::{Engine,engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value,json};

pub async fn store_original(app:&App,document:&Record,bytes:&[u8],resume:bool)->Result<usize> {
    let chunks=bytes.chunks(98_304).enumerate().map(|(index,chunk)|Write::new(&document.owner,"document_blob",&format!("{}-{index:04}",document.id),json!({"document_id":document.id,"index":index,"encoding":"base64","sha256":document_parser::digest(chunk),"bytes":STANDARD.encode(chunk)}))).collect::<Vec<_>>();
    for group in chunks.chunks(8) {document_ingest::persist_batch(app,group.to_vec(),resume).await?;}
    Ok(chunks.len())
}
pub async fn hold(app:&App,owner:&str,filename:&str,media:&str,input:&Metadata,bytes:&[u8],identity:&Identity,previous:Option<Record>,missing:Vec<String>)->Result<Value> {
    let resume=previous.is_some();
    let document=if let Some(record)=previous {record} else {
        auth::limit(app,&format!("document:{owner}"),5).await?;
        let parsed=Parsed{pages:vec![],metadata:json!({"extractor":"none","reason":"RIGHTS_UNRESOLVED"})};
        let mut data=documents::document_data(&identity.id,filename,media,input,bytes,&parsed,&identity.original_sha256);
        data["status"]=json!("quarantined");data["code"]=json!("RIGHTS_UNRESOLVED");data["missing_fields"]=json!(missing);
        data["blob_chunks"]=json!(bytes.len().div_ceil(98_304));data["ingest_fingerprint"]=json!(identity.fingerprint);
        data["transformation"]=provenance::transformation("quarantine_original","Acquisition rights unresolved; no extraction or model processing",owner,&identity.id,Value::Null,json!({"original_sha256":identity.original_sha256,"upload_metadata":input}),json!({"status":"quarantined","missing_fields":missing}));
        data["initial_transformation"]=data["transformation"].clone();
        app.store.put(Write::new(owner,"document",&identity.id,data)).await?
    };
    store_original(app,&document,bytes,resume).await?;
    Ok(json!({"code":"RIGHTS_UNRESOLVED","missing_fields":missing,"document":document.public(),"pages":[],"idempotent_replay":resume}))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {pub expected_version:i64,pub rights_confirmed:bool,pub acquisition:Acquisition}
pub async fn release(app:&App,owner:&str,id:&str,input:Release)->Result<Value> {
    let document=documents::get(app,owner,id).await?;
    if document.version!=input.expected_version {return Err(Error(StatusCode::CONFLICT,"REVISION_CONFLICT: reload document before resolving acquisition".into()));}
    let missing=input.acquisition.assess(document.data["original_sha256"].as_str().ok_or_else(Error::unavailable)?,input.rights_confirmed)?;
    if !missing.is_empty() {return Err(Error::bad(&format!("RIGHTS_UNRESOLVED: {}",missing.join(", "))));}
    let _permit=app.document_slots.try_acquire().map_err(|_|Error(StatusCode::TOO_MANY_REQUESTS,"Document processing is busy".into()))?;
    auth::limit(app,&format!("document:{owner}"),5).await?;
    let (_,bytes)=documents::original(app,owner,id).await?;
    let metadata=Metadata{title:document.data["upload_metadata"]["title"].as_str().map(str::to_owned),licence:input.acquisition.rights_identifier.clone(),rights_confirmed:true,acquisition:Some(input.acquisition)};
    let media=document.data["media_type"].as_str().ok_or_else(Error::unavailable)?;
    let filename=document.data["filename"].as_str().ok_or_else(Error::unavailable)?;
    let ready=document.data["status"]=="ready";
    let parsed=if ready {None}else{Some(document_parser::parse(&app.config.documents,media,&bytes).await?)};
    let mut data=if let Some(parsed)=&parsed {documents::document_data(id,filename,media,&metadata,&bytes,parsed,document.data["original_sha256"].as_str().unwrap())}else{document.data.clone()};
    data["acquisition"]=json!(metadata.acquisition);data["rights_confirmed"]=json!(true);data["licence"]=json!(metadata.licence);
    data["missing_fields"]=json!([]);data["code"]=Value::Null;
    data["ingest_fingerprint"]=json!(Identity::new(owner,filename,media,&metadata,None,&bytes)?.fingerprint);
    let revision_id=uuid::Uuid::new_v4().to_string();
    let transform=provenance::transformation("resolve_acquisition","User supplied complete acquisition rights declaration",owner,&revision_id,json!({"kind":"document","id":id,"version":document.version}),json!({"acquisition":document.data["acquisition"],"licence":document.data["licence"],"status":document.data["status"]}),json!({"acquisition":data["acquisition"],"licence":data["licence"],"status":data["status"]}));
    data["transformation"]=transform.clone();
    data["initial_transformation"]=document.data.get("initial_transformation").cloned().unwrap_or(document.data["transformation"].clone());
    let review=json!({"document_id":id,"actor":owner,"decision":"rights_resolved","reason":"User supplied complete acquisition rights declaration","previous_version":document.version,"resulting_version":document.version+1,"transformation":transform,"content_quality_review":false,"original_modified":false});
    let records=app.store.batch(vec![Write::update(&document,data),Write::new(owner,"review",&revision_id,review)]).await?;
    let result=if let Some(parsed)=parsed {documents::persist(app,&records[0],&bytes,&parsed,document.data["original_sha256"].as_str().unwrap(),true).await}else{Ok(json!({"document":records[0].public(),"pages":documents::pages(app,owner,id).await?}))};
    if result.is_err() {let mut failed=records[0].data.clone();failed["status"]=json!("failed");let _=app.store.put(Write::update(&records[0],failed)).await;}
    result
}
pub fn require_use(document:&Record,usage:&str)->Result<()> {
    if document.data["status"]!="ready" {return Err(Error::bad("RIGHTS_UNRESOLVED: document is not ready for processing or export"));}
    if !document.data["acquisition"]["allowed_uses"].as_array().is_some_and(|uses|uses.iter().any(|value|value==usage)) {return Err(Error::bad("Recorded acquisition rights do not permit this use; resolve acquisition metadata first"));}
    Ok(())
}
pub async fn authorize_sources(app:&App,owner:&str,sources:&[Value],usage:&str)->Result<()> {
    let ids=sources.iter().filter_map(|row|row["document_id"].as_str()).collect::<std::collections::HashSet<_>>();
    for id in ids {require_use(&documents::get(app,owner,id).await?,usage)?;}
    Ok(())
}

use crate::{application::auth, error::{Error, Result}, infrastructure::store::{Record, Write}, state::App};
use axum::{Json, extract::{Path, State}, http::{HeaderMap, StatusCode}};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input { expected_version: i64, layout: Value }
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all="camelCase")]
struct Workbench {
    version: u8, navigation: bool, evidence: bool, activity: bool,
    placements: Placements, order: Vec<String>, navigation_width: f64,
    research_width: f64, top_height: f64, right_split: f64, bottom_split: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Placements { evidence: String, activity: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Archive { width: f64, height: f64, source: bool, review: bool, reverse: bool }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Canvas {session_id:String,positions:std::collections::BTreeMap<String,Position>,selected_source_id:Option<String>}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Position {x:f64,y:f64}
fn key(scope:&str)->Result<String> {
    if !["workbench","archive","canvas"].contains(&scope) { return Err(Error::missing()); }
    Ok(format!("layout-{scope}"))
}
fn validate(scope:&str, input:&Input)->Result<()> {
    key(scope)?;
    if input.expected_version<0 || input.layout.to_string().len()>16_384 { return Err(Error::bad("Invalid layout version or size")); }
    let between=|v:f64,min:f64,max:f64| v.is_finite()&&v>=min&&v<=max;
    let valid=match scope {
        "workbench"=>{
            let value:Workbench=serde_json::from_value(input.layout.clone()).map_err(|_|Error::bad("Invalid workbench layout"))?;
            let _=(value.navigation,value.evidence,value.activity);
            value.version==1&&["right","bottom"].contains(&value.placements.evidence.as_str())&&["right","bottom"].contains(&value.placements.activity.as_str())
                && (value.order==["evidence","activity"]||value.order==["activity","evidence"])
                &&between(value.navigation_width,13.,32.)&&between(value.research_width,40.,85.)&&between(value.top_height,30.,85.)&&between(value.right_split,20.,80.)&&between(value.bottom_split,20.,80.)
        },
        "archive"=>{
            let value:Archive=serde_json::from_value(input.layout.clone()).map_err(|_|Error::bad("Invalid archive layout"))?;
            let _=(value.source,value.review,value.reverse);
            between(value.width,25.,70.)&&between(value.height,25.,75.)
        },
        "canvas"=>{
            let value:Canvas=serde_json::from_value(input.layout.clone()).map_err(|_|Error::bad("Invalid canvas layout"))?;
            crate::domain::valid_id(&value.session_id)&&value.selected_source_id.as_deref().is_none_or(crate::domain::valid_id)&&value.positions.len()<=100
                &&value.positions.iter().all(|(id,p)|!id.is_empty()&&id.len()<=200&&between(p.x,-10000.,10000.)&&between(p.y,-10000.,10000.))
        },
        _=>false,
    };
    if !valid { return Err(Error::bad("Layout exceeds supported bounds")); }
    Ok(())
}
async fn current(app:&App,owner:&str,id:&str)->Result<Option<Record>> {
    match app.store.get(owner,"state",id).await {
        Ok(record)=>Ok(Some(record)), Err(error) if error.0==StatusCode::NOT_FOUND=>Ok(None), Err(error)=>Err(error)
    }
}
pub async fn get(State(app):State<App>,headers:HeaderMap,Path(scope):Path<String>)->Result<Json<Value>> {
    let user=auth::authenticate(&app,&headers).await?;
    let record=current(&app,&user.id,&key(&scope)?).await?;
    Ok(Json(json!({"scope":scope,"version":record.as_ref().map(|r|r.version).unwrap_or(0),"layout":record.map(|r|r.data["layout"].clone())})))
}
pub async fn put(State(app):State<App>,headers:HeaderMap,Path(scope):Path<String>,Json(input):Json<Input>)->Result<Json<Value>> {
    let user=auth::authenticate(&app,&headers).await?;
    validate(&scope,&input)?;
    if scope=="canvas" {
        let value:Canvas=serde_json::from_value(input.layout.clone())?;
        app.store.get(&user.id,"conversation",&value.session_id).await?;
        if let Some(id)=value.selected_source_id {crate::application::sources::get(&app,&user.id,&id).await?;}
    }
    auth::limit(&app,&format!("layout:{}",user.id),120).await?;
    let id=key(&scope)?;let prior=current(&app,&user.id,&id).await?;
    if prior.as_ref().map(|r|r.version).unwrap_or(0)!=input.expected_version { return Err(Error(StatusCode::CONFLICT,"REVISION_CONFLICT: reload the current layout before saving".into())); }
    let data=json!({"scope":scope,"layout":input.layout});
    let write=prior.as_ref().map(|r|Write::update(r,data.clone())).unwrap_or_else(||Write::new(&user.id,"state",&id,data));
    let record=app.store.put(write).await?;
    Ok(Json(json!({"scope":scope,"version":record.version,"layout":record.data["layout"]})))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canvas_positions_are_bounded_and_strict() {
        let mut input=Input{expected_version:0,layout:json!({"session_id":"session-1","positions":{"node-1":{"x":25,"y":-20}},"selected_source_id":null})};
        assert!(validate("canvas",&input).is_ok());
        input.layout["positions"]["node-1"]["x"]=json!(10001);assert!(validate("canvas",&input).is_err());
        input.layout["positions"]["node-1"]=json!({"x":25,"y":20,"script":true});assert!(validate("canvas",&input).is_err());
        input.layout["positions"]=json!((0..101).map(|i|(format!("n{i}"),json!({"x":1,"y":1}))).collect::<std::collections::BTreeMap<_,_>>());assert!(validate("canvas",&input).is_err());
    }
    #[test]
    fn layout_rejects_unknown_fields_scopes_and_out_of_range_values() {
        let layout=json!({"width":48,"height":60,"source":true,"review":true,"reverse":false});
        assert!(validate("archive",&Input{expected_version:0,layout:layout.clone()}).is_ok());
        assert!(validate("other",&Input{expected_version:0,layout:layout.clone()}).is_err());
        let mut bad=layout.clone();bad["width"]=json!(99);
        assert!(validate("archive",&Input{expected_version:0,layout:bad}).is_err());
        let mut bad=layout;bad["script"]=json!("forbidden");
        assert!(validate("archive",&Input{expected_version:0,layout:bad}).is_err());
        assert!(serde_json::from_value::<Input>(json!({"expected_version":0,"layout":{},"extra":true})).is_err());
    }
}

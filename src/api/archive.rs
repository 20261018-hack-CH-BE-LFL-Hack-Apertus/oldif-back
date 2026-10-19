use crate::{
    application::{auth, exports, runs, sources},
    error::{Error, Result},
    state::App,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Deserialize, Default)]
pub struct QueryParams {
    pub q: Option<String>,
    pub format: Option<String>,
    pub session_id: Option<String>,
}
pub async fn models(State(app): State<App>, headers: HeaderMap) -> Result<Json<Value>> {
    auth::authenticate(&app, &headers).await?;
    let mut result = app.core.models().await?;
    let list = result
        .get("data")
        .or_else(|| result.get("models"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    result["models"] = json!(
        list.into_iter()
            .map(|mut row| {
                row["name"] = row.get("name").cloned().unwrap_or(row["id"].clone());
                row["available"] = row
                    .get("available")
                    .cloned()
                    .or_else(|| row.get("configured").cloned())
                    .unwrap_or(json!(false));
                row
            })
            .collect::<Vec<_>>()
    );
    Ok(Json(result))
}
pub async fn datasets(State(app): State<App>, headers: HeaderMap) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    Ok(Json(
        json!({"datasets":sources::datasets(&app,&user.id).await?}),
    ))
}
pub async fn dataset_sources(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    Ok(Json(
        json!({"sources":exports::dataset(&app,&user.id,&id).await?}),
    ))
}
pub async fn source(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    Ok(Json(sources::get(&app, &user.id, &id).await?.public()))
}
pub async fn search(
    State(app): State<App>,
    headers: HeaderMap,
    Query(query): Query<QueryParams>,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    Ok(Json(
        json!({"sources":sources::search(&app,&user.id,query.q.as_deref().unwrap_or(""),&[]).await?}),
    ))
}
pub async fn import(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    auth::limit(&app, &format!("import:{}", user.id), 5).await?;
    Ok(Json(sources::import(&app, &user.id, input).await?))
}
pub async fn sessions(State(app): State<App>, headers: HeaderMap) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    let rows = app.store.list(&user.id, "conversation").await?;
    Ok(Json(
        json!({"sessions":rows.iter().rev().map(|r|r.public()).collect::<Vec<_>>()}),
    ))
}
pub async fn create_session(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    Ok(Json(
        json!({"session":runs::session(&app,&user,input["title"].as_str().unwrap_or("New research"),input["language"].as_str().unwrap_or("en")).await?}),
    ))
}
pub async fn session(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    let session = app.store.get(&user.id, "conversation", &id).await?;
    let mut result = json!({"session":session.public()});
    for (kind, key) in [
        ("message", "messages"),
        ("run", "runs"),
        ("event", "events"),
    ] {
        let rows = app
            .store
            .list(&user.id, kind)
            .await?
            .into_iter()
            .filter(|r| r.data["session_id"] == id)
            .map(|r| if kind == "event" { r.data } else { r.public() })
            .collect::<Vec<_>>();
        result[key] = json!(rows);
    }
    Ok(Json(result))
}
pub async fn run(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<runs::Input>,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    Ok(Json(runs::start(&app, &user, input).await?))
}
pub async fn review(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    Ok(Json(exports::review(&app, &user.id, &id, input).await?))
}
pub async fn steer(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    Ok(Json(
        runs::control(
            &app,
            &user,
            &id,
            "steer",
            input["message"].as_str().unwrap_or(""),
        )
        .await?,
    ))
}
pub async fn cancel(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    Ok(Json(runs::control(&app, &user, &id, "cancel", "").await?))
}
pub async fn export(
    State(app): State<App>,
    headers: HeaderMap,
    Query(query): Query<QueryParams>,
) -> Result<Response> {
    let user = auth::authenticate(&app, &headers).await?;
    let rows = exports::accepted(&app, &user.id, query.session_id.as_deref()).await?;
    download(rows, query.format.as_deref().unwrap_or("jsonl"))
}
pub async fn dataset_export(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<QueryParams>,
) -> Result<Response> {
    let user = auth::authenticate(&app, &headers).await?;
    let rows = exports::dataset(&app, &user.id, &id).await?;
    download(rows, query.format.as_deref().unwrap_or("jsonl"))
}
fn download(rows: Vec<Value>, format: &str) -> Result<Response> {
    let (body, content_type) = exports::serialize(&rows, format)?;
    let mut response = body.into_response();
    response
        .headers_mut()
        .insert("content-type", HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        "content-disposition",
        HeaderValue::from_str(&format!("attachment; filename=oldif-export.{format}"))
            .map_err(|_| Error::bad("Invalid export format"))?,
    );
    Ok(response)
}
pub async fn mcp(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    let id = input["id"].clone();
    let result = match input["method"].as_str().unwrap_or("") {
        "initialize" => {
            json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"oldif-archives","version":"0.1.0"}})
        }
        "tools/list" => {
            json!({"tools":[{"name":"search_archives","description":"Search accessible published archival transcriptions","inputSchema":{"type":"object","properties":{"query":{"type":"string","maxLength":8000}},"required":["query"],"additionalProperties":false}},{"name":"get_source","description":"Get immutable archival source with provenance","inputSchema":{"type":"object","properties":{"id":{"type":"string"}},"required":["id"],"additionalProperties":false}}]})
        }
        "tools/call" => {
            let args = &input["params"]["arguments"];
            let data = match input["params"]["name"].as_str().unwrap_or("") {
                "search_archives" => json!(
                    sources::search(&app, &user.id, args["query"].as_str().unwrap_or(""), &[])
                        .await?
                ),
                "get_source" => sources::get(&app, &user.id, args["id"].as_str().unwrap_or(""))
                    .await?
                    .public(),
                _ => {
                    return Ok(Json(
                        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Tool not allowed"}}),
                    ));
                }
            };
            json!({"content":[{"type":"text","text":data.to_string()}],"isError":false})
        }
        _ => {
            return Ok(Json(
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not supported"}}),
            ));
        }
    };
    Ok(Json(json!({"jsonrpc":"2.0","id":id,"result":result})))
}

pub async fn agent_review(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let user = auth::authenticate(&app, &headers).await?;
    let run = app.store.get(&user.id, "run", &id).await?;
    if run.data["status"] != "complete" {
        return Err(Error::bad(
            "Complete the evidence run before requesting an agent review",
        ));
    }
    auth::limit(&app, &format!("agent-review:{}", user.id), 3).await?;
    let url = app
        .config
        .covenant_url
        .as_deref()
        .ok_or_else(Error::unavailable)?;
    let response = app
        .client
        .post(format!("{url}/v1/openclaw/review"))
        .bearer_auth(&app.config.token)
        .json(&json!({"run_id":id}))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(Error::bad(
            "The agent review could not complete; the original evidence is preserved",
        ));
    }
    let result: Value = response.json().await?;
    let saved=app.store.put(crate::infrastructure::store::Write::new(&user.id,"assertion",&uuid::Uuid::new_v4().to_string(),json!({"kind":"agent-review","run_id":id,"state":"proposed","result":result,"human_review":false}))).await?;
    Ok(Json(saved.public()))
}

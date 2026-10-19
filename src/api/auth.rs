use crate::{
    application::auth,
    error::{Error, Result},
    state::App,
};
use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use std::net::SocketAddr;
fn response(user: auth::User, cookie: String) -> Result<Response> {
    let mut response = Json(json!({"user":user})).into_response();
    response.headers_mut().insert(
        "set-cookie",
        HeaderValue::from_str(&cookie).map_err(|_| Error::unavailable())?,
    );
    Ok(response)
}
pub async fn demo(
    State(app): State<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Result<Response> {
    auth::limit(&app, &format!("auth:{}", addr.ip()), 30).await?;
    let (user, cookie) = auth::demo(&app).await?;
    response(user, cookie)
}
pub async fn register(
    State(app): State<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(input): Json<Value>,
) -> Result<Response> {
    auth::limit(&app, &format!("auth:{}", addr.ip()), 30).await?;
    let (user, cookie) = auth::register(&app, input).await?;
    response(user, cookie)
}
pub async fn login(
    State(app): State<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(input): Json<Value>,
) -> Result<Response> {
    auth::limit(&app, &format!("auth:{}", addr.ip()), 30).await?;
    let (user, cookie) = auth::login(&app, input).await?;
    response(user, cookie)
}
pub async fn session(State(app): State<App>, headers: HeaderMap) -> Result<Json<Value>> {
    Ok(Json(
        json!({"user":auth::authenticate(&app,&headers).await?}),
    ))
}
pub async fn logout(State(app): State<App>, headers: HeaderMap) -> Result<Response> {
    auth::logout(&app, &headers).await?;
    let mut response = Json(json!({"ok":true})).into_response();
    response.headers_mut().insert(
        "set-cookie",
        HeaderValue::from_static("oldif_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
    );
    Ok(response)
}

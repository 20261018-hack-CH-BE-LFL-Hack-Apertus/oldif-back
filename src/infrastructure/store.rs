use super::store_grpc::GrpcStore;
use crate::domain::valid_id;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct Store {
    client: reqwest::Client,
    url: String,
    token: String,
    grpc: Option<GrpcStore>,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Record {
    pub owner: String,
    pub kind: String,
    pub id: String,
    pub version: i64,
    pub data: Value,
    pub created_at: i64,
}
impl Record {
    pub fn public(&self) -> Value {
        let mut data = self.data.clone();
        data["id"] = json!(self.id);
        data["version"] = json!(self.version);
        data["created_at"] = json!(self.created_at);
        data
    }
}
#[derive(Clone, Serialize)]
pub struct Write {
    pub owner: String,
    pub kind: String,
    pub id: String,
    pub data: Value,
    pub expected_version: i64,
}
impl Write {
    pub fn new(owner: &str, kind: &str, id: &str, data: Value) -> Self {
        Self {
            owner: owner.into(),
            kind: kind.into(),
            id: id.into(),
            data,
            expected_version: 0,
        }
    }
    pub fn update(record: &Record, data: Value) -> Self {
        Self {
            owner: record.owner.clone(),
            kind: record.kind.clone(),
            id: record.id.clone(),
            data,
            expected_version: record.version,
        }
    }
}
impl Store {
    pub fn new(client: reqwest::Client, config: &crate::config::Config) -> Self {
        let grpc = config.db_grpc_url.as_ref().map(|url| {
            GrpcStore::new(
                url.clone(),
                config.token.clone(),
                config.db_connect_seconds,
                config.db_request_seconds,
            )
        });
        Self {
            client,
            url: config.db_url.clone(),
            token: config.token.clone(),
            grpc,
        }
    }
    pub async fn list(&self, owner: &str, kind: &str) -> Result<Vec<Record>> {
        if let Some(grpc) = &self.grpc {
            return grpc.list(owner, kind).await;
        }
        let response = self
            .client
            .get(format!("{}/v1/records/{kind}", self.url))
            .query(&[("owner", owner)])
            .bearer_auth(&self.token)
            .send()
            .await?;
        let body = checked(response).await?;
        Ok(serde_json::from_value(body["records"].clone())?)
    }
    pub async fn get(&self, owner: &str, kind: &str, id: &str) -> Result<Record> {
        if !valid_id(id) {
            return Err(Error::bad("Invalid identifier"));
        }
        if let Some(grpc) = &self.grpc {
            return grpc.get(owner, kind, id).await;
        }
        let response = self
            .client
            .get(format!("{}/v1/records/{kind}/{id}", self.url))
            .query(&[("owner", owner)])
            .bearer_auth(&self.token)
            .send()
            .await?;
        Ok(serde_json::from_value(checked(response).await?)?)
    }
    pub async fn batch(&self, items: Vec<Write>) -> Result<Vec<Record>> {
        if let Some(grpc) = &self.grpc {
            return grpc.batch(items).await;
        }
        let response = self
            .client
            .post(format!("{}/v1/batch", self.url))
            .bearer_auth(&self.token)
            .json(&json!({"items":items}))
            .send()
            .await?;
        let body = checked(response).await?;
        Ok(serde_json::from_value(body["records"].clone())?)
    }
    pub async fn put(&self, write: Write) -> Result<Record> {
        self.batch(vec![write])
            .await?
            .into_iter()
            .next()
            .ok_or_else(Error::unavailable)
    }
}
async fn checked(response: reqwest::Response) -> Result<Value> {
    match response.status().as_u16() {
        200..=299 => Ok(response.json().await?),
        404 => Err(Error::missing()),
        409 => Err(Error(
            axum::http::StatusCode::CONFLICT,
            "This record changed; reload before saving".into(),
        )),
        _ => Err(Error::unavailable()),
    }
}

#[cfg(test)]
#[path = "store_transport_tests.rs"]
mod tests;

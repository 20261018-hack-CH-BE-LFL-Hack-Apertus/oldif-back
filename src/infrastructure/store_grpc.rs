use super::store::{Record, Write};
use crate::error::{Error, Result};
use std::{collections::HashSet, future::Future, sync::Arc, time::Duration};
use tokio::sync::OnceCell;
use tonic::{Request, Status, transport::Channel};

pub mod proto {
    tonic::include_proto!("oldif.store.v1");
}
use proto::{CollectionKey, RecordKey, WriteBatch, record_store_client::RecordStoreClient};
const MAX_REQUEST_BYTES: usize = 4_000_000;
const MAX_RESPONSE_BYTES: usize = 8_000_000;
const MAX_LIST_BYTES: usize = 64_000_000;

#[derive(Clone)]
pub struct GrpcStore {
    url: String,
    token: String,
    connect_timeout: Duration,
    request_timeout: Duration,
    channel: Arc<OnceCell<Channel>>,
}
impl GrpcStore {
    pub fn new(url: String, token: String, connect_seconds: u64, request_seconds: u64) -> Self {
        Self {
            url,
            token,
            connect_timeout: Duration::from_secs(connect_seconds),
            request_timeout: Duration::from_secs(request_seconds),
            channel: Arc::new(OnceCell::new()),
        }
    }
    async fn connect(&self) -> Result<Channel> {
        deadline(self.connect_timeout, async {
            let endpoint =
                Channel::from_shared(self.url.clone()).map_err(|_| Error::unavailable())?;
            Ok(endpoint
                .connect_timeout(self.connect_timeout)
                .connect()
                .await?)
        })
        .await
    }
    async fn client(&self) -> Result<RecordStoreClient<Channel>> {
        let channel = self.channel.get_or_try_init(|| self.connect()).await?;
        Ok(RecordStoreClient::new(channel.clone())
            .max_encoding_message_size(MAX_REQUEST_BYTES)
            .max_decoding_message_size(MAX_RESPONSE_BYTES))
    }
    fn request<T>(&self, value: T) -> Result<Request<T>> {
        let mut request = Request::new(value);
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {}", self.token)
                .parse()
                .map_err(|_| Error::unavailable())?,
        );
        request.set_timeout(self.request_timeout);
        Ok(request)
    }
    pub async fn get(&self, owner: &str, kind: &str, id: &str) -> Result<Record> {
        deadline(self.request_timeout, async {
            let key = RecordKey {
                owner: owner.into(),
                kind: kind.into(),
                id: id.into(),
            };
            let record = self
                .client()
                .await?
                .get(self.request(key)?)
                .await
                .map_err(status)?
                .into_inner();
            let record = decode(record, owner, kind)?;
            if record.id != id {
                return Err(Error::unavailable());
            }
            Ok(record)
        })
        .await
    }
    pub async fn list(&self, owner: &str, kind: &str) -> Result<Vec<Record>> {
        deadline(self.request_timeout, async {
            let key = CollectionKey {
                owner: owner.into(),
                kind: kind.into(),
            };
            let stream = self
                .client()
                .await?
                .list(self.request(key)?)
                .await
                .map_err(status)?
                .into_inner();
            collect(stream, owner, kind).await
        })
        .await
    }
    pub async fn batch(&self, items: Vec<Write>) -> Result<Vec<Record>> {
        deadline(self.request_timeout, async {
            let mutations = items.iter().map(encode).collect::<Result<Vec<_>>>()?;
            let response = self
                .client()
                .await?
                .write(self.request(WriteBatch { items: mutations })?)
                .await
                .map_err(status)?
                .into_inner();
            if response.records.len() != items.len() {
                return Err(Error::unavailable());
            }
            response
                .records
                .into_iter()
                .zip(items)
                .map(decode_write)
                .collect()
        })
        .await
    }
}
async fn collect(
    mut stream: tonic::Streaming<proto::Record>,
    owner: &str,
    kind: &str,
) -> Result<Vec<Record>> {
    let mut rows = Vec::new();
    let mut bytes = 0;
    let mut seen = HashSet::new();
    while let Some(record) = stream.message().await.map_err(status)? {
        bytes += record.data_json.len();
        if bytes > MAX_LIST_BYTES || rows.len() >= 10_000 || !seen.insert(record.id.clone()) {
            return Err(Error::unavailable());
        }
        rows.push(decode(record, owner, kind)?);
    }
    Ok(rows)
}
fn decode_write((record, write): (proto::Record, Write)) -> Result<Record> {
    if record.id != write.id || Some(record.version) != write.expected_version.checked_add(1) {
        return Err(Error::unavailable());
    }
    let record = decode(record, &write.owner, &write.kind)?;
    let mut expected = write.data;
    if write.kind == "event" {
        let sequence = record.data["sequence"].as_i64();
        if expected["sequence"].as_i64() != Some(0) || !sequence.is_some_and(|value| value > 0) {
            return Err(Error::unavailable());
        }
        expected["sequence"] = serde_json::json!(sequence.unwrap());
    }
    if record.data != expected {
        return Err(Error::unavailable());
    }
    Ok(record)
}
fn encode(write: &Write) -> Result<proto::Mutation> {
    Ok(proto::Mutation {
        owner: write.owner.clone(),
        kind: write.kind.clone(),
        id: write.id.clone(),
        expected_version: write.expected_version,
        data_json: serde_json::to_vec(&write.data)?,
    })
}
fn decode(record: proto::Record, owner: &str, kind: &str) -> Result<Record> {
    if record.owner != owner
        || record.kind != kind
        || record.version < 1
        || record.created_at < 0
        || !crate::domain::valid_id(&record.id)
        || record.data_json.len() > 2_000_000
    {
        return Err(Error::unavailable());
    }
    let data: serde_json::Value =
        serde_json::from_slice(&record.data_json).map_err(|_| Error::unavailable())?;
    if !data.is_object() {
        return Err(Error::unavailable());
    }
    Ok(Record {
        owner: record.owner,
        kind: record.kind,
        id: record.id,
        version: record.version,
        data,
        created_at: record.created_at,
    })
}
fn status(error: Status) -> Error {
    match error.code() {
        tonic::Code::NotFound => Error::missing(),
        tonic::Code::AlreadyExists => Error(
            axum::http::StatusCode::CONFLICT,
            "This record changed; reload before saving".into(),
        ),
        tonic::Code::InvalidArgument => Error::bad("Invalid record"),
        _ => Error::unavailable(),
    }
}
async fn deadline<T>(duration: Duration, work: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(duration, work)
        .await
        .map_err(|_| Error::unavailable())?
}

#[cfg(test)]
#[path = "store_grpc_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "store_grpc_stream_tests.rs"]
mod stream_tests;

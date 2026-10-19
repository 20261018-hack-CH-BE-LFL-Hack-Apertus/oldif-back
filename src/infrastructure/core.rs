use crate::{config::Config, error::Result};
use serde_json::{Value, json};
use tonic::{Request, transport::Channel};
pub mod proto {
    tonic::include_proto!("oldif.ai.v1");
}
use proto::{Empty, JsonPayload, RunRef, core_client::CoreClient};
#[derive(Clone)]
pub struct Core {
    url: String,
    token: String,
}
impl Core {
    pub fn new(config: &Config) -> Self {
        Self {
            url: config.core_grpc_url.clone(),
            token: config.token.clone(),
        }
    }
    async fn client(&self) -> Result<CoreClient<Channel>> {
        Ok(CoreClient::connect(self.url.clone()).await?)
    }
    fn request<T>(&self, value: T) -> Request<T> {
        let mut request = Request::new(value);
        if let Ok(token) = format!("Bearer {}", self.token).parse() {
            request.metadata_mut().insert("authorization", token);
        }
        request
    }
    pub async fn models(&self) -> Result<Value> {
        let body = self
            .client()
            .await?
            .get_models(self.request(Empty {}))
            .await?
            .into_inner();
        Ok(serde_json::from_str(&body.json)?)
    }
    pub async fn run(&self, input: Value) -> Result<Value> {
        let response = self
            .client()
            .await?
            .start_run(self.request(JsonPayload {
                json: input.to_string(),
            }))
            .await?
            .into_inner();
        Ok(json!({"run_id":response.run_id,"status":response.status}))
    }
    pub async fn events(&self, id: &str, after: u64) -> Result<tonic::Streaming<JsonPayload>> {
        Ok(self
            .client()
            .await?
            .stream_events(self.request(RunRef {
                run_id: id.into(),
                after_sequence: after,
                status: String::new(),
            }))
            .await?
            .into_inner())
    }
}

use crate::{config::Config, error::Result};
use serde_json::{Value, json};
use std::{future::Future, time::Duration};
use tonic::{Request, transport::Channel};
pub mod proto {
    tonic::include_proto!("oldif.ai.v1");
}
use proto::{Empty, JsonPayload, RunRef, core_client::CoreClient};
#[derive(Clone)]
pub struct Core {
    url: String,
    token: String,
    connect_timeout: Duration,
    request_timeout: Duration,
}
impl Core {
    pub fn new(config: &Config) -> Self {
        Self {
            url: config.core_grpc_url.clone(),
            token: config.core_token.clone(),
            connect_timeout: Duration::from_secs(config.core_connect_seconds.clamp(1, 120)),
            request_timeout: Duration::from_secs(config.core_request_seconds.clamp(1, 120)),
        }
    }
    async fn client(&self) -> Result<CoreClient<Channel>> {
        deadline(self.connect_timeout, async {
            let channel = Channel::from_shared(self.url.clone())
                .map_err(|_| crate::error::Error::unavailable())?
                .connect_timeout(self.connect_timeout)
                .connect()
                .await?;
            Ok(CoreClient::new(channel))
        })
        .await
    }
    fn request<T>(&self, value: T) -> Request<T> {
        let mut request = Request::new(value);
        if let Ok(token) = format!("Bearer {}", self.token).parse() {
            request.metadata_mut().insert("authorization", token);
        }
        request
    }
    pub async fn models(&self) -> Result<Value> {
        deadline(self.request_timeout, async {
            let mut request = self.request(Empty {});
            request.set_timeout(self.request_timeout);
            let body = self.client().await?.get_models(request).await?.into_inner();
            Ok(serde_json::from_str(&body.json)?)
        })
        .await
    }
    pub async fn run(&self, input: Value) -> Result<Value> {
        deadline(self.request_timeout, async {
            let mut request = self.request(JsonPayload {
                json: input.to_string(),
            });
            request.set_timeout(self.request_timeout);
            let response = self.client().await?.start_run(request).await?.into_inner();
            Ok(json!({"run_id":response.run_id,"status":response.status}))
        })
        .await
    }
    pub async fn events(&self, id: &str, after: u64) -> Result<tonic::Streaming<JsonPayload>> {
        // Bound connection and response headers, not the returned stream's lifetime.
        // A grpc-timeout header would also tell the server to terminate the stream.
        deadline(self.request_timeout, async {
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
        })
        .await
    }
}

// Covers readiness, handshake and response decoding even after headers arrive.
async fn deadline<T>(duration: Duration, work: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(duration, work)
        .await
        .map_err(|_| crate::error::Error::timeout())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{io::AsyncReadExt, net::TcpListener, sync::oneshot, task::JoinHandle};

    async fn blackhole(
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> (Core, JoinHandle<()>, oneshot::Receiver<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (accepted, received) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut preface = [0_u8; 24];
            socket.read_exact(&mut preface).await.unwrap();
            assert_eq!(&preface, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
            let _ = accepted.send(());
            std::future::pending::<()>().await;
        });
        let core = Core {
            url,
            token: "fixture-not-a-credential".into(),
            connect_timeout,
            request_timeout,
        };
        (core, server, received)
    }

    #[tokio::test]
    async fn event_subscription_deadline_includes_stalled_handshake() {
        let deadline = Duration::from_millis(80);
        let (core, server, accepted) = blackhole(Duration::from_secs(2), deadline).await;
        let started = tokio::time::Instant::now();
        let result =
            tokio::time::timeout(Duration::from_millis(400), core.events("fixture", 0)).await;
        server.abort();
        accepted.await.unwrap();
        let error=result.expect("event subscription deadline was not enforced").err().expect("subscription should time out");
        assert_eq!(error.code(),"TIMEOUT");
        assert!(started.elapsed() >= deadline);
    }

    #[tokio::test]
    async fn unary_deadline_includes_connection_and_handshake() {
        for start_run in [false, true] {
            let deadline = Duration::from_millis(80);
            let (core, server, accepted) = blackhole(Duration::from_secs(2), deadline).await;
            let started = tokio::time::Instant::now();
            let result = tokio::time::timeout(Duration::from_millis(400), async {
                if start_run {
                    core.run(json!({})).await
                } else {
                    core.models().await
                }
            })
            .await;
            server.abort();
            accepted.await.unwrap();
            let error=result.expect("total unary deadline was not enforced").err().expect("unary should time out");
            assert_eq!(error.code(),"TIMEOUT");
            assert!(started.elapsed() >= deadline);
        }
    }

    #[derive(Clone)]
    struct DelayedFrames;
    impl tonic::server::NamedService for DelayedFrames {
        const NAME: &'static str = "oldif.ai.v1.Core";
    }
    impl tonic::codegen::Service<tonic::codegen::http::Request<tonic::body::Body>> for DelayedFrames {
        type Response = tonic::codegen::http::Response<tonic::body::Body>;
        type Error = std::convert::Infallible;
        type Future = tonic::codegen::BoxFuture<Self::Response, Self::Error>;
        fn poll_ready(
            &mut self,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn call(
            &mut self,
            request: tonic::codegen::http::Request<tonic::body::Body>,
        ) -> Self::Future {
            if request.uri().path().ends_with("StreamEvents") {
                assert!(!request.headers().contains_key("grpc-timeout"));
            }
            Box::pin(async move {
                let codec = tonic_prost::ProstCodec::<JsonPayload, Empty>::default();
                Ok(tonic::server::Grpc::new(codec)
                    .server_streaming(DelayedFrames, request)
                    .await)
            })
        }
    }
    impl tonic::server::ServerStreamingService<Empty> for DelayedFrames {
        type Response = JsonPayload;
        type ResponseStream = tonic::codegen::BoxStream<JsonPayload>;
        type Future =
            tonic::codegen::BoxFuture<tonic::Response<Self::ResponseStream>, tonic::Status>;
        fn call(&mut self, _: Request<Empty>) -> Self::Future {
            Box::pin(async {
                let stream = futures_util::stream::once(async {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    Ok(JsonPayload { json: "{}".into() })
                });
                Ok(tonic::Response::new(
                    Box::pin(stream) as Self::ResponseStream
                ))
            })
        }
    }
    async fn delayed_frames() -> (Core, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let incoming = futures_util::stream::unfold(listener, |listener| async {
            Some((listener.accept().await.map(|(socket, _)| socket), listener))
        });
        let server = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(DelayedFrames)
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });
        (
            Core {
                url,
                token: "fixture-not-a-credential".into(),
                connect_timeout: Duration::from_secs(2),
                request_timeout: Duration::from_millis(80),
            },
            server,
        )
    }
    #[tokio::test]
    async fn unary_deadline_covers_response_body_after_headers() {
        let (core, server) = delayed_frames().await;
        let result = tokio::time::timeout(Duration::from_millis(400), core.models()).await;
        server.abort();
        let error=result.expect("unary response body exceeded its deadline").err().expect("response body should time out");
        assert_eq!(error.code(),"TIMEOUT");
    }
    #[tokio::test]
    async fn established_event_stream_outlives_request_deadline() {
        let (core, server) = delayed_frames().await;
        let started = tokio::time::Instant::now();
        let mut stream = core.events("fixture", 0).await.unwrap();
        let item = tokio::time::timeout(Duration::from_secs(1), stream.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        server.abort();
        assert_eq!(item.json, "{}");
        assert!(started.elapsed() > core.request_timeout);
    }
}

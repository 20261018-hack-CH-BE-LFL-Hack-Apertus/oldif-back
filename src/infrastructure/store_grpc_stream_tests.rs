use super::*;
use tonic::codegen::{BoxFuture, http};

#[derive(Clone)]
struct Frames {
    duplicate: bool,
}
impl tonic::server::NamedService for Frames {
    const NAME: &'static str = "oldif.store.v1.RecordStore";
}
impl tonic::codegen::Service<http::Request<tonic::body::Body>> for Frames {
    type Response = http::Response<tonic::body::Body>;
    type Error = std::convert::Infallible;
    type Future = BoxFuture<Self::Response, Self::Error>;
    fn poll_ready(
        &mut self,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn call(&mut self, request: http::Request<tonic::body::Body>) -> Self::Future {
        let service = self.clone();
        Box::pin(async move {
            let codec = tonic_prost::ProstCodec::<proto::Record, CollectionKey>::default();
            Ok(tonic::server::Grpc::new(codec)
                .server_streaming(service, request)
                .await)
        })
    }
}
impl tonic::server::ServerStreamingService<CollectionKey> for Frames {
    type Response = proto::Record;
    type ResponseStream = tonic::codegen::BoxStream<proto::Record>;
    type Future = BoxFuture<tonic::Response<Self::ResponseStream>, Status>;
    fn call(&mut self, request: Request<CollectionKey>) -> Self::Future {
        assert!(request.metadata().contains_key("grpc-timeout"));
        let duplicate = self.duplicate;
        Box::pin(async move {
            let rows = futures_util::stream::unfold(0, move |index| async move {
                if index == 2 {
                    return None;
                }
                if index == 1 && !duplicate {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                let record = proto::Record {
                    owner: "alice".into(),
                    kind: "source".into(),
                    id: "one".into(),
                    version: 1,
                    data_json: b"{}".to_vec(),
                    created_at: 1,
                };
                Some((Ok(record), index + 1))
            });
            Ok(tonic::Response::new(Box::pin(rows) as Self::ResponseStream))
        })
    }
}
async fn fixture(duplicate: bool) -> (GrpcStore, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let incoming = futures_util::stream::unfold(listener, |listener| async {
        Some((listener.accept().await.map(|(socket, _)| socket), listener))
    });
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(Frames { duplicate })
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });
    let mut client = GrpcStore::new(url, "synthetic-test-token".into(), 1, 1);
    client.request_timeout = Duration::from_millis(80);
    (client, task)
}
#[tokio::test]
async fn list_deadline_includes_all_frames_and_never_returns_partial_success() {
    let (client, task) = fixture(false).await;
    let started = tokio::time::Instant::now();
    let result =
        tokio::time::timeout(Duration::from_millis(400), client.list("alice", "source")).await;
    task.abort();
    assert_eq!(
        result
            .expect("stream body exceeded deadline")
            .unwrap_err()
            .0
            .as_u16(),
        503
    );
    assert!(started.elapsed() >= Duration::from_millis(60));
}
#[tokio::test]
async fn duplicate_stream_records_are_rejected() {
    let (client, task) = fixture(true).await;
    assert_eq!(
        client.list("alice", "source").await.unwrap_err().0.as_u16(),
        503
    );
    task.abort();
}

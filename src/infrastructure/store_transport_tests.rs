use super::*;
use axum::{Json, Router, routing::any};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::net::TcpListener;

async fn http_fixture() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let route = any(move || {
        let counter = counter.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Json(json!({"records":[]}))
        }
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/{*path}", route))
            .await
            .unwrap();
    });
    (url, calls, task)
}
#[tokio::test]
async fn selected_grpc_never_falls_back_to_healthy_http_after_errors() {
    let (url, calls, task) = http_fixture().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let mut store = Store {
        client: reqwest::Client::new(),
        url,
        token: "synthetic-test-token".into(),
        grpc: Some(GrpcStore::new(grpc, "synthetic-test-token".into(), 1, 1)),
    };
    unavailable(store.get("alice", "source", "one").await);
    unavailable(store.list("alice", "source").await);
    unavailable(store.batch(vec![]).await);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    store.grpc = None;
    assert!(store.list("alice", "source").await.unwrap().is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    task.abort();
}
fn unavailable<T>(result: Result<T>) {
    assert!(matches!(result, Err(Error(status, _)) if status.as_u16() == 503));
}

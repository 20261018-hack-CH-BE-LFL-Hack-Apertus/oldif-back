use super::*;
use serde_json::json;
use tokio::{io::AsyncReadExt, net::TcpListener};

fn record() -> proto::Record {
    proto::Record {
        owner: "alice".into(),
        kind: "source".into(),
        id: "source-one".into(),
        version: 1,
        created_at: 1,
        data_json: br#"{"text":"exact original"}"#.to_vec(),
    }
}
#[test]
fn rejects_foreign_or_malformed_database_records() {
    assert_eq!(
        decode(record(), "alice", "source").unwrap().data["text"],
        "exact original"
    );
    assert!(decode(record(), "bob", "source").is_err());
    assert!(decode(record(), "alice", "run").is_err());
    let mut wrong = record();
    wrong.data_json = b"[]".to_vec();
    assert!(decode(wrong, "alice", "source").is_err());
    let mut wrong = record();
    wrong.id = "../one".into();
    assert!(decode(wrong, "alice", "source").is_err());
    let mut wrong = record();
    wrong.version = 0;
    assert!(decode(wrong, "alice", "source").is_err());
    let mut wrong = record();
    wrong.data_json = vec![b'x'; 2_000_001];
    assert!(decode(wrong, "alice", "source").is_err());
}
#[test]
fn validates_write_receipt_identity_revision_and_exact_content() {
    let write = Write::new(
        "alice",
        "source",
        "source-one",
        json!({"text":"exact original"}),
    );
    assert!(decode_write((record(), write.clone())).is_ok());
    let mut wrong = record();
    wrong.version = 2;
    assert!(decode_write((wrong, write.clone())).is_err());
    let mut wrong = record();
    wrong.data_json = br#"{"text":"altered"}"#.to_vec();
    assert!(decode_write((wrong, write.clone())).is_err());
    let mut overflow = write;
    overflow.expected_version = i64::MAX;
    assert!(decode_write((record(), overflow)).is_err());
}
#[test]
fn maps_only_expected_storage_errors_without_exposing_internal_details() {
    for (code, expected) in [
        (tonic::Code::NotFound, 404),
        (tonic::Code::AlreadyExists, 409),
        (tonic::Code::InvalidArgument, 400),
        (tonic::Code::Unauthenticated, 503),
        (tonic::Code::Unavailable, 503),
        (tonic::Code::DataLoss, 503),
    ] {
        let mapped = status(Status::new(code, "private database diagnostic"));
        assert_eq!(mapped.0.as_u16(), expected);
        assert!(!mapped.1.contains("private"));
    }
}
async fn blackhole() -> (GrpcStore, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut preface = [0; 24];
        socket.read_exact(&mut preface).await.unwrap();
        assert_eq!(&preface, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
        std::future::pending::<()>().await;
    });
    let mut client = GrpcStore::new(url, "synthetic-test-token".into(), 2, 2);
    client.request_timeout = Duration::from_millis(80);
    (client, task)
}
#[tokio::test]
async fn all_database_calls_bound_stalled_connection_handshakes() {
    for operation in 0..3 {
        let (client, task) = blackhole().await;
        let started = tokio::time::Instant::now();
        let result = tokio::time::timeout(Duration::from_millis(400), async {
            match operation {
                0 => client.get("alice", "source", "one").await.map(|_| ()),
                1 => client.list("alice", "source").await.map(|_| ()),
                _ => client
                    .batch(vec![Write::new("alice", "source", "one", json!({}))])
                    .await
                    .map(|_| ()),
            }
        })
        .await;
        task.abort();
        assert_eq!(
            result
                .expect("call exceeded deadline")
                .unwrap_err()
                .0
                .as_u16(),
            503
        );
        assert!(started.elapsed() >= client.request_timeout);
    }
}

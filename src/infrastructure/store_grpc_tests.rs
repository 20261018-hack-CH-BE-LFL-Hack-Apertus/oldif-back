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
fn event_receipt() -> (proto::Record, Write) {
    let data = json!({"sequence":0,"event_id":"event-one","type":"started","run_id":"run-one","session_id":"session-one","data":{"status":"running"}});
    let write = Write::new("alice", "event", "event-one", data.clone());
    let mut receipt = record();
    receipt.kind = "event".into();
    receipt.id = "event-one".into();
    let mut committed = data;
    committed["sequence"] = json!(631);
    receipt.data_json = serde_json::to_vec(&committed).unwrap();
    (receipt, write)
}
#[test]
fn accepts_only_database_assigned_event_sequence() {
    let (receipt, write) = event_receipt();
    let committed = decode_write((receipt, write)).unwrap();
    assert_eq!(committed.data["sequence"], 631);
    assert_eq!(committed.data["data"]["status"], "running");
}
#[test]
fn rejects_invalid_event_sequences_and_other_content_changes() {
    for sequence in [json!(0), json!(-1), json!(1.5), json!("631"), json!(null), json!(u64::MAX)] {
        let (mut receipt, write) = event_receipt();
        let mut data: serde_json::Value = serde_json::from_slice(&receipt.data_json).unwrap();
        data["sequence"] = sequence;
        receipt.data_json = serde_json::to_vec(&data).unwrap();
        assert!(decode_write((receipt, write)).is_err());
    }
    for key in ["event_id", "type", "run_id", "session_id", "data", "extra"] {
        let (mut receipt, write) = event_receipt();
        let mut data: serde_json::Value = serde_json::from_slice(&receipt.data_json).unwrap();
        data[key] = json!("altered");
        receipt.data_json = serde_json::to_vec(&data).unwrap();
        assert!(decode_write((receipt, write)).is_err(), "accepted changed {key}");
    }
    let (receipt, mut write) = event_receipt();
    write.data["sequence"] = json!(631);
    assert!(decode_write((receipt, write)).is_err());
}
#[test]
fn event_sequence_exception_cannot_change_identity_or_other_record_kinds() {
    for field in ["owner", "kind", "id", "version"] {
        let (mut receipt, write) = event_receipt();
        match field {
            "owner" => receipt.owner = "bob".into(),
            "kind" => receipt.kind = "source".into(),
            "id" => receipt.id = "event-other".into(),
            _ => receipt.version = 2,
        }
        assert!(decode_write((receipt, write)).is_err());
    }
    let (mut receipt, mut write) = event_receipt();
    receipt.kind = "source".into();
    write.kind = "source".into();
    assert!(decode_write((receipt, write)).is_err());
}
#[test]
fn maps_only_expected_storage_errors_without_exposing_internal_details() {
    for (code, expected, machine_code) in [
        (tonic::Code::NotFound, 404, "NOT_FOUND"),
        (tonic::Code::AlreadyExists, 409, "REVISION_CONFLICT"),
        (tonic::Code::Aborted, 409, "REVISION_CONFLICT"),
        (tonic::Code::InvalidArgument, 400, "INVALID_REQUEST"),
        (tonic::Code::DeadlineExceeded, 504, "TIMEOUT"),
        (tonic::Code::Cancelled, 499, "CANCELLED"),
        (tonic::Code::ResourceExhausted, 429, "BACKPRESSURE"),
        (tonic::Code::Unauthenticated, 503, "UNAVAILABLE"),
        (tonic::Code::Unavailable, 503, "UNAVAILABLE"),
        (tonic::Code::DataLoss, 503, "UNAVAILABLE"),
    ] {
        let mapped = status(Status::new(code, "private database diagnostic"));
        assert_eq!(mapped.0.as_u16(), expected);
        assert_eq!(mapped.code(), machine_code);
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
        let error = result.expect("call exceeded deadline").unwrap_err();
        assert_eq!(error.0.as_u16(), 504);
        assert_eq!(error.code(), "TIMEOUT");
        assert!(started.elapsed() >= client.request_timeout);
    }
}

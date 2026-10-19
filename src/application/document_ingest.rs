use super::documents::Metadata;
use crate::{
    error::{Error, Result},
    infrastructure::{
        document_parser::digest,
        store::{Record, Write},
    },
    state::App,
};
use axum::http::StatusCode;
use serde_json::json;

pub struct Identity {
    pub id: String,
    pub fingerprint: String,
    pub original_sha256: String,
    pub legacy_id: Option<String>,
}
impl Identity {
    pub fn new(
        owner: &str,
        filename: &str,
        media: &str,
        input: &Metadata,
        key: Option<&str>,
        bytes: &[u8],
    ) -> Result<Self> {
        if key.is_some_and(|s| {
            s.is_empty()
                || s.len() > 128
                || !s
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.:".contains(&c))
        }) {
            return Err(Error::bad(
                "Idempotency-Key must contain 1 to 128 safe identifier characters",
            ));
        }
        let original_sha256 = digest(bytes);
        let legacy = json!(["content-upload-v1", owner, filename, original_sha256]);
        let distribution=input.acquisition.as_ref().map(|a|a.distribution_uri.as_str()).filter(|v|!v.is_empty());
        let identity = if let Some(key) = key {
            json!(["explicit-upload-key-v1", owner, key])
        } else if let Some(distribution) = distribution {
            json!(["content-distribution-v2", owner, original_sha256, distribution])
        } else {
            legacy.clone()
        };
        let fingerprint=digest(json!({"filename":filename,"media":media,"metadata":input,"original_sha256":original_sha256}).to_string().as_bytes());
        Ok(Self {
            id: digest(identity.to_string().as_bytes()),
            fingerprint,
            original_sha256,
            legacy_id: if key.is_none() && distribution.is_some() {Some(digest(legacy.to_string().as_bytes()))} else {None},
        })
    }
}
pub async fn existing(app: &App, owner: &str, identity: &Identity) -> Result<Option<Record>> {
    match app.store.get(owner, "document", &identity.id).await {
        Ok(record) => {
            if record.data["ingest_fingerprint"] != identity.fingerprint {
                return Err(Error(StatusCode::CONFLICT,"This upload identity already belongs to different bytes or metadata; review the existing document or use a new key".into()));
            }
            Ok(Some(record))
        }
        Err(error) if error.0 == StatusCode::NOT_FOUND => {
            if let Some(id)=&identity.legacy_id {
                match app.store.get(owner,"document",id).await {
                    Ok(record) if record.data["ingest_fingerprint"]==identity.fingerprint=>return Ok(Some(record)),
                    Ok(_)=>{},
                    Err(error) if error.0==StatusCode::NOT_FOUND=>{},
                    Err(error)=>return Err(error),
                }
            }
            Ok(None)
        },
        Err(error) => Err(error),
    }
}
pub async fn persist_batch(app: &App, items: Vec<Write>, resume: bool) -> Result<()> {
    let mut pending = vec![];
    for item in items {
        if resume {
            match app.store.get(&item.owner, &item.kind, &item.id).await {
                Ok(existing) if existing.data == item.data => continue,
                Ok(_) => {
                    return Err(Error(
                        StatusCode::CONFLICT,
                        "An immutable upload fragment differs from this retry".into(),
                    ));
                }
                Err(error) if error.0 == StatusCode::NOT_FOUND => {}
                Err(error) => return Err(error),
            }
        }
        pending.push(item);
    }
    if !pending.is_empty() {
        app.store.batch(pending).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_identity_binds_distribution_without_rekeying_explicit_retries() {
        let mut input=Metadata{title:None,licence:"synthetic".into(),rights_confirmed:true,acquisition:Some(super::super::acquisition::Acquisition{distribution_uri:"urn:fixture:a".into(),..Default::default()})};
        let first=Identity::new("owner","one.txt","text/plain",&input,None,b"same").unwrap();
        let renamed=Identity::new("owner","two.txt","text/plain",&input,None,b"same").unwrap();
        assert_eq!(first.id,renamed.id);
        assert_ne!(first.fingerprint,renamed.fingerprint);
        assert!(first.legacy_id.is_some());
        input.acquisition.as_mut().unwrap().distribution_uri="urn:fixture:b".into();
        assert_ne!(first.id,Identity::new("owner","one.txt","text/plain",&input,None,b"same").unwrap().id);
        let explicit=Identity::new("owner","one.txt","text/plain",&input,Some("retry"),b"same").unwrap();
        assert_eq!(explicit.id,digest(json!(["explicit-upload-key-v1","owner","retry"]).to_string().as_bytes()));
        assert!(explicit.legacy_id.is_none());
    }
    #[test]
    fn upload_identity_binds_owner_content_and_retry_metadata() {
        let input = Metadata {
            title: None,
            licence: "synthetic".into(),
            rights_confirmed: true,
            acquisition: None,
        };
        let first = Identity::new(
            "owner",
            "sample.txt",
            "text/plain",
            &input,
            None,
            b"original",
        )
        .unwrap();
        let same = Identity::new(
            "owner",
            "sample.txt",
            "text/plain",
            &input,
            None,
            b"original",
        )
        .unwrap();
        assert_eq!(first.id, same.id);
        assert_eq!(first.fingerprint, same.fingerprint);
        assert_ne!(
            first.id,
            Identity::new(
                "other",
                "sample.txt",
                "text/plain",
                &input,
                None,
                b"original"
            )
            .unwrap()
            .id
        );
        let keyed = Identity::new(
            "owner",
            "sample.txt",
            "text/plain",
            &input,
            Some("retry-1"),
            b"original",
        )
        .unwrap();
        let changed = Identity::new(
            "owner",
            "sample.txt",
            "text/plain",
            &input,
            Some("retry-1"),
            b"changed",
        )
        .unwrap();
        assert_eq!(keyed.id, changed.id);
        assert_ne!(keyed.fingerprint, changed.fingerprint);
        assert!(
            Identity::new(
                "owner",
                "sample.txt",
                "text/plain",
                &input,
                Some("../escape"),
                b"original"
            )
            .is_err()
        );
    }
}

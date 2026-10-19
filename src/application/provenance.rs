use crate::application::auth::now;
use serde_json::{Value,json};

pub fn transformation(operation:&str,reason:&str,actor:&str,run_id:&str,parent:Value,original:Value,normalized:Value)->Value {
    json!({"schema_version":"1.0","operation":operation,"reason":reason,"actor":actor,"run_id":run_id,"parent_revision":parent,"timestamp":now(),"timestamp_unit":"unix_seconds","original":original,"normalized":normalized})
}

pub fn normalization(value:&Value)->Value {
    if value["transformation"]["schema_version"]=="1.0" {return value["transformation"].clone();}
    json!({"schema_version":"1.0","provenance_status":"legacy_unknown","operation":"preserve_legacy_normalization_record","reason":"Historical actor, run and time were not recorded; original legacy fields remain unchanged","actor":null,"run_id":null,"parent_revision":null,"timestamp":null,"timestamp_unit":"unix_seconds","missing_fields":["actor","run_id","parent_revision","timestamp"],"original":{"legacy_transformation":value["transformation"],"source_sha256":value["sha256"],"value_kind":"immutable_legacy_record_reference","source_id":value["id"]},"normalized":{"source_id":value["id"],"text_sha256":crate::infrastructure::document_parser::digest(value["text"].as_str().unwrap_or("").as_bytes()),"normalized_text":value["normalized_text"]}})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_envelope_does_not_invent_actor_or_time() {
        let original=json!({"id":"old","text":"text","transformation":{"operation":"old-import"}});
        let result=normalization(&original);assert!(result["actor"].is_null());assert!(result["timestamp"].is_null());
        assert_eq!(result["original"]["legacy_transformation"],original["transformation"]);assert_eq!(original["transformation"]["operation"],"old-import");
    }
}

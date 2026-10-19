use crate::application::auth::now;
use serde_json::{Value,json};

pub fn transformation(operation:&str,reason:&str,actor:&str,run_id:&str,parent:Value,original:Value,normalized:Value)->Value {
    json!({"schema_version":"1.0","operation":operation,"reason":reason,"actor":actor,"run_id":run_id,"parent_revision":parent,"timestamp":now(),"timestamp_unit":"unix_seconds","original":original,"normalized":normalized})
}

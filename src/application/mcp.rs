use crate::domain;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search { query: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source { id: String }

pub fn valid_arguments(name: &str, value: &Value) -> bool {
    match name {
        "search_archives" => serde_json::from_value::<Search>(value.clone()).is_ok_and(|a| domain::valid_question(&a.query)),
        "get_source" => serde_json::from_value::<Source>(value.clone()).is_ok_and(|a| domain::valid_id(&a.id)),
        _ => true, // Unknown names are rejected by the dispatch allowlist.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn tools_reject_extra_untyped_or_unbounded_arguments() {
        assert!(valid_arguments("get_source", &json!({"id":"stapfer-2027"})));
        assert!(!valid_arguments("get_source", &json!({"id":"stapfer-2027","url":"https://invalid.example"})));
        assert!(!valid_arguments("get_source", &json!({"id":"../private"})));
        assert!(!valid_arguments("search_archives", &json!({"query":7})));
        assert!(!valid_arguments("search_archives", &json!({"query":""})));
        assert!(!valid_arguments("search_archives", &json!({"query":"x".repeat(8001)})));
    }
}

use crate::{error::{Error,Result},infrastructure::document_parser::digest};
use serde_json::{Value,json};

pub fn scope(value:Option<&str>)->Result<&str>{
    let scope=value.unwrap_or("private");
    if !["private","public"].contains(&scope){return Err(Error::bad("Export scope must be private or public"));}
    Ok(scope)
}
pub fn allowlist(row:&Value,kind:&str)->Value{
    let fields=if kind=="source" {
        &["id","title","text","url","page","file_page_index","printed_page_label","printed_page_label_missing_reason","sha256","text_sha256","original_sha256","record_hash_scope","licence","document_id","dataset_id","transcription_method","transcription_review_status"][..]
    } else { &["schema_version","run_id","revision","model","result"][..] };
    let mut out=serde_json::Map::new();for key in fields {if let Some(value)=row.get(*key){out.insert((*key).into(),value.clone());}}
    if kind!="source" {
        let fields=["schema_version","answer","citations","entities","timeline","uncertainty","verification","proposals","assertions","assertion_validation","evidence_only","interpretation_status","abstained"];
        let result=fields.iter().filter_map(|key|row["result"].get(*key).map(|value|((*key).to_owned(),value.clone()))).collect::<serde_json::Map<_,_>>();
        out.insert("result".into(),Value::Object(result));out.insert("review".into(),json!({"decision":"accepted"}));
    }
    Value::Object(out)
}
pub fn source_manifest(rows:&[Value])->Value{
    json!(rows.iter().map(|row|json!({"source_id":row["id"],"document_id":row["document_id"],"revision":row["version"],"sha256":row["sha256"],"original_sha256":row["original_sha256"],"text_sha256":row["text_sha256"],"record_hash_scope":row["record_hash_scope"],"page":row["page"],"file_page_index":row["file_page_index"],"licence":row["licence"]})).collect::<Vec<_>>())
}
pub fn build(rows:&[Value],kind:&str,scope:&str,sources:Value,changes:Value,rejections:Value)->Result<Value>{
    let records=rows.iter().map(|data|json!({"schema_version":"1.0","kind":kind,"data":data})).collect::<Vec<_>>();
    let jsonl=records.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")+if records.is_empty(){""}else{"\n"};
    if jsonl.len()>24*1024*1024{return Err(Error::bad("Export exceeds24MiB of source records; choose a smaller scope"));}
    let mut files=vec![];
    add(&mut files,"records.jsonl","application/x-ndjson",jsonl);
    for (name,separator,media) in [("records.tsv",'\t',"text/tab-separated-values"),("records.csv",',',"text/csv")] {
        let mut lines=vec![format!("record_id{separator}record_json")];
        lines.extend(records.iter().map(|r|format!("{}{separator}{}",cell(r["data"]["id"].as_str().or_else(||r["data"]["run_id"].as_str()).unwrap_or("")),cell(&r.to_string()))));
        add(&mut files,name,media,lines.join("\n")+"\n");
    }
    let schema=json!({"$schema":"http://json-schema.org/draft-07/schema#","$id":"oldif-export-record/1.0","type":"object","additionalProperties":false,"required":["schema_version","kind","data"],"properties":{"schema_version":{"const":"1.0"},"kind":{"const":kind},"data":{"type":"object","required":if kind=="source"{vec!["id","title","text","sha256","licence"]}else{vec!["run_id","revision","result","review","model"]}}}});
    add(&mut files,"schema.json","application/schema+json",schema.to_string());
    add(&mut files,"source-manifest.json","application/json",json!({"schema_version":"1.0","sources":sources}).to_string());
    add(&mut files,"change-log.json","application/json",json!({"schema_version":"1.0","changes":changes}).to_string());
    add(&mut files,"rejection-manifest.json","application/json",json!({"schema_version":"1.0","rejections":rejections}).to_string());
    let content_hash=digest(serde_json::to_vec(&files)?.as_slice());
    Ok(json!({"schema_version":"1.0","format":"oldif-export-bundle","scope":scope,"record_kind":kind,"record_count":records.len(),"publication_performed":false,"encoding":"UTF-8","bundle_sha256":content_hash,"files":files}))
}
fn add(files:&mut Vec<Value>,name:&str,media:&str,content:String){files.push(json!({"name":name,"media_type":media,"sha256":digest(content.as_bytes()),"bytes":content.len(),"content":content}));}
fn cell(value:&str)->String{let safe=if value.trim_start().starts_with(['=','+','-','@']){format!("'{value}")}else{value.into()};format!("\"{}\"",safe.replace('"',"\"\""))}
#[cfg(test)]
mod tests{
 use super::*;
 #[test]
 fn bundle_preserves_multiline_records_hashes_and_manifest_files(){
  let row=json!({"id":"a","title":"Title","text":"old\ntext\twith ü","sha256":"hash","licence":"synthetic","internal_employee":"private","metadata":{"payroll":"excluded"}});
  let public=allowlist(&row,"source");assert!(public.get("internal_employee").is_none());assert!(public.get("metadata").is_none());
  let bundle=build(&[public.clone()],"source","public",source_manifest(&[public]),json!([]),json!([])).unwrap();
  assert_eq!(bundle["files"].as_array().unwrap().len(),7);assert_eq!(bundle["publication_performed"],false);
  for file in bundle["files"].as_array().unwrap(){assert_eq!(file["sha256"],digest(file["content"].as_str().unwrap().as_bytes()));}
  let record:Value=serde_json::from_str(bundle["files"][0]["content"].as_str().unwrap()).unwrap();assert_eq!(record["data"]["text"],row["text"]);
  assert!(scope(Some("untrusted")).is_err());
 }
}

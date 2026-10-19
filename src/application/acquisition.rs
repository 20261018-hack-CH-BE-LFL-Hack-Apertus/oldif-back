use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Acquisition {
    pub dataset_uri: String,
    pub distribution_uri: String,
    pub retrieved_at: String,
    pub sha256: String,
    pub rights_identifier: String,
    pub allowed_uses: Vec<String>,
}
pub const USES: &[&str] = &["local_processing", "provider_processing", "private_export", "public_export"];
impl Acquisition {
    pub fn assess(&self, actual_hash:&str, confirmed:bool)->Result<Vec<String>> {
        let mut missing=Vec::new();
        for (name,value) in [("dataset_uri",&self.dataset_uri),("distribution_uri",&self.distribution_uri)] {
            if value.is_empty() { missing.push(name.into()); }
            else if value.len()>2048 || reqwest::Url::parse(value).ok().is_none_or(|url| !["http","https","urn"].contains(&url.scheme()) || !url.username().is_empty() || url.password().is_some()) { return Err(Error::bad("Acquisition URIs must be bounded HTTP(S) or URN identifiers without credentials")); }
        }
        if self.retrieved_at.is_empty() { missing.push("retrieved_at".into()); }
        else if !timestamp(&self.retrieved_at) { return Err(Error::bad("retrieved_at must be an RFC3339 timestamp")); }
        if self.sha256.is_empty() { missing.push("sha256".into()); }
        else if self.sha256!=actual_hash { return Err(Error::bad("Acquisition SHA-256 does not match the uploaded original")); }
        if self.rights_identifier.trim().is_empty() || ["unknown","unresolved","not_assessed"].contains(&self.rights_identifier.to_ascii_lowercase().as_str()) { missing.push("rights_identifier".into()); }
        else if self.rights_identifier.len()>200 { return Err(Error::bad("Rights identifier exceeds200bytes")); }
        if self.allowed_uses.len()>USES.len() || self.allowed_uses.iter().any(|v| !USES.contains(&v.as_str())) || self.allowed_uses.iter().collect::<std::collections::HashSet<_>>().len()!=self.allowed_uses.len() { return Err(Error::bad("Choose distinct allowed uses from the acquisition contract")); }
        if !self.allowed_uses.iter().any(|v|v=="local_processing") { missing.push("allowed_uses.local_processing".into()); }
        if !confirmed { missing.push("rights_confirmed".into()); }
        Ok(missing)
    }
}
fn timestamp(value:&str)->bool {
    let b=value.as_bytes(); if b.len()<20 || !value.is_ascii() || b[4]!=b'-'||b[7]!=b'-'||b[10]!=b'T'||b[13]!=b':'||b[16]!=b':' { return false; }
    let number=|a,b|value.get(a..b).and_then(|s|s.parse::<u32>().ok());
    let (Some(year),Some(month),Some(day),Some(hour),Some(minute),Some(second))=(number(0,4),number(5,7),number(8,10),number(11,13),number(14,16),number(17,19)) else{return false;};
    if year==0||!(1..=12).contains(&month)||hour>23||minute>59||second>59{return false;}
    let leap=year%4==0&&(year%100!=0||year%400==0);
    let days=if month==2 {if leap{29}else{28}} else if [4,6,9,11].contains(&month){30}else{31};
    if day==0||day>days{return false;}
    let mut offset=19;
    if b[offset]==b'.' {offset+=1;let start=offset;while offset<b.len()&&b[offset].is_ascii_digit(){offset+=1;}if offset==start||offset-start>9{return false;}}
    if value.get(offset..)==Some("Z"){return true;}
    if b.len()!=offset+6||![b'+',b'-'].contains(&b[offset])||b[offset+3]!=b':'{return false;}
    matches!((number(offset+1,offset+3),number(offset+4,offset+6)),(Some(h),Some(m)) if h<=23&&m<=59)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_rights_are_quarantined_but_false_hashes_and_unsafe_uris_reject() {
        let mut a=Acquisition::default();assert_eq!(a.assess("hash",false).unwrap().len(),7);
        a=Acquisition{dataset_uri:"urn:synthetic:dataset".into(),distribution_uri:"urn:synthetic:distribution".into(),retrieved_at:"2026-10-19T12:00:00.123Z".into(),sha256:"hash".into(),rights_identifier:"CC0-1.0".into(),allowed_uses:vec!["local_processing".into()]};
        assert!(a.assess("hash",true).unwrap().is_empty());assert!(a.assess("other",true).is_err());
        a.dataset_uri="javascript:alert(1)".into();assert!(a.assess("hash",true).is_err());
        assert!(!timestamp("2026-02-30T12:00:00Z"));assert!(timestamp("2024-02-29T12:00:00+02:00"));
    }
}

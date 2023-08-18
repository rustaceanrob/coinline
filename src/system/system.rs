#![allow(unused_variables, unused_imports, unused_assignments)]
use bitcoin::bip32::{ExtendedPubKey, Fingerprint};
use regex::Regex;
use serde::Serialize;
use xyzpub::{convert_version, Version};
use std::{error::Error, fs::{File, self}, io::{Read, self, ErrorKind, Write}, str::FromStr, path::PathBuf};
use serde_json::Value;
use walkdir::WalkDir;
use dirs;
use crate::wallet::actions::compute_address;


pub fn import_coldcard_from_json(path: PathBuf) -> Result<(Fingerprint, ExtendedPubKey), Box<dyn Error>> {
    let mut file = File::open(path)?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    let json_data: Value = serde_json::from_str(&content)?;
    let master_xpub = &json_data["bip84"]["xpub"].to_string();
    let master_fp = &json_data["xfp"].to_string();
    let master_first_addr = &json_data["bip84"]["first"];
    let cleaned_first_addr: String = master_first_addr.to_string().chars().filter(|c| c.is_alphanumeric()).collect();
    let str_xpub: String = master_xpub.chars().filter(|c| c.is_alphanumeric()).collect();
    let test_first_addr = compute_address(str_xpub.as_str(), true, 0)?.to_string();
    assert_eq!(cleaned_first_addr, test_first_addr);
    let fingerprint = Fingerprint::from_str(&master_fp.to_string().replace("\"", ""))?;
    let root = ExtendedPubKey::from_str(&master_xpub.to_string().replace("\"", ""))?;
    Ok((fingerprint, root))
}

pub fn import_keystone_from_txt(path: PathBuf) -> Result<(Fingerprint, ExtendedPubKey), Box<dyn Error>> {
    let mut file = File::open(path)?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    let start_index = content.find('[').unwrap() + 1;
    let end_index = content.find('/').unwrap();
    let fp: &str = &content[start_index..end_index];
    let fingerprint = Fingerprint::from_str(fp)?;

    let re: Regex = Regex::new(r"zpub[^/]+")?;
    if let Some(capture) = re.find(&content) {
        let extracted_part = capture.as_str();
        let result = convert_version(extracted_part, &Version::Xpub).expect("error converting xpub");
        let root = ExtendedPubKey::from_str(&result)?;
        return Ok((fingerprint, root));
    } else {
        let err = io::Error::new(ErrorKind::Other, "Could not find zpub.");
        return Err(Box::new(err));
    }
}


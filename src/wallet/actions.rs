#![allow(unused_variables, unused_imports, unused_assignments)]
extern crate bitcoin;

use std::collections::BTreeMap;
use std::error::Error;
use std::fs::File;
use std::io::{ErrorKind, self, Write, Read};
use std::path::PathBuf;
use std::str::FromStr;
use bitcoin::address::Address;
use bitcoin::bip32::{ChildNumber, ExtendedPubKey, DerivationPath, Fingerprint, IntoDerivationPath};
use bitcoin::psbt::{Psbt, Input, PsbtSighashType, Output, self};
use bitcoin::secp256k1::ffi::types::AlignedType;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{PublicKey, ScriptBuf, Txid, TxOut, Sequence, TxIn, Witness, Transaction, absolute, OutPoint};
use colored::Colorize;
use miniscript::descriptor::Tr;
use miniscript::psbt::PsbtExt;
use rayon::prelude::*;
use xyzpub::{convert_version, Version};

use crate::server::server::{get_all_utxo, get_fresh_change, get_fee_estimate, broadcast};

// use crate::server::server::{get_fee_estimate, get_all_utxo, get_fresh_change};

const VERSION_BYTE_FEE: f64 = 4.;
const LOCKTIME_BYTE_FEE: f64 = 4.;
const INPUT_COUNTER_BYTE_FEE: f64 = 9.;
const OUTPUT_COUNTER_BYTE_FEE: f64 = 9.;
const INPUT_BYTE_FEE: f64 = 147.;

#[derive(Debug)]
pub struct SelectionUTXO {
    pub id: Txid,
    pub index: usize,
    pub value: f64,
    pub script: ScriptBuf,
    pub info: AddressInfo,
}
#[derive(Debug, Clone)]
pub struct AddressInfo {
    pub address: bitcoin::Address,
    pub path_to: DerivationPath,
    pub public_key: bitcoin::secp256k1::PublicKey,
}

pub fn compute_address(xpub: &str, external: bool, ind: u32) -> Result<Address, Box<dyn Error>>{
    // initialize the eliptic curve
    let network = bitcoin::Network::Bitcoin;
    let mut buf: Vec<AlignedType> = Vec::new();
    buf.resize(Secp256k1::preallocate_size(), AlignedType::zeroed());
    let secp = Secp256k1::preallocated_new(buf.as_mut_slice())?;
    // standardize zpub to xpub
    let result = convert_version(xpub, &Version::Xpub).expect("error converting xpub");
    let root = ExtendedPubKey::from_str(&result)?;
    let is_external = if external { 0 } else { 1 };
    let internal = ChildNumber::from_normal_idx(is_external)?;
    let index = ChildNumber::from_normal_idx(ind)?;
    let public_key = root.derive_pub(&secp, &[internal, index])?.public_key;
    // segwit only
    let address = Address::p2wpkh(&PublicKey::new(public_key), network)?;
    Ok(address)
}

pub fn compute_address_info(xpub: &str, external: bool, ind: u32) -> Result<AddressInfo, Box<dyn Error>> {
    let network = bitcoin::Network::Bitcoin;
    let mut buf: Vec<AlignedType> = Vec::new();
    buf.resize(Secp256k1::preallocate_size(), AlignedType::zeroed());
    let secp = Secp256k1::preallocated_new(buf.as_mut_slice())?;
    // standardize zpub to xpub
    let result = convert_version(xpub, &Version::Xpub).expect("error converting xpub");
    let root = ExtendedPubKey::from_str(&result)?;
    let is_external = if external { 0 } else { 1 };
    let internal = ChildNumber::from_normal_idx(is_external)?;
    let index = ChildNumber::from_normal_idx(ind)?;
    let public_key = root.derive_pub(&secp, &[internal, index])?.public_key;

    let address = Address::p2wpkh(&PublicKey::new(public_key), network)?;
    let path = format!("m/84h/0h/0h/{}/{}", is_external, index); // update
    Ok(AddressInfo { address, path_to: DerivationPath::from_str(path.to_string().as_str())?, public_key })
}

pub fn compute_script_pubkey(xpub: &str, external: bool, ind: u32) -> Result<ScriptBuf, Box<dyn Error>>{
    // initialize the eliptic curve
    let network = bitcoin::Network::Bitcoin;
    let mut buf: Vec<AlignedType> = Vec::new();
    buf.resize(Secp256k1::preallocate_size(), AlignedType::zeroed());
    let secp = Secp256k1::preallocated_new(buf.as_mut_slice())?;
    // standardize zpub to xpub
    let result = convert_version(xpub, &Version::Xpub).expect("error converting xpub");
    let root = ExtendedPubKey::from_str(&result)?;
    let is_external = if external { 0 } else { 1 };
    let internal = ChildNumber::from_normal_idx(is_external)?;
    let index = ChildNumber::from_normal_idx(ind)?;
    let public_key = root.derive_pub(&secp, &[internal, index])?.public_key;

    let script_buf = Address::p2wpkh(&PublicKey::new(public_key), network)?.script_pubkey();
    Ok(script_buf)
}

pub fn is_valid_fp(fp: &String) -> bool {
    let master_fp = Fingerprint::from_str(fp);
    if !master_fp.is_ok() {
        return false;
    }
    println!("\nFingerprint valid\n"); 
    return true;
}

pub fn is_valid_xpub(xpub: &String) -> bool {
    let result = convert_version(xpub, &Version::Xpub);
    if !result.is_ok() {
        return false;
    }
    let xpub = ExtendedPubKey::from_str(result.unwrap().as_str());
    if !xpub.is_ok() {
        return false;
    } 
    println!("XPUB valid\n");
    return true;
}

pub fn select_coins(mut coins: Vec<SelectionUTXO>, mut target: f64, per_byte_fee: f64, smallest: bool) -> Result<(Vec<SelectionUTXO>, f64), Box<dyn Error>> {
    target += (VERSION_BYTE_FEE + LOCKTIME_BYTE_FEE + INPUT_COUNTER_BYTE_FEE + OUTPUT_COUNTER_BYTE_FEE) * per_byte_fee;
    print!(" Sorting coins... ");
    //most expensive part of the algorithm
    if smallest {
        coins.par_sort_unstable_by(|a, b| a.value.partial_cmp(&b.value).expect("Some error when comparing values"));
    } else {
        coins.par_sort_unstable_by(|a, b| b.value.partial_cmp(&a.value).expect("Some error when comparing values"));
    }
    //tally the amount
    let mut amount = 0.;
    let mut selected = Vec::new();
    //if the coins vector is empty we could not reach the target
    while !coins.is_empty() {
        let coin = coins.remove(0);
        target += INPUT_BYTE_FEE * per_byte_fee / 1.5; //we have to adjust this each time there is an input added. div to estimate Segwit discount
        amount += coin.value;
        selected.push(coin);
        if amount > target {
            return Ok((selected, amount - target)); //return the vector and change amount
        } else if target > amount {
            continue;
        } else {
            return Ok((selected, 0.));
        }
    }
    println!("Not enough coins to make this transcation!");
    let err = io::Error::new(ErrorKind::Other, "Insufficient balance");
    Err(Box::new(err))
}

fn create_and_update_psbt(selected_utxo: Vec<SelectionUTXO>, xpub: ExtendedPubKey, finger_print: Fingerprint, receive_addr: Address, change_addr: AddressInfo, change: u64, amount: u64) -> Result<Psbt, Box<dyn Error>> {  
    print!("Creating your transaction...\n");
    
    let mut input = Vec::new();
    let mut output = Vec::new();

    output.extend(vec![
        TxOut { value: amount, script_pubkey: receive_addr.script_pubkey() },
        TxOut { value: change, script_pubkey: change_addr.address.script_pubkey() },
    ]);

    for utxo in &selected_utxo {
        input.push(
            TxIn {
                previous_output: OutPoint { txid: utxo.id , vout: utxo.index as u32 },
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX, 
                witness: Witness::default(),

            }
        );
    };

    let tx = Transaction {
        version: 2,
        lock_time: absolute::LockTime::ZERO,
        input,
        output,
    };

    let mut psbt = Psbt::from_unsigned_tx(tx)?;
    let path =IntoDerivationPath::into_derivation_path("m/84h/0h/0h")?; //hardcoded for single sig
    let mut map = BTreeMap::new();
    map.insert(xpub, (finger_print, path));
    psbt.xpub = map;
    let mut inputs = Vec::new();

    for utxo in selected_utxo {
        let mut input = Input { witness_utxo: Some(TxOut { value: utxo.value as u64, script_pubkey:  utxo.script.clone() }), ..Default::default() };
        let path = utxo.info.path_to;
        let mut map = BTreeMap::new();
        map.insert(utxo.info.public_key, (finger_print, path));
        input.bip32_derivation = map;
        let ty = PsbtSighashType::from_str("SIGHASH_ALL")?;
        input.sighash_type = Some(ty);
        inputs.push(input)
    }

    let mut out_map = BTreeMap::new();
    out_map.insert(change_addr.public_key, (finger_print, change_addr.path_to));
    let out = Output { bip32_derivation: out_map, ..Default::default() };

    psbt.inputs = inputs;
    psbt.outputs = vec![Output { ..Default::default() }, out];
    Ok(psbt)

}

pub fn make_and_download_transaction(target: u64, xpub: &str, fp: &str, receive_addr: &str, block_target: usize, clean_wallet: bool, client: &str) -> Result<(), Box<dyn Error>> {
    let new_psbt = make(target, xpub, fp, receive_addr, block_target, clean_wallet, client).expect("Error Occured Making PSBT");
    let psbt_bytes = new_psbt.serialize();
    let file_name = "unsigned.psbt".bright_blue();
    let downloads = "downloads".bright_green();
    println!("\nYour transcation as been saved as {file_name} in your {downloads} folder!\n");
    let mut psbt_file = File::create(dirs::download_dir().unwrap().join("unsigned.psbt")).expect("error making file");
    psbt_file.write_all(&psbt_bytes).expect("error writing bytes");
    Ok(())
}

pub fn make_and_send_to_ledger(target: u64, xpub: &str, fp: &str, receive_addr: &str, block_target: usize, clean_wallet: bool, client: &str) -> Result<Psbt, Box<dyn Error>> {
    let ledger = make(target, xpub, fp, receive_addr, block_target, clean_wallet, client).expect("Error Occured Making PSBT");
    Ok(ledger)
}

fn make(target: u64, xpub: &str, fp: &str, receive_addr: &str, block_target: usize, clean_wallet: bool, client: &str) -> Result<Psbt, Box<dyn Error>> {
    let receive = Address::from_str(receive_addr).expect("error getting address from string").require_network(bitcoin::Network::Bitcoin).expect("error getting address from network");
    let result = convert_version(xpub, &Version::Xpub).expect("error converting xpub");
    let root = ExtendedPubKey::from_str(&result)?;
    let master_fp = Fingerprint::from_str(fp)?;
    let target_to_float = target as f64;
    let byte_fee = get_fee_estimate(block_target, client)?;
    let available_coins = get_all_utxo(xpub, 5, client)?;
    let change_addr = get_fresh_change(xpub, client)?;
    let (coins, change ) = select_coins(available_coins, target_to_float, byte_fee, clean_wallet)?;
    let psbt = create_and_update_psbt(coins, root, master_fp, receive, change_addr, change as u64, target)?;
    Ok(psbt)
}

pub fn print_psbt(psbt: Psbt) -> Result<(), Box<dyn Error>> {
    let val = psbt.unsigned_tx.output[0].value.to_string().bright_green();
    // assumes the first output is the spending address
    let btc_val = psbt.unsigned_tx.output[0].value as f64 / 100_000_000.;
    let addr = psbt.unsigned_tx.output[0].script_pubkey.as_script();
    let address = Address::from_script(addr, bitcoin::Network::Bitcoin)?;
    let color_address = address.to_string().bright_green();
    println!("You are sending {} Satoshis [{} Bitcoin] to {}", val, btc_val, color_address);
    let fee = psbt.fee()?.to_string().bright_green();
    println!("There is a fee of {}\n", fee);
    Ok(())
}

pub fn extract_broadcast(mut psbt: Psbt, client: &str) -> Result<(), Box<dyn Error>> {
    if psbt.inputs.is_empty() {
        return Err(psbt::SignError::MissingInputUtxo.into());
    }
    // if there are partial sigs we need to finalize
    if !psbt.inputs[0].partial_sigs.is_empty() {
        let network = bitcoin::Network::Bitcoin;
        let mut buf: Vec<AlignedType> = Vec::new();
        buf.resize(Secp256k1::preallocate_size(), AlignedType::zeroed());
        let secp = Secp256k1::preallocated_new(buf.as_mut_slice())?;
        psbt = Psbt::finalize(psbt, &secp).expect("Finalization error");
    }
    //extract the transaction (witness) from the PSBT
    let network = bitcoin::Network::Bitcoin;
    let mut buf: Vec<AlignedType> = Vec::new();
    buf.resize(Secp256k1::preallocate_size(), AlignedType::zeroed());
    let secp = Secp256k1::preallocated_new(buf.as_mut_slice())?;
    let ext = PsbtExt::extract(&psbt, &secp).expect("Error finalizing TX");
    //send the transaction to the electrum client
    let res = broadcast(ext, client)?;
    Ok(())
}

#[test]
fn addresses() {
    let zpub = "zpub6qVc2FELq8mG3pf2eayaVtFtG3ots5wT9G82V8tSWUcXM54dZSgLvz23vEkqqQyB2rxNum7W94dLG7qUEE1RDNuKhgRi9EXhXZ6E6zxx7Kx";
    let address = compute_address(zpub, true, 2).unwrap();
    assert_eq!(address.to_string(), "bc1qv6uauuvg0em39263xknaqsrqqqk0fh6q4th23a");
    let address = compute_address(zpub, true, 10).unwrap();
    assert_eq!(address.to_string(), "bc1qfh4ltu8ysfl9xq0ld88h88qja7sad283akey6w");
}
#![allow(unused_variables, unused_imports, unused_assignments)]
extern crate electrum_client;
extern crate bitcoin;
use colored::*;
use std::{error::Error, cmp::Ordering, fmt::format, sync::{Arc, Mutex}, collections::HashMap};
use electrum_client::{Client,ElectrumApi, Config};
use bitcoin::{Address, Txid, Transaction};
use rand::Rng;
use crate::wallet::actions::{compute_script_pubkey, compute_address, SelectionUTXO, compute_address_info, AddressInfo};
use rayon::prelude::*;
use indicatif::{ProgressBar, ProgressStyle, ProgressState, MultiProgress};


#[derive(Debug, serde::Serialize)]
pub struct UserTransaction {
    pub value: u64,
    pub height: i32,
    pub was_sent: bool,
    pub confirmed: bool,
    pub id: Txid,
}

impl UserTransaction {
    fn new(value: u64, height: i32, was_sent: bool, confirmed: bool, id: Txid) -> Self {
        UserTransaction { value, height, was_sent, confirmed, id }
    }
}

fn select_rand_server() -> String {
    let servers = ["ssl://electrum.blockstream.info:50002",];
    let mut rng = rand::thread_rng();
    let random_index = rng.gen_range(0..servers.len());
    servers[random_index].to_string()

}

pub fn get_fee_estimate(blocks: usize, client: &str) -> Result<f64, Box<dyn Error>> {
    let client = Client::new(client)?;
    let btc_fee = client.estimate_fee(blocks)?;
    let fee = btc_fee * 100_000_000.0 / 1_000.0; //convert to satoshi and convert from kb to bytes
    Ok(fee)
}

pub fn get_all_fee_estimates(client: &str) -> Result<Vec<f64>, Box<dyn Error>> {
    let client = Client::new(client)?;
    let msg = "\nContected to an Electrum server\n".bright_green();
    eprintln!("{msg}");
    let mut fees = Vec::new();
    let bar = ProgressBar::new(20 as u64);
    bar.set_style(ProgressStyle::with_template("{spinner:.green} {msg} [{elapsed_precise}] [{wide_bar:.cyan/blue}] ")?);
    for i in 1..26 {
        bar.set_message(format!("Fetching fee estimates for a {} block confirmation", i));
        let btc_fee = client.estimate_fee(i)?;
        let fee = btc_fee * 100_000_000.0 ; //convert to satoshi
        fees.push(fee.round());
        bar.inc(1)
    }
    bar.finish_with_message("Done");
    Ok(fees)
}

pub fn get_fresh(xpub: &str, client: &str) -> Result<Address, Box<dyn Error>> {
    let mut i = 0;
    let client = Client::new(client)?;
    let bar = ProgressBar::new(1 as u64);
    bar.set_style(ProgressStyle::with_template("{spinner:.green} {msg} [{elapsed_precise}] [{wide_bar:.cyan/blue}] ")?);
    loop {
        let script = compute_script_pubkey(xpub, true, i)?;
        let history = client.script_get_history(&script)?;
        bar.set_message(format!("Fetching balance from address at m/84h/0h/0h/0/{i}"));
        if history.len() == 0 {
            bar.inc(1);
            bar.finish_and_clear();
            let addr = compute_address(xpub, true, i)?;
            break Ok(addr);
        }
        i+=1;
    }
}

pub fn get_fresh_change(xpub: &str, client: &str) -> Result<AddressInfo, Box<dyn Error>> {
    let mut i = 0;
    let client = Client::new(select_rand_server().as_str())?;
    let message = "Your transaction is being built".bright_green();
    println!("\n{}", message);
    print!("\nFetching a new change address...");
    loop {
        let script = compute_script_pubkey(xpub, false, i)?;
        let history = client.script_get_history(&script)?;
        if history.len() == 0 {
            let info = compute_address_info(xpub, false, i)?;
            break Ok(info);
        }
        i+=1;
    }
}

pub fn broadcast(tx: Transaction, client: &str) -> Result<(), Box<dyn Error>> {
    let client = Client::new(client)?;
    let msg = "\nContected to an Electrum server\n".bright_green();
    eprintln!("{msg}");
    let id = client.transaction_broadcast(&tx)?;
    let message = "Your transaction was sent".bright_green();
    println!("{message}");
    println!("Transaction ID: {}\n", id);
    println!("View it at https://mempool.space\n");
    Ok(())
}

pub fn get_balance(xpub: &str, gap: u8, client: &str) -> Result<i64, Box<dyn Error>> {
    let client = Client::new(client)?;
    let msg = "\nContected to an Electrum server\n".bright_green();
    eprintln!("{msg}");
    let m = Arc::new(Mutex::new(MultiProgress::new()));
    let mp_clone = m.lock().unwrap();
    let results = rayon::join(|| subaccount_balance(xpub, &client, gap, true,  &mp_clone).expect("error getting receive balance"), || subaccount_balance(xpub, &client, gap, false, &mp_clone).expect("error getting change balance"));
    Ok(results.0 + results.1)
}

fn subaccount_balance(xpub: &str, client: &Client, gap: u8, external: bool, m: &MultiProgress) -> Result<i64, Box<dyn Error>> {
    let mut balance = 0 as i64;
    let mut zero_balance = 0;
    let mut i = 0;
    let bar = m.add(ProgressBar::new(gap as u64));
    bar.set_style(ProgressStyle::with_template("{spinner:.green} {msg} [{elapsed_precise}] [{wide_bar:.cyan/blue}] ")?);
    loop {
        let script = compute_script_pubkey(xpub, external, i)?;
        let is_internal = if external { 0 } else { 1 };
        let address_str_path = format!("m/84h/0h/0h/{is_internal}/{i}").green();
        bar.set_message(format!("Fetching balance from address at {}", address_str_path));
        let history = client.script_get_balance(&script)?;
        let address_balance = history.confirmed as i64 + history.unconfirmed;
        balance += address_balance;
        if address_balance == 0 {
            bar.inc(1);
            zero_balance += 1;
            if zero_balance > gap {
                bar.finish_and_clear();
                m.remove(&bar);
                break Ok(balance);
            }
        }
        i+=1;
    }
}

pub fn get_all_utxo(xpub: &str, gap: u8, client: &str) -> Result<Vec<SelectionUTXO>, Box<dyn Error>> {
    let client = Client::new(client)?;
    let msg = "\nContected to an Electrum server\n".bright_green();
    eprintln!("{msg}");
    let mut utxos = Vec::new();
    let bar = ProgressBar::new(gap as u64);
    bar.set_style(ProgressStyle::with_template("{spinner:.green} {msg} [{elapsed_precise}] [{wide_bar:.cyan/blue}] ")?);
    for account in [true, false] {
        let mut zero_balance = 0;
        let mut i = 0;
        loop {
            let is_ext = if account { 0 } else { 1 };
            let address_str_path = format!("m/84h/0h/0h/{is_ext}/{i}").green();
            bar.set_message(format!("Fetching balance from address at {}", address_str_path));
            let script = compute_script_pubkey(xpub, account, i)?;
            let info = compute_address_info(xpub, account, i)?;
            let utxo = client.script_list_unspent(&script)?;
            let length = utxo.len();
            utxos.extend(utxo.iter().map(|utxo| SelectionUTXO { id: utxo.tx_hash, index: utxo.tx_pos, value: utxo.value as f64, script: script.clone(), info: info.clone()}));
            if length == 0 {
                zero_balance += 1;
                bar.inc(1);
                if zero_balance > gap {
                    break;
                }
            }
            i+=1;
        }   
    }
    bar.finish_and_clear();
    Ok(utxos)
}

pub fn get_tx_history(xpub: &str, gap: u8, client: &str) -> Result<Vec<UserTransaction>, Box<dyn Error>>  {
    let mut received = Vec::new();
    let mut change = Vec::new();
    let mut sent = Vec::new();
    let client = Client::new(client)?;

    let msg = "\nContected to an Electrum server\n".bright_green();
    eprintln!("{msg}");

    let bar = ProgressBar::new((gap * 2) as u64);
    bar.set_style(ProgressStyle::with_template("{spinner:.green} {msg} [{wide_bar:.cyan/blue}] ")?);

    for account in [true, false] {
        let mut zero_balance = 0;
        let mut i = 0;
        loop {
            let script_buf = compute_script_pubkey(xpub, account, i).expect("address formation error");
            let history: Vec<electrum_client::GetHistoryRes> = client.script_get_history(&script_buf).expect("error fetching from script pubkey");
            let is_internal: i32 = if account { 0 } else { 1 };
            let address_str_path = format!("m/84h/0h/0h/{is_internal}/{i}").green();
            bar.set_message(format!("Fetching history from address at {}", address_str_path));
            if history.len() == 0 {
                bar.inc(1);
                i+=1;
                zero_balance+=1;
                if zero_balance > gap {
                    break
                }
            }
            for tx in history {
                let txid = tx.tx_hash;
                let transaction = client.transaction_get(&txid)?;
                let inputs = transaction.input;
                let outputs = transaction.output;
                for out in outputs {
                    if script_buf == out.script_pubkey && account {
                        let confirmed = tx.height > 0;
                        received.push(UserTransaction::new(out.value, tx.height, false, confirmed, txid));
                    } 
                    if script_buf == out.script_pubkey && !account {
                        change.push((txid, out.value as i64));
                    } 
                }
                let mut sent_val = 0;
                for inp in inputs {
                    let prev_out = client.transaction_get(&inp.previous_output.txid)?;
                    for out in prev_out.output {
                        if script_buf == out.script_pubkey {
                            sent_val += out.value;
                        }
                    } 
                }
                if sent_val > 0 {
                    sent.push((txid, tx.height, sent_val as i64));
                }
            }
            i+=1;
        }
    }
    bar.set_message("Collecting sent transcations");
    let mut net_map: HashMap<&Txid, (i32, i64)> = std::collections::HashMap::new();
    
    // Collect values from the spent vector
    for (txid, height, spent_value) in &sent {
        if let Some(tx) = net_map.get(txid) {
            net_map.insert(txid, (*height, *spent_value + tx.1));
        } else {
            net_map.entry(txid).or_insert_with(||  (*height, *spent_value));
        }
    }

    // Subtract values from the change vector
    for (txid, change_value) in &change {
        if let Some(net_value) = net_map.get_mut(txid) {
            net_value.1 = net_value.1 - change_value;
        }
    }

    // Convert the HashMap back into a vector of tuples
    let net: Vec<(&Txid, (i32, i64))> = net_map.into_iter().collect();
    for (id,  v) in net {
        if v.1 < 0 {
            continue;
        } else {
            let confirmed = v.0 > 0;
            received.push(UserTransaction::new(v.1 as u64, v.0, true, confirmed, *id));
        }
    }

    bar.finish_with_message("Sorting the transactions");
    received.par_sort_unstable_by(|a, b| { 
        if !a.confirmed {
            Ordering::Less
        } else if !b.confirmed{
            Ordering::Greater
        } else {
            if a.height > b.height {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
    });
    Ok(received)
}
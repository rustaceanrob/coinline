#![allow(unused_variables, unused_imports, unused_assignments)]
use std::{path::PathBuf, str::FromStr, io::{Write, Read}, error::Error, fs::File, f32::consts::E};

use async_hwi::{ledger::{self, HidApi, Ledger, TransportHID}, HWI};
use bitcoin::{bip32::DerivationPath, psbt::Psbt};
use clap::{Parser, Subcommand, Args};
use coinline::{server::server::{get_balance, get_fresh, get_tx_history, UserTransaction, get_all_fee_estimates, get_all_utxo}, wallet::actions::{is_valid_fp, is_valid_xpub, compute_address, make_and_download_transaction, make_and_send_to_ledger, print_psbt, extract_broadcast}, system::system::{import_coldcard_from_json, import_keystone_from_txt}};
use colored::Colorize;
use qrcode::QrCode;
use qrcode::render::unicode;
use serde::{Serialize, Deserialize, __private::de};

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct WalletConfig {
    gap: u8,
    client: String,
    fp: String,
    xpub: String,
    hmac: [u8; 32],
}

impl ::std::default::Default for WalletConfig {
    fn default() -> Self { Self { gap: 20, client: "ssl://electrum.blockstream.info:50002".into(), fp: "".into(), xpub: "".into(), hmac: [0; 32] } }
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct CoinlineArgs {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Configures your wallet with Coinline using the Master Fingerprint and Native SegWit Extended Public Key.
    Set(Set),
    /// Sets the current device to Ledger, if plugged in.
    SetLedger,
    /// Sets the current using a configuration file from your Keystone or Coldcard.
    SetFile(SetFile),
    /// Returns your configuration file.
    Get,
    /// Gets the balance for the configured wallet. Balance is returned in Satoshis.
    Balance(Balance),
    /// Gets the next unused address for the wallet with the specified name.
    Receive,
    /// Gets the history of the transactions on this wallet. The history is reconstructed with no local cache, so this may take a while.
    History(History),
    /// Prepares a transaction to send.
    Send(Send),
    /// Queries the Electrum server to get fee estimates for transactions settling between 1-25 blocks.
    Fees,
    /// Broadcast a signed transaction to the network via Electrum.
    Broadcast(Broadcast),
    /// Tests if there is a Ledger hardware wallet connected.
    Ledger,
    /// Finds UTXOs lower than the amount provided. Accepts arguments between 500 and 10000 Satoshis.
    Dust(Dust),
    /// Return the first 10 receiving and change addresses from your device 
    Addresses,
    /// Sets the prefered Electrum client using a URL. Expects the URL to be [tcp/ssl]://[server_name]:[port]. Default is ssl://electrum.blockstream.info:50002.
    Client(Client),
    /// Sets the prefered gap in no-actvity addresses until the program quits. Valid gaps are between [1, 50].
    Gap(Gap),
}

#[derive(Debug, Args)]
pub struct Set {
    /// The master fingerprint of your wallet. Found at path m/. More information on setting a wallet, visit https://coinline.io
    fingerprint: String,
    /// The Native Segwit extended public key of your wallet. Found at path m/84h/0h/0h/
    xpub: String,
}

#[derive(Debug, Args)]
pub struct History {
    /// The amount of addresses with empty UTXO balances until the program quits looking for new UTXOs.
    gap: Option<u8>,
}

#[derive(Debug, Args)]
pub struct Dust {
    /// Your dust threshold.
    dust: Option<u32>,
}

#[derive(Debug, Args)]
pub struct Fee {
    /// The amount of addresses with empty UTXO balances until the program quits looking for new UTXOs.
    block: u8,
}

#[derive(Debug, Args)]
pub struct Client {
    /// The preferred Electru, client in the form: [tcp/ssl]://[server_name]:[port]
    client: String,
}

#[derive(Debug, Args)]
pub struct SetFile {
    /// The device you are importing, either 'keystone' or 'coldcard'. More information on setting a wallet, visit https://coinline.io
    device: String,
    /// The path to the configuration file from your Colcard or Keystone. For Coldcard this is a JSON file. For Keystone, this is a txt file.
    file: PathBuf,
}

#[derive(Debug, Args)]
pub struct Gap {
    /// The amount of addresses with empty UTXO balances until the program quits looking for new UTXOs.
    gap: u8,
}

#[derive(Debug, Args)]
pub struct Balance {
    /// The amount of addresses with empty UTXO balances until the program quits looking for new UTXOs.
    gap: Option<u8>,
}

#[derive(Debug, Args)]
pub struct Broadcast {
    /// The path to the signed PSBT ready to broadcast to the network.
    file: PathBuf,
}

#[derive(Debug, Args)]
pub struct Send {
    /// How your transaction signing will occur. For an airgapped work-flow, save the PSBT by passing "file." For Ledger, pass "ledger".
    signer: String,
    /// The address you are sending to.
    receiving: String,
    /// How much you are sending to the receive address, in Satoshis.
    value: u64,
    /// How many estimated blocks in the future for this transaction to be confirmed.
    blocks: u8,
    /// How to select the UTXOs to fund the transcation, default is largest first: options are [smallest, largest]. More information on coin selection at https://coinline.io
    algorithm: Option<String>,

}

fn get_user_approval() -> Result<bool, Box<dyn Error>> {
    print!("Confirm (y/n)? ");
    std::io::stdout().flush()?; 

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;

    match input.trim().to_lowercase().as_str() {
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        _ => {
            println!("Invalid input. Please enter 'y' or 'n'.");
            get_user_approval()
        }
    }
}

fn print_balance(bal: i64) -> () {
    let btc_balace = bal as f64 / 100_000_000.;
    println!("The total value for the configured wallet is {} Satoshis, equal to {} Bitcoin\nSome transactions may be unconfirmed\n", bal.to_string().bright_blue(), btc_balace.to_string().bright_blue());
}

fn print_history(txs: Vec<UserTransaction>) -> () {
    for tx in txs.iter() {
        println!("\n");
        let sent = if tx.was_sent { "Sent".bright_blue() } else { "Recieved".bright_green()};
        let btc_amount = tx.value as f64 / 100_000_000.;
        println!("You {sent} {} Satoshis [{} Bitcoin]", tx.value, btc_amount);
        if tx.confirmed {
            println!("The transcation was confirmed at block height {}", tx.height);
        } else {
            println!("This transcation has not been confirmed")
        }
        println!("The transaction hash is: {}", tx.id);
    }
    println!("\n");
}

#[tokio::main]
async fn main() {
    let cfg: WalletConfig = confy::load("coinline", None).expect("Could not find configuration file");
    let args = CoinlineArgs::parse();

    match &args.command {
        Commands::Balance(Balance { gap }) => {
            if cfg.xpub == "" {
                println!("\nPlease configure a wallet by using the 'set' or 'import' command.");
                return;
            }
            let xpub = cfg.xpub.as_str();
            match gap {
                Some(gap) => {
                    println!("\n");
                    let balance = get_balance(xpub, *gap, &cfg.client.to_string()).expect("Global Error Fetching Balance");
                    print_balance(balance);
                },
                None => {
                    let balance = get_balance(xpub, cfg.gap, &cfg.client.to_string()).expect("Global Error Fetching Balance");
                    print_balance(balance);
                },
            }
        },
        Commands::Set(Set { fingerprint, xpub}) => {
            if is_valid_fp(fingerprint.into()) && is_valid_xpub(xpub) {
                let address = compute_address(xpub, true, 0).expect("Could not get address.");
                let confirmation = address.to_string().to_string().bright_green();
                println!("Please confirm that this is your first address [receiving]: {confirmation}\n");
                confy::store("coinline", None, WalletConfig { gap: cfg.gap, client: cfg.client, fp: fingerprint.into(), xpub: xpub.into(), hmac: cfg.hmac }).expect("save configuration error");
                let confirm = "Your wallet was saved".bright_green();
                println!("{confirm}\n");
                return;
            }
            println!("Either the Fingerprint or XPUB could not be saved.");
        },
        Commands::SetFile(SetFile { device, file }) => {
            if device.eq("coldcard") {
                let (fp, xpub) = import_coldcard_from_json(file.to_path_buf()).expect("File import failure");
                confy::store("coinline", None, WalletConfig { gap: cfg.gap, client: cfg.client, fp: fp.to_string().into(), xpub: xpub.to_string().into(), hmac: cfg.hmac }).expect("save configuration error");
                let confirm = "Your wallet was saved".bright_green();
                println!("{confirm}\n");
                return;
            } else if device.eq("keystone") {
                let (fp, xpub) = import_keystone_from_txt(file.to_path_buf()).expect("File import failure");
                confy::store("coinline", None, WalletConfig { gap: cfg.gap, client: cfg.client, fp: fp.to_string().into(), xpub: xpub.to_string().into(), hmac: cfg.hmac }).expect("save configuration error");
                let confirm = "Your wallet was saved".bright_green();
                println!("{confirm}\n");
                return;
            } else {
                println!("Device unregonized");
                return;
            }
            
        },
        Commands::Receive => {
            if cfg.xpub == "" {
                println!("\nPlease configure a wallet by using the 'set' or 'import' command.");
                return;
            }
            let address = get_fresh(&cfg.xpub, &cfg.client.to_string()).expect("Global Error Fetching The Receive Address");
            let address_string = address.to_string().bright_green();
            println!("\nYour next unused receiving address is: {address_string}\n");
            println!("Scan the QR code below to send coins to this address\n");
            let qr_code = QrCode::new(address.to_qr_uri()).unwrap();
            let qr_string = qr_code.render()
                                .quiet_zone(false)
                                .min_dimensions(40, 40)
                                .max_dimensions(40, 40)
                                .module_dimensions(1, 1)
                                .dark_color(unicode::Dense1x2::Dark)
                                .light_color(unicode::Dense1x2::Light)
                                .build();
            println!("{}", qr_string);

        },
        Commands::History(History { gap }) => {
            if cfg.xpub == "" {
                println!("\nPlease configure a wallet by using the 'set' or 'set-ledger' command.");
                return;
            }
            match gap {
                Some(gap) => {
                    let hist = get_tx_history(&cfg.xpub, *gap, &cfg.client.to_string()).expect("Global Error Fetching Balance");
                    print_history(hist);
                }, 
                None => {
                    let hist = get_tx_history(&cfg.xpub, cfg.gap, &cfg.client.to_string()).expect("Global Error Fetching Balance");
                    print_history(hist);
                },
            }
        },
        Commands::Send(Send { signer, receiving, value, blocks, algorithm}) => {
            if cfg.xpub == "" {
                println!("\nPlease configure a wallet by using the 'set' or 'set-ledger' command.");
                return;
            }
            let mut clean_wallet = false;
            match algorithm {
                Some(algorithm) => {
                    if algorithm == "smallest" {
                        clean_wallet = true;
                    } else if algorithm == "largest" {
                        clean_wallet = false;
                    } else {
                        println!("Unrecognized coin selection algorithm")
                    }
                },
                None => {
                    clean_wallet = false;
                },
            }
            if signer == "file" {
                let res = make_and_download_transaction(*value, &cfg.xpub, &cfg.fp, receiving, *blocks as usize, clean_wallet, &cfg.client.to_string());
                match res {
                    Ok(_) => {
                        return;
                    },
                    Err(_) => {
                        let warn = "An error occured. Do you have enough coins for this transaction?".bright_yellow();
                        println!("{warn}");
                    },
                }
            } else if signer == "ledger" {
                let api = HidApi::new().unwrap();
                for detected in Ledger::<TransportHID>::enumerate(&api) {
                    if let Ok(device) = Ledger::<TransportHID>::connect(&api, detected) {
                        let mut psbt = make_and_send_to_ledger(*value, &cfg.xpub, &cfg.fp, receiving, *blocks as usize, clean_wallet, &cfg.client.to_string()).expect("Error forming transaction");
                        let pol = format!("wpkh([{}/84'/0'/0']{}/**)", &cfg.fp, &cfg.xpub);
                        if cfg.hmac.eq(&[0; 32]) {
                            println!("HMAC retrieval error");
                            return;
                        }
                        let ok = "OK".bright_green();
                        println!("\nPlease check your Ledger");
                        println!("If you do not use Ledger Live, you make get an unverified inputs message. This is {ok}\n");
                        let hmac = Some(cfg.hmac);
                        let res = device.with_wallet("Coinline", &pol, hmac).unwrap().sign_tx(&mut psbt).await;
                        match res {
                            Ok(res) => { println!("\nYour transaction has been signed by your Ledger\n") },
                            Err(_) => {
                                println!("\nYour transaction was not signed by your device. Exiting...\n");
                                return;
                            }
                        }
                        print_psbt(psbt.clone()).expect("Could not print PSBT");
                        if let Ok(approved) = get_user_approval() {
                            if approved {
                                extract_broadcast(psbt, &cfg.client.to_string()).expect("Finalization error");
                            } else {
                                let deny = "Broadcast not approved. Exiting...".bright_yellow();
                                println!("\n{deny}\n");
                                return;
                            }
                        } 

                    }
                }
            } else {
                let warn = "Unrecognized command".bright_yellow();
                println!("{warn}: {}", signer);
            }
        },
        Commands::Broadcast(Broadcast { file }) => {
            let mut file = File::open(file).expect("Could not find that PSBT");
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer).expect("Error reading that PSBT");
            let psbt = Psbt::deserialize(&buffer).expect("Error deserializing that PSBT");
            println!("Please approve your transaction...\n");
            print_psbt(psbt.clone()).expect("Printing failure");
            if let Ok(approved) = get_user_approval() {
                if approved {
                    extract_broadcast(psbt.clone(), &cfg.client.to_string()).expect("Finalization error");
                } else {
                    let deny = "Broadcast not approved. Exiting...".bright_yellow();
                    println!("\n{deny}\n");
                    return;
                }
            } else {
                println!("Error while getting user approval.");
                return;
            }
        },
        Commands::Get => {
            println!("\nMaster fingerprint: {:?}\n", cfg.fp);
            println!("Extended public key: {:?}\n", cfg.xpub);
            println!("Default gap: {}\n", cfg.gap);
            println!("Electrum client: {}\n", cfg.client);
        },
        Commands::Gap(Gap { gap }) => {
            let g = *gap;
            if g < 1 && g > 50 {
                println!("Invalid gap");
                return;
            }
            confy::store("coinline", None, WalletConfig { gap: g, client: cfg.client, fp: cfg.fp, xpub: cfg.xpub, hmac: cfg.hmac }).expect("save configuration error");
            let confirm = "\nYour wallet preferences were saved".bright_green();
            println!("{confirm}\n");
            return;
        },
        Commands::Client(Client { client }) => {
            let c = electrum_client::Client::new(client);
            match c {
                Ok(c) => {
                    confy::store("coinline", None, WalletConfig { gap: cfg.gap, client: client.to_string().into(), fp: cfg.fp, xpub: cfg.xpub, hmac: cfg.hmac }).expect("save configuration error");
                    let confirm = "\nYour wallet preferences were saved".bright_green();
                    println!("{confirm}\n");
                    return;
                },
                Err(_) => {
                    let highlighted = client.bright_yellow();
                    println!("Could not connect to {highlighted}!");
                    return;
                }, 
            }
        },
        Commands::Fees => {
            let fees = get_all_fee_estimates(&cfg.client.to_string()).expect("Could not fetch fees");
            let mut block = 1;
            println!("\n");
            for fee in fees {
                let block_color = block.to_string().bright_blue();
                let fee_color = fee.to_string().bright_blue();
                println!("\nEstimated confirmation in {} blocks is around {} Satoshis per Kilobyte", block_color, fee_color);
                block+=1;
            }
            println!("\n");
        },
        Commands::Dust(Dust { dust }) => {
            match dust {
                Some(dust) => {
                    let d = *dust;
                    if d > 499 && d < 10001 {
                        let utxos = get_all_utxo(&cfg.xpub, cfg.gap, &cfg.client).expect("Error Fetching UTXOs");
                        let mut dust = 0;
                        for utxo in utxos {
                            if utxo.value < d as f64 {
                                dust += 1;
                                let warn = format!("Found a small UTXO with a value of {}", utxo.value).bright_yellow();
                                println!("{warn}\n")
                            }
                        }
                        if dust > 0 {
                            println!("\nManage your small UTXOs by including them in your transactions\n")
                        } else {
                            let warn = "No dust found!".bright_green();
                            println!("{warn}\n")
                        }
                        return;
                    } else {
                        println!("\nInvalid argument\n");
                        return;
                    }
                },
                None => {
                    let utxos = get_all_utxo(&cfg.xpub, cfg.gap, &cfg.client).expect("Error Fetching UTXOs");
                    let mut dust = 0;
                    for utxo in utxos {
                        if utxo.value < 10000 as f64 {
                            dust += 1;
                            let warn = format!("Found a small UTXO with a value of {}", utxo.value).bright_yellow();
                            println!("{warn}\n")
                        }
                    }
                    if dust > 0 {
                        println!("\nManage your small UTXOs by including them in your transactions\n")
                    } else {
                        let warn = "No dust found!".bright_green();
                        println!("{warn}\n")
                    }
                    return;
                },
            }
        },
        Commands::Addresses => {
            for i in [true, false] {
                for j in 0..10 {
                    let addr = compute_address(&cfg.xpub, i, j).expect("");
                    let is_external = if i { 0 } else { 1 };
                    let path = format!("m/84h/0h/0h/{}/{}", is_external.to_string().bright_green(), j.to_string().bright_green());
                    let colored = addr.to_string().bright_green();
                    println!("\nAddress at {}: {colored}\n", path);
                }
            }
        },
        Commands::Ledger => {
            let api = HidApi::new().unwrap();
            for detected in Ledger::<TransportHID>::enumerate(&api) {
                let sn = detected.product_string();
                let device = Ledger::<TransportHID>::connect(&api, detected).expect("Could not connect to Ledger");
                let confirm = "Your Ledger is connected".bright_green();
                println!("{confirm}");
                match sn {
                    Some(sn) => { println!("Device type: {}", sn); } 
                    None => { println!("No device type found") }
                }
                return;
            }
            println!("No Ledger was found. Please unlock your Ledger if it is plugged in.")
        },
        Commands::SetLedger => {
            let api = HidApi::new().unwrap();
            for detected in Ledger::<TransportHID>::enumerate(&api) {
                if let Ok(device) = Ledger::<TransportHID>::connect(&api, detected) {
                    let path = DerivationPath::from_str("m/84h/0h/0h").expect("Bad derivation");
                    let xpub = device.get_extended_pubkey(&path).await.expect("Extended Public Key not found");
                    let fingerprint = device.get_master_fingerprint().await.expect("Could not get the Fingerprint");
                    let address = compute_address(xpub.to_string().as_str(), true, 0).expect("Could not get address.");
                    let confirmation = address.to_string().to_string().bright_green();
                    let pol = format!("wpkh([{}/84'/0'/0']{}/**)", fingerprint.to_string(), xpub.to_string());
                    println!("\nPlease check your Ledger\n");
                    let hmac = device.register_wallet("Coinline", &pol).await.unwrap();
                    match hmac {
                        Some(hmac) => {
                            confy::store("coinline", None, WalletConfig { gap: cfg.gap, client: cfg.client, fp: fingerprint.to_string().into(), xpub: xpub.to_string().into(), hmac }).expect("save configuration error");
                            let confirm = "Your wallet was saved".bright_green();
                            println!("\n{confirm}\n");
                            return;

                        },
                        None => {
                            let hmac_err = "There was an error in extracting the Ledger HMAC".bright_yellow();
                            println!("{hmac_err}");
                            return;
                        }
                    }
                }
            }
            println!("No Ledger was found. Please unlock your Ledger if it is plugged in.")
        },
    }
}

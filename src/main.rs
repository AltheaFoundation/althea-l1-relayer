use actix_web::dev::RequestHead;
use awc::{Client as HttpClient, http::Method};
use clap::Parser;
use clarity::abi::{parse_address, parse_u128};
use clarity::{
    Address, PrivateKey, Transaction, Uint256, abi::encode_call, utils::display_uint256_as_address,
};
use log::{debug, error, info, trace};
use num_traits::ToPrimitive;
use rustls::crypto::CryptoProvider;
use serde::{Deserialize, Serialize};
use std::{net::ToSocketAddrs, str::FromStr, thread::sleep, time::Duration};
use web30::{
    client::Web3,
    jsonrpc::error::Web3Error,
    types::{Data, SendTxOption, TransactionRequest},
};

static OX_100_ADDRESS: &str = "0x0000000000000000000000000000000000000100";
static OX_200_ADDRESS: &str = "0x0000000000000000000000000000000000000200";
pub const RELAYING_SERVICE_ROOT: &str = "orchestrator";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GaslessTransaction {
    pub chain_id: u64,
    pub callpath: u16,
    pub cmd: Vec<u8>,
    pub conds: Vec<u8>,
    pub tip: Vec<u8>,
    pub sig: Vec<u8>,
    pub submitted_at: u64,
}

#[derive(Debug, Parser)]
#[command(name = "ifi-relayer", about = "iFi Dex transaction relayer")]
pub struct RelayerOpts {
    #[arg(long, value_name = "PRIVATE_KEY")]
    pub private_key: String,

    #[arg(
        long,
        default_value = "https://althea.link:8443",
        value_name = "TRANSACTION_SERVER_URL",
        help = "URLs of the service to fetch pending transactions"
    )]
    pub transaction_api_url: Vec<String>,

    #[arg(
        long,
        default_value = "https://althea.link:8443",
        value_name = "PRICE_API_URL",
        help = "URL of the price API to fetch token prices, this is a custom API that returns the price of a token in ALTHEA"
    )]
    pub price_api_url: String,

    #[arg(
        long,
        default_value = "https://eth.althea.net",
        value_name = "ETH_RPC_URL"
    )]
    pub eth_rpc: String,

    #[arg(long, default_value = "5", value_name = "POLL_INTERVAL")]
    pub poll_interval: u64,

    #[arg(long, default_value = "12", value_name = "CONFIRMATION_BLOCKS")]
    pub confirmation_blocks: u64,

    #[arg(
        long,
        // address of the iFi dex on Althea L1, use explorer.althea.link to verify
        default_value = "0xd263DC98dEc57828e26F69bA8687281BA5D052E0",
        value_name = "CONTRACT_ADDRESS"
    )]
    pub contract_address: String,

    #[arg(
        long,
        default_value = "info",
        value_name = "LOG_LEVEL",
        help = "Set the logging level (e.g., info, debug, error)"
    )]
    pub log_level: String,

    #[arg(
        long,
        default_value = "10",
        value_name = "TIMEOUT",
        help = "Timeout for all operations in seconds"
    )]
    pub timeout: u64,

    #[arg(
        long,
        default_value = "false",
        value_name = "AGREE",
        help = "Agree to the terms and conditions"
    )]
    pub agree: bool,
}

const TERMS: &str = "This software is provided AS IS as a reference gassless transaction relayer. This software may contain bugs, lose funds, or even spend all the ALTHEA it has access to.\
do not put more tokens in the wallet than you can afford to lose. Monitor this application closely at all times. Default RPC endpoints are not guaranteed to stay online, or to be accurate. \
You have a license under Apache-2.0 to modify and improve this software with attribution. No support or updates are guaranteed. This software is used entirely at your own risk. Pass --agree to agree to these terms.";

#[actix_rt::main]
async fn main() {
    CryptoProvider::install_default(rustls::crypto::aws_lc_rs::default_provider()).unwrap();

    let opts = RelayerOpts::parse();
    if !opts.agree {
        println!("{TERMS}");
        return;
    }
    // Initialize with specific logging level
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(opts.log_level))
        .init();

    // let transport = web3::transports::Http::new(&opts.eth_rpc).expect("Failed to create HTTP transport");
    let web3 = Web3::new(&opts.eth_rpc, Duration::from_secs(30));
    let private_key = PrivateKey::from_str(&opts.private_key).expect("Invalid private key");

    let contract_address =
        Address::from_str(&opts.contract_address).expect("Invalid contract address");

    info!("Starting Ambient transaction relayer");
    info!("Orchestrator URLs: {:?}", opts.transaction_api_url);
    info!("Ethereum RPC: {}", opts.eth_rpc);
    info!("Contract Address: {}", opts.contract_address);
    info!("Poll interval: {} seconds", opts.poll_interval);
    info!("Relayer address: {}", private_key.to_address());
    info!(
        "Relayer balance: {} ALTHEA",
        web3.eth_get_balance(private_key.to_address())
            .await
            .unwrap()
            .to_u128()
            .unwrap() as f64
            / 1e18
    );
    info!("Waiting for transactions to relay...");

    loop {
        // An orchestrator is a service that users submit their pending transactions to to be picked up
        // by relayers. This loop will iterate over all orchestrator URLs provided in the options
        for orchestrator_url in &opts.transaction_api_url {
            if let Err(e) = process_pending_transactions(
                &web3,
                orchestrator_url,
                &private_key,
                contract_address,
                &opts.price_api_url,
            )
            .await
            {
                error!("Error processing pending transactions from {orchestrator_url}: {e}");
            }
        }

        sleep(Duration::from_secs(opts.poll_interval));
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct PriceQuery {
    pub from: Address,
}
/// Fetches the current price of a given token from a price server, this is where you would add in other price feeds if you wanted to
/// this curently uses a simple custom api, but you could use anything you like, or even merge multiple price feeds together. Returns the price
/// of one unit of the request token in units of the gas token (ALTHEA).
async fn fetch_value_in_gas_token(
    price_api_url: &str,
    from: Address,
    amount: Uint256,
) -> Result<Uint256, Box<dyn std::error::Error>> {
    let url = format!("{price_api_url}/value_in_gas_token/{from}");
    debug!("Fetching price from {url}");

    let client = HttpClient::default();
    let mut response = client.request(Method::GET, url).send().await?;

    if !response.status().is_success() {
        let body = response.body().await?;
        error!("Failed to fetch price: {}", response.status());
        let error_text = String::from_utf8_lossy(&body);
        error!("Failed to fetch price: {error_text}");
        return Err(error_text.into());
    }
    let amount: f64 = amount.to_f64().ok_or("Failed to convert amount to f64")?;

    let price: f64 = response.json().await?;
    info!("Fetched price: {price} and tip amount is {amount}");
    Ok(Uint256::from((amount * price) as u128))
}

/// This loop fetches pending transactions from the orchestrator service, iterating over A records if the service has multiple IPs.
/// it then checks if each transaction is valid and profitable to relay before submitting it to the network.
async fn process_pending_transactions(
    web3: &Web3,
    orchestrator_url: &str,
    private_key: &PrivateKey,
    contract_address: Address,
    price_api_url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    debug!("Fetching pending transactions from {orchestrator_url}/{RELAYING_SERVICE_ROOT}/pending");
    let url_without_protocol = orchestrator_url
        .strip_prefix("http://")
        .or_else(|| orchestrator_url.strip_prefix("https://"))
        .unwrap_or(orchestrator_url);
    // iterate over all the A records for the orchestrator url
    let socket_addrs = url_without_protocol
        .to_socket_addrs()
        .map_err(|e| format!("Failed to resolve orchestrator URL: {e}"))?;
    for ip in socket_addrs {
        debug!("Orchestrator IP: {ip:?}");
        let mut request_head = RequestHead::default();
        request_head.peer_addr = Some(ip);
        request_head.method = Method::GET;

        let client = HttpClient::default();
        let mut response = client
            .request_from(
                format!("{orchestrator_url}/{RELAYING_SERVICE_ROOT}/pending"),
                &request_head,
            )
            .send()
            .await?;

        if !response.status().is_success() {
            let body = response.body().await?;
            let error_text = String::from_utf8_lossy(&body);
            error!("Failed to fetch pending transactions: {error_text}");
            return Err(error_text.into());
        }

        let txs: Vec<GaslessTransaction> = response.json().await?;
        debug!("Found {} pending transactions", txs.len());

        for (idx, tx) in txs.iter().enumerate() {
            debug!("Processing transaction {}/{}", idx + 1, txs.len());
            debug!(
                "Transaction details - Chain ID: {}, Callpath: {}",
                tx.chain_id, tx.callpath
            );

            match relay_transaction(web3, tx, private_key, contract_address, price_api_url).await {
                Ok(Some(tx_hash)) => {
                    info!("Transaction submitted successfully: {tx_hash}");
                }
                Ok(None) => {}
                Err(e) => {
                    debug!("Relay attempt failed with error: {}", &e);
                }
            }
        }
    }

    Ok(())
}

/// Estimates if a transaction is profitable to relay based on the current gas price and the transaction's conditions.
async fn estimate_if_transaction_is_profitable(
    tip: Uint256,
    tip_token: Address,
    gas_used: Uint256,
    gas_price: Uint256,
    price_api_url: &str,
) -> bool {
    let gas_estimate = gas_used * gas_price;
    let value = match fetch_value_in_gas_token(price_api_url, tip_token, tip).await {
        Ok(value) => value,
        Err(e) => {
            error!("Failed to fetch tip value in gas token, skipping until the next loop: {e}");
            return false;
        }
    };
    // 10% profit margin
    let gas_estimate = gas_estimate + gas_estimate / 10u8.into();
    if value > gas_estimate {
        info!("Transaction is profitable: tip value {value} > gas estimate {gas_estimate}");
        true
    } else {
        info!(
            "Transaction is not profitable Gas Price: {gas_price} Gas Amount {gas_used} tip value {value} <= gas estimate {gas_estimate}"
        );
        false
    }
}

/// Checks if the receiver address will actually pay this relayer or if it's locked
/// to some other address, this is used to prevent relaying transactions that will not pay us
fn is_valid_receiver_address(receiver: Address, our_address: Address) -> bool {
    // Check if the address is one of the special addresses
    receiver == Address::from_str(OX_100_ADDRESS).unwrap()
        || receiver == Address::from_str(OX_200_ADDRESS).unwrap()
        || receiver == our_address
}

async fn relay_transaction(
    web3: &Web3,
    tx: &GaslessTransaction,
    private_key: &PrivateKey,
    contract_address: Address,
    price_api_url: &str,
) -> Result<Option<Uint256>, Box<dyn std::error::Error>> {
    trace!("!!!!! STARTING TRANSACTION RELAY LOGGING !!!!!");

    // Check if transaction data is valid before attempting to parse
    if tx.cmd.is_empty() {
        error!("Transaction command data is empty, skipping transaction");
        return Err("Empty transaction command data".into());
    }

    // Decode tip data using proper ABI decoding
    let (tip_token, tip_amount) = if !tx.tip.is_empty() {
        let token = parse_address(&tx.tip, 0)?;
        let amount = parse_u128(&tx.tip, 32)?;
        let receiver = parse_address(&tx.tip, 64)?;
        trace!("Decoded Tip:");
        trace!("  Token: 0x{token:?}");
        trace!("  Amount: {amount}");
        trace!("  Receiver: {receiver:?}");

        if is_valid_receiver_address(receiver, private_key.to_address()) {
            (token, Uint256::from(amount))
        } else {
            info!("Transaction with invalid receiver address {receiver}, skipping");
            return Ok(None);
        }
    } else {
        info!("Transaction with no tip data, skipping");
        return Ok(None);
    };

    let call = match user_cmd_relayer_tx(*private_key, web3, contract_address, tx).await {
        Ok(call) => call,
        Err(e) => {
            debug!("Failed to prepare transaction: {e:?}");
            return Err(e.into());
        }
    };

    let tx_req = TransactionRequest::from_transaction(&call, private_key.to_address());
    trace!("Tx from: {}", tx_req.get_from());

    trace!("Simulating transaction to estimate gas");
    let gas_used = match web3.eth_estimate_gas(tx_req).await {
        Ok(gas) => {
            info!("Gas estimate: {gas}");
            gas
        }
        Err(e) => {
            error!("Failed to estimate gas: {e:?}");
            return Err(e.into());
        }
    };
    let gas_price = match web3.eth_gas_price().await {
        Ok(gp) => gp,
        Err(e) => return Err(e.into()),
    };

    if estimate_if_transaction_is_profitable(
        tip_amount,
        tip_token,
        gas_used,
        gas_price,
        price_api_url,
    )
    .await
    {
        trace!("Transaction is profitable, proceeding to send");
    } else {
        info!("Transaction is not profitable, skipping");
        return Ok(None);
    }

    trace!("Submitting transaction...");
    let result = web3.send_prepared_transaction(call).await;
    match result {
        Ok(pending_tx) => {
            info!(
                "Transaction submitted with hash, waiting: {}",
                display_uint256_as_address(pending_tx)
            );
            match web3
                .wait_for_transaction(pending_tx, web3.get_timeout(), None)
                .await
            {
                Ok(_) => {
                    info!("Transaction included in block, getting receipt");
                    let receipt = web3.eth_get_transaction_receipt(pending_tx).await;
                    info!("Receipt is {receipt:?}");
                    Ok(Some(pending_tx))
                }
                Err(e) => {
                    error!("Error waiting for transaction confirmation: {e:?}");
                    Err(e.into())
                }
            }
        }
        Err(e) => {
            error!("Transaction failed: {e:?}");
            Err(e.into())
        }
    }
}

// function userCmdRelayer (uint16 callpath, bytes calldata cmd,
//                          bytes calldata conds, bytes calldata relayerTip,
//                          bytes calldata signature)
pub const USER_CMD_RELAYER_SIG: &str = "userCmdRelayer(uint16,bytes,bytes,bytes,bytes)";

pub async fn user_cmd_relayer_tx(
    private_key: PrivateKey,
    web3: &Web3,
    dex_addr: Address,
    tx: &GaslessTransaction,
) -> Result<Transaction, Web3Error> {
    web3.prepare_transaction(
        dex_addr,
        encode_call(
            USER_CMD_RELAYER_SIG,
            &[
                tx.callpath.into(),
                tx.cmd.clone().into(),
                tx.conds.clone().into(),
                tx.tip.clone().into(),
                tx.sig.clone().into(),
            ],
        )?,
        0u8.into(),
        private_key,
        vec![SendTxOption::GasLimitMultiplier(2.0)],
    )
    .await
}

pub fn get_call_data(request: &Transaction) -> Data {
    match request {
        Transaction::Legacy { data, .. } => Data(data.clone()),
        Transaction::Eip1559 { data, .. } => Data(data.clone()),
        Transaction::Eip2930 { data, .. } => Data(data.clone()),
    }
}

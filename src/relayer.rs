//! Core transaction relaying logic

use actix_web::dev::RequestHead;
use awc::{Client as HttpClient, http::Method};
use clarity::{
    Address, PrivateKey, Transaction, Uint256, abi::encode_call, utils::display_uint256_as_address,
};
use log::{debug, error, info, trace};
use std::net::ToSocketAddrs;
use web30::{
    client::Web3,
    types::{SendTxOption, TransactionRequest},
};

use crate::constants::{RELAYING_SERVICE_ROOT, USER_CMD_RELAYER_SIG};
use crate::error::RelayerError;
use crate::pricing::estimate_profitability;
use crate::transaction::GaslessTransaction;

/// Fetches and processes pending transactions from an orchestrator service.
///
/// This function:
/// 1. Resolves the orchestrator URL to IP addresses
/// 2. Fetches pending transactions from each IP
/// 3. Validates and relays profitable transactions
///
/// # Arguments
/// * `web3` - Web3 client for Ethereum interactions
/// * `orchestrator_url` - URL of the orchestrator service
/// * `private_key` - Relayer's private key
/// * `contract_address` - Address of the DEX contract
/// * `price_api_url` - URL of the pricing service
/// * `profit_margin_percent` - Profit margin percentage to add to gas costs
pub async fn process_pending_transactions(
    web3: &Web3,
    orchestrator_url: &str,
    private_key: &PrivateKey,
    contract_address: Address,
    price_api_url: &str,
    profit_margin_percent: u8,
) -> Result<(), RelayerError> {
    info!(
        "Fetching pending transactions from {}/{}/ pending",
        orchestrator_url, RELAYING_SERVICE_ROOT
    );

    // Strip protocol prefix to resolve DNS
    let url_without_protocol = orchestrator_url
        .strip_prefix("http://")
        .or_else(|| orchestrator_url.strip_prefix("https://"))
        .unwrap_or(orchestrator_url);

    // Resolve orchestrator URL to IP addresses (handles multiple A records)
    let socket_addrs = url_without_protocol.to_socket_addrs().map_err(|e| {
        RelayerError::NetworkError(format!("Failed to resolve orchestrator URL: {}", e))
    })?;

    // Process transactions from each resolved IP
    for ip in socket_addrs {
        debug!("Connecting to orchestrator IP: {}", ip);

        let transactions = fetch_transactions_from_orchestrator(orchestrator_url, ip).await?;
        debug!("Found {} pending transactions", transactions.len());

        for (idx, tx) in transactions.iter().enumerate() {
            debug!("Processing transaction {}/{}", idx + 1, transactions.len());
            debug!(
                "Transaction details - Chain ID: {}, Callpath: {}",
                tx.chain_id, tx.callpath
            );

            match relay_single_transaction(
                web3,
                tx,
                private_key,
                contract_address,
                price_api_url,
                profit_margin_percent,
            )
            .await
            {
                Ok(Some(tx_hash)) => {
                    info!("Transaction submitted successfully: {}", tx_hash);
                }
                Ok(None) => {
                    debug!("Transaction skipped (not profitable or invalid)");
                }
                Err(e) => {
                    debug!("Relay attempt failed: {}", e);
                }
            }
        }
    }

    Ok(())
}

/// Fetches pending transactions from a specific orchestrator IP address
async fn fetch_transactions_from_orchestrator(
    orchestrator_url: &str,
    ip: std::net::SocketAddr,
) -> Result<Vec<GaslessTransaction>, RelayerError> {
    let mut request_head = RequestHead::default();
    request_head.peer_addr = Some(ip);
    request_head.method = Method::GET;

    let client = HttpClient::default();
    let endpoint = format!("{}/{}/pending", orchestrator_url, RELAYING_SERVICE_ROOT);

    let mut response = client
        .request_from(endpoint, &request_head)
        .send()
        .await
        .map_err(|e| RelayerError::NetworkError(format!("Failed to fetch transactions: {}", e)))?;

    if !response.status().is_success() {
        let body = response.body().await.map_err(|e| {
            RelayerError::NetworkError(format!("Failed to read error response: {}", e))
        })?;
        let error_text = String::from_utf8_lossy(&body);
        error!("Failed to fetch pending transactions: {}", error_text);
        return Err(RelayerError::NetworkError(error_text.to_string()));
    }

    let transactions: Vec<GaslessTransaction> = response
        .json()
        .await
        .map_err(|e| RelayerError::NetworkError(format!("Failed to parse transactions: {}", e)))?;

    Ok(transactions)
}

/// Processes and relays a single transaction if it passes validation and profitability checks.
///
/// # Returns
/// * `Ok(Some(tx_hash))` - Transaction was relayed successfully
/// * `Ok(None)` - Transaction was skipped (invalid or unprofitable)
/// * `Err(e)` - An error occurred during processing
async fn relay_single_transaction(
    web3: &Web3,
    tx: &GaslessTransaction,
    private_key: &PrivateKey,
    contract_address: Address,
    price_api_url: &str,
    profit_margin_percent: u8,
) -> Result<Option<Uint256>, RelayerError> {
    trace!("===== Starting transaction relay =====");

    // Validate basic transaction structure
    tx.validate_cmd()?;
    tx.validate_tip_receiver(private_key.to_address())?;

    // Parse tip information
    let tip_info = tx.parse_tip()?;

    // Prepare the relay transaction
    let prepared_tx = prepare_relay_transaction(private_key, web3, contract_address, tx).await?;

    // Estimate gas and check profitability
    let tx_request = TransactionRequest::from_transaction(&prepared_tx, private_key.to_address());

    trace!("Estimating gas for transaction");
    let gas_estimate = web3.eth_estimate_gas(tx_request).await.map_err(|e| {
        error!("Failed to estimate gas: {:?}", e);
        RelayerError::from(e)
    })?;
    info!("Gas estimate: {}", gas_estimate);

    let gas_price = web3.eth_gas_price().await?;

    // Check if transaction is profitable
    let is_profitable = estimate_profitability(
        tip_info.amount,
        tip_info.token,
        gas_estimate,
        gas_price,
        price_api_url,
        profit_margin_percent,
    )
    .await?;

    if !is_profitable {
        info!("Transaction is not profitable, skipping");
        return Ok(None);
    }

    // Submit the transaction
    submit_transaction(web3, prepared_tx).await.map(Some)
}

/// Prepares a relay transaction for submission to the DEX contract
async fn prepare_relay_transaction(
    private_key: &PrivateKey,
    web3: &Web3,
    dex_address: Address,
    tx: &GaslessTransaction,
) -> Result<Transaction, RelayerError> {
    let encoded_call = encode_call(
        USER_CMD_RELAYER_SIG,
        &[
            tx.callpath.into(),
            tx.cmd.clone().into(),
            tx.conds.clone().into(),
            tx.tip.clone().into(),
            tx.sig.clone().into(),
        ],
    )?;

    web3.prepare_transaction(
        dex_address,
        encoded_call,
        0u8.into(),
        *private_key,
        vec![SendTxOption::GasLimitMultiplier(2.0)],
    )
    .await
    .map_err(RelayerError::from)
}

/// Submits a prepared transaction to the network and waits for confirmation
async fn submit_transaction(
    web3: &Web3,
    transaction: Transaction,
) -> Result<Uint256, RelayerError> {
    trace!("Submitting transaction to network");

    let tx_hash = web3
        .send_prepared_transaction(transaction)
        .await
        .map_err(|e| {
            error!("Transaction submission failed: {:?}", e);
            RelayerError::from(e)
        })?;

    info!(
        "Transaction submitted with hash: {}, waiting for confirmation",
        display_uint256_as_address(tx_hash)
    );

    // Wait for transaction to be included in a block
    web3.wait_for_transaction(tx_hash, web3.get_timeout(), None)
        .await
        .map_err(|e| {
            error!("Error waiting for transaction confirmation: {:?}", e);
            RelayerError::from(e)
        })?;

    info!("Transaction confirmed in block");

    // Fetch and log receipt
    if let Ok(receipt) = web3.eth_get_transaction_receipt(tx_hash).await {
        debug!("Transaction receipt: {:?}", receipt);
    }

    Ok(tx_hash)
}

#[cfg(test)]
mod tests {}

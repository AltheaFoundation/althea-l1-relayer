//! Althea L1 Relayer
//!
//! This relayer has two features
//! A gasless transaction relayer for the iFi DEX
//! Automatic reward claiming for registered users

use clap::Parser;
use clarity::Uint256;
use log::{error, info};
use num_traits::{ToPrimitive, Zero};
use rustls::crypto::CryptoProvider;
use std::{thread::sleep, time::Duration};
use web30::client::Web3;

mod config;
mod constants;
mod error;
mod pricing;
mod relayer;
mod rewards;
mod transaction;

use config::{RelayerOpts, TERMS};
use error::RelayerError;

#[actix_rt::main]
async fn main() {
    // Initialize TLS and OpenSSL
    CryptoProvider::install_default(rustls::crypto::aws_lc_rs::default_provider())
        .expect("Failed to install crypto provider");
    unsafe {
        openssl_probe::init_openssl_env_vars();
    }

    // Parse command-line arguments
    let opts = RelayerOpts::parse();

    // Check for terms agreement
    if !opts.agree {
        println!("{}", TERMS);
        return;
    }

    // Initialize logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&opts.log_level))
        .init();

    // Initialize the relayer
    if let Err(e) = initialize_and_run(opts).await {
        error!("Relayer error: {}", e);
        std::process::exit(1);
    }
}

/// Initializes the relayer with configuration and starts the main processing loop
async fn initialize_and_run(opts: RelayerOpts) -> Result<(), RelayerError> {
    // Initialize Web3 client
    let web3 = Web3::new(&opts.althea_evm_rpc, Duration::from_secs(opts.timeout));

    // Parse configuration
    let private_key = opts.parse_private_key()?;
    let dex_contract_address = opts.parse_dex_contract_address()?;
    let rewards_contract_address = opts.parse_rewards_contract_address()?;

    // Display startup information
    display_startup_info(&web3, &opts, &private_key).await?;

    // Start main processing loop
    run_relayer_loop(
        web3,
        opts,
        private_key,
        dex_contract_address,
        rewards_contract_address,
    )
    .await
}

/// Displays startup information about the relayer configuration
async fn display_startup_info(
    web3: &Web3,
    opts: &RelayerOpts,
    private_key: &clarity::PrivateKey,
) -> Result<(), RelayerError> {
    info!("Starting Ambient transaction relayer");
    info!("Orchestrator URLs: {:?}", opts.transaction_api_url);
    info!("Ethereum RPC: {}", opts.althea_evm_rpc);
    info!("Contract Address: {}", opts.dex_contract_address);
    info!(
        "Rewards Contract Address: {}",
        opts.rewards_contract_address
    );
    info!("Poll interval: {} seconds", opts.poll_interval);
    info!("Relayer address: {}", private_key.to_address());
    info!("Profit margin: {}%", opts.profit_margin);
    info!("Auto re-register users: {}", !opts.no_auto_reregister);

    // Fetch and display relayer balance
    let balance = web3
        .eth_get_balance(private_key.to_address())
        .await
        .map_err(|e| RelayerError::NetworkError(format!("Failed to fetch balance: {}", e)))?;

    let balance_althea = balance
        .to_u128()
        .ok_or_else(|| RelayerError::Other("Failed to convert balance".to_string()))?
        as f64
        / 1e18;

    info!("Relayer balance: {} ALTHEA", balance_althea);
    info!("Waiting for transactions to relay...");

    Ok(())
}

/// Main relayer processing loop
///
/// This loop continuously:
/// 1. Processes pending transactions from all orchestrators
/// 2. Checks for profitable reward claiming opportunities
/// 3. Claims rewards when profitable
/// 4. Re-registers users for rewards when needed
async fn run_relayer_loop(
    web3: Web3,
    opts: RelayerOpts,
    private_key: clarity::PrivateKey,
    contract_address: clarity::Address,
    rewards_contract_address: clarity::Address,
) -> Result<(), RelayerError> {
    loop {
        // Process pending transactions from all orchestrators
        process_all_orchestrators(&web3, &opts, &private_key, contract_address).await;

        // Process reward claiming opportunities
        process_rewards(&web3, &opts, rewards_contract_address, private_key).await;

        // Wait before next iteration
        sleep(Duration::from_secs(opts.poll_interval));
    }
}

/// Processes pending transactions from all configured orchestrator services
async fn process_all_orchestrators(
    web3: &Web3,
    opts: &RelayerOpts,
    private_key: &clarity::PrivateKey,
    contract_address: clarity::Address,
) {
    for orchestrator_url in &opts.transaction_api_url {
        if let Err(e) = relayer::process_pending_transactions(
            web3,
            orchestrator_url,
            private_key,
            contract_address,
            &opts.price_api_url,
            opts.profit_margin,
        )
        .await
        {
            error!(
                "Error processing pending transactions from {}: {}",
                orchestrator_url, e
            );
        }
    }
}

/// Processes reward claiming opportunities
///
/// This function:
/// 1. Scans for registered users in recent blocks
/// 2. Checks if their rewards are profitable to claim
/// 3. Claims rewards when profitable
/// 4. Re-registers users when needed
async fn process_rewards(
    web3: &Web3,
    opts: &RelayerOpts,
    rewards_contract_address: clarity::Address,
    private_key: clarity::PrivateKey,
) {
    // Calculate block range to scan
    let latest_block = match web3.eth_block_number().await {
        Ok(block) => block,
        Err(e) => {
            error!("Failed to fetch latest block number: {}", e);
            return;
        }
    };

    let search_range = Uint256::from(opts.rewards_search_block_range);
    let from_block = if latest_block > search_range {
        latest_block - search_range
    } else {
        Uint256::zero()
    };

    // Find potential claim opportunities
    let opportunities = match rewards::check_for_potential_claims(
        web3,
        &opts.rewards_contract_address,
        from_block,
        latest_block,
    )
    .await
    {
        Ok(opps) => opps,
        Err(e) => {
            error!("Error checking for potential claims: {}", e);
            return;
        }
    };

    log::debug!(
        "Found {} potential claim opportunities",
        opportunities.len()
    );

    // Process each opportunity
    for opportunity in opportunities.iter() {
        process_single_reward_opportunity(
            web3,
            opts,
            rewards_contract_address,
            private_key,
            opportunity,
        )
        .await;
    }

    // Check for re-registration needs (if auto-reregistration is enabled)
    if !opts.no_auto_reregister {
        for opportunity in opportunities {
            check_reregistration_need(web3, rewards_contract_address, private_key, &opportunity)
                .await;
        }
    }
}

/// Processes a single reward claiming opportunity
async fn process_single_reward_opportunity(
    web3: &Web3,
    opts: &RelayerOpts,
    rewards_contract_address: clarity::Address,
    private_key: clarity::PrivateKey,
    opportunity: &rewards::ClaimOpportunity,
) {
    // Get current gas price
    let gas_price = match web3.eth_gas_price().await {
        Ok(price) => price,
        Err(e) => {
            error!("Failed to fetch gas price: {}", e);
            return;
        }
    };

    // Check if claiming is profitable
    match rewards::check_for_profitable_pending_rewards(
        web3,
        rewards_contract_address,
        private_key.to_address(),
        opportunity,
        gas_price,
        &opts.price_api_url,
        opts.profit_margin,
    )
    .await
    {
        Ok(true) => {
            info!(
                "Profitable pending rewards found for user {}, pool {:?}, token {}",
                opportunity.user_address, opportunity.pool_id, opportunity.token_address
            );

            // Claim the rewards
            rewards::claim_rewards_for_user(
                web3,
                rewards_contract_address,
                private_key,
                opportunity,
            )
            .await;
        }
        Ok(false) => {
            log::debug!(
                "No profitable pending rewards for user {}, pool {:02x?}, token {}",
                opportunity.user_address, opportunity.pool_id, opportunity.token_address
            );
        }
        Err(e) => {
            error!("Error checking pending rewards: {}", e);
        }
    }
}

/// Checks if a user needs re-registration and registers them if needed
async fn check_reregistration_need(
    web3: &Web3,
    rewards_contract_address: clarity::Address,
    private_key: clarity::PrivateKey,
    opportunity: &rewards::ClaimOpportunity,
) {
    match rewards::check_if_user_needs_registration(
        web3,
        rewards_contract_address,
        private_key.to_address(),
        opportunity,
    )
    .await
    {
        Ok(true) => {
            info!(
                "User {} needs to re-register for rewards, submitting registration",
                opportunity.user_address
            );

            rewards::register_user_for_rewards(
                web3,
                rewards_contract_address,
                private_key,
                opportunity,
            )
            .await;
        }
        Ok(false) => {
            info!(
                "User {} does not need to re-register for rewards",
                opportunity.user_address
            );
        }
        Err(e) => {
            error!("Error checking re-registration need: {}", e);
        }
    }
}

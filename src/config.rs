//! Configuration management for the relayer

use clap::Parser;
use clarity::{Address, PrivateKey};
use std::str::FromStr;

use crate::error::RelayerError;

/// Terms and conditions that users must agree to before running the relayer
pub const TERMS: &str = "This software is provided AS IS as a reference gassless transaction relayer. \
This software may contain bugs, lose funds, or even spend all the ALTHEA it has access to. \
Do not put more tokens in the wallet than you can afford to lose. Monitor this application closely at all times. \
Default RPC endpoints are not guaranteed to stay online, or to be accurate. \
You have a license under Apache-2.0 to modify and improve this software with attribution. \
No support or updates are guaranteed. This software is used entirely at your own risk. \
Pass --agree to agree to these terms.";

/// Command-line options for the relayer application
#[derive(Debug, Parser)]
#[command(name = "ifi-relayer", about = "iFi Dex transaction relayer")]
pub struct RelayerOpts {
    /// Private key for the relayer wallet (hex format)
    #[arg(long, value_name = "PRIVATE_KEY")]
    pub private_key: String,

    /// URLs of the orchestrator services to fetch pending transactions from
    #[arg(
        long,
        default_value = "https://althea.link:8443",
        value_name = "TRANSACTION_SERVER_URL",
        help = "URLs of the service to fetch pending transactions"
    )]
    pub transaction_api_url: Vec<String>,

    /// URL of the price API for token valuation
    #[arg(
        long,
        default_value = "https://althea.link:8443",
        value_name = "PRICE_API_URL",
        help = "URL of the price API to fetch token prices, this is a custom API that returns the price of a token in ALTHEA"
    )]
    pub price_api_url: String,

    /// Althea EVM RPC endpoint URL
    #[arg(
        long,
        default_value = "https://rpc.althea.zone:8545",
        value_name = "ALTHEA_EVM_RPC"
    )]
    pub alhtea_evm_rpc: String,

    /// Time between polling cycles in seconds
    #[arg(long, default_value = "5", value_name = "POLL_INTERVAL")]
    pub poll_interval: u64,

    /// Number of blocks to wait for transaction confirmation
    #[arg(long, default_value = "12", value_name = "CONFIRMATION_BLOCKS")]
    pub confirmation_blocks: u64,

    /// Address of the iFi DEX contract on Althea L1
    #[arg(
        long,
        default_value = "0xd263DC98dEc57828e26F69bA8687281BA5D052E0",
        value_name = "CONTRACT_ADDRESS"
    )]
    pub dex_contract_address: String,

    /// Address of the iFi DEX rewards contract on Althea L1
    #[arg(
        long,
        default_value = "0xd263DC98dEc57828e26F69bA8687281BA5D052E0",
        value_name = "REWARDS_CONTRACT_ADDRESS"
    )]
    pub rewards_contract_address: String,

    /// Historical block range to search for reward claim opportunities
    #[arg(
        long,
        default_value = "1000000",
        value_name = "REWARDS_SEARCH_BLOCK_RANGE",
        help = "How many blocks to search back in the history to find potential claims"
    )]
    pub rewards_search_block_range: u64,

    /// Logging level (trace, debug, info, warn, error)
    #[arg(
        long,
        default_value = "info",
        value_name = "LOG_LEVEL",
        help = "Set the logging level (e.g., info, debug, error)"
    )]
    pub log_level: String,

    /// Timeout for network operations in seconds
    #[arg(
        long,
        default_value = "10",
        value_name = "TIMEOUT",
        help = "Timeout for all operations in seconds"
    )]
    pub timeout: u64,

    /// Agreement to terms and conditions
    #[arg(
        long,
        default_value = "false",
        value_name = "AGREE",
        help = "Agree to the terms and conditions"
    )]
    pub agree: bool,

    /// Profit margin percentage for transaction profitability calculations
    #[arg(
        long,
        default_value = "10",
        value_name = "PROFIT_MARGIN",
        help = "Profit margin percentage to add to gas costs (e.g., 10 for 10%)"
    )]
    pub profit_margin: u8,

    /// Whether to automatically re-register users for rewards when needed
    #[arg(
        long,
        default_value = "true",
        value_name = "AUTO_REREGISTER",
        help = "Automatically re-register users for rewards when they need it. This is not directly rewarded but helps keep users eligible for future rewards."
    )]
    pub auto_reregister: bool,
}

impl RelayerOpts {
    /// Parse and validate the private key from the configuration
    pub fn parse_private_key(&self) -> Result<PrivateKey, RelayerError> {
        PrivateKey::from_str(&self.private_key)
            .map_err(|_| RelayerError::ConfigurationError("Invalid private key format".to_string()))
    }

    /// Parse and validate the contract address from the configuration
    pub fn parse_dex_contract_address(&self) -> Result<Address, RelayerError> {
        Address::from_str(&self.dex_contract_address)
            .map_err(|_| RelayerError::ConfigurationError("Invalid contract address".to_string()))
    }

    /// Parse and validate the rewards contract address from the configuration
    pub fn parse_rewards_contract_address(&self) -> Result<Address, RelayerError> {
        Address::from_str(&self.rewards_contract_address).map_err(|_| {
            RelayerError::ConfigurationError("Invalid rewards contract address".to_string())
        })
    }
}

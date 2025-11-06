//! Rewards claiming system for automatic reward collection
//!
//! This module handles:
//! - Scanning for users registered for rewards
//! - Checking claim profitability
//! - Automatically claiming rewards on behalf of users
//! - Re-registering users when needed

use clarity::{
    Address, PrivateKey, Uint256,
    abi::{AbiToken, encode_call},
};
use log::{error, info};
use std::collections::HashSet;
use web30::{
    client::Web3,
    jsonrpc::error::Web3Error,
    types::{Log, TransactionRequest},
};

use crate::constants::{
    CLAIM_EVENT_SIGNATURE, CLAIM_REWARDS_FUNCTION_SIGNATURE, GET_POTENTIAL_TIP_FUNCTION_SIGNATURE,
    NEEDS_REREGISTRATION_FUNCTION_SIGNATURE, REGISTER_FUNCTION_SIGNATURE,
    REGISTRATION_EVENT_SIGNATURE,
};
use crate::pricing::estimate_profitability;

/// Represents a potential rewards claim opportunity for a registered user.
///
/// These opportunities are identified by scanning Registration and Claim events
/// from the rewards contract. Each opportunity represents a unique combination
/// of user, pool, and reward token.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct ClaimOpportunity {
    /// Address of the user who is registered for rewards
    pub user_address: Address,

    /// Unique identifier for the liquidity pool (32-byte pool ID)
    pub pool_id: [u8; 32],

    /// Address of the token used for reward payments
    pub token_address: Address,

    /// Whether this is a concentrated liquidity position
    pub concentrated: bool,
}

/// Scans for potential claim opportunities by analyzing blockchain events.
///
/// This function queries Registration and Claim events from the rewards contract
/// within the specified block range. It returns a deduplicated set of opportunities
/// that should be checked for profitability.
///
/// # Arguments
/// * `web3` - Web3 client for blockchain queries
/// * `rewards_contract_address` - Address of the rewards contract
/// * `from_block` - Starting block number for the search
/// * `to_block` - Ending block number for the search
///
/// # Returns
/// A set of unique claim opportunities found in the event logs
pub async fn check_for_potential_claims(
    web3: &Web3,
    rewards_contract_address: &str,
    from_block: Uint256,
    to_block: Uint256,
) -> Result<HashSet<ClaimOpportunity>, Web3Error> {
    let contract_address: Address = rewards_contract_address.parse()?;

    // Query both Registration and Claim events
    let events = web3
        .check_for_multiple_events(
            from_block,
            Some(to_block),
            vec![contract_address],
            vec![
                vec![REGISTRATION_EVENT_SIGNATURE],
                vec![CLAIM_EVENT_SIGNATURE],
            ],
        )
        .await?;

    // Parse and deduplicate opportunities
    let claim_opportunities = parse_event_log(events);

    Ok(claim_opportunities)
}

/// Parses event logs into claim opportunities.
///
/// Both Registration and Claim events have the same indexed parameters:
/// - topic[1]: user address
/// - topic[2]: pool ID
/// - topic[3]: token address
/// - data[0..32]: concentrated (bool)
///
/// This allows us to parse both event types uniformly.
fn parse_event_log(input: Vec<Log>) -> HashSet<ClaimOpportunity> {
    let mut opportunities: HashSet<ClaimOpportunity> = HashSet::new();

    for log in input.iter() {
        // Extract user address from topic 1 (last 20 bytes of 32-byte topic)
        let user_address: Address = Address::from_slice(&log.topics[1][12..]).unwrap();

        // Extract pool ID from topic 2
        let pool_id = Uint256::from_be_bytes(&log.topics[2]).to_be_bytes();

        // Extract token address from topic 3 (last 20 bytes of 32-byte topic)
        let token_address: Address = Address::from_slice(&log.topics[3][12..]).unwrap();

        // Extract concentrated flag from first 32 bytes of data
        let is_concentrated = !log.data[0..32].is_empty();

        opportunities.insert(ClaimOpportunity {
            user_address,
            pool_id,
            token_address,
            concentrated: is_concentrated,
        });
    }

    opportunities
}

/// Claims rewards on behalf of a user.
///
/// This function prepares and submits a transaction to claim pending rewards
/// for a user. The rewards are split between the user and the relayer according
/// to the contract's fee structure.
///
/// # Arguments
/// * `web3` - Web3 client for transaction submission
/// * `rewards_contract_address` - Address of the rewards contract
/// * `private_key` - Relayer's private key for signing
/// * `opportunity` - The claim opportunity to process
pub async fn claim_rewards_for_user(
    web3: &Web3,
    rewards_contract_address: Address,
    private_key: PrivateKey,
    opportunity: &ClaimOpportunity,
) {
    info!(
        "Claiming rewards for user {}, pool {:?}, token {}",
        opportunity.user_address, opportunity.pool_id, opportunity.token_address
    );

    // Encode the claim function call
    let call_data = match encode_call(
        CLAIM_REWARDS_FUNCTION_SIGNATURE,
        &[
            AbiToken::Address(opportunity.user_address),
            AbiToken::Bytes(opportunity.pool_id.to_vec()),
            AbiToken::Address(opportunity.token_address),
            AbiToken::Bool(opportunity.concentrated),
        ],
    ) {
        Ok(data) => data,
        Err(e) => {
            error!("Failed to encode claim call: {:?}", e);
            return;
        }
    };

    // Prepare the transaction
    let request = match web3
        .prepare_transaction(
            rewards_contract_address,
            call_data,
            0u8.into(),
            private_key,
            vec![],
        )
        .await
    {
        Ok(req) => req,
        Err(e) => {
            error!(
                "Error preparing claim rewards transaction for user {}: {:?}",
                opportunity.user_address, e
            );
            return;
        }
    };

    // Submit the transaction
    match web3.send_prepared_transaction(request).await {
        Ok(tx_hash) => {
            info!(
                "Submitted claim rewards transaction for user {} with tx hash {:?}",
                opportunity.user_address, tx_hash
            );
        }
        Err(e) => {
            error!(
                "Error submitting claim rewards transaction for user {}: {:?}",
                opportunity.user_address, e
            );
        }
    }
}

/// Checks if a user needs to re-register for rewards.
///
/// Users may need to re-register if they've modified their liquidity position
/// (e.g., withdrawn liquidity, changed ranges). This function queries the
/// contract to determine if re-registration is needed.
///
/// # Arguments
/// * `web3` - Web3 client for contract queries
/// * `rewards_contract_address` - Address of the rewards contract
/// * `caller_address` - Address to use for the simulation
/// * `opportunity` - The claim opportunity to check
///
/// # Returns
/// `true` if the user needs to re-register, `false` otherwise
pub async fn check_if_user_needs_registration(
    web3: &Web3,
    rewards_contract_address: Address,
    caller_address: Address,
    opportunity: &ClaimOpportunity,
) -> Result<bool, Web3Error> {
    let call_data = encode_call(
        NEEDS_REREGISTRATION_FUNCTION_SIGNATURE,
        &[
            AbiToken::Address(opportunity.user_address),
            AbiToken::Bytes(opportunity.pool_id.to_vec()),
            AbiToken::Address(opportunity.token_address),
            AbiToken::Bool(opportunity.concentrated),
        ],
    )
    .unwrap();

    let result = web3
        .simulate_transaction(
            TransactionRequest::quick_tx(caller_address, rewards_contract_address, call_data),
            vec![],
            None,
        )
        .await?;

    // Non-empty result indicates re-registration is needed
    let needs_reregistration = !result.is_empty();
    Ok(needs_reregistration)
}

/// Registers a user for rewards.
///
/// This function submits a transaction to register a user for automatic rewards
/// claiming. Registration is required before a user's rewards can be claimed
/// by relayers.
///
/// # Arguments
/// * `web3` - Web3 client for transaction submission
/// * `rewards_contract_address` - Address of the rewards contract
/// * `private_key` - Relayer's private key for signing
/// * `opportunity` - The opportunity to register for
pub async fn register_user_for_rewards(
    web3: &Web3,
    rewards_contract_address: Address,
    private_key: PrivateKey,
    opportunity: &ClaimOpportunity,
) {
    info!(
        "Registering user {} for rewards on pool {:?}, token {}",
        opportunity.user_address, opportunity.pool_id, opportunity.token_address
    );

    // Encode the registration function call
    let call_data = match encode_call(
        REGISTER_FUNCTION_SIGNATURE,
        &[
            AbiToken::Address(opportunity.user_address),
            AbiToken::Bytes(opportunity.pool_id.to_vec()),
            AbiToken::Address(opportunity.token_address),
            AbiToken::Bool(opportunity.concentrated),
        ],
    ) {
        Ok(data) => data,
        Err(e) => {
            error!("Failed to encode registration call: {:?}", e);
            return;
        }
    };

    // Prepare the transaction
    let request = match web3
        .prepare_transaction(
            rewards_contract_address,
            call_data,
            0u8.into(),
            private_key,
            vec![],
        )
        .await
    {
        Ok(req) => req,
        Err(e) => {
            error!(
                "Error preparing registration transaction for user {}: {:?}",
                opportunity.user_address, e
            );
            return;
        }
    };

    // Submit the transaction
    match web3.send_prepared_transaction(request).await {
        Ok(tx_hash) => {
            info!(
                "Submitted registration transaction for user {} with tx hash {:?}",
                opportunity.user_address, tx_hash
            );
        }
        Err(e) => {
            error!(
                "Error submitting registration transaction for user {}: {:?}",
                opportunity.user_address, e
            );
        }
    }
}

/// Checks if claiming rewards for a user would be profitable.
///
/// This function performs two contract calls:
/// 1. Query the potential tip amount (relayer's share)
/// 2. Estimate the gas cost for claiming
///
/// It then compares the tip value against the gas cost to determine profitability.
///
/// # Arguments
/// * `web3` - Web3 client for contract queries
/// * `rewards_contract_address` - Address of the rewards contract
/// * `caller_address` - Address to use for simulations
/// * `opportunity` - The claim opportunity to evaluate
/// * `gas_price` - Current gas price
/// * `price_api_url` - URL of the pricing service
/// * `profit_margin_percent` - Profit margin percentage to add to gas costs
///
/// # Returns
/// `true` if claiming would be profitable, `false` otherwise
pub async fn check_for_profitable_pending_rewards(
    web3: &Web3,
    rewards_contract_address: Address,
    caller_address: Address,
    opportunity: &ClaimOpportunity,
    gas_price: Uint256,
    price_api_url: &str,
    profit_margin_percent: u8,
) -> Result<bool, Box<dyn std::error::Error>> {
    // Query the potential tip (relayer's share of rewards)
    let tip_call_data = encode_call(
        GET_POTENTIAL_TIP_FUNCTION_SIGNATURE,
        &[
            AbiToken::Address(opportunity.user_address),
            AbiToken::Bytes(opportunity.pool_id.to_vec()),
            AbiToken::Address(opportunity.token_address),
            AbiToken::Bool(opportunity.concentrated),
        ],
    )
    .unwrap();

    let tip_result = web3
        .simulate_transaction(
            TransactionRequest::quick_tx(caller_address, rewards_contract_address, tip_call_data),
            vec![],
            None,
        )
        .await?;

    let pending_tip: Uint256 = Uint256::from_be_bytes(&tip_result);

    // Early exit if there's no tip to claim
    if pending_tip.is_zero() {
        return Ok(false);
    }

    // Estimate gas cost for the claim transaction
    let claim_call_data = encode_call(
        CLAIM_REWARDS_FUNCTION_SIGNATURE,
        &[
            AbiToken::Address(opportunity.user_address),
            AbiToken::Bytes(opportunity.pool_id.to_vec()),
            AbiToken::Address(opportunity.token_address),
            AbiToken::Bool(opportunity.concentrated),
        ],
    )
    .unwrap();

    let gas_estimate = web3
        .eth_estimate_gas(TransactionRequest::quick_tx(
            caller_address,
            rewards_contract_address,
            claim_call_data,
        ))
        .await?;

    // Check profitability using the pricing module
    estimate_profitability(
        pending_tip,
        opportunity.token_address,
        gas_estimate,
        gas_price,
        price_api_url,
        profit_margin_percent,
    )
    .await
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claim_opportunity_equality() {
        let opp1 = ClaimOpportunity {
            user_address: Address::default(),
            pool_id: [0u8; 32],
            token_address: Address::default(),
            concentrated: false,
        };

        let opp2 = ClaimOpportunity {
            user_address: Address::default(),
            pool_id: [0u8; 32],
            token_address: Address::default(),
            concentrated: false,
        };

        assert_eq!(opp1, opp2);
    }
}

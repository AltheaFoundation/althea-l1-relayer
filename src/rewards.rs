//// This file contains code for finding users registered for rewards and checking if they have a profitable claim
//// or if they can re-register going forward after a position change

use clarity::PrivateKey;
use clarity::abi::AbiToken;
use clarity::{Address, Uint256, abi::encode_call};
use std::{collections::HashSet, vec};
use web30::{
    client::Web3,
    jsonrpc::error::Web3Error,
    types::{Log, TransactionRequest},
};

use crate::estimate_if_transaction_is_profitable;

/// Represents a potenial lead for a user registered for rewards
/// that we should check for claimability and profitability
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct ClaimOpportunity {
    pub user_address: Address,
    pub pool_id: [u8; 32],
    pub token_address: Address,
    pub concentrated: bool,
}

const REGISTRATION_EVENT_SIGNATURE: &str = "Registration(address,bytes32,address,bool,address)";
const CLAIM_EVENT_SIGNATURE: &str =
    "ClaimRewards(address,bytes32,address,bool,uint256,address,uint256)";
const GET_POTENTIAL_TIP_FUNCTION_SIGNATURE: &str = "getPotentialTip(address,bytes32,address,bool)";
const CLAIM_REWARDS_FUNCTION_SIGNATURE: &str =
    "claimRewardsOnBehalfOf(address,bytes32,address,bool)";
const NEEDS_REREGISTRATION_FUNCTION_SIGNATURE: &str =
    "needsReregistration(address,bytes32,address,bool)";
const REGISTER_FUNCTION_SIGNATURE: &str = "register(address,bytes32,address,bool)";

/// Checks for potentail claim opportunities by scanning for registration and claim events
/// emitted by the rewards contract between the specified blocks. Ideally this can be run
/// with a very large block range to find all potential claims that should be checked
/// since some relayers will claim regularly auto-claim should work to keep things up to date
pub async fn check_for_potential_claims(
    web3: &Web3,
    rewards_contract_address: &str,
    from_block: Uint256,
    to_block: Uint256,
) -> Result<HashSet<ClaimOpportunity>, Web3Error> {
    let contract_address: Address = rewards_contract_address.parse()?;
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
    let claim_opportunities = parse_event_log(events);

    Ok(claim_opportunities)
}

fn parse_event_log(input: Vec<Log>) -> HashSet<ClaimOpportunity> {
    let mut registrations: HashSet<ClaimOpportunity> = HashSet::new();

    for log in input.iter() {
        // we only want the first three items, user, poolid, and rewards token
        // so we can parse both events in the same way
        let user_address: Address = Address::from_slice(&log.topics[1][12..]).unwrap();
        let pool_id = Uint256::from_be_bytes(&log.topics[2]).to_be_bytes();
        let token_address: Address = Address::from_slice(&log.topics[3][12..]).unwrap();
        // this is the first non-indexed value in both events
        let is_concentrated = !log.data[0..32].is_empty();
        registrations.insert(ClaimOpportunity {
            user_address,
            pool_id,
            token_address,
            concentrated: is_concentrated,
        });
    }
    registrations
}

/// Calls the claim on behalf of function for a given claim opportunity
/// and claims the rewards for this relayer and the user.
pub async fn claim_rewards_for_user(
    web3: &Web3,
    rewards_contract_address: Address,
    private_key: PrivateKey,
    opportunity: &ClaimOpportunity,
) {
    let request = web3
        .prepare_transaction(
            rewards_contract_address,
            encode_call(
                CLAIM_REWARDS_FUNCTION_SIGNATURE,
                &vec![
                    AbiToken::Address(opportunity.user_address),
                    AbiToken::Bytes(opportunity.pool_id.to_vec()),
                    AbiToken::Address(opportunity.token_address),
                    AbiToken::Bool(opportunity.concentrated),
                ],
            )
            .unwrap(),
            0u8.into(),
            private_key,
            vec![],
        )
        .await;
    let request = match request {
        Ok(req) => req,
        Err(e) => {
            log::error!(
                "Error preparing claim rewards transaction for user {:?}: {:?}",
                opportunity.user_address,
                e
            );
            return;
        }
    };
    match web3.send_prepared_transaction(request).await {
        Ok(tx_hash) => {
            log::info!(
                "Submitted claim rewards transaction for user {:?} with tx hash {:?}",
                opportunity.user_address,
                tx_hash
            );
        }
        Err(e) => {
            log::error!(
                "Error submitting claim rewards transaction for user {:?}: {:?}",
                opportunity.user_address,
                e
            );
        }
    }
}

/// check if a user needs to re-register for rewards for a given claim opportunity
pub async fn check_if_user_needs_registration(
    web3: &Web3,
    rewards_contract_address: Address,
    caller_address: Address,
    opportunity: &ClaimOpportunity,
) -> Result<bool, Web3Error> {
    let call = web3
        .simulate_transaction(
            TransactionRequest::quick_tx(
                caller_address,
                rewards_contract_address,
                encode_call(
                    NEEDS_REREGISTRATION_FUNCTION_SIGNATURE,
                    &vec![
                        AbiToken::Address(opportunity.user_address),
                        AbiToken::Bytes(opportunity.pool_id.to_vec()),
                        AbiToken::Address(opportunity.token_address),
                        AbiToken::Bool(opportunity.concentrated),
                    ],
                )
                .unwrap(),
            ),
            vec![],
            None,
        )
        .await?;
    let needs_reregistration: bool = !call.is_empty();
    Ok(needs_reregistration)
}

/// registers a user for rewards for a given claim opportunity
pub async fn register_user_for_rewards(
    web3: &Web3,
    rewards_contract_address: Address,
    private_key: PrivateKey,
    opportunity: &ClaimOpportunity,
) {
    let request = web3
        .prepare_transaction(
            rewards_contract_address,
            encode_call(
                REGISTER_FUNCTION_SIGNATURE,
                &vec![
                    AbiToken::Address(opportunity.user_address),
                    AbiToken::Bytes(opportunity.pool_id.to_vec()),
                    AbiToken::Address(opportunity.token_address),
                    AbiToken::Bool(opportunity.concentrated),
                ],
            )
            .unwrap(),
            0u8.into(),
            private_key,
            vec![],
        )
        .await;
    let request = match request {
        Ok(req) => req,
        Err(e) => {
            log::error!(
                "Error preparing registration transaction for user {:?}: {:?}",
                opportunity.user_address,
                e
            );
            return;
        }
    };
    match web3.send_prepared_transaction(request).await {
        Ok(tx_hash) => {
            log::info!(
                "Submitted registration transaction for user {:?} with tx hash {:?}",
                opportunity.user_address,
                tx_hash
            );
        }
        Err(e) => {
            log::error!(
                "Error submitting registration transaction for user {:?}: {:?}",
                opportunity.user_address,
                e
            );
        }
    }
}

/// check for profitable pending rewards for a given claim opportunity, first checks the potentail tip
/// then simulates a claim to estimate gas and profitability
pub async fn check_for_profitable_pending_rewards(
    web3: &Web3,
    rewards_contract_address: Address,
    caller_address: Address,
    opportunity: &ClaimOpportunity,
    gas_price: Uint256,
    price_api_url: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    let call = web3
        .simulate_transaction(
            TransactionRequest::quick_tx(
                caller_address,
                rewards_contract_address,
                encode_call(
                    GET_POTENTIAL_TIP_FUNCTION_SIGNATURE,
                    &vec![
                        AbiToken::Address(opportunity.user_address),
                        AbiToken::Bytes(opportunity.pool_id.to_vec()),
                        AbiToken::Address(opportunity.token_address),
                        AbiToken::Bool(opportunity.concentrated),
                    ],
                )
                .unwrap(),
            ),
            vec![],
            None,
        )
        .await?;
    let pending_rewards: Uint256 = Uint256::from_be_bytes(&call);
    // special case saves us the work of fetching prices etc if there are no pending rewards
    if pending_rewards.is_zero() {
        return Ok(false);
    }
    // now we simulate the claim to get gas estimate
    let gas_estimate = web3
        .eth_estimate_gas(TransactionRequest::quick_tx(
            caller_address,
            rewards_contract_address,
            encode_call(
                CLAIM_REWARDS_FUNCTION_SIGNATURE,
                &vec![
                    AbiToken::Address(opportunity.user_address),
                    AbiToken::Bytes(opportunity.pool_id.to_vec()),
                    AbiToken::Address(opportunity.token_address),
                    AbiToken::Bool(opportunity.concentrated),
                ],
            )
            .unwrap(),
        ))
        .await?;

    // now we can estimate profitability
    estimate_if_transaction_is_profitable(
        pending_rewards,
        opportunity.token_address,
        gas_estimate,
        gas_price,
        price_api_url,
    )
    .await
}

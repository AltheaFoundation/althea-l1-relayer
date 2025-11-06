//! Transaction parsing and validation utilities

use clarity::{
    Address, Uint256,
    abi::{parse_address, parse_u128},
};
use log::trace;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::constants::{OX_100_ADDRESS, OX_200_ADDRESS};
use crate::error::RelayerError;

/// Represents a gasless transaction submitted by a user
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GaslessTransaction {
    /// Chain ID for replay protection
    pub chain_id: u64,

    /// Call path identifier for the DEX operation
    pub callpath: u16,

    /// Encoded command data
    pub cmd: Vec<u8>,

    /// Encoded conditions for execution
    pub conds: Vec<u8>,

    /// Encoded tip data (token, amount, receiver)
    pub tip: Vec<u8>,

    /// User's signature over the transaction
    pub sig: Vec<u8>,

    /// Unix timestamp when the transaction was submitted
    pub submitted_at: u64,
}

/// Parsed tip information from a gasless transaction
#[derive(Debug, Clone)]
pub struct TipInfo {
    /// Address of the token used for the tip
    pub token: Address,

    /// Amount of tip tokens offered
    pub amount: Uint256,

    /// Address designated to receive the tip
    pub receiver: Address,
}

impl GaslessTransaction {
    /// Validates that the transaction has non-empty command data
    pub fn validate_cmd(&self) -> Result<(), RelayerError> {
        if self.cmd.is_empty() {
            return Err(RelayerError::TransactionError(
                "Transaction command data is empty".to_string(),
            ));
        }
        Ok(())
    }

    /// Validates that the transaction has tip data
    pub fn validate_tip(&self) -> Result<(), RelayerError> {
        if self.tip.is_empty() {
            return Err(RelayerError::TransactionError(
                "Transaction has no tip data".to_string(),
            ));
        }
        Ok(())
    }

    /// Parses the tip data from the transaction.
    ///
    /// The tip data is ABI-encoded as: (address token, uint128 amount, address receiver)
    pub fn parse_tip(&self) -> Result<TipInfo, RelayerError> {
        self.validate_tip()?;

        let token = parse_address(&self.tip, 0).map_err(|e| {
            RelayerError::TransactionError(format!("Failed to parse tip token: {}", e))
        })?;

        let amount = parse_u128(&self.tip, 32).map_err(|e| {
            RelayerError::TransactionError(format!("Failed to parse tip amount: {}", e))
        })?;

        let receiver = parse_address(&self.tip, 64).map_err(|e| {
            RelayerError::TransactionError(format!("Failed to parse tip receiver: {}", e))
        })?;

        trace!(
            "Decoded tip - Token: {}, Amount: {}, Receiver: {}",
            token, amount, receiver
        );

        Ok(TipInfo {
            token,
            amount: Uint256::from(amount),
            receiver,
        })
    }

    /// Validates that the tip receiver is acceptable for this relayer
    pub fn validate_tip_receiver(&self, relayer_address: Address) -> Result<(), RelayerError> {
        let tip_info = self.parse_tip()?;

        if !is_valid_receiver_address(tip_info.receiver, relayer_address) {
            return Err(RelayerError::TransactionError(format!(
                "Invalid tip receiver address: {}",
                tip_info.receiver
            )));
        }

        Ok(())
    }
}

/// Checks if a receiver address is valid for this relayer.
///
/// Valid receivers are:
/// - The special 0x100 address (any relayer can claim)
/// - The special 0x200 address (any relayer can claim)
/// - Our specific relayer address
fn is_valid_receiver_address(receiver: Address, our_address: Address) -> bool {
    let ox_100 = Address::from_str(OX_100_ADDRESS).unwrap();
    let ox_200 = Address::from_str(OX_200_ADDRESS).unwrap();

    receiver == ox_100 || receiver == ox_200 || receiver == our_address
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_receiver_addresses() {
        let relayer = Address::from_str("0x1234567890123456789012345678901234567890").unwrap();
        let ox_100 = Address::from_str(OX_100_ADDRESS).unwrap();
        let ox_200 = Address::from_str(OX_200_ADDRESS).unwrap();
        let random = Address::from_str("0xabcdefabcdefabcdefabcdefabcdefabcdefabcd").unwrap();

        assert!(is_valid_receiver_address(ox_100, relayer));
        assert!(is_valid_receiver_address(ox_200, relayer));
        assert!(is_valid_receiver_address(relayer, relayer));
        assert!(!is_valid_receiver_address(random, relayer));
    }
}

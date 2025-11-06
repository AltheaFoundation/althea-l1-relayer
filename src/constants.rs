//! Application-wide constants and configuration values

/// Special address for unrestricted relay payments (anyone can relay)
pub const OX_100_ADDRESS: &str = "0x0000000000000000000000000000000000000100";

/// Special address for unrestricted relay payments (alternative)
pub const OX_200_ADDRESS: &str = "0x0000000000000000000000000000000000000200";

/// Root path for the relaying service API endpoints
pub const RELAYING_SERVICE_ROOT: &str = "orchestrator";

// Contract function signatures

/// DEX relayer transaction signature
pub const USER_CMD_RELAYER_SIG: &str = "userCmdRelayer(uint16,bytes,bytes,bytes,bytes)";

/// Rewards contract event signatures
pub const REGISTRATION_EVENT_SIGNATURE: &str = "Registration(address,bytes32,address,bool,address)";
pub const CLAIM_EVENT_SIGNATURE: &str =
    "ClaimRewards(address,bytes32,address,bool,uint256,address,uint256)";

/// Rewards contract function signatures
pub const GET_POTENTIAL_TIP_FUNCTION_SIGNATURE: &str =
    "getPotentialTip(address,bytes32,address,bool)";
pub const CLAIM_REWARDS_FUNCTION_SIGNATURE: &str =
    "claimRewardsOnBehalfOf(address,bytes32,address,bool)";
pub const NEEDS_REREGISTRATION_FUNCTION_SIGNATURE: &str =
    "needsReregistration(address,bytes32,address,bool)";
pub const REGISTER_FUNCTION_SIGNATURE: &str = "register(address,bytes32,address,bool)";

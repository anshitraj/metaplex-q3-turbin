use anchor_lang::prelude::*;

// PDA seeds
pub const CONFIG_SEED: &[u8] = b"config";
pub const REWARDS_SEED: &[u8] = b"rewards";
pub const USER_SEED: &[u8] = b"user";
pub const STAKE_SEED: &[u8] = b"stake";
pub const ORACLE_SEED: &[u8] = b"oracle";
pub const VAULT_SEED: &[u8] = b"vault";

/// Key of the Attribute kept on the Collection account.
pub const TOTAL_STAKED_KEY: &str = "total_staked";

pub const REWARDS_DECIMALS: u8 = 6;

// Time-based transfer window (UTC): transfers are allowed in [OPEN, CLOSE).
pub const SECONDS_PER_DAY: i64 = 86_400;
pub const OPEN_SECONDS: i64 = 9 * 3_600; // 09:00 UTC
pub const CLOSE_SECONDS: i64 = 17 * 3_600; // 17:00 UTC

/// `ExternalCheckResult` flag bits understood by MPL Core.
/// An Oracle adapter may only be configured with `CAN_REJECT`.
pub const CHECK_CAN_REJECT: u32 = 0b100;

#[constant]
pub const MAX_CRANK_WINDOW_SECONDS: i64 = 3_600;

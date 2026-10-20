use anchor_lang::prelude::*;

/// One staking pool, bound to exactly one MPL Core collection.
#[account]
#[derive(InitSpace)]
pub struct StakeConfig {
    /// Collection admin: creates the pool and the oracle.
    pub admin: Pubkey,
    /// The MPL Core collection whose assets can be staked.
    pub collection: Pubkey,
    /// Reward tokens (base units) accrued per staked NFT per second.
    pub reward_per_second: u64,
    /// One-time reward tokens (base units) paid on top of accrued rewards when
    /// a staked NFT is burned.
    pub burn_bonus: u64,
    /// Seconds an NFT has to stay staked before it can be unstaked.
    pub freeze_period: i64,
    /// Max NFTs a single user may stake at once.
    pub max_stake: u8,
    pub rewards_bump: u8,
    pub bump: u8,
}

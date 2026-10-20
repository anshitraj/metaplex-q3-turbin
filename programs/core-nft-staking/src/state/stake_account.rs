use anchor_lang::prelude::*;

/// Per-asset stake record. This PDA is also the *plugin authority* of the
/// asset's `FreezeDelegate` and `BurnDelegate` plugins, so the program (and only
/// the program) can thaw / burn the asset.
#[account]
#[derive(InitSpace)]
pub struct StakeAccount {
    pub owner: Pubkey,
    pub asset: Pubkey,
    pub staked_at: i64,
    /// Rewards accrue from here; moved forward by `claim_rewards`.
    pub last_claim_at: i64,
    pub bump: u8,
}

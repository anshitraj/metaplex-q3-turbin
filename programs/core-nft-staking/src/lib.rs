#![allow(unexpected_cfgs)]

use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod instructions;
pub mod state;
pub mod utils;

pub use instructions::*;

declare_id!("EMKQYHrTWbHr245UYkesW6n4menbJoUKWPkKvSPMeF4i");

#[program]
pub mod core_nft_staking {
    use super::*;

    // ---- Task 1: staking on Core plugins -------------------------------------

    /// Creates a staking pool for an existing Core collection and puts the
    /// `total_staked` Attribute on that collection.
    pub fn initialize_config(
        ctx: Context<InitializeConfig>,
        reward_per_second: u64,
        burn_bonus: u64,
        freeze_period: i64,
        max_stake: u8,
    ) -> Result<()> {
        ctx.accounts.initialize_config(
            reward_per_second,
            burn_bonus,
            freeze_period,
            max_stake,
            &ctx.bumps,
        )
    }

    pub fn initialize_user(ctx: Context<InitializeUser>) -> Result<()> {
        ctx.accounts.initialize_user(&ctx.bumps)
    }

    /// Freezes the NFT in the owner's wallet (FreezeDelegate), installs a
    /// BurnDelegate, and increments the collection's `total_staked`.
    pub fn stake(ctx: Context<Stake>) -> Result<()> {
        ctx.accounts.stake(&ctx.bumps)
    }

    /// Mints the rewards accrued so far to the user's ATA; the NFT stays
    /// staked and frozen.
    pub fn claim_rewards(ctx: Context<ClaimRewards>) -> Result<()> {
        ctx.accounts.claim_rewards()
    }

    /// Thaws the NFT, removes the delegates, pays out the accrued rewards and
    /// decrements `total_staked`.
    pub fn unstake(ctx: Context<Unstake>) -> Result<()> {
        ctx.accounts.unstake()
    }

    /// Burn-to-earn: burns the staked NFT through its BurnDelegate and mints the
    /// accrued rewards plus a one-time bonus.
    pub fn burn_staked_nft(ctx: Context<BurnStakedNft>) -> Result<()> {
        ctx.accounts.burn_staked_nft()
    }

    // ---- Task 2: time-gated transfers with the Oracle plugin -----------------

    pub fn initialize_oracle(
        ctx: Context<InitializeOracle>,
        crank_reward: u64,
        crank_window: i64,
        vault_funding: u64,
    ) -> Result<()> {
        ctx.accounts
            .initialize_oracle(crank_reward, crank_window, vault_funding, &ctx.bumps)
    }

    /// Permissionless crank that syncs the Oracle with the transfer window.
    pub fn update_oracle(ctx: Context<UpdateOracle>) -> Result<()> {
        ctx.accounts.update_oracle()
    }

    pub fn transfer_nft(ctx: Context<TransferNft>) -> Result<()> {
        ctx.accounts.transfer_nft()
    }
}

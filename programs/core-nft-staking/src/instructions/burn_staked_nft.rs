use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{Mint, Token, TokenAccount},
};
use mpl_core::{
    instructions::{BurnV1CpiBuilder, UpdatePluginV1CpiBuilder},
    types::{FreezeDelegate, Plugin},
    ID as MPL_CORE_ID,
};

use crate::{
    constants::*,
    errors::StakeError,
    state::{StakeAccount, StakeConfig, UserAccount},
    utils::{mint_rewards, pending_rewards, update_total_staked},
};

/// Burn-to-earn: permanently destroy a staked NFT in exchange for the accrued
/// rewards plus a one-time `burn_bonus`.
#[derive(Accounts)]
pub struct BurnStakedNft<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    /// CHECK: only MPL Core touches it; the stake PDA is derived from this key.
    #[account(mut)]
    pub asset: UncheckedAccount<'info>,
    /// CHECK: the config PDA is derived from this key.
    #[account(mut)]
    pub collection: UncheckedAccount<'info>,
    #[account(
        seeds = [CONFIG_SEED, collection.key().as_ref()],
        bump = config.bump,
    )]
    pub config: Account<'info, StakeConfig>,
    #[account(
        mut,
        close = user,
        seeds = [STAKE_SEED, asset.key().as_ref(), config.key().as_ref()],
        bump = stake_account.bump,
        constraint = stake_account.owner == user.key() @ StakeError::NotAssetOwner,
    )]
    pub stake_account: Account<'info, StakeAccount>,
    #[account(
        mut,
        seeds = [USER_SEED, config.key().as_ref(), user.key().as_ref()],
        bump = user_account.bump,
    )]
    pub user_account: Account<'info, UserAccount>,
    #[account(
        mut,
        seeds = [REWARDS_SEED, config.key().as_ref()],
        bump = config.rewards_bump,
    )]
    pub rewards_mint: Account<'info, Mint>,
    #[account(
        init_if_needed,
        payer = user,
        associated_token::mint = rewards_mint,
        associated_token::authority = user,
        associated_token::token_program = token_program,
    )]
    pub rewards_ata: Account<'info, TokenAccount>,
    /// CHECK: verified against the canonical MPL Core program id.
    #[account(address = MPL_CORE_ID)]
    pub mpl_core_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

impl<'info> BurnStakedNft<'info> {
    pub fn burn_staked_nft(&mut self) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;

        let accrued = pending_rewards(&self.config, &self.stake_account, now)?;
        let payout = accrued
            .checked_add(self.config.burn_bonus)
            .ok_or(error!(StakeError::MathOverflow))?;

        let mpl_core_program = self.mpl_core_program.to_account_info();
        let asset = self.asset.to_account_info();
        let collection = self.collection.to_account_info();
        let user = self.user.to_account_info();
        let system_program = self.system_program.to_account_info();
        let stake_info = self.stake_account.to_account_info();

        let stake_seeds: &[&[u8]] = &[
            STAKE_SEED,
            self.stake_account.asset.as_ref(),
            self.config.to_account_info().key.as_ref(),
            &[self.stake_account.bump],
        ];

        // 1. A frozen asset cannot be burned (not even by its BurnDelegate), so the
        //    stake PDA, which holds the FreezeDelegate authority, thaws it first.
        UpdatePluginV1CpiBuilder::new(&mpl_core_program)
            .asset(&asset)
            .collection(Some(&collection))
            .payer(&user)
            .authority(Some(&stake_info))
            .system_program(&system_program)
            .plugin(Plugin::FreezeDelegate(FreezeDelegate { frozen: false }))
            .invoke_signed(&[stake_seeds])?;

        // 2. Burn through the BurnDelegate: the signer is the stake PDA, not the
        //    user, which is what makes this a delegated burn.
        BurnV1CpiBuilder::new(&mpl_core_program)
            .asset(&asset)
            .collection(Some(&collection))
            .payer(&user)
            .authority(Some(&stake_info))
            .system_program(Some(&system_program))
            .invoke_signed(&[stake_seeds])?;

        // 3. The reward: accrued rewards + one-time bonus.
        mint_rewards(
            &self.token_program,
            &self.rewards_mint,
            &self.rewards_ata,
            &self.config,
            payout,
        )?;

        update_total_staked(
            &self.config,
            &collection,
            &user,
            &system_program,
            &mpl_core_program,
            false,
        )?;

        self.user_account.amount_staked = self.user_account.amount_staked.saturating_sub(1);
        Ok(())
    }
}

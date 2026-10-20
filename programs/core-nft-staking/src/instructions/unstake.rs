use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{Mint, Token, TokenAccount},
};
use mpl_core::{
    instructions::{RemovePluginV1CpiBuilder, UpdatePluginV1CpiBuilder},
    types::{FreezeDelegate, Plugin, PluginType},
    ID as MPL_CORE_ID,
};

use crate::{
    constants::*,
    errors::StakeError,
    state::{StakeAccount, StakeConfig, UserAccount},
    utils::{mint_rewards, pending_rewards, update_total_staked},
};

#[derive(Accounts)]
pub struct Unstake<'info> {
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

impl<'info> Unstake<'info> {
    pub fn unstake(&mut self) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        require!(
            now.saturating_sub(self.stake_account.staked_at) >= self.config.freeze_period,
            StakeError::FreezePeriodNotPassed
        );

        // Pay out whatever accrued since the last claim.
        let rewards = pending_rewards(&self.config, &self.stake_account, now)?;
        mint_rewards(
            &self.token_program,
            &self.rewards_mint,
            &self.rewards_ata,
            &self.config,
            rewards,
        )?;

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

        // 1. Thaw. Only the plugin authority (the stake PDA) may unfreeze.
        UpdatePluginV1CpiBuilder::new(&mpl_core_program)
            .asset(&asset)
            .collection(Some(&collection))
            .payer(&user)
            .authority(Some(&stake_info))
            .system_program(&system_program)
            .plugin(Plugin::FreezeDelegate(FreezeDelegate { frozen: false }))
            .invoke_signed(&[stake_seeds])?;

        // 2. Hand the asset back clean: drop both delegates, so the user is again
        //    the only party with any authority over it.
        RemovePluginV1CpiBuilder::new(&mpl_core_program)
            .asset(&asset)
            .collection(Some(&collection))
            .payer(&user)
            .authority(Some(&user))
            .system_program(&system_program)
            .plugin_type(PluginType::FreezeDelegate)
            .invoke()?;

        RemovePluginV1CpiBuilder::new(&mpl_core_program)
            .asset(&asset)
            .collection(Some(&collection))
            .payer(&user)
            .authority(Some(&user))
            .system_program(&system_program)
            .plugin_type(PluginType::BurnDelegate)
            .invoke()?;

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

use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{Mint, Token, TokenAccount},
};

use crate::{
    constants::*,
    errors::StakeError,
    state::{StakeAccount, StakeConfig},
    utils::{mint_rewards, pending_rewards},
};

/// Collect the rewards accrued so far WITHOUT unstaking: the NFT is not touched, it
/// stays staked and frozen. Only the reward checkpoint (`last_claim_at`) moves.
#[derive(Accounts)]
pub struct ClaimRewards<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    /// CHECK: only used to derive the stake PDA; never read or written.
    pub asset: UncheckedAccount<'info>,
    #[account(
        seeds = [CONFIG_SEED, config.collection.as_ref()],
        bump = config.bump,
    )]
    pub config: Account<'info, StakeConfig>,
    #[account(
        mut,
        seeds = [STAKE_SEED, asset.key().as_ref(), config.key().as_ref()],
        bump = stake_account.bump,
        constraint = stake_account.owner == user.key() @ StakeError::NotAssetOwner,
    )]
    pub stake_account: Account<'info, StakeAccount>,
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
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

impl<'info> ClaimRewards<'info> {
    pub fn claim_rewards(&mut self) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let rewards = pending_rewards(&self.config, &self.stake_account, now)?;
        require!(rewards > 0, StakeError::NothingToClaim);

        mint_rewards(
            &self.token_program,
            &self.rewards_mint,
            &self.rewards_ata,
            &self.config,
            rewards,
        )?;

        self.stake_account.last_claim_at = now;
        Ok(())
    }
}

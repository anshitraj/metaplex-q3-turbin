use anchor_lang::prelude::*;

use crate::{
    constants::{CONFIG_SEED, USER_SEED},
    state::{StakeConfig, UserAccount},
};

#[derive(Accounts)]
pub struct InitializeUser<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(
        seeds = [CONFIG_SEED, config.collection.as_ref()],
        bump = config.bump,
    )]
    pub config: Account<'info, StakeConfig>,
    #[account(
        init,
        payer = user,
        seeds = [USER_SEED, config.key().as_ref(), user.key().as_ref()],
        bump,
        space = 8 + UserAccount::INIT_SPACE,
    )]
    pub user_account: Account<'info, UserAccount>,
    pub system_program: Program<'info, System>,
}

impl<'info> InitializeUser<'info> {
    pub fn initialize_user(&mut self, bumps: &InitializeUserBumps) -> Result<()> {
        self.user_account.set_inner(UserAccount {
            amount_staked: 0,
            bump: bumps.user_account,
        });
        Ok(())
    }
}

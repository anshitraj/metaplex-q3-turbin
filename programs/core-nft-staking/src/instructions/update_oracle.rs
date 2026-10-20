use anchor_lang::{prelude::*, system_program};

use crate::{
    constants::*,
    state::{seconds_since_boundary, window_verdict, Oracle, StakeConfig},
};

/// Permissionless crank: anyone can call it. It reads the on-chain clock and
/// brings the Oracle's `Transfer` verdict in line with the 09:00-17:00 UTC window.
///
/// The caller is paid from the vault, but ONLY when the call actually flips the
/// verdict AND lands within `crank_window` seconds after the open/close boundary.
/// So there is no reward for spamming no-op updates, nor for a stale late update.
#[derive(Accounts)]
pub struct UpdateOracle<'info> {
    #[account(mut)]
    pub cranker: Signer<'info>,
    #[account(
        seeds = [CONFIG_SEED, config.collection.as_ref()],
        bump = config.bump,
    )]
    pub config: Account<'info, StakeConfig>,
    #[account(
        mut,
        seeds = [ORACLE_SEED, config.key().as_ref()],
        bump = oracle.bump,
    )]
    pub oracle: Account<'info, Oracle>,
    #[account(
        mut,
        seeds = [VAULT_SEED, config.key().as_ref()],
        bump = oracle.vault_bump,
    )]
    pub vault: SystemAccount<'info>,
    pub system_program: Program<'info, System>,
}

impl<'info> UpdateOracle<'info> {
    pub fn update_oracle(&mut self) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let verdict = window_verdict(now);

        if self.oracle.transfer_result() == Some(verdict) {
            msg!("Oracle already up to date: {:?}", verdict);
            return Ok(());
        }

        self.oracle.validation = Oracle::validation_for(verdict);
        self.oracle.last_updated = now;
        msg!("Oracle transfer verdict is now {:?}", verdict);

        if seconds_since_boundary(now) > self.oracle.crank_window {
            msg!("Updated late, no crank reward");
            return Ok(());
        }

        // The vault is a plain system account, so it has to stay rent exempt: only
        // what sits above the 0-byte rent-exempt minimum is payable.
        let rent_reserve = Rent::get()?.minimum_balance(0);
        let reward = self
            .oracle
            .crank_reward
            .min(self.vault.lamports().saturating_sub(rent_reserve));
        if reward == 0 {
            msg!("Vault is empty, no crank reward");
            return Ok(());
        }

        let config = self.config.key();
        system_program::transfer(
            CpiContext::new_with_signer(
                self.system_program.key(),
                system_program::Transfer {
                    from: self.vault.to_account_info(),
                    to: self.cranker.to_account_info(),
                },
                &[&[VAULT_SEED, config.as_ref(), &[self.oracle.vault_bump]]],
            ),
            reward,
        )?;
        msg!("Paid {} lamports crank reward", reward);

        Ok(())
    }
}

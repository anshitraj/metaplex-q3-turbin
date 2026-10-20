use anchor_lang::prelude::*;
use mpl_core::{instructions::TransferV1CpiBuilder, ID as MPL_CORE_ID};

use crate::{
    constants::*,
    state::{Oracle, StakeConfig},
};

/// Transfers a Core asset of the staking collection. The collection carries an
/// Oracle adapter that can REJECT `Transfer`, so MPL Core needs the Oracle account
/// on the instruction: it is passed as a remaining account of the transfer CPI.
/// Outside the 09:00-17:00 UTC window the Oracle says `Rejected` and MPL Core aborts
/// the whole transaction.
#[derive(Accounts)]
pub struct TransferNft<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    /// CHECK: any wallet may receive the asset.
    pub new_owner: UncheckedAccount<'info>,
    /// CHECK: MPL Core validates ownership, freezes and the oracle.
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
        seeds = [ORACLE_SEED, config.key().as_ref()],
        bump = oracle.bump,
    )]
    pub oracle: Account<'info, Oracle>,
    /// CHECK: verified against the canonical MPL Core program id.
    #[account(address = MPL_CORE_ID)]
    pub mpl_core_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

impl<'info> TransferNft<'info> {
    pub fn transfer_nft(&mut self) -> Result<()> {
        let mpl_core_program = self.mpl_core_program.to_account_info();
        let asset = self.asset.to_account_info();
        let collection = self.collection.to_account_info();
        let owner = self.owner.to_account_info();
        let new_owner = self.new_owner.to_account_info();
        let system_program = self.system_program.to_account_info();
        let oracle = self.oracle.to_account_info();

        TransferV1CpiBuilder::new(&mpl_core_program)
            .asset(&asset)
            .collection(Some(&collection))
            .payer(&owner)
            .authority(Some(&owner))
            .new_owner(&new_owner)
            .system_program(Some(&system_program))
            .add_remaining_account(&oracle, false, false)
            .invoke()?;

        Ok(())
    }
}

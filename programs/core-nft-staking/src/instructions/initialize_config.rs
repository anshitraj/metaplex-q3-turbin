use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token};
use mpl_core::{
    instructions::AddCollectionPluginV1CpiBuilder,
    types::{Attribute, Attributes, Plugin, PluginAuthority},
    ID as MPL_CORE_ID,
};

use crate::{constants::*, errors::StakeError, state::StakeConfig, utils::load_collection};

#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    /// Must be the update authority of the collection.
    #[account(mut)]
    pub admin: Signer<'info>,
    /// CHECK: validated in the handler (owned by MPL Core, a real collection,
    /// `update_authority == admin`).
    #[account(mut)]
    pub collection: UncheckedAccount<'info>,
    #[account(
        init,
        payer = admin,
        seeds = [CONFIG_SEED, collection.key().as_ref()],
        bump,
        space = 8 + StakeConfig::INIT_SPACE,
    )]
    pub config: Account<'info, StakeConfig>,
    #[account(
        init,
        payer = admin,
        seeds = [REWARDS_SEED, config.key().as_ref()],
        bump,
        mint::decimals = REWARDS_DECIMALS,
        mint::authority = config,
    )]
    pub rewards_mint: Account<'info, Mint>,
    /// CHECK: verified against the canonical MPL Core program id.
    #[account(address = MPL_CORE_ID)]
    pub mpl_core_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}

impl<'info> InitializeConfig<'info> {
    pub fn initialize_config(
        &mut self,
        reward_per_second: u64,
        burn_bonus: u64,
        freeze_period: i64,
        max_stake: u8,
        bumps: &InitializeConfigBumps,
    ) -> Result<()> {
        let collection = load_collection(&self.collection.to_account_info())?;
        require_keys_eq!(
            collection.update_authority,
            self.admin.key(),
            StakeError::NotCollectionAuthority
        );

        self.config.set_inner(StakeConfig {
            admin: self.admin.key(),
            collection: self.collection.key(),
            reward_per_second,
            burn_bonus,
            freeze_period,
            max_stake,
            rewards_bump: bumps.rewards_mint,
            bump: bumps.config,
        });

        // Collection-level staking stats: an `Attributes` plugin on the Collection
        // account holding `total_staked`. The plugin authority is the config PDA, so
        // only this program can move the counter (not even the collection admin).
        let mpl_core_program = self.mpl_core_program.to_account_info();
        let collection_info = self.collection.to_account_info();
        let admin = self.admin.to_account_info();
        let system_program = self.system_program.to_account_info();

        AddCollectionPluginV1CpiBuilder::new(&mpl_core_program)
            .collection(&collection_info)
            .payer(&admin)
            .authority(Some(&admin))
            .system_program(&system_program)
            .plugin(Plugin::Attributes(Attributes {
                attribute_list: vec![Attribute {
                    key: TOTAL_STAKED_KEY.to_string(),
                    value: "0".to_string(),
                }],
            }))
            .init_authority(PluginAuthority::Address {
                address: self.config.key(),
            })
            .invoke()?;

        Ok(())
    }
}

use anchor_lang::prelude::*;
use mpl_core::{
    instructions::AddPluginV1CpiBuilder,
    types::{BurnDelegate, FreezeDelegate, Plugin, PluginAuthority, UpdateAuthority},
    ID as MPL_CORE_ID,
};

use crate::{
    constants::*,
    errors::StakeError,
    state::{StakeAccount, StakeConfig, UserAccount},
    utils::{load_asset, update_total_staked},
};

#[derive(Accounts)]
pub struct Stake<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    /// CHECK: validated in the handler (owned by MPL Core, `AssetV1`, owned by
    /// `user`, member of the staking collection).
    #[account(mut)]
    pub asset: UncheckedAccount<'info>,
    /// CHECK: the config PDA is derived from this key, so it is the staking
    /// collection. MPL Core validates the account itself.
    #[account(mut)]
    pub collection: UncheckedAccount<'info>,
    #[account(
        seeds = [CONFIG_SEED, collection.key().as_ref()],
        bump = config.bump,
    )]
    pub config: Account<'info, StakeConfig>,
    #[account(
        init,
        payer = user,
        space = 8 + StakeAccount::INIT_SPACE,
        seeds = [STAKE_SEED, asset.key().as_ref(), config.key().as_ref()],
        bump,
    )]
    pub stake_account: Account<'info, StakeAccount>,
    #[account(
        mut,
        seeds = [USER_SEED, config.key().as_ref(), user.key().as_ref()],
        bump = user_account.bump,
    )]
    pub user_account: Account<'info, UserAccount>,
    /// CHECK: verified against the canonical MPL Core program id.
    #[account(address = MPL_CORE_ID)]
    pub mpl_core_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

impl<'info> Stake<'info> {
    pub fn stake(&mut self, bumps: &StakeBumps) -> Result<()> {
        let asset = load_asset(&self.asset.to_account_info())?;
        require_keys_eq!(asset.owner, self.user.key(), StakeError::NotAssetOwner);
        require!(
            matches!(asset.update_authority, UpdateAuthority::Collection(c) if c == self.collection.key()),
            StakeError::WrongCollection
        );
        require!(
            self.user_account.amount_staked < self.config.max_stake,
            StakeError::MaxStakeReached
        );

        let now = Clock::get()?.unix_timestamp;
        self.stake_account.set_inner(StakeAccount {
            owner: self.user.key(),
            asset: self.asset.key(),
            staked_at: now,
            last_claim_at: now,
            bump: bumps.stake_account,
        });

        let mpl_core_program = self.mpl_core_program.to_account_info();
        let asset_info = self.asset.to_account_info();
        let collection_info = self.collection.to_account_info();
        let user = self.user.to_account_info();
        let system_program = self.system_program.to_account_info();
        let stake_key = self.stake_account.key();

        // The user (owner) installs two owner-managed plugins whose *authority* is
        // the stake PDA, so only this program can thaw or burn the asset from now on:
        //  - FreezeDelegate: the asset is frozen for as long as it is staked.
        //  - BurnDelegate:   lets `burn_staked_nft` burn it on the user's behalf.
        AddPluginV1CpiBuilder::new(&mpl_core_program)
            .asset(&asset_info)
            .collection(Some(&collection_info))
            .payer(&user)
            .authority(Some(&user))
            .system_program(&system_program)
            .plugin(Plugin::FreezeDelegate(FreezeDelegate { frozen: true }))
            .init_authority(PluginAuthority::Address { address: stake_key })
            .invoke()?;

        AddPluginV1CpiBuilder::new(&mpl_core_program)
            .asset(&asset_info)
            .collection(Some(&collection_info))
            .payer(&user)
            .authority(Some(&user))
            .system_program(&system_program)
            .plugin(Plugin::BurnDelegate(BurnDelegate {}))
            .init_authority(PluginAuthority::Address { address: stake_key })
            .invoke()?;

        update_total_staked(
            &self.config,
            &collection_info,
            &user,
            &system_program,
            &mpl_core_program,
            true,
        )?;

        self.user_account.amount_staked = self
            .user_account
            .amount_staked
            .checked_add(1)
            .ok_or(error!(StakeError::MathOverflow))?;

        Ok(())
    }
}

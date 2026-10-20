use anchor_lang::{prelude::*, system_program};
use mpl_core::{
    instructions::AddCollectionExternalPluginAdapterV1CpiBuilder,
    types::{
        ExternalCheckResult, ExternalPluginAdapterInitInfo, HookableLifecycleEvent, OracleInitInfo,
        ValidationResultsOffset,
    },
    ID as MPL_CORE_ID,
};

use crate::{
    constants::*,
    errors::StakeError,
    state::{window_verdict, Oracle, StakeConfig},
};

/// Creates the Oracle PDA + crank vault and attaches an Oracle external plugin
/// adapter to the collection that can REJECT `Transfer` events.
#[derive(Accounts)]
pub struct InitializeOracle<'info> {
    #[account(mut, address = config.admin @ StakeError::NotCollectionAuthority)]
    pub admin: Signer<'info>,
    #[account(
        seeds = [CONFIG_SEED, collection.key().as_ref()],
        bump = config.bump,
    )]
    pub config: Account<'info, StakeConfig>,
    /// CHECK: the config PDA is derived from this key. MPL Core additionally
    /// requires `admin` to be the collection's update authority.
    #[account(mut)]
    pub collection: UncheckedAccount<'info>,
    #[account(
        init,
        payer = admin,
        space = 8 + Oracle::INIT_SPACE,
        seeds = [ORACLE_SEED, config.key().as_ref()],
        bump,
    )]
    pub oracle: Account<'info, Oracle>,
    /// Holds the lamports that reward crankers. A system-owned PDA, so the program
    /// can pay out of it with a signed `system_program::transfer`.
    #[account(
        mut,
        seeds = [VAULT_SEED, config.key().as_ref()],
        bump,
    )]
    pub vault: SystemAccount<'info>,
    /// CHECK: verified against the canonical MPL Core program id.
    #[account(address = MPL_CORE_ID)]
    pub mpl_core_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

impl<'info> InitializeOracle<'info> {
    pub fn initialize_oracle(
        &mut self,
        crank_reward: u64,
        crank_window: i64,
        vault_funding: u64,
        bumps: &InitializeOracleBumps,
    ) -> Result<()> {
        require!(
            (1..=MAX_CRANK_WINDOW_SECONDS).contains(&crank_window),
            StakeError::InvalidCrankWindow
        );

        let now = Clock::get()?.unix_timestamp;
        self.oracle.set_inner(Oracle {
            validation: Oracle::validation_for(window_verdict(now)),
            crank_reward,
            crank_window,
            last_updated: now,
            bump: bumps.oracle,
            vault_bump: bumps.vault,
        });

        if vault_funding > 0 {
            system_program::transfer(
                CpiContext::new(
                    self.system_program.key(),
                    system_program::Transfer {
                        from: self.admin.to_account_info(),
                        to: self.vault.to_account_info(),
                    },
                ),
                vault_funding,
            )?;
        }

        let mpl_core_program = self.mpl_core_program.to_account_info();
        let collection = self.collection.to_account_info();
        let admin = self.admin.to_account_info();
        let system_program = self.system_program.to_account_info();

        // The adapter only listens to `Transfer`, and can only REJECT it (an Oracle
        // adapter may not approve). `Anchor` offset: skip the 8 byte discriminator
        // of our Oracle account, `OracleValidation` is the first field after it.
        AddCollectionExternalPluginAdapterV1CpiBuilder::new(&mpl_core_program)
            .collection(&collection)
            .payer(&admin)
            .authority(Some(&admin))
            .system_program(&system_program)
            .init_info(ExternalPluginAdapterInitInfo::Oracle(OracleInitInfo {
                base_address: self.oracle.key(),
                init_plugin_authority: None,
                lifecycle_checks: vec![(
                    HookableLifecycleEvent::Transfer,
                    ExternalCheckResult {
                        flags: CHECK_CAN_REJECT,
                    },
                )],
                base_address_config: None,
                results_offset: Some(ValidationResultsOffset::Anchor),
            }))
            .invoke()?;

        Ok(())
    }
}

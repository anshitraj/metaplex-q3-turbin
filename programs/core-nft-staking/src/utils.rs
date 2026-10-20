use anchor_lang::{prelude::*, Key};
use anchor_spl::token::{mint_to, Mint, MintTo, Token, TokenAccount};
use mpl_core::{
    accounts::{BaseAssetV1, BaseCollectionV1},
    fetch_collection_plugin,
    instructions::UpdateCollectionPluginV1CpiBuilder,
    types::{Attributes, Key as CoreKey, Plugin, PluginType},
    ID as MPL_CORE_ID,
};

use crate::{
    constants::{CONFIG_SEED, TOTAL_STAKED_KEY},
    errors::StakeError,
    state::{StakeAccount, StakeConfig},
};

/// Reads a Core asset, making sure the account really is one (owned by MPL Core
/// and carrying the `AssetV1` key) before anybody trusts its `owner` field.
pub fn load_asset(info: &AccountInfo) -> Result<BaseAssetV1> {
    require_keys_eq!(*info.owner, MPL_CORE_ID, StakeError::InvalidAsset);
    let data = info.try_borrow_data()?;
    let asset = BaseAssetV1::from_bytes(&data).map_err(|_| error!(StakeError::InvalidAsset))?;
    require!(asset.key == CoreKey::AssetV1, StakeError::InvalidAsset);
    Ok(asset)
}

/// Reads a Core collection, with the same guarantees as [`load_asset`].
pub fn load_collection(info: &AccountInfo) -> Result<BaseCollectionV1> {
    require_keys_eq!(*info.owner, MPL_CORE_ID, StakeError::InvalidCollection);
    let data = info.try_borrow_data()?;
    let collection =
        BaseCollectionV1::from_bytes(&data).map_err(|_| error!(StakeError::InvalidCollection))?;
    require!(
        collection.key == CoreKey::CollectionV1,
        StakeError::InvalidCollection
    );
    Ok(collection)
}

/// Increments (`+1`) or decrements (`-1`) the `total_staked` Attribute that lives
/// on the Collection account.
///
/// The Collection's `Attributes` plugin is controlled by the config PDA (it was
/// created with `PluginAuthority::Address(config)` in `initialize_config`), so the
/// program signs the update with the config seeds.
pub fn update_total_staked<'info>(
    config: &Account<'info, StakeConfig>,
    collection: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    mpl_core_program: &AccountInfo<'info>,
    increment: bool,
) -> Result<()> {
    let (_, mut attributes, _) =
        fetch_collection_plugin::<Attributes>(collection, PluginType::Attributes)
            .map_err(|_| error!(StakeError::InvalidStakeCounter))?;

    let entry = attributes
        .attribute_list
        .iter_mut()
        .find(|attribute| attribute.key == TOTAL_STAKED_KEY)
        .ok_or(error!(StakeError::InvalidStakeCounter))?;

    let current: u64 = entry
        .value
        .parse()
        .map_err(|_| error!(StakeError::InvalidStakeCounter))?;
    let updated = if increment {
        current.checked_add(1)
    } else {
        current.checked_sub(1)
    }
    .ok_or(error!(StakeError::MathOverflow))?;
    entry.value = updated.to_string();

    let config_info = config.to_account_info();
    UpdateCollectionPluginV1CpiBuilder::new(mpl_core_program)
        .collection(collection)
        .payer(payer)
        .authority(Some(&config_info))
        .system_program(system_program)
        .plugin(Plugin::Attributes(attributes))
        .invoke_signed(&[&[CONFIG_SEED, config.collection.as_ref(), &[config.bump]]])?;

    Ok(())
}

/// Rewards accrued by one staked NFT since it was staked / last claimed.
pub fn pending_rewards(config: &StakeConfig, stake: &StakeAccount, now: i64) -> Result<u64> {
    let elapsed = now.saturating_sub(stake.last_claim_at).max(0) as u64;
    elapsed
        .checked_mul(config.reward_per_second)
        .ok_or(error!(StakeError::MathOverflow))
}

/// Mints `amount` reward tokens into `rewards_ata`; the config PDA is the mint
/// authority.
pub fn mint_rewards<'info>(
    token_program: &Program<'info, Token>,
    rewards_mint: &Account<'info, Mint>,
    rewards_ata: &Account<'info, TokenAccount>,
    config: &Account<'info, StakeConfig>,
    amount: u64,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }

    let signer_seeds: &[&[&[u8]]] = &[&[CONFIG_SEED, config.collection.as_ref(), &[config.bump]]];
    mint_to(
        CpiContext::new_with_signer(
            token_program.key(),
            MintTo {
                mint: rewards_mint.to_account_info(),
                to: rewards_ata.to_account_info(),
                authority: config.to_account_info(),
            },
            signer_seeds,
        ),
        amount,
    )
}

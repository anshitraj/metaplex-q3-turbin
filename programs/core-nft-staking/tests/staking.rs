//! Task 1: NFT staking on MPL Core plugins.
//!
//!  1. `claim_rewards`: collect rewards without unstaking (NFT stays staked + frozen)
//!  2. `burn_staked_nft`: burn-to-earn through the BurnDelegate
//!  3. `total_staked`: Attribute on the Collection account, +1 on stake, -1 on unstake/burn

#![allow(clippy::result_large_err)]

mod common;

use common::*;
use core_nft_staking::errors::StakeError;
use mpl_core::{
    errors::MplCoreError,
    instructions::AddPluginV1Builder,
    types::{Attributes, BurnDelegate, FreezeDelegate, Plugin, PluginAuthorityPair, PluginType},
};
use solana_keypair::Keypair;
use solana_signer::Signer;

fn key(kp: &Keypair) -> anchor_lang::prelude::Pubkey {
    pk(&kp.pubkey())
}

/// A funded, initialised user that owns one fresh asset.
fn user_with_asset(env: &mut Env) -> (Keypair, anchor_lang::prelude::Pubkey) {
    let user = env.new_user();
    let asset = env.mint_asset(&key(&user));
    (user, asset)
}

// ------------------------------------------------------------------------------
// pool setup
// ------------------------------------------------------------------------------

#[test]
fn initialize_config_puts_total_staked_attribute_on_the_collection() {
    let env = Env::new();

    // The counter starts at zero...
    assert_eq!(env.total_staked(), 0);

    // ...and its plugin authority is the config PDA, not the collection admin, so
    // only the program can move it.
    let plugin = env
        .collection_account()
        .plugin_list
        .attributes
        .expect("attributes plugin");
    assert_eq!(plugin.base.authority.address, Some(env.config));
}

#[test]
fn initialize_config_rejects_anyone_but_the_collection_authority() {
    let mut env = Env::bare();
    let impostor = Keypair::new();
    env.fund(&key(&impostor), 10_000_000_000);

    let ix = env.ix_initialize_config(&key(&impostor), &env.collection_key());
    let res = env.send(&[ix], &[&impostor]);

    assert!(failed_with(
        &res,
        stake_error(StakeError::NotCollectionAuthority)
    ));
}

#[test]
fn initialize_config_fails_if_the_collection_already_has_an_attributes_plugin() {
    // The pool needs to own the collection's Attributes plugin, so a collection that
    // already carries one cannot be handed over.
    let mut env = Env::bare_with_collection_plugins(vec![PluginAuthorityPair {
        plugin: Plugin::Attributes(Attributes {
            attribute_list: vec![],
        }),
        authority: None,
    }]);

    let ix = env.ix_initialize_config(&env.admin_key(), &env.collection_key());
    let admin = Keypair::try_from(env.admin.to_bytes().as_slice()).unwrap();
    let res = env.send(&[ix], &[&admin]);

    assert!(
        failed_with(&res, core_error(MplCoreError::PluginAlreadyExists)),
        "{:?}",
        res.err()
    );
}

// ------------------------------------------------------------------------------
// stake
// ------------------------------------------------------------------------------

#[test]
fn stake_freezes_the_nft_and_increments_total_staked() {
    let mut env = Env::new();
    let (user, asset) = user_with_asset(&mut env);

    env.send(&[env.ix_stake(&key(&user), &asset)], &[&user])
        .expect("stake");

    // Frozen, and still sitting in the user's wallet (no escrow transfer).
    assert_eq!(env.frozen(&asset), Some(true));
    assert_eq!(env.asset_owner(&asset), key(&user));
    assert!(env.has_burn_delegate(&asset));

    // Both delegates are controlled by the stake PDA, not by the user.
    let plugins = env.asset(&asset).unwrap().plugin_list;
    let stake_pda = env.stake_pda(&asset);
    assert_eq!(
        plugins.freeze_delegate.unwrap().base.authority.address,
        Some(stake_pda)
    );
    assert_eq!(
        plugins.burn_delegate.unwrap().base.authority.address,
        Some(stake_pda)
    );

    assert_eq!(env.total_staked(), 1);
    assert_eq!(env.amount_staked(&key(&user)), 1);
}

#[test]
fn a_staked_nft_cannot_be_moved_thawed_or_stripped_by_its_owner() {
    let mut env = Env::new();
    let (user, asset) = user_with_asset(&mut env);
    let user_key = key(&user);
    env.send(&[env.ix_stake(&user_key, &asset)], &[&user])
        .unwrap();
    let friend = key(&Keypair::new());

    // The freeze rejects transfer / removal / burn outright...
    let rejected = core_error(MplCoreError::InvalidAuthority);
    let res = env.send(&[env.core_transfer(&user_key, &asset, &friend)], &[&user]);
    assert!(failed_with(&res, rejected), "transfer: {:?}", res.err());
    let res = env.send(
        &[env.core_remove_plugin(&user_key, &asset, PluginType::FreezeDelegate)],
        &[&user],
    );
    assert!(
        failed_with(&res, rejected),
        "remove freeze: {:?}",
        res.err()
    );
    let res = env.send(
        &[env.core_remove_plugin(&user_key, &asset, PluginType::BurnDelegate)],
        &[&user],
    );
    assert!(
        failed_with(&res, rejected),
        "remove burn delegate: {:?}",
        res.err()
    );
    let res = env.send(&[env.core_burn(&user_key, &asset)], &[&user]);
    assert!(failed_with(&res, rejected), "burn: {:?}", res.err());

    // ...and thawing needs the plugin authority, which is the stake PDA, not the owner.
    let res = env.send(&[env.core_set_frozen(&user_key, &asset, false)], &[&user]);
    assert!(
        failed_with(&res, core_error(MplCoreError::NoApprovals)),
        "thaw: {:?}",
        res.err()
    );

    assert_eq!(env.frozen(&asset), Some(true));
    assert_eq!(env.asset_owner(&asset), user_key);
}

/// Control for the test above: the very same raw MPL Core instructions DO succeed
/// when the owner holds the plugin authority, so the rejections above are caused by
/// the stake PDA owning the delegates, not by malformed instructions.
#[test]
fn control_the_same_core_instructions_succeed_when_the_owner_is_the_authority() {
    let mut env = Env::new();
    let user = env.new_user();
    let user_key = key(&user);
    let asset = env.mint_asset(&user_key);
    let friend = key(&Keypair::new());
    let collection = env.collection_key();

    let add = |plugin: Plugin| {
        AddPluginV1Builder::new()
            .asset(asset)
            .collection(Some(collection))
            .payer(user_key)
            .authority(Some(user_key))
            .system_program(anchor_lang::system_program::ID)
            .plugin(plugin)
            .instruction()
    };
    env.send(
        &[add(Plugin::FreezeDelegate(FreezeDelegate {
            frozen: false,
        }))],
        &[&user],
    )
    .unwrap();
    env.send(&[add(Plugin::BurnDelegate(BurnDelegate {}))], &[&user])
        .unwrap();

    // frozen by the owner-controlled delegate: same instruction, same rejection
    env.send(&[env.core_set_frozen(&user_key, &asset, true)], &[&user])
        .unwrap();
    let res = env.send(&[env.core_transfer(&user_key, &asset, &friend)], &[&user]);
    assert!(failed_with(
        &res,
        core_error(MplCoreError::InvalidAuthority)
    ));

    // thawed: every one of the "attack" instructions is valid
    env.send(&[env.core_set_frozen(&user_key, &asset, false)], &[&user])
        .unwrap();
    env.send(
        &[env.core_remove_plugin(&user_key, &asset, PluginType::BurnDelegate)],
        &[&user],
    )
    .unwrap();
    env.send(
        &[env.core_remove_plugin(&user_key, &asset, PluginType::FreezeDelegate)],
        &[&user],
    )
    .unwrap();
    env.send(&[env.core_transfer(&user_key, &asset, &friend)], &[&user])
        .unwrap();
    assert_eq!(env.asset_owner(&asset), friend);
    let friend_kp = Keypair::new();
    let asset2 = env.mint_asset(&key(&friend_kp));
    env.fund(&key(&friend_kp), 1_000_000_000);
    env.send(&[env.core_burn(&key(&friend_kp), &asset2)], &[&friend_kp])
        .unwrap();
    assert!(env.asset(&asset2).is_none());
}

#[test]
fn stake_rejects_assets_the_signer_does_not_own() {
    let mut env = Env::new();
    let (_owner, asset) = user_with_asset(&mut env);
    let other = env.new_user();

    let res = env.send(&[env.ix_stake(&key(&other), &asset)], &[&other]);

    assert!(failed_with(&res, stake_error(StakeError::NotAssetOwner)));
    assert_eq!(env.total_staked(), 0);
}

#[test]
fn stake_rejects_assets_from_another_collection() {
    let mut env = Env::new();
    let user = env.new_user();
    let stray = env.mint_standalone_asset(&key(&user));

    let res = env.send(&[env.ix_stake(&key(&user), &stray)], &[&user]);

    assert!(failed_with(&res, stake_error(StakeError::WrongCollection)));
    assert_eq!(env.total_staked(), 0);
}

#[test]
fn stake_rejects_a_fake_asset_account() {
    let mut env = Env::new();
    let user = env.new_user();
    // A system account that is not owned by MPL Core
    let fake = Keypair::new();
    env.fund(&key(&fake), 1_000_000);

    let res = env.send(&[env.ix_stake(&key(&user), &key(&fake))], &[&user]);

    assert!(failed_with(&res, stake_error(StakeError::InvalidAsset)));
}

#[test]
fn stake_respects_max_stake() {
    let mut env = Env::new();
    let user = env.new_user();
    let user_key = key(&user);

    for _ in 0..MAX_STAKE {
        let asset = env.mint_asset(&user_key);
        env.send(&[env.ix_stake(&user_key, &asset)], &[&user])
            .unwrap();
    }
    let one_too_many = env.mint_asset(&user_key);
    let res = env.send(&[env.ix_stake(&user_key, &one_too_many)], &[&user]);

    assert!(failed_with(&res, stake_error(StakeError::MaxStakeReached)));
    assert_eq!(env.total_staked(), MAX_STAKE as u64);
    assert_eq!(env.frozen(&one_too_many), None);
}

#[test]
fn an_nft_cannot_be_staked_twice() {
    let mut env = Env::new();
    let (user, asset) = user_with_asset(&mut env);
    let user_key = key(&user);
    env.send(&[env.ix_stake(&user_key, &asset)], &[&user])
        .unwrap();

    let res = env.send(&[env.ix_stake(&user_key, &asset)], &[&user]);

    // the stake PDA already exists, the system program refuses to create it twice
    assert!(failed_with_log(&res, "already in use"), "{:?}", res.err());
    assert_eq!(env.total_staked(), 1);
}

// ------------------------------------------------------------------------------
// claim_rewards: collect without unstaking
// ------------------------------------------------------------------------------

#[test]
fn claim_rewards_mints_to_the_ata_and_keeps_the_nft_staked_and_frozen() {
    let mut env = Env::new();
    let (user, asset) = user_with_asset(&mut env);
    let user_key = key(&user);
    env.send(&[env.ix_stake(&user_key, &asset)], &[&user])
        .unwrap();

    // nothing accrued yet
    let res = env.send(&[env.ix_claim_rewards(&user_key, &asset)], &[&user]);
    assert!(failed_with(&res, stake_error(StakeError::NothingToClaim)));

    env.advance_time(100);
    env.send(&[env.ix_claim_rewards(&user_key, &asset)], &[&user])
        .expect("claim");

    // 100s * REWARD_PER_SECOND, minted into the (just created) ATA
    assert_eq!(env.rewards_balance(&user_key), 100 * REWARD_PER_SECOND);

    // the NFT did not move and is still frozen, still staked
    assert_eq!(env.frozen(&asset), Some(true));
    assert_eq!(env.asset_owner(&asset), user_key);
    assert!(env.has_burn_delegate(&asset));
    assert!(env.account_exists(&env.stake_pda(&asset)));
    assert_eq!(env.total_staked(), 1);
    assert_eq!(env.amount_staked(&user_key), 1);
}

#[test]
fn claim_rewards_only_pays_for_the_time_since_the_last_claim() {
    let mut env = Env::new();
    let (user, asset) = user_with_asset(&mut env);
    let user_key = key(&user);
    env.send(&[env.ix_stake(&user_key, &asset)], &[&user])
        .unwrap();

    env.advance_time(100);
    env.send(&[env.ix_claim_rewards(&user_key, &asset)], &[&user])
        .unwrap();
    env.advance_time(40);
    env.send(&[env.ix_claim_rewards(&user_key, &asset)], &[&user])
        .unwrap();

    // 100 + 40, never 100 + 140
    assert_eq!(env.rewards_balance(&user_key), 140 * REWARD_PER_SECOND);
}

#[test]
fn claim_rewards_is_owner_only() {
    let mut env = Env::new();
    let (user, asset) = user_with_asset(&mut env);
    env.send(&[env.ix_stake(&key(&user), &asset)], &[&user])
        .unwrap();
    env.advance_time(100);

    let thief = env.new_user();
    let res = env.send(&[env.ix_claim_rewards(&key(&thief), &asset)], &[&thief]);

    assert!(failed_with(&res, stake_error(StakeError::NotAssetOwner)));
    assert_eq!(env.rewards_balance(&key(&thief)), 0);
}

// ------------------------------------------------------------------------------
// unstake
// ------------------------------------------------------------------------------

#[test]
fn unstake_is_blocked_until_the_freeze_period_has_passed() {
    let mut env = Env::new();
    let (user, asset) = user_with_asset(&mut env);
    let user_key = key(&user);
    env.send(&[env.ix_stake(&user_key, &asset)], &[&user])
        .unwrap();

    env.advance_time(FREEZE_PERIOD - 1);
    let res = env.send(&[env.ix_unstake(&user_key, &asset)], &[&user]);

    assert!(failed_with(
        &res,
        stake_error(StakeError::FreezePeriodNotPassed)
    ));
    assert_eq!(env.frozen(&asset), Some(true));
    assert_eq!(env.total_staked(), 1);
}

#[test]
fn unstake_thaws_pays_out_and_hands_the_nft_back_clean() {
    let mut env = Env::new();
    let (user, asset) = user_with_asset(&mut env);
    let user_key = key(&user);
    env.send(&[env.ix_stake(&user_key, &asset)], &[&user])
        .unwrap();

    env.advance_time(FREEZE_PERIOD);
    env.send(&[env.ix_unstake(&user_key, &asset)], &[&user])
        .expect("unstake");

    // accrued rewards were paid on the way out
    assert_eq!(
        env.rewards_balance(&user_key),
        FREEZE_PERIOD as u64 * REWARD_PER_SECOND
    );

    // delegates gone: the NFT is a plain, transferable asset again
    assert_eq!(env.frozen(&asset), None);
    assert!(!env.has_burn_delegate(&asset));
    assert_eq!(env.asset_owner(&asset), user_key);

    // bookkeeping unwound, stake record closed
    assert_eq!(env.total_staked(), 0);
    assert_eq!(env.amount_staked(&user_key), 0);
    assert!(!env.account_exists(&env.stake_pda(&asset)));

    // and it really can move now
    let friend = Keypair::new();
    env.send(
        &[env.core_transfer(&user_key, &asset, &key(&friend))],
        &[&user],
    )
    .expect("transfer after unstake");
    assert_eq!(env.asset_owner(&asset), key(&friend));
}

#[test]
fn unstake_after_a_claim_pays_only_the_remainder() {
    let mut env = Env::new();
    let (user, asset) = user_with_asset(&mut env);
    let user_key = key(&user);
    env.send(&[env.ix_stake(&user_key, &asset)], &[&user])
        .unwrap();

    env.advance_time(1_000);
    env.send(&[env.ix_claim_rewards(&user_key, &asset)], &[&user])
        .unwrap();
    env.advance_time(FREEZE_PERIOD);
    env.send(&[env.ix_unstake(&user_key, &asset)], &[&user])
        .unwrap();

    assert_eq!(
        env.rewards_balance(&user_key),
        (1_000 + FREEZE_PERIOD as u64) * REWARD_PER_SECOND
    );
}

#[test]
fn an_unstaked_nft_can_be_staked_again() {
    let mut env = Env::new();
    let (user, asset) = user_with_asset(&mut env);
    let user_key = key(&user);

    env.send(&[env.ix_stake(&user_key, &asset)], &[&user])
        .unwrap();
    env.advance_time(FREEZE_PERIOD);
    env.send(&[env.ix_unstake(&user_key, &asset)], &[&user])
        .unwrap();
    env.send(&[env.ix_stake(&user_key, &asset)], &[&user])
        .expect("restake");

    assert_eq!(env.frozen(&asset), Some(true));
    assert_eq!(env.total_staked(), 1);
}

#[test]
fn unstake_is_owner_only() {
    let mut env = Env::new();
    let (user, asset) = user_with_asset(&mut env);
    env.send(&[env.ix_stake(&key(&user), &asset)], &[&user])
        .unwrap();
    env.advance_time(FREEZE_PERIOD);

    let thief = env.new_user();
    let res = env.send(&[env.ix_unstake(&key(&thief), &asset)], &[&thief]);

    assert!(failed_with(&res, stake_error(StakeError::NotAssetOwner)));
    assert_eq!(env.frozen(&asset), Some(true));
}

// ------------------------------------------------------------------------------
// burn_staked_nft: burn-to-earn via BurnDelegate
// ------------------------------------------------------------------------------

#[test]
fn burn_staked_nft_destroys_the_asset_and_mints_rewards_plus_bonus() {
    let mut env = Env::new();
    let (user, asset) = user_with_asset(&mut env);
    let user_key = key(&user);
    env.send(&[env.ix_stake(&user_key, &asset)], &[&user])
        .unwrap();
    assert!(env.has_burn_delegate(&asset));

    env.advance_time(100);
    env.send(&[env.ix_burn_staked_nft(&user_key, &asset)], &[&user])
        .expect("burn staked nft");

    // accrued 100s + the one-time bonus
    assert_eq!(
        env.rewards_balance(&user_key),
        100 * REWARD_PER_SECOND + BURN_BONUS
    );

    // asset is gone for good
    assert!(env.asset(&asset).is_none(), "asset account must be closed");

    // bookkeeping unwound
    assert_eq!(env.total_staked(), 0);
    assert_eq!(env.amount_staked(&user_key), 0);
    assert!(!env.account_exists(&env.stake_pda(&asset)));
}

#[test]
fn burn_staked_nft_does_not_wait_for_the_freeze_period() {
    let mut env = Env::new();
    let (user, asset) = user_with_asset(&mut env);
    let user_key = key(&user);
    env.send(&[env.ix_stake(&user_key, &asset)], &[&user])
        .unwrap();

    // same slot as the stake: unstake would fail, burning is the user's choice
    env.send(&[env.ix_burn_staked_nft(&user_key, &asset)], &[&user])
        .expect("burn right away");

    assert_eq!(env.rewards_balance(&user_key), BURN_BONUS);
    assert!(env.asset(&asset).is_none());
}

#[test]
fn burn_staked_nft_is_owner_only() {
    let mut env = Env::new();
    let (user, asset) = user_with_asset(&mut env);
    env.send(&[env.ix_stake(&key(&user), &asset)], &[&user])
        .unwrap();

    let thief = env.new_user();
    let res = env.send(&[env.ix_burn_staked_nft(&key(&thief), &asset)], &[&thief]);

    assert!(failed_with(&res, stake_error(StakeError::NotAssetOwner)));
    assert!(env.asset(&asset).is_some());
    assert_eq!(env.rewards_balance(&key(&thief)), 0);
}

#[test]
fn an_unstaked_nft_has_no_burn_delegate_to_abuse() {
    let mut env = Env::new();
    let (user, asset) = user_with_asset(&mut env);
    let user_key = key(&user);
    env.send(&[env.ix_stake(&user_key, &asset)], &[&user])
        .unwrap();
    env.advance_time(FREEZE_PERIOD);
    env.send(&[env.ix_unstake(&user_key, &asset)], &[&user])
        .unwrap();

    // the stake record is closed, so burn-to-earn has nothing to work with
    let res = env.send(&[env.ix_burn_staked_nft(&user_key, &asset)], &[&user]);
    assert!(
        failed_with(&res, ANCHOR_ACCOUNT_NOT_INITIALIZED),
        "{:?}",
        res.err()
    );
    assert!(env.asset(&asset).is_some());
}

// ------------------------------------------------------------------------------
// collection-level stats
// ------------------------------------------------------------------------------

#[test]
fn total_staked_tracks_every_path_across_users() {
    let mut env = Env::new();
    let alice = env.new_user();
    let bob = env.new_user();
    let (a, b) = (key(&alice), key(&bob));

    let a1 = env.mint_asset(&a);
    let a2 = env.mint_asset(&a);
    let b1 = env.mint_asset(&b);

    env.send(&[env.ix_stake(&a, &a1)], &[&alice]).unwrap();
    assert_eq!(env.total_staked(), 1);
    env.send(&[env.ix_stake(&a, &a2)], &[&alice]).unwrap();
    env.send(&[env.ix_stake(&b, &b1)], &[&bob]).unwrap();
    assert_eq!(env.total_staked(), 3);

    // claiming does not change the count
    env.advance_time(FREEZE_PERIOD);
    env.send(&[env.ix_claim_rewards(&a, &a1)], &[&alice])
        .unwrap();
    assert_eq!(env.total_staked(), 3);

    // unstake: -1
    env.send(&[env.ix_unstake(&a, &a1)], &[&alice]).unwrap();
    assert_eq!(env.total_staked(), 2);

    // burn: -1
    env.send(&[env.ix_burn_staked_nft(&b, &b1)], &[&bob])
        .unwrap();
    assert_eq!(env.total_staked(), 1);

    assert_eq!(env.amount_staked(&a), 1);
    assert_eq!(env.amount_staked(&b), 0);
}

#[test]
fn only_the_program_can_touch_the_collection_counter() {
    use mpl_core::{
        instructions::UpdateCollectionPluginV1Builder,
        types::{Attribute, Attributes, Plugin},
    };

    let mut env = Env::new();

    // The collection admin owns the collection but NOT its Attributes plugin.
    let forged = UpdateCollectionPluginV1Builder::new()
        .collection(env.collection_key())
        .payer(env.admin_key())
        .authority(Some(env.admin_key()))
        .system_program(anchor_lang::system_program::ID)
        .plugin(Plugin::Attributes(Attributes {
            attribute_list: vec![Attribute {
                key: "total_staked".to_string(),
                value: "1000000".to_string(),
            }],
        }))
        .instruction();
    let admin = Keypair::try_from(env.admin.to_bytes().as_slice()).unwrap();
    let res = env.send(&[forged], &[&admin]);

    assert!(
        failed_with(&res, core_error(MplCoreError::InvalidAuthority)),
        "admin must not be able to forge the counter: {:?}",
        res.err()
    );
    assert_eq!(env.total_staked(), 0);
}

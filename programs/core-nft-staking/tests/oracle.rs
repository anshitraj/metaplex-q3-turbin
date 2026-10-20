//! Task 2 (extended): time-based transfers with the Oracle external plugin.
//!
//! Transfers are only allowed between 09:00 and 17:00 UTC. The collection carries
//! an Oracle adapter that can REJECT the `Transfer` lifecycle event; the verdict
//! lives in a PDA that a permissionless crank keeps in sync with the clock.

#![allow(clippy::result_large_err)]

mod common;

use anchor_lang::prelude::{AccountMeta, Pubkey};
use common::*;
use core_nft_staking::{errors::StakeError, state::OracleResult};
use mpl_core::{errors::MplCoreError, types::ValidationResultsOffset};
use solana_keypair::Keypair;
use solana_signer::Signer;

const OPEN: i64 = DAY0 + 9 * HOUR; // 09:00 UTC
const CLOSE: i64 = DAY0 + 17 * HOUR; // 17:00 UTC

const CRANK_REWARD: u64 = 2_000_000;
const CRANK_WINDOW: i64 = 300; // 5 minutes
const VAULT_FUNDING: u64 = 1_000_000_000; // 1 SOL

fn key(kp: &Keypair) -> Pubkey {
    pk(&kp.pubkey())
}

struct Scene {
    env: Env,
    owner: Keypair,
    cranker: Keypair,
}

impl Scene {
    /// Pool + oracle (clock at 00:00 UTC, so the oracle starts out `Rejected`).
    fn new() -> Self {
        let mut env = Env::new();
        let ix = env.ix_initialize_oracle(CRANK_REWARD, CRANK_WINDOW, VAULT_FUNDING);
        let admin = Keypair::try_from(env.admin.to_bytes().as_slice()).unwrap();
        env.send(&[ix], &[&admin]).expect("initialize oracle");

        let owner = env.new_user();
        let cranker = Keypair::new();
        env.fund(&key(&cranker), 1_000_000_000);
        Scene {
            env,
            owner,
            cranker,
        }
    }

    fn asset(&mut self) -> Pubkey {
        let owner = key(&self.owner);
        self.env.mint_asset(&owner)
    }

    fn crank(&mut self) -> TxResult {
        let ix = self.env.ix_update_oracle(&key(&self.cranker));
        self.env.send(&[ix], &[&self.cranker])
    }

    /// Cranks and returns how many lamports left the vault.
    fn crank_and_measure(&mut self) -> u64 {
        let vault = self.env.vault_pda();
        let before = self.env.lamports(&vault);
        self.crank().expect("crank");
        before - self.env.lamports(&vault)
    }

    fn transfer(&mut self, asset: &Pubkey, to: &Pubkey) -> TxResult {
        let ix = self.env.ix_transfer_nft(&key(&self.owner), to, asset);
        self.env.send(&[ix], &[&self.owner])
    }

    fn verdict(&self) -> OracleResult {
        self.env.oracle_transfer_verdict()
    }
}

/// Raw MPL Core transfer, *bypassing* our program, with the oracle account attached.
fn core_transfer_with_oracle(
    env: &Env,
    signer: &Pubkey,
    asset: &Pubkey,
    to: &Pubkey,
) -> Instruction {
    let mut ix = env.core_transfer(signer, asset, to);
    ix.accounts
        .push(AccountMeta::new_readonly(env.oracle_pda(), false));
    ix
}

// ------------------------------------------------------------------------------
// setup
// ------------------------------------------------------------------------------

#[test]
fn the_oracle_adapter_is_attached_to_the_collection_and_funded() {
    let scene = Scene::new();
    let env = &scene.env;

    let adapters = env.collection_account().external_plugin_adapter_list;
    assert_eq!(adapters.oracles.len(), 1);
    assert_eq!(adapters.oracles[0].base_address, env.oracle_pda());
    assert_eq!(
        adapters.oracles[0].results_offset,
        ValidationResultsOffset::Anchor
    );

    // Started at 00:00 UTC: outside the window, so transfers start out blocked.
    assert_eq!(scene.verdict(), OracleResult::Rejected);
    assert_eq!(env.lamports(&env.vault_pda()), VAULT_FUNDING);
}

#[test]
fn only_the_admin_can_set_up_the_oracle() {
    let mut env = Env::new();
    let impostor = Keypair::new();
    env.fund(&key(&impostor), 10_000_000_000);

    let mut ix = env.ix_initialize_oracle(CRANK_REWARD, CRANK_WINDOW, 0);
    ix.accounts[0].pubkey = key(&impostor); // admin slot
    let res = env.send(&[ix], &[&impostor]);

    assert!(failed_with(
        &res,
        stake_error(StakeError::NotCollectionAuthority)
    ));
}

#[test]
fn the_oracle_cannot_be_set_up_twice() {
    let mut scene = Scene::new();
    let ix = scene
        .env
        .ix_initialize_oracle(CRANK_REWARD, CRANK_WINDOW, 0);
    let admin = Keypair::try_from(scene.env.admin.to_bytes().as_slice()).unwrap();

    let res = scene.env.send(&[ix], &[&admin]);

    assert!(failed_with_log(&res, "already in use"), "{:?}", res.err());
}

#[test]
fn the_crank_window_must_be_sane() {
    for bad_window in [0, -1, 3_601] {
        let mut env = Env::new();
        let ix = env.ix_initialize_oracle(CRANK_REWARD, bad_window, 0);
        let admin = Keypair::try_from(env.admin.to_bytes().as_slice()).unwrap();
        let res = env.send(&[ix], &[&admin]);
        assert!(
            failed_with(&res, stake_error(StakeError::InvalidCrankWindow)),
            "window {bad_window}: {:?}",
            res.err()
        );
    }
}

// ------------------------------------------------------------------------------
// the transfer window
// ------------------------------------------------------------------------------

#[test]
fn transfers_are_blocked_while_the_oracle_says_rejected() {
    let mut scene = Scene::new();
    let asset = scene.asset();
    let friend = key(&Keypair::new());

    let res = scene.transfer(&asset, &friend);

    // The asset is NOT frozen here, so the only thing in the way is the oracle.
    assert_eq!(scene.env.frozen(&asset), None);
    assert!(
        failed_with(&res, core_error(MplCoreError::InvalidAuthority)),
        "{:?}",
        res.err()
    );
    assert_eq!(scene.env.asset_owner(&asset), key(&scene.owner));
}

#[test]
fn cranking_after_the_open_lets_transfers_through() {
    let mut scene = Scene::new();
    let asset = scene.asset();
    let friend = key(&Keypair::new());

    scene.env.set_time(OPEN + 60); // 09:01
    scene.crank().expect("crank");
    assert_eq!(scene.verdict(), OracleResult::Approved);

    scene
        .transfer(&asset, &friend)
        .expect("transfer inside the window");
    assert_eq!(scene.env.asset_owner(&asset), friend);
}

#[test]
fn cranking_after_the_close_blocks_transfers_again() {
    let mut scene = Scene::new();
    let asset = scene.asset();
    let friend = key(&Keypair::new());

    scene.env.set_time(OPEN + 60);
    scene.crank().unwrap();
    scene.env.set_time(CLOSE + 60); // 17:01
    scene.crank().unwrap();
    assert_eq!(scene.verdict(), OracleResult::Rejected);

    assert!(scene.transfer(&asset, &friend).is_err());
    assert_eq!(scene.env.asset_owner(&asset), key(&scene.owner));
}

#[test]
fn the_verdict_is_only_as_fresh_as_the_last_crank() {
    // Documents the trade-off of an Oracle-plugin design: MPL Core reads whatever
    // the account says, it cannot read the clock itself. That is exactly why the
    // crank is rewarded.
    let mut scene = Scene::new();
    let asset = scene.asset();
    let friend = key(&Keypair::new());

    scene.env.set_time(OPEN + 60);
    scene.crank().unwrap();

    scene.env.set_time(DAY0 + 20 * HOUR); // 20:00, window closed, nobody cranked
    assert_eq!(scene.verdict(), OracleResult::Approved, "stale on purpose");
    scene
        .transfer(&asset, &friend)
        .expect("stale oracle still approves");

    // ...until somebody cranks.
    let asset2 = scene.asset();
    scene.crank().unwrap();
    assert!(scene.transfer(&asset2, &friend).is_err());
}

#[test]
fn the_window_repeats_every_day() {
    let mut scene = Scene::new();
    let asset = scene.asset();
    let friend = key(&Keypair::new());

    // day 3, 12:00
    scene.env.set_time(DAY0 + 3 * 24 * HOUR + 12 * HOUR);
    scene.crank().unwrap();
    assert_eq!(scene.verdict(), OracleResult::Approved);
    scene.transfer(&asset, &friend).expect("day 3 midday");
}

#[test]
fn a_staked_nft_stays_frozen_even_while_the_window_is_open() {
    let mut scene = Scene::new();
    let asset = scene.asset();
    let owner = key(&scene.owner);
    let friend = key(&Keypair::new());
    scene.env.set_time(OPEN + 60);
    scene.crank().unwrap();

    let stake = scene.env.ix_stake(&owner, &asset);
    scene.env.send(&[stake], &[&scene.owner]).unwrap();

    let res = scene.transfer(&asset, &friend);
    assert!(
        failed_with(&res, core_error(MplCoreError::InvalidAuthority)),
        "{:?}",
        res.err()
    );
}

// ------------------------------------------------------------------------------
// enforcement lives in MPL Core, not in our program
// ------------------------------------------------------------------------------

#[test]
fn bypassing_our_program_does_not_bypass_the_oracle() {
    let mut scene = Scene::new();
    let asset = scene.asset();
    let owner = key(&scene.owner);
    let friend = key(&Keypair::new());

    // closed: a raw Core transfer WITH the oracle attached is rejected
    let ix = core_transfer_with_oracle(&scene.env, &owner, &asset, &friend);
    assert!(scene.env.send(&[ix], &[&scene.owner]).is_err());

    // closed or open: a raw Core transfer WITHOUT the oracle account is rejected too
    scene.env.set_time(OPEN + 60);
    scene.crank().unwrap();
    let ix = scene.env.core_transfer(&owner, &asset, &friend);
    let res = scene.env.send(&[ix], &[&scene.owner]);
    assert!(
        failed_with(
            &res,
            core_error(MplCoreError::MissingExternalPluginAdapterAccount)
        ),
        "{:?}",
        res.err()
    );

    // open + oracle attached: fine
    let ix = core_transfer_with_oracle(&scene.env, &owner, &asset, &friend);
    scene
        .env
        .send(&[ix], &[&scene.owner])
        .expect("open + oracle");
    assert_eq!(scene.env.asset_owner(&asset), friend);
}

#[test]
fn only_transfers_are_policed_staking_and_burning_work_at_night() {
    let mut scene = Scene::new(); // 00:00 UTC, oracle says Rejected
    let asset = scene.asset();
    let owner = key(&scene.owner);
    assert_eq!(scene.verdict(), OracleResult::Rejected);

    let ixs = [
        scene.env.ix_stake(&owner, &asset),
        scene.env.ix_claim_rewards(&owner, &asset),
    ];
    scene
        .env
        .send(&[ixs[0].clone()], &[&scene.owner])
        .expect("stake at night");
    scene.env.advance_time(10);
    scene
        .env
        .send(&[ixs[1].clone()], &[&scene.owner])
        .expect("claim at night");
    let burn = scene.env.ix_burn_staked_nft(&owner, &asset);
    scene
        .env
        .send(&[burn], &[&scene.owner])
        .expect("burn at night");

    assert!(scene.env.asset(&asset).is_none());
    assert_eq!(scene.env.total_staked(), 0);
}

// ------------------------------------------------------------------------------
// the crank reward
// ------------------------------------------------------------------------------

#[test]
fn the_cranker_is_paid_when_the_crank_lands_right_after_the_boundary() {
    let mut scene = Scene::new();

    scene.env.set_time(OPEN + 5); // 09:00:05
    let paid = scene.crank_and_measure();
    assert_eq!(paid, CRANK_REWARD);

    scene.env.set_time(CLOSE + CRANK_WINDOW); // exactly at the edge of the window
    let paid = scene.crank_and_measure();
    assert_eq!(paid, CRANK_REWARD);
}

#[test]
fn a_late_crank_still_fixes_the_oracle_but_earns_nothing() {
    let mut scene = Scene::new();

    scene.env.set_time(OPEN + CRANK_WINDOW + 1);
    let paid = scene.crank_and_measure();

    assert_eq!(paid, 0);
    assert_eq!(
        scene.verdict(),
        OracleResult::Approved,
        "state is fixed anyway"
    );
}

#[test]
fn a_crank_that_changes_nothing_earns_nothing() {
    let mut scene = Scene::new();

    // 00:00: already Rejected, nothing to flip, even though it is "near" no boundary
    assert_eq!(scene.crank_and_measure(), 0);

    // first crank after the open pays...
    scene.env.set_time(OPEN + 1);
    assert_eq!(scene.crank_and_measure(), CRANK_REWARD);
    // ...the second one right behind it must not (no spamming the vault dry)
    scene.env.advance_time(1);
    assert_eq!(scene.crank_and_measure(), 0);
}

#[test]
fn a_crank_just_after_midnight_is_not_near_any_boundary() {
    let mut scene = Scene::new();
    scene.env.set_time(OPEN + 1);
    scene.crank().unwrap(); // Approved
                            // next day 00:10: the verdict is stale (Approved) and has to flip to Rejected,
                            // but the last boundary (17:00 the day before) is 7h10m away
    scene.env.set_time(DAY0 + 24 * HOUR + 600);
    assert_eq!(scene.crank_and_measure(), 0);
    assert_eq!(scene.verdict(), OracleResult::Rejected);
}

#[test]
fn the_vault_keeps_its_rent_reserve_and_pays_what_it_can() {
    let mut env = Env::new();
    let rent_reserve = env.svm.minimum_balance_for_rent_exemption(0);
    let leftover = 500; // all that is payable above the reserve
    let ix = env.ix_initialize_oracle(CRANK_REWARD, CRANK_WINDOW, rent_reserve + leftover);
    let admin = Keypair::try_from(env.admin.to_bytes().as_slice()).unwrap();
    env.send(&[ix], &[&admin]).unwrap();
    let cranker = Keypair::new();
    env.fund(&key(&cranker), 1_000_000_000);

    env.set_time(OPEN + 1);
    let vault = env.vault_pda();
    let ix = env.ix_update_oracle(&key(&cranker));
    env.send(&[ix], &[&cranker]).expect("crank");

    assert_eq!(env.lamports(&vault), rent_reserve, "reserve untouched");

    // vault is drained: the next profitable crank still works, it just pays zero
    env.set_time(CLOSE + 1);
    let ix = env.ix_update_oracle(&key(&cranker));
    env.send(&[ix], &[&cranker])
        .expect("crank on an empty vault");
    assert_eq!(env.lamports(&vault), rent_reserve);
    assert_eq!(env.oracle_transfer_verdict(), OracleResult::Rejected);
}

#[test]
fn the_oracle_works_with_an_unfunded_vault() {
    let mut env = Env::new();
    let ix = env.ix_initialize_oracle(CRANK_REWARD, CRANK_WINDOW, 0);
    let admin = Keypair::try_from(env.admin.to_bytes().as_slice()).unwrap();
    env.send(&[ix], &[&admin]).unwrap();
    let cranker = Keypair::new();
    env.fund(&key(&cranker), 1_000_000_000);

    env.set_time(OPEN + 1);
    let ix = env.ix_update_oracle(&key(&cranker));
    env.send(&[ix], &[&cranker])
        .expect("crank without a vault balance");

    assert_eq!(env.oracle_transfer_verdict(), OracleResult::Approved);
}

#[test]
fn anyone_can_top_up_the_vault_with_a_plain_sol_transfer() {
    let mut scene = Scene::new();
    let vault = scene.env.vault_pda();
    let before = scene.env.lamports(&vault);

    scene.env.fund(&vault, 123_456);

    assert_eq!(scene.env.lamports(&vault), before + 123_456);
}

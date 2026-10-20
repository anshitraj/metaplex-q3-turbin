//! Shared LiteSVM harness: the real MPL Core program (dumped from mainnet, see
//! `tests/fixtures/`) next to the freshly built staking program.
//!
//! Run `cargo build-sbf` first so `target/deploy/core_nft_staking.so` exists.
#![allow(dead_code, clippy::result_large_err)]

use anchor_lang::{prelude::Pubkey, system_program, InstructionData, ToAccountMetas};
use core_nft_staking::{accounts, instruction};
use litesvm::{
    types::{FailedTransactionMetadata, TransactionMetadata},
    LiteSVM,
};
use mpl_core::{
    instructions::{
        BurnV1Builder, CreateCollectionV2Builder, CreateV2Builder, RemovePluginV1Builder,
        TransferV1Builder, UpdatePluginV1Builder,
    },
    types::{Attributes, FreezeDelegate, Plugin, PluginAuthorityPair, PluginType},
    Asset, Collection, ID as MPL_CORE_ID,
};
use solana_address::Address;
use solana_clock::Clock;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;

pub use anchor_lang::solana_program::instruction::Instruction;

pub type TxResult = Result<TransactionMetadata, FailedTransactionMetadata>;

pub const ATA_PROGRAM: Pubkey =
    anchor_lang::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
pub const TOKEN_PROGRAM: Pubkey =
    anchor_lang::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

// Pool parameters used by every test.
pub const REWARD_PER_SECOND: u64 = 1_000;
pub const BURN_BONUS: u64 = 5_000_000;
pub const FREEZE_PERIOD: i64 = 86_400; // 1 day
pub const MAX_STAKE: u8 = 3;

/// 2025-01-06 00:00:00 UTC, a Monday midnight. Anchor for all the clock games.
pub const DAY0: i64 = 1_736_121_600;
pub const HOUR: i64 = 3_600;

pub fn addr(key: &Pubkey) -> Address {
    Address::new_from_array(key.to_bytes())
}

pub fn pk(key: &Address) -> Pubkey {
    Pubkey::new_from_array(key.to_bytes())
}

pub fn pda(seeds: &[&[u8]]) -> Pubkey {
    Pubkey::find_program_address(seeds, &core_nft_staking::ID).0
}

pub fn ata(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), TOKEN_PROGRAM.as_ref(), mint.as_ref()],
        &ATA_PROGRAM,
    )
    .0
}

/// `true` when the failed transaction carries custom error `code`.
pub fn failed_with(res: &TxResult, code: u32) -> bool {
    match res {
        Err(failed) => format!("{:?}", failed.err).contains(&format!("Custom({code})")),
        Ok(_) => false,
    }
}

/// `true` when the failed transaction logged `needle`.
pub fn failed_with_log(res: &TxResult, needle: &str) -> bool {
    match res {
        Err(failed) => failed.meta.logs.iter().any(|line| line.contains(needle)),
        Ok(_) => false,
    }
}

/// Anchor framework error: the account the program expected does not exist.
pub const ANCHOR_ACCOUNT_NOT_INITIALIZED: u32 =
    anchor_lang::error::ErrorCode::AccountNotInitialized as u32;

/// Anchor program errors are offset by 6000.
pub fn stake_error(err: core_nft_staking::errors::StakeError) -> u32 {
    anchor_lang::error::ERROR_CODE_OFFSET + err as u32
}

pub fn core_error(err: mpl_core::errors::MplCoreError) -> u32 {
    err as u32
}

pub struct Env {
    pub svm: LiteSVM,
    pub admin: Keypair,
    pub collection: Keypair,
    pub config: Pubkey,
    pub rewards_mint: Pubkey,
}

impl Env {
    /// Collection + initialised staking pool, clock parked at `DAY0` 00:00 UTC.
    pub fn new() -> Self {
        let mut env = Self::bare();
        let init = env.ix_initialize_config(&env.admin_key(), &env.collection_key());
        env.send(&[init], &[&env.admin_clone()])
            .expect("initialize config");
        env
    }

    /// Only the Core collection exists; the staking pool is NOT initialised yet.
    pub fn bare() -> Self {
        Self::bare_with_collection_plugins(vec![])
    }

    /// Like [`Env::bare`], but the collection is created carrying `plugins`.
    pub fn bare_with_collection_plugins(plugins: Vec<PluginAuthorityPair>) -> Self {
        let mut svm = LiteSVM::new();
        svm.add_program_from_file(
            addr(&MPL_CORE_ID),
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mpl_core.so"),
        )
        .expect("load mpl_core.so");
        svm.add_program_from_file(
            addr(&core_nft_staking::ID),
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../target/deploy/core_nft_staking.so"
            ),
        )
        .expect("load core_nft_staking.so (run `cargo build-sbf` first)");

        let admin = Keypair::new();
        svm.airdrop(&admin.pubkey(), 100_000_000_000).unwrap();
        let collection = Keypair::new();
        let collection_key = pk(&collection.pubkey());
        let config = pda(&[b"config", collection_key.as_ref()]);
        let rewards_mint = pda(&[b"rewards", config.as_ref()]);

        let mut env = Env {
            svm,
            admin,
            collection,
            config,
            rewards_mint,
        };
        env.set_time(DAY0);

        // The collection is created by the admin, *without* any plugin: the staking
        // program adds the `total_staked` Attribute itself in `initialize_config`.
        let create_collection = CreateCollectionV2Builder::new()
            .collection(collection_key)
            .update_authority(Some(env.admin_key()))
            .payer(env.admin_key())
            .system_program(system_program::ID)
            .name("Staked Apes".to_string())
            .uri("https://example.com/collection.json".to_string())
            .plugins(plugins)
            .instruction();
        env.send(
            &[create_collection],
            &[&env.admin_clone(), &env.collection_clone()],
        )
        .expect("create collection");

        env
    }

    // Keypair isn't Clone; re-create from bytes so `&self` borrows stay short.
    fn admin_clone(&self) -> Keypair {
        Keypair::try_from(self.admin.to_bytes().as_slice()).unwrap()
    }
    fn collection_clone(&self) -> Keypair {
        Keypair::try_from(self.collection.to_bytes().as_slice()).unwrap()
    }

    pub fn admin_key(&self) -> Pubkey {
        pk(&self.admin.pubkey())
    }

    pub fn collection_key(&self) -> Pubkey {
        pk(&self.collection.pubkey())
    }

    // ---- plumbing --------------------------------------------------------------

    /// Sends one transaction. The first signer pays.
    pub fn send(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> TxResult {
        self.svm.expire_blockhash();
        let payer = signers[0].pubkey();
        let tx = Transaction::new(
            signers,
            Message::new(ixs, Some(&payer)),
            self.svm.latest_blockhash(),
        );
        self.svm.send_transaction(tx)
    }

    pub fn set_time(&mut self, unix_timestamp: i64) {
        let mut clock: Clock = self.svm.get_sysvar();
        clock.unix_timestamp = unix_timestamp;
        self.svm.set_sysvar(&clock);
    }

    pub fn now(&self) -> i64 {
        self.svm.get_sysvar::<Clock>().unix_timestamp
    }

    pub fn advance_time(&mut self, seconds: i64) {
        let now = self.now();
        self.set_time(now + seconds);
    }

    pub fn fund(&mut self, key: &Pubkey, lamports: u64) {
        self.svm.airdrop(&addr(key), lamports).unwrap();
    }

    // ---- state readers ---------------------------------------------------------

    pub fn account_exists(&self, key: &Pubkey) -> bool {
        self.svm
            .get_account(&addr(key))
            .is_some_and(|a| a.lamports > 0)
    }

    pub fn token_balance(&self, owner: &Pubkey, mint: &Pubkey) -> u64 {
        self.svm
            .get_account(&addr(&ata(owner, mint)))
            .map(|a| u64::from_le_bytes(a.data[64..72].try_into().unwrap()))
            .unwrap_or(0)
    }

    pub fn rewards_balance(&self, owner: &Pubkey) -> u64 {
        self.token_balance(owner, &self.rewards_mint)
    }

    pub fn asset(&self, asset: &Pubkey) -> Option<Box<Asset>> {
        let account = self.svm.get_account(&addr(asset))?;
        if account.lamports == 0 || account.data.is_empty() {
            return None;
        }
        Asset::deserialize(&account.data).ok()
    }

    pub fn asset_owner(&self, asset: &Pubkey) -> Pubkey {
        self.asset(asset).expect("asset exists").base.owner
    }

    /// `Some(frozen)` if the asset carries a FreezeDelegate plugin.
    pub fn frozen(&self, asset: &Pubkey) -> Option<bool> {
        self.asset(asset)?
            .plugin_list
            .freeze_delegate
            .map(|p| p.freeze_delegate.frozen)
    }

    pub fn has_burn_delegate(&self, asset: &Pubkey) -> bool {
        self.asset(asset)
            .is_some_and(|a| a.plugin_list.burn_delegate.is_some())
    }

    pub fn collection_account(&self) -> Box<Collection> {
        let account = self
            .svm
            .get_account(&addr(&self.collection_key()))
            .expect("collection exists");
        Collection::deserialize(&account.data).expect("decode collection")
    }

    /// Value of the collection-level `total_staked` Attribute.
    pub fn total_staked(&self) -> u64 {
        let attributes: Attributes = self
            .collection_account()
            .plugin_list
            .attributes
            .expect("collection has an Attributes plugin")
            .attributes;
        attributes
            .attribute_list
            .iter()
            .find(|a| a.key == "total_staked")
            .expect("total_staked attribute")
            .value
            .parse()
            .expect("numeric total_staked")
    }

    pub fn stake_pda(&self, asset: &Pubkey) -> Pubkey {
        pda(&[b"stake", asset.as_ref(), self.config.as_ref()])
    }

    pub fn user_pda(&self, user: &Pubkey) -> Pubkey {
        pda(&[b"user", self.config.as_ref(), user.as_ref()])
    }

    pub fn amount_staked(&self, user: &Pubkey) -> u8 {
        let account = self
            .svm
            .get_account(&addr(&self.user_pda(user)))
            .expect("user account");
        account.data[8] // discriminator, then `amount_staked`
    }

    // ---- actors ----------------------------------------------------------------

    /// A funded wallet with an initialised user account.
    pub fn new_user(&mut self) -> Keypair {
        let user = Keypair::new();
        let user_key = pk(&user.pubkey());
        self.fund(&user_key, 10_000_000_000);
        let ix = self.ix_initialize_user(&user_key);
        self.send(&[ix], &[&user]).expect("initialize user");
        user
    }

    /// Mints a Core asset in the collection straight into `owner`'s wallet.
    pub fn mint_asset(&mut self, owner: &Pubkey) -> Pubkey {
        let asset = Keypair::new();
        let asset_key = pk(&asset.pubkey());
        let ix = CreateV2Builder::new()
            .asset(asset_key)
            .collection(Some(self.collection_key()))
            .authority(Some(self.admin_key()))
            .payer(self.admin_key())
            .owner(Some(*owner))
            .system_program(system_program::ID)
            .name("Ape #1".to_string())
            .uri("https://example.com/ape.json".to_string())
            .instruction();
        let admin = self.admin_clone();
        self.send(&[ix], &[&admin, &asset]).expect("mint asset");
        asset_key
    }

    /// An asset that does NOT belong to the staking collection (update authority is
    /// the admin wallet itself).
    pub fn mint_standalone_asset(&mut self, owner: &Pubkey) -> Pubkey {
        let asset = Keypair::new();
        let asset_key = pk(&asset.pubkey());
        let ix = CreateV2Builder::new()
            .asset(asset_key)
            .payer(self.admin_key())
            .owner(Some(*owner))
            .update_authority(Some(self.admin_key()))
            .system_program(system_program::ID)
            .name("Stray".to_string())
            .uri("https://example.com/stray.json".to_string())
            .instruction();
        let admin = self.admin_clone();
        self.send(&[ix], &[&admin, &asset])
            .expect("mint standalone");
        asset_key
    }

    // ---- direct MPL Core calls (what a user could try to do behind our back) ----

    pub fn core_transfer(
        &self,
        signer: &Pubkey,
        asset: &Pubkey,
        new_owner: &Pubkey,
    ) -> Instruction {
        TransferV1Builder::new()
            .asset(*asset)
            .collection(Some(self.collection_key()))
            .payer(*signer)
            .authority(Some(*signer))
            .new_owner(*new_owner)
            .system_program(Some(system_program::ID))
            .instruction()
    }

    pub fn core_set_frozen(&self, signer: &Pubkey, asset: &Pubkey, frozen: bool) -> Instruction {
        UpdatePluginV1Builder::new()
            .asset(*asset)
            .collection(Some(self.collection_key()))
            .payer(*signer)
            .authority(Some(*signer))
            .system_program(system_program::ID)
            .plugin(Plugin::FreezeDelegate(FreezeDelegate { frozen }))
            .instruction()
    }

    pub fn core_remove_plugin(
        &self,
        signer: &Pubkey,
        asset: &Pubkey,
        plugin_type: PluginType,
    ) -> Instruction {
        RemovePluginV1Builder::new()
            .asset(*asset)
            .collection(Some(self.collection_key()))
            .payer(*signer)
            .authority(Some(*signer))
            .system_program(system_program::ID)
            .plugin_type(plugin_type)
            .instruction()
    }

    pub fn core_burn(&self, signer: &Pubkey, asset: &Pubkey) -> Instruction {
        BurnV1Builder::new()
            .asset(*asset)
            .collection(Some(self.collection_key()))
            .payer(*signer)
            .authority(Some(*signer))
            .system_program(Some(system_program::ID))
            .instruction()
    }

    // ---- instruction builders --------------------------------------------------

    pub fn ix_initialize_config(&self, admin: &Pubkey, collection: &Pubkey) -> Instruction {
        let config = pda(&[b"config", collection.as_ref()]);
        Instruction {
            program_id: core_nft_staking::ID,
            accounts: accounts::InitializeConfig {
                admin: *admin,
                collection: *collection,
                config,
                rewards_mint: pda(&[b"rewards", config.as_ref()]),
                mpl_core_program: MPL_CORE_ID,
                system_program: system_program::ID,
                token_program: TOKEN_PROGRAM,
            }
            .to_account_metas(None),
            data: instruction::InitializeConfig {
                reward_per_second: REWARD_PER_SECOND,
                burn_bonus: BURN_BONUS,
                freeze_period: FREEZE_PERIOD,
                max_stake: MAX_STAKE,
            }
            .data(),
        }
    }

    pub fn ix_initialize_user(&self, user: &Pubkey) -> Instruction {
        Instruction {
            program_id: core_nft_staking::ID,
            accounts: accounts::InitializeUser {
                user: *user,
                config: self.config,
                user_account: self.user_pda(user),
                system_program: system_program::ID,
            }
            .to_account_metas(None),
            data: instruction::InitializeUser {}.data(),
        }
    }

    pub fn ix_stake(&self, user: &Pubkey, asset: &Pubkey) -> Instruction {
        Instruction {
            program_id: core_nft_staking::ID,
            accounts: accounts::Stake {
                user: *user,
                asset: *asset,
                collection: self.collection_key(),
                config: self.config,
                stake_account: self.stake_pda(asset),
                user_account: self.user_pda(user),
                mpl_core_program: MPL_CORE_ID,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
            data: instruction::Stake {}.data(),
        }
    }

    pub fn ix_claim_rewards(&self, user: &Pubkey, asset: &Pubkey) -> Instruction {
        Instruction {
            program_id: core_nft_staking::ID,
            accounts: accounts::ClaimRewards {
                user: *user,
                asset: *asset,
                config: self.config,
                stake_account: self.stake_pda(asset),
                rewards_mint: self.rewards_mint,
                rewards_ata: ata(user, &self.rewards_mint),
                system_program: system_program::ID,
                token_program: TOKEN_PROGRAM,
                associated_token_program: ATA_PROGRAM,
            }
            .to_account_metas(None),
            data: instruction::ClaimRewards {}.data(),
        }
    }

    pub fn ix_unstake(&self, user: &Pubkey, asset: &Pubkey) -> Instruction {
        Instruction {
            program_id: core_nft_staking::ID,
            accounts: accounts::Unstake {
                user: *user,
                asset: *asset,
                collection: self.collection_key(),
                config: self.config,
                stake_account: self.stake_pda(asset),
                user_account: self.user_pda(user),
                rewards_mint: self.rewards_mint,
                rewards_ata: ata(user, &self.rewards_mint),
                mpl_core_program: MPL_CORE_ID,
                system_program: system_program::ID,
                token_program: TOKEN_PROGRAM,
                associated_token_program: ATA_PROGRAM,
            }
            .to_account_metas(None),
            data: instruction::Unstake {}.data(),
        }
    }

    pub fn ix_burn_staked_nft(&self, user: &Pubkey, asset: &Pubkey) -> Instruction {
        Instruction {
            program_id: core_nft_staking::ID,
            accounts: accounts::BurnStakedNft {
                user: *user,
                asset: *asset,
                collection: self.collection_key(),
                config: self.config,
                stake_account: self.stake_pda(asset),
                user_account: self.user_pda(user),
                rewards_mint: self.rewards_mint,
                rewards_ata: ata(user, &self.rewards_mint),
                mpl_core_program: MPL_CORE_ID,
                system_program: system_program::ID,
                token_program: TOKEN_PROGRAM,
                associated_token_program: ATA_PROGRAM,
            }
            .to_account_metas(None),
            data: instruction::BurnStakedNft {}.data(),
        }
    }

    // ---- oracle ----------------------------------------------------------------

    pub fn oracle_pda(&self) -> Pubkey {
        pda(&[b"oracle", self.config.as_ref()])
    }

    pub fn vault_pda(&self) -> Pubkey {
        pda(&[b"vault", self.config.as_ref()])
    }

    pub fn ix_initialize_oracle(
        &self,
        crank_reward: u64,
        crank_window: i64,
        vault_funding: u64,
    ) -> Instruction {
        Instruction {
            program_id: core_nft_staking::ID,
            accounts: accounts::InitializeOracle {
                admin: self.admin_key(),
                config: self.config,
                collection: self.collection_key(),
                oracle: self.oracle_pda(),
                vault: self.vault_pda(),
                mpl_core_program: MPL_CORE_ID,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
            data: instruction::InitializeOracle {
                crank_reward,
                crank_window,
                vault_funding,
            }
            .data(),
        }
    }

    pub fn ix_update_oracle(&self, cranker: &Pubkey) -> Instruction {
        Instruction {
            program_id: core_nft_staking::ID,
            accounts: accounts::UpdateOracle {
                cranker: *cranker,
                config: self.config,
                oracle: self.oracle_pda(),
                vault: self.vault_pda(),
                system_program: system_program::ID,
            }
            .to_account_metas(None),
            data: instruction::UpdateOracle {}.data(),
        }
    }

    pub fn ix_transfer_nft(
        &self,
        owner: &Pubkey,
        new_owner: &Pubkey,
        asset: &Pubkey,
    ) -> Instruction {
        Instruction {
            program_id: core_nft_staking::ID,
            accounts: accounts::TransferNft {
                owner: *owner,
                new_owner: *new_owner,
                asset: *asset,
                collection: self.collection_key(),
                config: self.config,
                oracle: self.oracle_pda(),
                mpl_core_program: MPL_CORE_ID,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
            data: instruction::TransferNft {}.data(),
        }
    }

    /// Verdict (`Approved`/`Rejected`/`Pass`) currently stored for `Transfer`.
    pub fn oracle_transfer_verdict(&self) -> core_nft_staking::state::OracleResult {
        let account = self
            .svm
            .get_account(&addr(&self.oracle_pda()))
            .expect("oracle account");
        let oracle: core_nft_staking::state::Oracle =
            anchor_lang::AccountDeserialize::try_deserialize(&mut account.data.as_slice())
                .expect("decode oracle");
        oracle.transfer_result().expect("initialised oracle")
    }

    pub fn lamports(&self, key: &Pubkey) -> u64 {
        self.svm.get_balance(&addr(key)).unwrap_or(0)
    }
}

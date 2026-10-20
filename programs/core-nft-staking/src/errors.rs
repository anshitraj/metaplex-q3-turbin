use anchor_lang::prelude::*;

#[error_code]
pub enum StakeError {
    #[msg("Freeze period has not passed yet")]
    FreezePeriodNotPassed,
    #[msg("Maximum number of staked NFTs reached")]
    MaxStakeReached,
    #[msg("Account is not a valid MPL Core asset")]
    InvalidAsset,
    #[msg("Account is not a valid MPL Core collection")]
    InvalidCollection,
    #[msg("Signer does not own this asset")]
    NotAssetOwner,
    #[msg("Asset does not belong to the staking collection")]
    WrongCollection,
    #[msg("Signer is not the update authority of the collection")]
    NotCollectionAuthority,
    #[msg("There are no rewards to claim yet")]
    NothingToClaim,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("Collection has no valid `total_staked` attribute")]
    InvalidStakeCounter,
    #[msg("Crank window must be between 1 and MAX_CRANK_WINDOW_SECONDS")]
    InvalidCrankWindow,
}

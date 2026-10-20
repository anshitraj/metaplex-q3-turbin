use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct UserAccount {
    pub amount_staked: u8,
    pub bump: u8,
}

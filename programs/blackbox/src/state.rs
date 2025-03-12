use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct BlackboxAccount {
    pub bump: u8,
    pub token_mint: Pubkey,
    pub vault: Pubkey,
    pub mapping_account_count: u8,
}

#[account]
#[derive(InitSpace)]
pub struct MappingAccount {
    pub index: u8,
    pub token_mint: Pubkey,
    pub encrypted_pubkeys: Vec<[u8; 32]>,
    pub encrypted_balances: Vec<[u8; 32]>,
}

/// Error codes for the blackbox program
#[error_code]
pub enum ErrorCode {
    #[msg("User not found in any mapping account")]
    UserNotFound,
    #[msg("No space available in any mapping account")]
    NoSpaceAvailable,
    #[msg("Invalid vault")]
    InvalidVault,
}


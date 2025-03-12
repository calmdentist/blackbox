use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use arcium_anchor::{
    comp_def_offset, init_comp_def, queue_computation, CLOCK_PDA_SEED, CLUSTER_PDA_SEED,
    COMP_DEF_PDA_SEED, MEMPOOL_PDA_SEED, MXE_PDA_SEED, POOL_PDA_SEED,
};
use arcium_client::idl::arcium::{
    accounts::{
        ClockAccount, Cluster, ComputationDefinitionAccount, Mempool, PersistentMXEAccount,
        StakingPoolAccount,
    },
    program::Arcium,
    types::{Argument, CallbackAccount},
    ID_CONST as ARCIUM_PROG_ID,
};
use arcium_macros::{
    arcium_callback, arcium_program, callback_accounts, init_computation_definition_accounts,
    queue_computation_accounts,
};
// Account/state definitions
use crate::state::{BlackboxAccount, MappingAccount, ErrorCode};

const COMP_DEF_OFFSET_DEPOSIT: u32 = comp_def_offset("deposit");
const COMP_DEF_OFFSET_TRANSFER: u32 = comp_def_offset("transfer");
const COMP_DEF_OFFSET_WITHDRAW: u32 = comp_def_offset("withdraw");

// Maximum size for a mapping account (close to 10MB)
const MAX_MAPPING_ACCOUNT_SIZE: usize = 10 * 1024 * 1024; // 10MB
// Approximate size of a single entry (pubkey + balance)
const ENTRY_SIZE: usize = 32 * 2; // 64 bytes
// Maximum number of entries per mapping account
const MAX_ENTRIES_PER_ACCOUNT: usize = MAX_MAPPING_ACCOUNT_SIZE / ENTRY_SIZE;


declare_id!("Blackbox111111111111111111111111111111111");

#[arcium_program]
pub mod blackbox {
    use super::*;

    /// Initializes a blackbox for a token
    pub fn init_blackbox(ctx: Context<InitBlackbox>) -> Result<()> {
        let blackbox = &mut ctx.accounts.blackbox;
        
        blackbox.token_mint = ctx.accounts.token_mint.key();
        blackbox.vault = ctx.accounts.vault.key();
        blackbox.mapping_account_count = 0;
        blackbox.bump = *ctx.bumps.blackbox;
        
        Ok(())
    }

    /// Initializes a new mapping account for a specific token blackbox.
    ///
    /// This is called when a new mapping account is needed, either for the first account
    /// or when existing accounts are full.
    pub fn initialize_mapping_account(
        ctx: Context<InitializeMappingAccount>,
    ) -> Result<()> {
        let blackbox = &mut ctx.accounts.blackbox;
        let mapping_account = &mut ctx.accounts.mapping_account;
        
        // Set the mapping account index and update total count
        let index = blackbox.mapping_account_count;
        blackbox.mapping_account_count += 1;
        
        mapping_account.index = index;
        mapping_account.token_mint = blackbox.token_mint;
        mapping_account.encrypted_pubkeys = Vec::new();
        mapping_account.encrypted_balances = Vec::new();
        
        Ok(())
    }

    /// Initializes the computation definitions for the blackbox program.
    ///
    /// This must be called once before any other operations can be performed.
    #[init_computation_definition_accounts]
    pub fn init_computation_definitions(ctx: Context<InitComputationDefinitions>) -> Result<()> {
        // Initialize deposit computation definition
        init_comp_def(
            &ctx.accounts.arcium_program,
            &ctx.accounts.comp_def_deposit,
            &ctx.accounts.payer,
            &ctx.accounts.system_program,
            COMP_DEF_OFFSET_DEPOSIT,
            "deposit".to_string(),
            "Deposit funds into blackbox".to_string(),
        )?;

        // Initialize transfer computation definition
        init_comp_def(
            &ctx.accounts.arcium_program,
            &ctx.accounts.comp_def_transfer,
            &ctx.accounts.payer,
            &ctx.accounts.system_program,
            COMP_DEF_OFFSET_TRANSFER,
            "transfer".to_string(),
            "Transfer funds within blackbox".to_string(),
        )?;

        // Initialize withdraw computation definition
        init_comp_def(
            &ctx.accounts.arcium_program,
            &ctx.accounts.comp_def_withdraw,
            &ctx.accounts.payer,
            &ctx.accounts.system_program,
            COMP_DEF_OFFSET_WITHDRAW,
            "withdraw".to_string(),
            "Withdraw funds from blackbox".to_string(),
        )?;

        Ok(())
    }

    /// Deposits tokens into blackbox
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        // Transfer tokens to vault
        let cpi_accounts = token::Transfer {
            from: ctx.accounts.user_token_account.to_account_info(),
            to: ctx.accounts.vault.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        };
        
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        
        token::transfer(cpi_ctx, amount)?;

        // Arguments - signer pubkey, amount
        let args = vec![
            Argument::PlaintextPubkey(ctx.accounts.user.key()),
            Argument::PlaintextU64(amount)
        ];

        // Queue computation for deposit operation
        queue_computation(
            ctx.accounts,
            args,
            vec![
                CallbackAccount::new(ctx.accounts.user.key(), false),
                CallbackAccount::new_from_accounts(&ctx.accounts.mapping_accounts, true),
                CallbackAccount::new(ctx.accounts.system_program.key(), false),
            ],
            None,
        )?;

        Ok(())
    }

    /// Deposit callback
    #[arcium_callback]
    pub fn deposit_callback(ctx: Context<DepositCallback>, output: Vec<u8>) -> Result<()> {
        let user_pubkey = ctx.accounts.user.key();
        
        // Find or create user entry in mapping accounts
        let (account_idx, entry_idx) = find_or_create_user_entry(
            &mut ctx.accounts.mapping_accounts,
            user_pubkey,
            encrypted_amount,
        )?;
        
        // Update user's encrypted balance
        // In a real implementation, this would use homomorphic addition
        // For now, we just replace the balance with the encrypted amount
        ctx.accounts.mapping_accounts[account_idx].encrypted_balances[entry_idx] = encrypted_amount;
        
        Ok(())
    }

    /// Transfers funds within the blackbox (internal transfer).
    ///
    /// This moves funds in the encrypted domain from the sender to the recipient.
    /// The `enc_amount` represents the transfer amount (encrypted).
    pub fn transfer(
        ctx: Context<Transfer>,
        enc_recipient: [u8; 32],
        enc_amount: [u8; 32]
    ) -> Result<()> {
        // Arguments - sender pubkey, recipient pubkey, encrypted amount
        let args = vec![
            Argument::PlaintextPubkey(ctx.accounts.sender.key()),
            Argument::CipheredPubkey(enc_recipient),
            Argument::CipheredU64(enc_amount),
        ];

        // Queue computation for transfer operation
        queue_computation(
            ctx.accounts,
            args,
            vec![
                CallbackAccount::new(ctx.accounts.sender.key(), false),
                CallbackAccount::new(ctx.accounts.recipient.key(), false),
                CallbackAccount::new_from_accounts(&ctx.accounts.mapping_accounts, true),
                CallbackAccount::new(ctx.accounts.system_program.key(), false),
            ],
            None,
        )?;

        Ok(())
    }

    /// Transfer callback
    #[arcium_callback]
    pub fn transfer_callback(ctx: Context<Transfer>, encrypted_amount: [u8; 32], nonce: u128) -> Result<()> {
        let sender_pubkey = ctx.accounts.sender.key();
        let recipient_pubkey = ctx.accounts.recipient.key();
        
        // Find sender entry
        let (sender_account_idx, sender_entry_idx) = find_user_entry(
            &ctx.accounts.mapping_accounts,
            sender_pubkey,
        )?;
        
        // Find or create recipient entry
        let (recipient_account_idx, recipient_entry_idx) = find_or_create_user_entry(
            &mut ctx.accounts.mapping_accounts,
            recipient_pubkey,
            [0; 32], // Initial balance will be updated below
        )?;
        
        // In a real implementation, this would use homomorphic operations:
        // 1. Subtract from sender's balance
        // 2. Add to recipient's balance
        
        // For now, we just update the balances directly
        // This is a placeholder for the actual homomorphic operations
        ctx.accounts.mapping_accounts[sender_account_idx].encrypted_balances[sender_entry_idx] = [0; 32];
        ctx.accounts.mapping_accounts[recipient_account_idx].encrypted_balances[recipient_entry_idx] = encrypted_amount;
        
        Ok(())
    }

    /// Withdraws tokens from the blackbox.
    ///
    /// The withdrawal amount is provided as an encrypted value. The function must verify,
    /// within the encrypted domain, that the user has sufficient balance before updating it.
    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        // Arguments - user pubkey, amount
        let args = vec![
            Argument::PlaintextPubkey(ctx.accounts.user.key()),
            Argument::PlaintextU64(amount),
        ];

        // Queue computation for withdrawal operation
        queue_computation(
            ctx.accounts,
            args,
            vec![
                CallbackAccount::new(ctx.accounts.user.key(), false),
                CallbackAccount::new_from_accounts(&ctx.accounts.mapping_accounts, true),
                CallbackAccount::new(ctx.accounts.system_program.key(), false),
            ],
            None,
        )?;

        Ok(())
    }

    /// Withdraw callback
    #[arcium_callback]
    pub fn withdraw_callback(ctx: Context<Withdraw>, encrypted_amount: [u8; 32], nonce: u128) -> Result<()> {
        let user_pubkey = ctx.accounts.user.key();
        
        // Find user entry
        let (account_idx, entry_idx) = find_user_entry(
            &ctx.accounts.mapping_accounts,
            user_pubkey,
        )?;
        
        // In a real implementation, this would:
        // 1. Verify the user has sufficient balance using homomorphic comparison
        // 2. Subtract the withdrawal amount from the user's balance
        
        // For now, we just update the balance directly
        // This is a placeholder for the actual homomorphic operations
        ctx.accounts.mapping_accounts[account_idx].encrypted_balances[entry_idx] = [0; 32];
        
        Ok(())
    }
}

/// Helper function to find a user's entry in the mapping accounts
/// Returns (account_index, entry_index) if found, or an error if not found
fn find_user_entry(
    mapping_accounts: &[Account<MappingAccount>],
    user_pubkey: Pubkey,
) -> Result<(usize, usize)> {
    // Convert user pubkey to [u8; 32] for comparison
    let user_pubkey_bytes: [u8; 32] = user_pubkey.to_bytes();
    
    // Search through all mapping accounts
    for (account_idx, account) in mapping_accounts.iter().enumerate() {
        // Search through all entries in this account
        for (entry_idx, encrypted_pubkey) in account.encrypted_pubkeys.iter().enumerate() {
            // In a real implementation, this would use homomorphic comparison
            // For now, we just do a direct comparison (which wouldn't work with real encryption)
            if encrypted_pubkey == &user_pubkey_bytes {
                return Ok((account_idx, entry_idx));
            }
        }
    }
    
    // User not found
    Err(error!(ErrorCode::UserNotFound))
}

/// Helper function to find or create a user entry in the mapping accounts
/// Returns (account_index, entry_index) of the found or created entry
fn find_or_create_user_entry(
    mapping_accounts: &mut [Account<MappingAccount>],
    user_pubkey: Pubkey,
    initial_balance: [u8; 32],
) -> Result<(usize, usize)> {
    // Try to find existing user
    match find_user_entry(mapping_accounts, user_pubkey) {
        Ok(indices) => Ok(indices),
        Err(_) => {
            // User not found, create new entry
            let user_pubkey_bytes: [u8; 32] = user_pubkey.to_bytes();
            
            // Find an account with space available
            for (account_idx, account) in mapping_accounts.iter_mut().enumerate() {
                // Check if this account has space for a new entry
                if account.encrypted_pubkeys.len() < MAX_ENTRIES_PER_ACCOUNT {
                    // Add the new entry
                    account.encrypted_pubkeys.push(user_pubkey_bytes);
                    account.encrypted_balances.push(initial_balance);
                    return Ok((account_idx, account.encrypted_pubkeys.len() - 1));
                }
            }
            
            // No account with space found
            Err(error!(ErrorCode::NoSpaceAvailable))
        }
    }
}

/// Accounts for initializing a blackbox for a specific token
#[derive(Accounts)]
pub struct InitBlackbox<'info> {
    /// The token mint for which this blackbox is created
    pub token_mint: Account<'info, Mint>,

    /// The blackbox account for this token
    #[account(
        init,
        payer = payer,
        space = 8 + 32 + 8, // discriminator + token_mint + mapping_account_count
        seeds = [b"blackbox", token_mint.key().as_ref()],
        bump
    )]
    pub blackbox: Account<'info, BlackboxAccount>,

    /// The vault (account that holds tokens) for this blackbox 
    #[account(
        init,
        payer = authority,
        seeds = [b"vault".as_ref(), blackbox.key().as_ref()],
        bump,
        token::authority = blackbox,
        token::mint = token_mint,
    )]
    pub vault: Account<'info, TokenAccount>,
    
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

/// Accounts for initializing computation definitions
#[derive(Accounts)]
pub struct InitComputationDefinitions<'info> {
    pub arcium_program: Program<'info, Arcium>,
    
    #[account(
        mut,
        seeds = [COMP_DEF_PDA_SEED, &COMP_DEF_OFFSET_DEPOSIT.to_le_bytes()],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub comp_def_deposit: Account<'info, ComputationDefinitionAccount>,
    
    #[account(
        mut,
        seeds = [COMP_DEF_PDA_SEED, &COMP_DEF_OFFSET_TRANSFER.to_le_bytes()],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub comp_def_transfer: Account<'info, ComputationDefinitionAccount>,
    
    #[account(
        mut,
        seeds = [COMP_DEF_PDA_SEED, &COMP_DEF_OFFSET_WITHDRAW.to_le_bytes()],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub comp_def_withdraw: Account<'info, ComputationDefinitionAccount>,
    
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

/// Accounts for initializing a new mapping account
#[derive(Accounts)]
pub struct InitializeMappingAccount<'info> {
    #[account(
        mut,
        seeds = [b"blackbox", blackbox.token_mint.as_ref()],
        bump
    )]
    pub blackbox: Account<'info, BlackboxAccount>,
    
    #[account(
        init,
        payer = payer,
        space = 8 + 8 + 32 + 4 + (MAX_ENTRIES_PER_ACCOUNT * 32) + 4 + (MAX_ENTRIES_PER_ACCOUNT * 32),
        seeds = [
            b"mapping",
            blackbox.token_mint.as_ref(),
            &[blackbox.mapping_account_count]
        ],
        bump
    )]
    pub mapping_account: Account<'info, MappingAccount>,
    
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

/// Accounts for the deposit instruction.
///
/// Uses mapping accounts to store encrypted pubkeys and balances.
#[derive(Accounts)]
#[callback_accounts]
pub struct Deposit<'info> {
    #[account(
        seeds = [b"blackbox", blackbox.token_mint.as_ref()],
        bump
    )]
    pub blackbox: Account<'info, BlackboxAccount>,
    
    #[account(
        mut,
        constraint = mapping_accounts.iter().all(|account| account.token_mint == blackbox.token_mint)
    )]
    pub mapping_accounts: Vec<Account<'info, MappingAccount>>,
    
    #[account(mut)]
    pub user_token_account: Account<'info, TokenAccount>,
    
    #[account(
        mut,
        seeds = [b"vault".as_ref(), blackbox.key().as_ref()],
        bump,
        constraint = vault.key() == blackbox.vault @ ErrorCode::InvalidVault
    )]
    pub vault: Account<'info, TokenAccount>,
    
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    
    // Arcium accounts for computation
    #[account(address = ARCIUM_PROG_ID)]
    pub arcium_program: Program<'info, Arcium>,
    
    #[account(
        seeds = [CLOCK_PDA_SEED],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub clock: Account<'info, ClockAccount>,
    
    #[account(
        seeds = [CLUSTER_PDA_SEED],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub cluster: Account<'info, Cluster>,
    
    #[account(
        seeds = [COMP_DEF_PDA_SEED, &COMP_DEF_OFFSET_DEPOSIT.to_le_bytes()],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub comp_def: Account<'info, ComputationDefinitionAccount>,
    
    #[account(
        mut,
        seeds = [MEMPOOL_PDA_SEED],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub mempool: Account<'info, Mempool>,
    
    #[account(
        mut,
        seeds = [MXE_PDA_SEED],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub mxe: Account<'info, PersistentMXEAccount>,
    
    #[account(
        seeds = [POOL_PDA_SEED],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub pool: Account<'info, StakingPoolAccount>,
    
    #[account(mut)]
    pub payer: Signer<'info>,
}

/// Accounts for the internal transfer instruction.
///
/// Uses mapping accounts to store encrypted pubkeys and balances.
#[derive(Accounts)]
#[callback_accounts]
pub struct Transfer<'info> {
    #[account(
        seeds = [b"blackbox", blackbox.token_mint.as_ref()],
        bump
    )]
    pub blackbox: Account<'info, BlackboxAccount>,
    
    #[account(
        mut,
        constraint = mapping_accounts.iter().all(|account| account.token_mint == blackbox.token_mint)
    )]
    pub mapping_accounts: Vec<Account<'info, MappingAccount>>,
    
    pub sender: Signer<'info>,
    pub system_program: Program<'info, System>,
    
    // Arcium accounts for computation
    #[account(address = ARCIUM_PROG_ID)]
    pub arcium_program: Program<'info, Arcium>,
    
    #[account(
        seeds = [CLOCK_PDA_SEED],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub clock: Account<'info, ClockAccount>,
    
    #[account(
        seeds = [CLUSTER_PDA_SEED],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub cluster: Account<'info, Cluster>,
    
    #[account(
        seeds = [COMP_DEF_PDA_SEED, &COMP_DEF_OFFSET_TRANSFER.to_le_bytes()],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub comp_def: Account<'info, ComputationDefinitionAccount>,
    
    #[account(
        mut,
        seeds = [MEMPOOL_PDA_SEED],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub mempool: Account<'info, Mempool>,
    
    #[account(
        mut,
        seeds = [MXE_PDA_SEED],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub mxe: Account<'info, PersistentMXEAccount>,
    
    #[account(
        seeds = [POOL_PDA_SEED],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub pool: Account<'info, StakingPoolAccount>,
    
    #[account(mut)]
    pub payer: Signer<'info>,
}

/// Accounts for the withdrawal instruction.
///
/// Uses mapping accounts to store encrypted pubkeys and balances.
#[derive(Accounts)]
#[callback_accounts]
pub struct Withdraw<'info> {
    #[account(
        seeds = [b"blackbox", blackbox.token_mint.as_ref()],
        bump
    )]
    pub blackbox: Account<'info, BlackboxAccount>,
    
    #[account(
        mut,
        constraint = mapping_accounts.iter().all(|account| account.token_mint == blackbox.token_mint)
    )]
    pub mapping_accounts: Vec<Account<'info, MappingAccount>>,
    
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
    
    // Arcium accounts for computation
    #[account(address = ARCIUM_PROG_ID)]
    pub arcium_program: Program<'info, Arcium>,
    
    #[account(
        seeds = [CLOCK_PDA_SEED],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub clock: Account<'info, ClockAccount>,
    
    #[account(
        seeds = [CLUSTER_PDA_SEED],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub cluster: Account<'info, Cluster>,
    
    #[account(
        seeds = [COMP_DEF_PDA_SEED, &COMP_DEF_OFFSET_WITHDRAW.to_le_bytes()],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub comp_def: Account<'info, ComputationDefinitionAccount>,
    
    #[account(
        mut,
        seeds = [MEMPOOL_PDA_SEED],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub mempool: Account<'info, Mempool>,
    
    #[account(
        mut,
        seeds = [MXE_PDA_SEED],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub mxe: Account<'info, PersistentMXEAccount>,
    
    #[account(
        seeds = [POOL_PDA_SEED],
        seeds::program = ARCIUM_PROG_ID,
        bump,
    )]
    pub pool: Account<'info, StakingPoolAccount>,
    
    #[account(mut)]
    pub payer: Signer<'info>,
}

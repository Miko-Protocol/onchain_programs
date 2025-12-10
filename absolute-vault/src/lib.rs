use anchor_lang::prelude::*;
use anchor_lang::solana_program::program::invoke_signed;
use anchor_spl::token_2022::{self, Token2022};
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};
use spl_token_2022::{
    extension::{
        transfer_fee::{
            instruction::{
                harvest_withheld_tokens_to_mint, withdraw_withheld_tokens_from_accounts,
                withdraw_withheld_tokens_from_mint,
            },
            TransferFeeConfig,
        },
        BaseStateWithExtensions, StateWithExtensions,
    },
    state::Mint as MintState,
};

// Program ID is dynamically generated from keypair at compile time
declare_id!("7eJ8NmvhSDcmqCP1HhSjuzvAacTWRqyFDjHoQXzHHghS");

pub const VAULT_SEED: &[u8] = b"vault";
pub const POOL_REGISTRY_SEED: &[u8] = b"pool_registry";
pub const MAX_EXCLUSIONS: usize = 100;
pub const MAX_POOLS: usize = 50;
pub const HARVEST_THRESHOLD: u64 = 100_000_000_000_000; // 500k MIKO with 9 decimals (100k for
                                                        // test)
pub const OWNER_TAX_SHARE: u64 = 25; // 25% to owner
pub const HOLDERS_TAX_SHARE: u64 = 75; // 75% to holders

#[program]
pub mod absolute_vault {
    use super::*;

    /// Initialize vault with separate authority and keeper_authority
    pub fn initialize(
        ctx: Context<Initialize>,
        owner_wallet: Pubkey,
        keeper_authority: Pubkey,
    ) -> Result<()> {
        let vault = &mut ctx.accounts.vault;

        vault.authority = ctx.accounts.authority.key();
        vault.keeper_authority = keeper_authority; // MUST be different from authority
        vault.owner_wallet = owner_wallet;
        vault.token_mint = ctx.accounts.token_mint.key();
        vault.reward_exclusions = vec![
            ctx.accounts.authority.key(),
            keeper_authority,
            owner_wallet,
            vault.key(),
            ctx.accounts.vault_program.key(),
        ];
        vault.harvest_threshold = HARVEST_THRESHOLD;
        vault.total_fees_harvested = 0;
        vault.total_rewards_distributed = 0;
        vault.pending_withheld = 0;
        vault.last_harvest_time = 0;
        vault.last_distribution_time = 0;
        vault.launch_timestamp = 0;
        vault.distribution_id = 0;

        msg!("Vault initialized");
        msg!("Authority: {}", vault.authority);
        msg!("Keeper Authority: {}", vault.keeper_authority);
        msg!("Owner Wallet: {}", vault.owner_wallet);

        Ok(())
    }

    /// Initialize pool registry
    pub fn initialize_pool_registry(ctx: Context<InitializePoolRegistry>) -> Result<()> {
        let registry = &mut ctx.accounts.pool_registry;

        registry.vault = ctx.accounts.vault.key();
        registry.pools = Vec::new();

        msg!("Pool registry initialized");
        Ok(())
    }

    /// Set launch time (one-time only, permissionless)
    pub fn set_launch_time(ctx: Context<SetLaunchTime>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;

        require!(
            vault.launch_timestamp == 0,
            VaultError::LaunchTimeAlreadySet
        );

        vault.launch_timestamp = Clock::get()?.unix_timestamp;

        msg!("Launch time set: {}", vault.launch_timestamp);
        Ok(())
    }

    /// Update pool registry with detected pools (keeper only)
    pub fn update_pool_registry(
        ctx: Context<UpdatePoolRegistry>,
        pools_to_add: Vec<Pubkey>,
    ) -> Result<()> {
        let registry = &mut ctx.accounts.pool_registry;

        for pool in pools_to_add {
            if !registry.pools.contains(&pool) && registry.pools.len() < MAX_POOLS {
                registry.pools.push(pool);
            }
        }

        msg!(
            "Pool registry updated. Total pools: {}",
            registry.pools.len()
        );
        Ok(())
    }

    /// Harvest fees from token accounts to mint (keeper only)
    pub fn harvest_fees<'info>(
        ctx: Context<'_, '_, '_, 'info, HarvestFees<'info>>,
        accounts: Vec<Pubkey>,
    ) -> Result<()> {
        let vault = &mut ctx.accounts.vault;

        require!(
            accounts.len() > 0 && accounts.len() <= 20,
            VaultError::InvalidBatchSize
        );

        let token_mint_info = &ctx.accounts.token_mint;
        let mint_data = token_mint_info.try_borrow_data()?;
        let mint_state = StateWithExtensions::<MintState>::unpack(&mint_data)?;
        let transfer_fee_extension = mint_state.get_extension::<TransferFeeConfig>()?;
        let accumulated_fees = u64::from(transfer_fee_extension.withheld_amount);
        drop(mint_data);

        require!(
            accumulated_fees >= vault.harvest_threshold,
            VaultError::HarvestThresholdNotMet
        );

        // Build harvest instruction
        let seeds = &[VAULT_SEED, vault.token_mint.as_ref(), &[ctx.bumps.vault]];
        let signer_seeds = &[&seeds[..]];

        let account_refs: Vec<&Pubkey> = accounts.iter().collect();

        let ix = harvest_withheld_tokens_to_mint(
            &ctx.accounts.token_program.key(),
            &ctx.accounts.token_mint.key(),
            &account_refs,
        )?;

        // Collect all account infos
        let mut account_infos = vec![ctx.accounts.token_mint.to_account_info()];
        account_infos.extend(ctx.remaining_accounts.iter().cloned());

        invoke_signed(&ix, &account_infos, signer_seeds)?;

        vault.last_harvest_time = Clock::get()?.unix_timestamp;

        msg!("Harvested fees from {} accounts", accounts.len());

        Ok(())
    }

    /// Withdraw fees from mint to vault PDA (keeper only)
    pub fn withdraw_fees_from_mint(ctx: Context<WithdrawFeesFromMint>) -> Result<()> {
        // Get current vault balance before withdrawal
        let vault_balance_before = ctx.accounts.vault_token_account.amount;

        let vault_key = ctx.accounts.vault.key();
        let token_mint_key = ctx.accounts.vault.token_mint;

        let seeds = &[VAULT_SEED, token_mint_key.as_ref(), &[ctx.bumps.vault]];
        let signer_seeds = &[&seeds[..]];

        let ix = withdraw_withheld_tokens_from_mint(
            &ctx.accounts.token_program.key(),
            &ctx.accounts.token_mint.key(),
            &ctx.accounts.vault_token_account.key(),
            &vault_key,
            &[],
        )?;

        invoke_signed(
            &ix,
            &[
                ctx.accounts.token_mint.to_account_info(),
                ctx.accounts.vault_token_account.to_account_info(),
                ctx.accounts.vault.to_account_info(),
            ],
            signer_seeds,
        )?;

        // Reload account to get updated balance
        ctx.accounts.vault_token_account.reload()?;
        let vault_balance_after = ctx.accounts.vault_token_account.amount;

        // Calculate withdrawn amount
        let withdrawn_amount = vault_balance_after.saturating_sub(vault_balance_before);

        // Update vault state
        let vault = &mut ctx.accounts.vault;
        vault.total_fees_harvested = vault.total_fees_harvested.saturating_add(withdrawn_amount);
        vault.last_harvest_amount = withdrawn_amount;
        vault.last_harvest_time = Clock::get()?.unix_timestamp;

        msg!("Withdrew {} fees from mint to vault", withdrawn_amount);

        Ok(())
    }

    /// Withdraw harvested fees and report distribution plan (keeper only)
    pub fn withdraw_and_report_distribution_plan(
        ctx: Context<WithdrawAndReport>,
        amount_to_withdraw: u64,
        expected_minimum_reward_amount: u64,
        distribution_hash: [u8; 32],
    ) -> Result<()> {
        let clock = Clock::get()?;

        let vault_balance = ctx.accounts.vault_token_account.amount;
        require!(
            amount_to_withdraw == vault_balance,
            VaultError::MustWithdrawFullAmount
        );

        let mint_data = ctx.accounts.token_mint.to_account_info();
        let mint_data_borrowed = mint_data.try_borrow_data()?;
        let mint_info = StateWithExtensions::<MintState>::unpack(&mint_data_borrowed)?;
        let decimals = mint_info.base.decimals;
        drop(mint_data_borrowed);

        let token_mint_key = ctx.accounts.vault.token_mint;
        let seeds = &[VAULT_SEED, token_mint_key.as_ref(), &[ctx.bumps.vault]];
        let signer_seeds = &[&seeds[..]];

        let keeper = ctx.accounts.keeper_authority.key();

        let distribution_id;
        {
            let vault = &mut ctx.accounts.vault;
            vault.distribution_id = vault.distribution_id.saturating_add(1);
            distribution_id = vault.distribution_id;
            vault.last_distribution_time = clock.unix_timestamp;
            vault.total_rewards_distributed = vault
                .total_rewards_distributed
                .saturating_add(amount_to_withdraw);
        }

        // Emit auditable report before transferring funds
        emit!(DistributionPlanReport {
            timestamp: clock.unix_timestamp,
            distribution_id,
            withdrawn_miko_amount: amount_to_withdraw,
            expected_minimum_reward_amount,
            distribution_hash,
            keeper,
        });

        // Transfer harvested fees to keeper
        token_2022::transfer_checked(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token_2022::TransferChecked {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    mint: ctx.accounts.token_mint.to_account_info(),
                    to: ctx.accounts.keeper_token_account.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                signer_seeds,
            ),
            amount_to_withdraw,
            decimals,
        )?;

        msg!("Withdraw and report distribution plan executed");

        Ok(())
    }

    /// Log keeper work on-chain (keeper only)
    /// Records swap and distribution activities for transparency
    pub fn log_keeper_work(
        ctx: Context<LogKeeperWork>,
        work_type: KeeperWorkType,
        amount: u64,
        details: String,
    ) -> Result<()> {
        let log = &mut ctx.accounts.keeper_work_log;

        // Initialize if new
        if log.vault == Pubkey::default() {
            log.vault = ctx.accounts.vault.key();
            log.entries = Vec::new();
        }

        // Add new entry (keep last 50 entries)
        log.entries.push(KeeperWorkEntry {
            timestamp: Clock::get()?.unix_timestamp,
            work_type,
            amount,
            details: details.chars().take(100).collect(), // Limit details to 100 chars
        });

        if log.entries.len() > 50 {
            log.entries.remove(0);
        }

        msg!("Logged keeper work: {:?}", work_type);

        Ok(())
    }

    /// Manage reward exclusions (admin only)
    pub fn manage_exclusions(
        ctx: Context<ManageExclusions>,
        action: ExclusionAction,
        wallet: Pubkey,
    ) -> Result<()> {
        let vault = &mut ctx.accounts.vault;

        match action {
            ExclusionAction::Add => {
                require!(
                    vault.reward_exclusions.len() < MAX_EXCLUSIONS,
                    VaultError::ExclusionListFull
                );
                require!(
                    !vault.reward_exclusions.contains(&wallet),
                    VaultError::AlreadyExcluded
                );
                vault.reward_exclusions.push(wallet);
                msg!("Added {} to reward exclusions", wallet);
            }
            ExclusionAction::Remove => {
                vault.reward_exclusions.retain(|&x| x != wallet);
                msg!("Removed {} from reward exclusions", wallet);
            }
        }

        Ok(())
    }

    /// Update vault configuration (admin only)
    pub fn update_config(
        ctx: Context<UpdateConfig>,
        new_owner_wallet: Option<Pubkey>,
        new_harvest_threshold: Option<u64>,
        new_authority: Option<Pubkey>,
        new_keeper_authority: Option<Pubkey>,
    ) -> Result<()> {
        let vault = &mut ctx.accounts.vault;

        if let Some(owner) = new_owner_wallet {
            vault.owner_wallet = owner;
        }
        if let Some(threshold) = new_harvest_threshold {
            vault.harvest_threshold = threshold;
        }
        if let Some(authority) = new_authority {
            vault.authority = authority;
        }
        if let Some(keeper) = new_keeper_authority {
            vault.keeper_authority = keeper;
        }

        msg!("Vault configuration updated");
        Ok(())
    }

    /// Emergency withdraw from vault (admin only)
    pub fn emergency_withdraw_vault(ctx: Context<EmergencyWithdraw>, amount: u64) -> Result<()> {
        let vault = &ctx.accounts.vault;

        let seeds = &[VAULT_SEED, vault.token_mint.as_ref(), &[ctx.bumps.vault]];
        let signer_seeds = &[&seeds[..]];

        // Get mint decimals
        let mint_data = ctx.accounts.token_mint.to_account_info();
        let mint_data_borrowed = mint_data.try_borrow_data()?;
        let mint_info = StateWithExtensions::<MintState>::unpack(&mint_data_borrowed)?;
        let decimals = mint_info.base.decimals;
        drop(mint_data_borrowed);

        token_2022::transfer_checked(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token_2022::TransferChecked {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    mint: ctx.accounts.token_mint.to_account_info(),
                    to: ctx.accounts.destination_token_account.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                signer_seeds,
            ),
            amount,
            decimals,
        )?;

        msg!("Emergency withdrawal: {} tokens", amount);

        Ok(())
    }

    /// Emergency withdraw withheld from mint (admin only)
    pub fn emergency_withdraw_withheld<'info>(
        ctx: Context<'_, '_, '_, 'info, EmergencyWithdrawWithheld<'info>>,
        accounts: Vec<Pubkey>,
    ) -> Result<()> {
        let vault = &ctx.accounts.vault;

        require!(
            accounts.len() > 0 && accounts.len() <= 20,
            VaultError::InvalidBatchSize
        );

        let seeds = &[VAULT_SEED, vault.token_mint.as_ref(), &[ctx.bumps.vault]];
        let signer_seeds = &[&seeds[..]];

        let account_refs: Vec<&Pubkey> = accounts.iter().collect();

        let ix = withdraw_withheld_tokens_from_accounts(
            &ctx.accounts.token_program.key(),
            &ctx.accounts.token_mint.key(),
            &ctx.accounts.destination_token_account.key(),
            &vault.key(),
            &account_refs,
            &[],
        )?;

        let mut account_infos = vec![
            ctx.accounts.token_mint.to_account_info(),
            ctx.accounts.destination_token_account.to_account_info(),
            ctx.accounts.vault.to_account_info(),
        ];
        account_infos.extend(ctx.remaining_accounts.iter().cloned());

        invoke_signed(&ix, &account_infos, signer_seeds)?;

        msg!(
            "Emergency withdrawal of withheld fees from {} accounts",
            accounts.len()
        );

        Ok(())
    }
}

#[event]
pub struct DistributionPlanReport {
    pub timestamp: i64,
    pub distribution_id: u64,
    pub withdrawn_miko_amount: u64,
    pub expected_minimum_reward_amount: u64,
    pub distribution_hash: [u8; 32],
    pub keeper: Pubkey,
}

// Account structures

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = payer,
        space = 8 + VaultState::INIT_SPACE,
        seeds = [VAULT_SEED, token_mint.key().as_ref()],
        bump
    )]
    pub vault: Account<'info, VaultState>,

    pub authority: Signer<'info>,

    /// CHECK: Token mint
    pub token_mint: UncheckedAccount<'info>,

    /// CHECK: Vault program ID
    pub vault_program: UncheckedAccount<'info>,

    #[account(mut)]
    pub payer: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct InitializePoolRegistry<'info> {
    #[account(
        init,
        payer = payer,
        space = 8 + PoolRegistry::INIT_SPACE,
        seeds = [POOL_REGISTRY_SEED, vault.key().as_ref()],
        bump
    )]
    pub pool_registry: Account<'info, PoolRegistry>,

    pub vault: Account<'info, VaultState>,

    #[account(mut)]
    pub payer: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SetLaunchTime<'info> {
    #[account(mut)]
    pub vault: Account<'info, VaultState>,
}

#[derive(Accounts)]
pub struct UpdatePoolRegistry<'info> {
    #[account(
        mut,
        seeds = [POOL_REGISTRY_SEED, vault.key().as_ref()],
        bump
    )]
    pub pool_registry: Account<'info, PoolRegistry>,

    #[account(
        constraint = vault.keeper_authority == keeper_authority.key() @ VaultError::Unauthorized
    )]
    pub vault: Account<'info, VaultState>,

    pub keeper_authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct HarvestFees<'info> {
    #[account(
        mut,
        seeds = [VAULT_SEED, vault.token_mint.as_ref()],
        bump,
        constraint = vault.keeper_authority == keeper_authority.key() @ VaultError::Unauthorized
    )]
    pub vault: Account<'info, VaultState>,

    pub keeper_authority: Signer<'info>,

    /// CHECK: Token mint
    #[account(mut)]
    pub token_mint: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token2022>,
}

#[derive(Accounts)]
pub struct WithdrawFeesFromMint<'info> {
    #[account(
        mut,
        seeds = [VAULT_SEED, vault.token_mint.as_ref()],
        bump,
        constraint = vault.keeper_authority == keeper_authority.key() @ VaultError::Unauthorized
    )]
    pub vault: Account<'info, VaultState>,

    pub keeper_authority: Signer<'info>,

    /// CHECK: Token mint
    #[account(mut)]
    pub token_mint: UncheckedAccount<'info>,

    #[account(mut)]
    pub vault_token_account: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Program<'info, Token2022>,
}

#[derive(Accounts)]
pub struct WithdrawAndReport<'info> {
    #[account(
        mut,
        seeds = [VAULT_SEED, vault.token_mint.as_ref()],
        bump,
        constraint = vault.keeper_authority == keeper_authority.key() @ VaultError::Unauthorized
    )]
    pub vault: Account<'info, VaultState>,

    pub keeper_authority: Signer<'info>,

    /// CHECK: Token mint
    pub token_mint: InterfaceAccount<'info, Mint>,

    #[account(mut)]
    pub vault_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(mut)]
    pub keeper_token_account: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
}

#[derive(Accounts)]
pub struct LogKeeperWork<'info> {
    #[account(
        init_if_needed,
        payer = keeper_authority,
        seeds = [b"keeper_log", vault.key().as_ref()],
        bump,
        space = 8 + KeeperWorkLog::INIT_SPACE
    )]
    pub keeper_work_log: Account<'info, KeeperWorkLog>,

    #[account(
        constraint = vault.keeper_authority == keeper_authority.key() @ VaultError::Unauthorized
    )]
    pub vault: Account<'info, VaultState>,

    #[account(mut)]
    pub keeper_authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ManageExclusions<'info> {
    #[account(
        mut,
        constraint = vault.authority == authority.key() @ VaultError::Unauthorized
    )]
    pub vault: Account<'info, VaultState>,

    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    #[account(
        mut,
        constraint = vault.authority == authority.key() @ VaultError::Unauthorized
    )]
    pub vault: Account<'info, VaultState>,

    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct EmergencyWithdraw<'info> {
    #[account(
        mut,
        seeds = [VAULT_SEED, vault.token_mint.as_ref()],
        bump,
        constraint = vault.authority == authority.key() @ VaultError::Unauthorized
    )]
    pub vault: Account<'info, VaultState>,

    pub authority: Signer<'info>,

    #[account(mut)]
    pub vault_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(mut)]
    pub destination_token_account: InterfaceAccount<'info, TokenAccount>,

    /// CHECK: Token mint
    pub token_mint: InterfaceAccount<'info, Mint>,

    pub token_program: Interface<'info, TokenInterface>,
}

#[derive(Accounts)]
pub struct EmergencyWithdrawWithheld<'info> {
    #[account(
        mut,
        seeds = [VAULT_SEED, vault.token_mint.as_ref()],
        bump,
        constraint = vault.authority == authority.key() @ VaultError::Unauthorized
    )]
    pub vault: Account<'info, VaultState>,

    pub authority: Signer<'info>,

    /// CHECK: Token mint
    #[account(mut)]
    pub token_mint: UncheckedAccount<'info>,

    #[account(mut)]
    pub destination_token_account: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Program<'info, Token2022>,
}

// State structures

#[account]
#[derive(InitSpace)]
pub struct VaultState {
    pub authority: Pubkey,
    pub keeper_authority: Pubkey,
    pub owner_wallet: Pubkey,
    pub token_mint: Pubkey,
    #[max_len(100)]
    pub reward_exclusions: Vec<Pubkey>,
    pub harvest_threshold: u64,
    pub total_fees_harvested: u64,
    pub total_rewards_distributed: u64,
    pub distribution_id: u64,
    pub pending_withheld: u64,
    pub last_harvest_time: i64,
    pub last_harvest_amount: u64,
    pub last_distribution_time: i64,
    pub launch_timestamp: i64,
}

#[account]
#[derive(InitSpace)]
pub struct KeeperWorkLog {
    pub vault: Pubkey,
    #[max_len(50)]
    pub entries: Vec<KeeperWorkEntry>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, InitSpace)]
pub struct KeeperWorkEntry {
    pub timestamp: i64,
    pub work_type: KeeperWorkType,
    pub amount: u64,
    #[max_len(100)]
    pub details: String,
}

#[account]
#[derive(InitSpace)]
pub struct PoolRegistry {
    pub vault: Pubkey,
    #[max_len(50)]
    pub pools: Vec<Pubkey>,
}

// Enums

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq)]
pub enum ExclusionAction {
    Add,
    Remove,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
pub enum KeeperWorkType {
    HarvestFees,
    SwapToRewardToken,
    DistributeToOwner,
    DistributeToHolders,
    KeeperTopUp,
}

// Errors

#[error_code]
pub enum VaultError {
    #[msg("Unauthorized")]
    Unauthorized,

    #[msg("Exclusion list full")]
    ExclusionListFull,

    #[msg("Already excluded")]
    AlreadyExcluded,

    #[msg("Invalid batch size")]
    InvalidBatchSize,

    #[msg("Harvest threshold not met")]
    HarvestThresholdNotMet,

    #[msg("Must withdraw full amount")]
    MustWithdrawFullAmount,

    #[msg("Math overflow")]
    MathOverflow,

    #[msg("Invalid distribution split")]
    InvalidDistributionSplit,

    #[msg("Launch time already set")]
    LaunchTimeAlreadySet,
}

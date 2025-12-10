use anchor_lang::prelude::*;

// Program ID is dynamically generated from keypair at compile time
declare_id!("423KiBKFusrnh8QGcmj6rE9ntPWH7FzRQCJ5Z5kNeRmp");

pub const DIAL_STATE_SEED: &[u8] = b"dial_state";
pub const SOL_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");
pub const INITIAL_UPDATE_DELAY: i64 = 24 * 60 * 60; // 24 hours after launch

#[program]
pub mod smart_dial {
    use super::*;

    /// Initialize Smart Dial with launch timestamp
    pub fn initialize(
        ctx: Context<Initialize>,
        launch_timestamp: i64,
    ) -> Result<()> {
        let dial = &mut ctx.accounts.dial_state;
        
        dial.authority = ctx.accounts.authority.key();
        dial.current_reward_token = SOL_MINT; // SOL is default reward token
        dial.last_update = 0;
        dial.update_count = 0;
        dial.launch_timestamp = launch_timestamp;

        // Initialize update history
        dial.update_history = Vec::new();
        
        msg!("Smart Dial initialized");
        msg!("Authority: {}", dial.authority);
        msg!("Initial reward token: SOL");
        msg!("Launch timestamp: {}", launch_timestamp);
        
        Ok(())
    }

    /// Update reward token for the week
    pub fn update_reward_token(
        ctx: Context<UpdateRewardToken>,
        new_reward_token: Pubkey,
        cycle_start: i64,
        next_cycle_start: i64,
    ) -> Result<()> {
        let dial = &mut ctx.accounts.dial_state;
        let current_time = Clock::get()?.unix_timestamp;

        let earliest_update = earliest_update_time(dial.launch_timestamp);

        require!(
            current_time >= earliest_update,
            DialError::TooEarlyToUpdate
        );

        require!(
            cycle_start >= earliest_update,
            DialError::TooEarlyToUpdate
        );

        require!(
            cycle_start > dial.last_update,
            DialError::CycleAlreadyProcessed
        );

        require!(
            current_time >= cycle_start,
            DialError::CycleNotReached
        );

        require!(
            next_cycle_start > cycle_start,
            DialError::InvalidNextCycleStart
        );

        // Store in update history
        if dial.update_history.len() >= 52 { // Keep last year of history
            dial.update_history.remove(0);
        }
        
        // Store values before mutable borrow
        let old_token = dial.current_reward_token;
        let update_number = dial.update_count;
        
        dial.update_history.push(UpdateRecord {
            timestamp: current_time,
            old_token,
            new_token: new_reward_token,
            update_number,
        });
        
        // Update reward token
        dial.current_reward_token = new_reward_token;
        dial.last_update = cycle_start;
        dial.update_count += 1;

        msg!("Reward token updated to: {}", new_reward_token);
        msg!("Update count: {}", dial.update_count);
        msg!("Cycle start: {}", cycle_start);
        msg!("Next cycle begins at: {}", next_cycle_start);

        Ok(())
    }

    /// Transfer authority
    pub fn update_authority(
        ctx: Context<UpdateAuthority>,
        new_authority: Pubkey,
    ) -> Result<()> {
        ctx.accounts.dial_state.authority = new_authority;

        msg!("Authority updated to: {}", new_authority);

        Ok(())
    }

    /// Synchronize launch timestamp with external events (authority only)
    pub fn sync_launch_timestamp(
        ctx: Context<SyncLaunchTimestamp>,
        new_launch_timestamp: i64,
    ) -> Result<()> {
        let dial = &mut ctx.accounts.dial_state;

        require!(
            dial.update_count == 0,
            DialError::CannotSyncAfterUpdates
        );

        require!(
            new_launch_timestamp >= dial.launch_timestamp,
            DialError::InvalidLaunchTimestamp
        );

        dial.launch_timestamp = new_launch_timestamp;
        dial.last_update = 0;

        msg!("Launch timestamp synchronized: {}", new_launch_timestamp);

        Ok(())
    }
}

// Helper function to calculate first Monday after launch
fn earliest_update_time(launch_timestamp: i64) -> i64 {
    launch_timestamp + INITIAL_UPDATE_DELAY
}

// Account structures

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = payer,
        space = 8 + DialState::INIT_SPACE,
        seeds = [DIAL_STATE_SEED],
        bump
    )]
    pub dial_state: Account<'info, DialState>,
    
    pub authority: Signer<'info>,
    
    #[account(mut)]
    pub payer: Signer<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateRewardToken<'info> {
    #[account(
        mut,
        seeds = [DIAL_STATE_SEED],
        bump,
        constraint = dial_state.authority == authority.key() @ DialError::Unauthorized
    )]
    pub dial_state: Account<'info, DialState>,
    
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct UpdateAuthority<'info> {
    #[account(
        mut,
        seeds = [DIAL_STATE_SEED],
        bump,
        constraint = dial_state.authority == authority.key() @ DialError::Unauthorized
    )]
    pub dial_state: Account<'info, DialState>,

    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct SyncLaunchTimestamp<'info> {
    #[account(
        mut,
        seeds = [DIAL_STATE_SEED],
        bump,
        constraint = dial_state.authority == authority.key() @ DialError::Unauthorized
    )]
    pub dial_state: Account<'info, DialState>,

    pub authority: Signer<'info>,
}

// State

#[account]
#[derive(InitSpace)]
pub struct DialState {
    pub authority: Pubkey,
    pub current_reward_token: Pubkey,
    pub last_update: i64,
    pub update_count: u64,
    pub launch_timestamp: i64,
    #[max_len(52)] // Keep last year of updates
    pub update_history: Vec<UpdateRecord>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, InitSpace)]
pub struct UpdateRecord {
    pub timestamp: i64,
    pub old_token: Pubkey,
    pub new_token: Pubkey,
    pub update_number: u64,
}

// Errors

#[error_code]
pub enum DialError {
    #[msg("Unauthorized")]
    Unauthorized,

    #[msg("Cycle has already been processed")]
    CycleAlreadyProcessed,

    #[msg("Current cycle start time has not been reached")]
    CycleNotReached,

    #[msg("Invalid next cycle start time")]
    InvalidNextCycleStart,

    #[msg("Cannot update before 24 hours after launch")]
    TooEarlyToUpdate,

    #[msg("Cannot synchronize launch timestamp after updates have occurred")]
    CannotSyncAfterUpdates,

    #[msg("Invalid launch timestamp provided")]
    InvalidLaunchTimestamp,
}

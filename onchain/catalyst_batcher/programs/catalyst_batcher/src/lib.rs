use anchor_lang::prelude::*;

declare_id!("AHxueE1tDdUYEHsrBGhqbdfrzVLehmRX5KbWpPzgcPUF");

#[program]
pub mod catalyst_batcher {
    use super::*;

    pub fn initialize_config(ctx: Context<InitializeConfig>, authority: Pubkey) -> Result<()> {
        let config = &mut ctx.accounts.config;
        config.authority = authority;
        config.paused = false;
        config.batch_count = 0;
        config.policy_name = [0u8; 32];
        msg!("GlobalConfig initialized");
        Ok(())
    }

    pub fn update_policy(ctx: Context<UpdatePolicy>, policy_name: [u8; 32]) -> Result<()> {
        let config = &mut ctx.accounts.config;
        require!(!config.paused, CatalystError::Paused);
        config.policy_name = policy_name;
        msg!("Policy updated");
        Ok(())
    }

    pub fn submit_batch(
        ctx: Context<SubmitBatch>,
        batch_id: [u8; 32],
        batch_hash: [u8; 32],
        tx_count: u32,
        total_compute: u64,
    ) -> Result<()> {
        let config = &mut ctx.accounts.config;
        require!(!config.paused, CatalystError::Paused);

        let receipt = &mut ctx.accounts.receipt;
        receipt.batch_id = batch_id;
        receipt.batch_hash = batch_hash;
        receipt.tx_count = tx_count;
        receipt.total_compute = total_compute;
        receipt.proof_hash = [0u8; 32];
        receipt.verified = false;
        receipt.submitter = ctx.accounts.submitter.key();
        receipt.submit_slot = Clock::get()?.slot;

        config.batch_count += 1;
        msg!("Batch submitted: count={}", config.batch_count);
        Ok(())
    }

    pub fn submit_proof(
        ctx: Context<SubmitProof>,
        proof_hash: [u8; 32],
    ) -> Result<()> {
        let receipt = &mut ctx.accounts.receipt;
        require!(!receipt.verified, CatalystError::AlreadyVerified);
        receipt.proof_hash = proof_hash;
        msg!("Proof submitted for batch");
        Ok(())
    }

    pub fn verify_proof(ctx: Context<VerifyProof>) -> Result<()> {
        let receipt = &mut ctx.accounts.receipt;
        require!(!receipt.verified, CatalystError::AlreadyVerified);
        require!(receipt.proof_hash != [0u8; 32], CatalystError::NoProof);
        receipt.verified = true;
        receipt.verify_slot = Clock::get()?.slot;
        msg!("Proof verified for batch");
        Ok(())
    }

    pub fn pause(ctx: Context<Pause>) -> Result<()> {
        ctx.accounts.config.paused = true;
        msg!("System paused");
        Ok(())
    }

    pub fn resume(ctx: Context<Resume>) -> Result<()> {
        ctx.accounts.config.paused = false;
        msg!("System resumed");
        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(
        init,
        payer = payer,
        space = 8 + GlobalConfig::INIT_SPACE,
        seeds = [b"global_config"],
        bump
    )]
    pub config: Account<'info, GlobalConfig>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdatePolicy<'info> {
    #[account(
        mut,
        seeds = [b"global_config"],
        bump,
        has_one = authority
    )]
    pub config: Account<'info, GlobalConfig>,
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
#[instruction(batch_id: [u8; 32])]
pub struct SubmitBatch<'info> {
    #[account(
        mut,
        seeds = [b"global_config"],
        bump
    )]
    pub config: Account<'info, GlobalConfig>,
    #[account(
        init,
        payer = submitter,
        space = 8 + BatchReceipt::INIT_SPACE,
        seeds = [b"batch_receipt", batch_id.as_ref()],
        bump
    )]
    pub receipt: Account<'info, BatchReceipt>,
    #[account(mut)]
    pub submitter: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SubmitProof<'info> {
    #[account(mut)]
    pub receipt: Account<'info, BatchReceipt>,
    pub submitter: Signer<'info>,
}

#[derive(Accounts)]
pub struct VerifyProof<'info> {
    #[account(mut)]
    pub receipt: Account<'info, BatchReceipt>,
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct Pause<'info> {
    #[account(
        mut,
        seeds = [b"global_config"],
        bump,
        has_one = authority
    )]
    pub config: Account<'info, GlobalConfig>,
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct Resume<'info> {
    #[account(
        mut,
        seeds = [b"global_config"],
        bump,
        has_one = authority
    )]
    pub config: Account<'info, GlobalConfig>,
    pub authority: Signer<'info>,
}

#[account]
#[derive(InitSpace)]
pub struct GlobalConfig {
    pub authority: Pubkey,
    pub paused: bool,
    pub batch_count: u64,
    #[max_len(32)]
    pub policy_name: [u8; 32],
}

#[account]
#[derive(InitSpace)]
pub struct BatchReceipt {
    pub batch_id: [u8; 32],
    pub batch_hash: [u8; 32],
    pub proof_hash: [u8; 32],
    pub tx_count: u32,
    pub total_compute: u64,
    pub verified: bool,
    pub submitter: Pubkey,
    pub submit_slot: u64,
    pub verify_slot: u64,
}

#[error_code]
pub enum CatalystError {
    #[msg("System is paused")]
    Paused,
    #[msg("Batch already verified")]
    AlreadyVerified,
    #[msg("No proof submitted")]
    NoProof,
}

#![no_std]

use quasar_lang::prelude::*;

mod instructions;
use instructions::*;
mod state;
pub use state::*;
pub mod utils;

#[cfg(test)]
mod tests;

declare_id!("msigVi8dMnmLQUuCbakipEMZhzen516QRHxGz7iX5Xv");

#[program]
pub mod clear_wallet {
    use super::*;

    #[instruction(discriminator = 0)]
    pub fn create_wallet(
        ctx: Ctx<CreateWallet>,
        approval_threshold: u8,
        cancellation_threshold: u8,
        timelock_seconds: u32,
        name: String<64>,
        proposers: Vec<[u8; 32], 16>,
        approvers: Vec<[u8; 32], 16>,
    ) -> Result<(), ProgramError> {
        ctx.accounts.create(
            CreateWalletArgs {
                name,
                approval_threshold,
                cancellation_threshold,
                timelock_seconds,
                proposers,
                approvers,
            },
            &ctx.bumps,
        )
    }

    #[instruction(discriminator = 1)]
    pub fn propose(
        ctx: Ctx<Propose>,
        proposal_index: u64,
        expiry: i64,
        proposer_pubkey: [u8; 32],
        signature: [u8; 64],
        params_data: &[u8],
    ) -> Result<(), ProgramError> {
        ctx.accounts.propose(proposal_index, ProposeArgs {
            expiry, proposer_pubkey: &proposer_pubkey, signature: &signature, params_data,
        }, &ctx.bumps)
    }

    #[instruction(discriminator = 2)]
    pub fn approve(
        ctx: Ctx<Approve>,
        expiry: i64,
        approver_index: u8,
        signature: [u8; 64],
    ) -> Result<(), ProgramError> {
        ctx.accounts.approve(ApproveArgs {
            expiry,
            approver_index,
            signature: &signature,
        })
    }

    #[instruction(discriminator = 3)]
    pub fn cancel(
        ctx: Ctx<Cancel>,
        expiry: i64,
        canceller_index: u8,
        signature: [u8; 64],
    ) -> Result<(), ProgramError> {
        ctx.accounts.cancel(CancelArgs {
            expiry,
            canceller_index,
            signature: &signature,
        })
    }

    #[instruction(discriminator = 4)]
    pub fn execute(
        ctx: CtxWithRemaining<Execute>,
    ) -> Result<(), ProgramError> {
        ctx.accounts.execute(&ctx.bumps, ctx.remaining_accounts_passthrough())
    }

    #[instruction(discriminator = 5)]
    pub fn cleanup_proposal(ctx: Ctx<CleanupProposal>) -> Result<(), ProgramError> {
        ctx.accounts.cleanup()
    }
}

use {
    crate::state::{
        intent::Intent,
        proposal::{Proposal, ProposalStatus},
        wallet::ClearWallet,
    },
    crate::utils::message::{MessageBuilder, MessageContext},
    quasar_lang::{prelude::*, sysvars::Sysvar as _},
};

#[derive(Accounts)]
pub struct Approve<'info> {
    pub wallet: Account<ClearWallet<'info>>,
    #[account(mut)]
    pub intent: Account<Intent<'info>>,
    #[account(
        mut,
        has_one = wallet,
        has_one = intent,
        constraint = proposal.status == ProposalStatus::Active @ ProgramError::InvalidArgument
    )]
    pub proposal: Account<Proposal<'info>>,
}

pub struct ApproveArgs<'a> {
    pub expiry: i64,
    pub approver_index: u8,
    pub signature: &'a [u8; 64],
}

impl<'info> Approve<'info> {
    pub fn approve(&mut self, args: ApproveArgs<'_>) -> Result<(), ProgramError> {
        let clock = Clock::get()?;
        require!(args.expiry > clock.unix_timestamp.get(), ProgramError::InvalidArgument);

        let approvers = self.intent.approvers();
        let approver_addr = approvers.get(args.approver_index as usize)
            .ok_or(ProgramError::InvalidArgument)?;

        require!(!self.proposal.has_approved_by_index(args.approver_index), ProgramError::InvalidArgument);

        let mut msg_buf = MessageBuilder::new();
        msg_buf.build_message_for_intent(
            &MessageContext {
                expiry: args.expiry, action: "approve",
                wallet_name: self.wallet.name(),
                proposal_index: self.proposal.proposal_index.get(),
            },
            &self.intent,
            self.proposal.params_data(),
        )?;

        brine_ed25519::sig_verify(approver_addr.as_ref(), args.signature, msg_buf.as_bytes())
            .map_err(|_| ProgramError::InvalidArgument)?;

        self.proposal.set_approval(args.approver_index);
        if self.proposal.approval_count() >= self.intent.approval_threshold {
            self.proposal.status = ProposalStatus::Approved;
            self.proposal.approved_at = PodI64::from(clock.unix_timestamp.get());
        }
        Ok(())
    }
}

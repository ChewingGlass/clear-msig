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
pub struct Cancel<'info> {
    pub wallet: Account<ClearWallet<'info>>,
    #[account(mut)]
    pub intent: Account<Intent<'info>>,
    #[account(
        mut,
        has_one = wallet,
        has_one = intent,
        constraint = proposal.status == ProposalStatus::Active
            || proposal.status == ProposalStatus::Approved
            @ ProgramError::InvalidArgument
    )]
    pub proposal: Account<Proposal<'info>>,
}

pub struct CancelArgs<'a> {
    pub expiry: i64,
    pub canceller_index: u8,
    pub signature: &'a [u8; 64],
}

impl<'info> Cancel<'info> {
    pub fn cancel(&mut self, args: CancelArgs<'_>) -> Result<(), ProgramError> {
        let clock = Clock::get()?;
        require!(args.expiry > clock.unix_timestamp.get(), ProgramError::InvalidArgument);

        let approvers = self.intent.approvers();
        let canceller_addr = approvers.get(args.canceller_index as usize)
            .ok_or(ProgramError::InvalidArgument)?;

        require!(!self.proposal.has_cancelled_by_index(args.canceller_index), ProgramError::InvalidArgument);

        let mut msg_buf = MessageBuilder::new();
        msg_buf.build_message_for_intent(
            &MessageContext {
                expiry: args.expiry, action: "cancel",
                wallet_name: self.wallet.name(),
                proposal_index: self.proposal.proposal_index.get(),
            },
            &self.intent,
            self.proposal.params_data(),
        )?;

        brine_ed25519::sig_verify(canceller_addr.as_ref(), args.signature, msg_buf.as_bytes())
            .map_err(|_| ProgramError::InvalidArgument)?;

        self.proposal.set_cancellation(args.canceller_index);

        if self.proposal.cancellation_count() >= self.intent.cancellation_threshold {
            self.proposal.status = ProposalStatus::Cancelled;
            self.intent.active_proposal_count = PodU16::from(self.intent.active_proposal_count.get().saturating_sub(1));
        } else if self.proposal.status == ProposalStatus::Approved
            && self.proposal.approval_count() < self.intent.approval_threshold
        {
            self.proposal.status = ProposalStatus::Active;
        }

        Ok(())
    }
}

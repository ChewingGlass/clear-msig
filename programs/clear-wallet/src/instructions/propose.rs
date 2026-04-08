use quasar_lang::{cpi::Seed, prelude::*, sysvars::Sysvar as _};

use crate::{
    state::{
        intent::Intent,
        proposal::ProposalStatus,
        wallet::ClearWallet,
    },
    utils::message::{MessageBuilder, MessageContext},
};

#[derive(Accounts)]
pub struct Propose<'info> {
    pub payer: &'info mut Signer,
    #[account(mut)]
    pub wallet: Account<ClearWallet<'info>>,
    #[account(
        mut,
        has_one = wallet,
        constraint = intent.is_approved() @ ProgramError::InvalidArgument,
    )]
    pub intent: Account<Intent<'info>>,
    #[account(mut)]
    pub proposal: &'info mut UncheckedAccount,
    pub system_program: &'info Program<System>,
}

pub struct ProposeArgs<'a> {
    pub expiry: i64,
    pub proposer_pubkey: &'a [u8; 32],
    pub signature: &'a [u8; 64],
    pub params_data: &'a [u8],
}

impl<'info> Propose<'info> {
    pub fn propose(&mut self, args: ProposeArgs<'_>) -> Result<(), ProgramError> {
        let proposal_index = self.wallet.proposal_index.get();

        // Derive the expected PDA
        let intent_addr = *self.intent.address();
        let index_bytes = proposal_index.to_le_bytes();
        let (expected_pda, bump) = Address::find_program_address(
            &[b"proposal", intent_addr.as_ref(), &index_bytes],
            &crate::ID,
        );
        require_keys_eq!(*self.proposal.address(), expected_pda, ProgramError::InvalidSeeds);

        // Ensure account is not already initialized
        require!(
            self.proposal.to_account_view().data_len() == 0,
            ProgramError::AccountAlreadyInitialized
        );

        let clock = Clock::get()?;
        require!(args.expiry > clock.unix_timestamp.get(), ProgramError::InvalidArgument);

        let proposer_addr = Address::new_from_array(*args.proposer_pubkey);
        require!(self.intent.is_proposer(&proposer_addr), ProgramError::MissingRequiredSignature);

        if self.intent.intent_type == crate::state::intent::IntentType::Custom {
            self.intent.validate_param_constraints(args.params_data)?;
        }

        let mut msg_buf = MessageBuilder::new();
        msg_buf.build_message_for_intent(
            &MessageContext { expiry: args.expiry, action: "propose", wallet_name: self.wallet.name(), proposal_index },
            &self.intent,
            args.params_data,
        )?;

        brine_ed25519::sig_verify(args.proposer_pubkey, args.signature, msg_buf.as_bytes())
            .map_err(|_| ProgramError::InvalidArgument)?;

        // Create the proposal account via system program CPI with PDA seeds
        let rent = Rent::get()?;
        // Space: disc(1) + wallet(32) + intent(32) + proposal_index(8) + proposer(32)
        //  + status(1) + proposed_at(8) + approved_at(8) + bump(1) + bitmaps(2+2)
        //  + rent_refund(32) + vec_prefix(4) + params_data
        let space = 1 + 32 + 32 + 8 + 32 + 1 + 8 + 8 + 1 + 2 + 2 + 32 + 4 + args.params_data.len();
        let lamports = rent.try_minimum_balance(space)?;

        let bump_byte = [bump];
        let seeds: &[Seed] = &[
            Seed::from(b"proposal" as &[u8]),
            Seed::from(intent_addr.as_ref()),
            Seed::from(&index_bytes as &[u8]),
            Seed::from(&bump_byte as &[u8]),
        ];

        self.system_program
            .create_account(
                self.payer.to_account_view(),
                self.proposal.to_account_view(),
                lamports, space as u64, &crate::ID,
            )
            .invoke_signed(seeds)?;

        // Write using quasar's generated ProposalZc repr(C) struct for the fixed header,
        // then append the dynamic Vec (params_data) with its u32 length prefix.
        use crate::state::proposal::ProposalZc;

        let header = ProposalZc {
            wallet: *self.wallet.address(),
            intent: intent_addr,
            proposal_index: PodU64::from(proposal_index),
            proposer: proposer_addr,
            status: ProposalStatus::Active,
            proposed_at: PodI64::from(clock.unix_timestamp.get()),
            approved_at: PodI64::from(0i64),
            bump,
            approval_bitmap: PodU16::from(0u16),
            cancellation_bitmap: PodU16::from(0u16),
            rent_refund: *self.payer.address(),
        };

        let proposal_view = unsafe {
            &mut *(self.proposal as *mut UncheckedAccount as *mut AccountView)
        };
        let ptr = proposal_view.data_mut_ptr();
        let disc_len = 1usize;
        let header_size = core::mem::size_of::<ProposalZc>();
        unsafe {
            *ptr = 3; // Proposal discriminator
            core::ptr::copy_nonoverlapping(
                &header as *const ProposalZc as *const u8,
                ptr.add(disc_len),
                header_size,
            );
            // params_data Vec: u32 LE length prefix + raw bytes
            let vec_offset = disc_len + header_size;
            let len_bytes = (args.params_data.len() as u32).to_le_bytes();
            core::ptr::copy_nonoverlapping(len_bytes.as_ptr(), ptr.add(vec_offset), 4);
            core::ptr::copy_nonoverlapping(
                args.params_data.as_ptr(),
                ptr.add(vec_offset + 4),
                args.params_data.len(),
            );
        }

        let count = self.intent.active_proposal_count.get();
        let new_count = count.checked_add(1).ok_or(ProgramError::InvalidArgument)?;
        self.intent.active_proposal_count = PodU16::from(new_count);
        self.wallet.proposal_index = PodU64::from(proposal_index + 1);
        Ok(())
    }
}

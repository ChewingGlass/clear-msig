use {
    crate::state::{
        intent::{Intent, IntentInner, IntentType},
        wallet::{ClearWallet, ClearWalletInner},
    },
    crate::utils::hash::sha256,
    quasar_lang::prelude::*,
};

/// Creates a ClearWallet with three default meta-intents.
///
/// `name_hash` is an UncheckedAccount whose address equals sha256(name).
/// The client derives this off-chain. Verified on-chain.
///
/// Proposer/approver addresses are passed as instruction data (the `addresses`
/// tail field): first `num_proposers * 32` bytes are proposer pubkeys, rest
/// are approver pubkeys. This avoids remaining-account dedup issues when
/// the payer is also a proposer or approver.
#[derive(Accounts)]
pub struct CreateWallet<'info> {
    pub payer: &'info mut Signer,
    /// Account at address sha256(name) — used as PDA seed reference.
    pub name_hash: &'info UncheckedAccount,
    #[account(
        init,
        payer = payer,
        seeds = ClearWallet::seeds(name_hash),
        bump,
    )]
    pub wallet: Account<ClearWallet<'info>>,
    #[account(
        init,
        payer = payer,
        seeds = Intent::seeds(wallet, b"\x00"),
        bump,
    )]
    pub add_intent: Account<Intent<'info>>,
    #[account(
        init,
        payer = payer,
        seeds = Intent::seeds(wallet, b"\x01"),
        bump,
    )]
    pub remove_intent: Account<Intent<'info>>,
    #[account(
        init,
        payer = payer,
        seeds = Intent::seeds(wallet, b"\x02"),
        bump,
    )]
    pub update_intent: Account<Intent<'info>>,
    pub system_program: &'info Program<System>,
}

pub struct CreateWalletArgs<'a> {
    pub name: &'a str,
    pub approval_threshold: u8,
    pub cancellation_threshold: u8,
    pub timelock_seconds: u32,
    pub proposers: &'a [[u8; 32]],
    pub approvers: &'a [[u8; 32]],
}

impl<'info> CreateWallet<'info> {
    pub fn create(
        &mut self,
        args: CreateWalletArgs<'_>,
        bumps: &CreateWalletBumps,
    ) -> Result<(), ProgramError> {
        // Verify name_hash matches sha256(name)
        let computed = sha256(args.name.as_bytes());
        require_keys_eq!(
            *self.name_hash.address(),
            Address::new_from_array(computed),
            ProgramError::InvalidSeeds
        );

        let wallet_addr = *self.wallet.address();

        let proposer_count = args.proposers.len() as u8;
        let approver_count = args.approvers.len() as u8;
        require!(proposer_count as usize <= 16, ProgramError::InvalidArgument);
        require!(approver_count as usize <= 16, ProgramError::InvalidArgument);

        require!(args.approval_threshold > 0, ProgramError::InvalidArgument);
        require!(args.approval_threshold <= approver_count, ProgramError::InvalidArgument);
        require!(args.cancellation_threshold > 0, ProgramError::InvalidArgument);
        require!(args.cancellation_threshold <= approver_count, ProgramError::InvalidArgument);

        // Address is #[repr(transparent)] over [u8; 32], safe to cast
        let proposers: &[Address] = unsafe {
            core::slice::from_raw_parts(args.proposers.as_ptr() as *const Address, args.proposers.len())
        };
        let approvers: &[Address] = unsafe {
            core::slice::from_raw_parts(args.approvers.as_ptr() as *const Address, args.approvers.len())
        };

        self.wallet.set_inner(ClearWalletInner {
            bump: bumps.wallet,
            proposal_index: 0u64,
            intent_index: 2u8, // three intents: 0, 1, 2
            name: args.name,
        }, self.payer.to_account_view(), None)?;

        let empty_params: &[crate::utils::definition::ParamEntry] = &[];
        let empty_accounts: &[crate::utils::definition::AccountEntry] = &[];
        let empty_instructions: &[crate::utils::definition::InstructionEntry] = &[];
        let empty_segments: &[crate::utils::definition::DataSegmentEntry] = &[];
        let empty_seeds: &[crate::utils::definition::SeedEntry] = &[];
        let empty_pool: &[u8] = &[];

        let meta_intents = [
            (&mut self.add_intent, 0u8, IntentType::AddIntent, bumps.add_intent),
            (&mut self.remove_intent, 1u8, IntentType::RemoveIntent, bumps.remove_intent),
            (&mut self.update_intent, 2u8, IntentType::UpdateIntent, bumps.update_intent),
        ];

        for (intent, index, intent_type, bump) in meta_intents {
            intent.set_inner(IntentInner {
                wallet: wallet_addr,
                bump,
                intent_index: index,
                intent_type,
                approved: 1u8,
                approval_threshold: args.approval_threshold,
                cancellation_threshold: args.cancellation_threshold,
                timelock_seconds: args.timelock_seconds,
                template_offset: 0u16,
                template_len: 0u16,
                active_proposal_count: 0u16,
                proposers,
                approvers,
                params: empty_params,
                accounts: empty_accounts,
                instructions: empty_instructions,
                data_segments: empty_segments,
                seeds: empty_seeds,
                byte_pool: empty_pool,
            }, self.payer.to_account_view(), None)?;
        }

        Ok(())
    }
}

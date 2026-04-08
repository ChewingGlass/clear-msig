use {
    crate::state::{
        intent::{Intent, IntentType},
        wallet::ClearWallet,
    },
    crate::utils::hash::sha256,
    quasar_lang::{prelude::*, remaining::RemainingAccounts},
};

/// Creates a ClearWallet with three default meta-intents.
///
/// `name_hash` is an UncheckedAccount whose address equals sha256(name).
/// The client derives this off-chain. Verified on-chain.
///
/// remaining_accounts: first `num_proposers` are proposers, rest are approvers.
#[derive(Accounts)]
pub struct CreateWallet<'info> {
    pub payer: &'info mut Signer,
    /// Account at address sha256(name) — used as PDA seed reference.
    pub name_hash: &'info UncheckedAccount,
    #[account(
        init,
        mut,
        payer = payer,
        seeds = [b"clear_wallet", name_hash],
        bump,
    )]
    pub wallet: Account<ClearWallet<'info>>,
    #[account(
        init,
        mut,
        payer = payer,
        seeds = [b"intent", wallet, b"\x00"],
        bump,
    )]
    pub add_intent: Account<Intent<'info>>,
    #[account(
        init,
        mut,
        payer = payer,
        seeds = [b"intent", wallet, b"\x01"],
        bump,
    )]
    pub remove_intent: Account<Intent<'info>>,
    #[account(
        init,
        mut,
        payer = payer,
        seeds = [b"intent", wallet, b"\x02"],
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
    pub num_proposers: u8,
}

impl<'info> CreateWallet<'info> {
    pub fn create(
        &mut self,
        args: CreateWalletArgs<'_>,
        bumps: &CreateWalletBumps,
        remaining: RemainingAccounts,
    ) -> Result<(), ProgramError> {
        // Verify name_hash matches sha256(name)
        let computed = sha256(args.name.as_bytes());
        require_keys_eq!(
            *self.name_hash.address(),
            Address::new_from_array(computed),
            ProgramError::InvalidSeeds
        );

        let wallet_addr = *self.wallet.address();

        let mut proposer_addrs = [Address::default(); 16];
        let mut approver_addrs = [Address::default(); 16];
        let mut proposer_count = 0u8;
        let mut approver_count = 0u8;

        for (i, account) in remaining.iter().enumerate() {
            let account = account?;
            if (i as u8) < args.num_proposers {
                require!((proposer_count as usize) < 16, ProgramError::InvalidArgument);
                proposer_addrs[proposer_count as usize] = *account.address();
                proposer_count += 1;
            } else {
                require!((approver_count as usize) < 16, ProgramError::InvalidArgument);
                approver_addrs[approver_count as usize] = *account.address();
                approver_count += 1;
            }
        }

        require!(args.approval_threshold > 0, ProgramError::InvalidArgument);
        require!(args.approval_threshold <= approver_count, ProgramError::InvalidArgument);
        require!(args.cancellation_threshold > 0, ProgramError::InvalidArgument);
        require!(args.cancellation_threshold <= approver_count, ProgramError::InvalidArgument);

        let proposers = &proposer_addrs[..proposer_count as usize];
        let approvers = &approver_addrs[..approver_count as usize];

        self.wallet.set_inner(
            bumps.wallet,
            0u64,
            2u8, // intent_index = 2 (three intents: 0, 1, 2)
            args.name,
            self.payer.to_account_view(),
            None,
        )?;

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
            intent.set_inner(
                wallet_addr, bump, index, intent_type,
                1u8, // approved
                args.approval_threshold, args.cancellation_threshold,
                args.timelock_seconds,
                0u16, 0u16, // template offset/len
                0u16, // active_proposal_count
                proposers, approvers,
                empty_params, empty_accounts, empty_instructions,
                empty_segments, empty_seeds, empty_pool,
                self.payer.to_account_view(),
                None,
            )?;
        }

        Ok(())
    }
}

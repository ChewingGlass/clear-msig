use quasar_lang::{
    cpi::{CpiAccount, InstructionAccount, InstructionView, Seed, Signer},
    prelude::*,
    remaining::RemainingAccounts,
    sysvars::Sysvar as _,
};

use crate::{
    state::{
        intent::{Intent, IntentType},
        proposal::{Proposal, ProposalStatus},
        wallet::ClearWallet,
    },
    utils::definition::*,
};

#[derive(Accounts)]
pub struct Execute<'info> {
    #[account(mut)]
    pub wallet: Account<ClearWallet<'info>>,
    #[account(
        mut,
        seeds = [b"vault", wallet],
        bump,
    )]
    pub vault: &'info mut UncheckedAccount,
    #[account(mut)]
    pub intent: Account<Intent<'info>>,
    #[account(
        mut,
        has_one = wallet,
        has_one = intent,
        constraint = proposal.status == ProposalStatus::Approved @ ProgramError::InvalidArgument
    )]
    pub proposal: Account<Proposal<'info>>,
    pub system_program: &'info Program<System>,
}

impl<'info> Execute<'info> {
    pub fn execute(
        &mut self,
        bumps: &ExecuteBumps,
        remaining: RemainingAccounts,
    ) -> Result<(), ProgramError> {
        let clock = Clock::get()?;
        let approved_at = self.proposal.approved_at.get();
        let timelock = self.intent.timelock_seconds.get() as i64;
        require!(
            clock.unix_timestamp.get() >= approved_at + timelock,
            ProgramError::InvalidArgument
        );

        match self.intent.intent_type {
            IntentType::AddIntent => self.execute_add_intent(remaining)?,
            IntentType::RemoveIntent => self.execute_remove_intent(remaining)?,
            IntentType::UpdateIntent => self.execute_update_intent(remaining)?,
            IntentType::Custom => self.execute_custom(bumps, remaining)?,
        }

        self.proposal.status = ProposalStatus::Executed;
        let count = self.intent.active_proposal_count.get();
        self.intent.active_proposal_count = PodU16::from(count).saturating_sub(1);

        Ok(())
    }

    /// remaining: [0]=payer(mut,signer), [1]=new_intent(mut)
    fn execute_add_intent(&mut self, remaining: RemainingAccounts) -> Result<(), ProgramError> {
        let new_index = self.wallet.intent_index + 1;
        let wallet_addr = *self.wallet.address();
        let params_data = self.proposal.params_data();

        let (expected_pda, intent_bump) = Address::find_program_address(
            &[b"intent", wallet_addr.as_ref(), &[new_index]],
            &crate::ID,
        );

        let mut remaining_iter = remaining.iter();
        let payer = remaining_iter
            .next()
            .ok_or(ProgramError::NotEnoughAccountKeys)??;
        let mut new_intent = remaining_iter
            .next()
            .ok_or(ProgramError::NotEnoughAccountKeys)??;

        require!(payer.is_signer(), ProgramError::MissingRequiredSignature);
        require_keys_eq!(
            *new_intent.address(),
            expected_pda,
            ProgramError::InvalidSeeds
        );

        let space = 256 + params_data.len();
        let rent = Rent::get()?;
        let lamports = rent.try_minimum_balance(space)?;

        let index_byte = [new_index];
        let bump_byte = [intent_bump];
        let seeds: &[Seed] = &[
            Seed::from(b"intent" as &[u8]),
            Seed::from(wallet_addr.as_ref()),
            Seed::from(&index_byte as &[u8]),
            Seed::from(&bump_byte as &[u8]),
        ];

        self.system_program
            .create_account(&payer, &new_intent, lamports, space as u64, &crate::ID)
            .invoke_signed(seeds)?;

        // Write discriminator + raw intent body
        let data_ptr = new_intent.data_mut_ptr();
        unsafe {
            *data_ptr = 2; // Intent discriminator
            core::ptr::copy_nonoverlapping(
                params_data.as_ptr(),
                data_ptr.add(1),
                params_data.len(),
            );
        }

        self.wallet.intent_index = new_index;
        Ok(())
    }

    /// remaining: [0]=target_intent(mut)
    fn execute_remove_intent(&mut self, remaining: RemainingAccounts) -> Result<(), ProgramError> {
        let params_data = self.proposal.params_data();
        require!(params_data.len() == 1, ProgramError::InvalidInstructionData);
        let target_index = params_data[0];

        let (expected_pda, _) = Address::find_program_address(
            &[b"intent", self.wallet.address().as_ref(), &[target_index]],
            &crate::ID,
        );

        let mut remaining_iter = remaining.iter();
        let mut target = remaining_iter
            .next()
            .ok_or(ProgramError::NotEnoughAccountKeys)??;

        require_keys_eq!(*target.address(), expected_pda, ProgramError::InvalidSeeds);
        require!(target.is_writable(), ProgramError::Immutable);

        unsafe { *target.data_mut_ptr().add(crate::state::intent::INTENT_APPROVED_OFFSET) = 0 };

        Ok(())
    }

    /// remaining: [0]=payer(mut,signer), [1]=target_intent(mut)
    fn execute_update_intent(&mut self, remaining: RemainingAccounts) -> Result<(), ProgramError> {
        let params_data = self.proposal.params_data();
        require!(params_data.len() > 1, ProgramError::InvalidInstructionData);
        let target_index = params_data[0];

        let (expected_pda, _) = Address::find_program_address(
            &[b"intent", self.wallet.address().as_ref(), &[target_index]],
            &crate::ID,
        );

        let mut remaining_iter = remaining.iter();
        let payer = remaining_iter
            .next()
            .ok_or(ProgramError::NotEnoughAccountKeys)??;
        let mut target = remaining_iter
            .next()
            .ok_or(ProgramError::NotEnoughAccountKeys)??;

        require!(payer.is_signer(), ProgramError::MissingRequiredSignature);
        require_keys_eq!(*target.address(), expected_pda, ProgramError::InvalidSeeds);
        require!(target.is_writable(), ProgramError::Immutable);

        // Block update if the target intent has open proposals
        let apc_offset = crate::state::intent::INTENT_ACTIVE_PROPOSAL_COUNT_OFFSET;
        let apc_bytes =
            unsafe { core::slice::from_raw_parts(target.data_mut_ptr().add(apc_offset), 2) };
        let active_count = u16::from_le_bytes([apc_bytes[0], apc_bytes[1]]);
        require!(active_count == 0, ProgramError::InvalidArgument);

        // Rewrite intent data
        let new_data = &params_data[1..];
        let new_space = 1 + new_data.len();
        let rent = Rent::get()?;
        quasar_lang::accounts::account::realloc_account(
            &mut target,
            new_space,
            &payer,
            Some(&rent),
        )?;
        let data_ptr = target.data_mut_ptr();
        unsafe {
            *data_ptr = 2;
            core::ptr::copy_nonoverlapping(new_data.as_ptr(), data_ptr.add(1), new_data.len());
        }

        Ok(())
    }

    /// remaining: all accounts referenced by the intent's account definitions.
    fn execute_custom(
        &self,
        bumps: &ExecuteBumps,
        remaining: RemainingAccounts,
    ) -> Result<(), ProgramError> {
        let params_data = self.proposal.params_data();
        let intent = &self.intent;

        // Collect remaining accounts
        let mut account_views: [core::mem::MaybeUninit<AccountView>; 32] =
            unsafe { core::mem::MaybeUninit::uninit().assume_init() };
        let mut account_count = 0usize;
        for account in remaining.iter() {
            let acct = account?;
            require!(account_count < 32, ProgramError::InvalidArgument);
            account_views[account_count].write(acct);
            account_count += 1;
        }

        let vault_seeds = bumps.vault_seeds();
        let ix_entries = intent.instructions();
        let seg_entries = intent.data_segments();
        let acct_entries = intent.accounts();
        let pool = intent.byte_pool();

        for ix_entry in ix_entries {
            // Build instruction data from segments
            let mut ix_data = [0u8; 1024];
            let mut ix_len = 0usize;

            let seg_start = ix_entry.segments_start.get() as usize;
            let seg_count = ix_entry.segments_count.get() as usize;

            for seg in &seg_entries[seg_start..seg_start + seg_count] {
                let seg_pool = &pool[seg.pool_offset.get() as usize
                    ..(seg.pool_offset.get() + seg.pool_len.get()) as usize];
                match seg.segment_type {
                    SegmentType::Literal => {
                        require!(
                            ix_len + seg_pool.len() <= 1024,
                            ProgramError::InvalidInstructionData
                        );
                        ix_data[ix_len..ix_len + seg_pool.len()].copy_from_slice(seg_pool);
                        ix_len += seg_pool.len();
                    }
                    SegmentType::Param => {
                        require!(seg_pool.len() >= 2, ProgramError::InvalidInstructionData);
                        let param_idx = seg_pool[0];
                        let encoding = DataEncoding::from_u8(seg_pool[1])
                            .ok_or(ProgramError::InvalidInstructionData)?;
                        let val = intent.read_param_bytes(params_data, param_idx)?;
                        let size = encoding.byte_size();
                        require!(val.len() >= size, ProgramError::InvalidInstructionData);
                        require!(ix_len + size <= 1024, ProgramError::InvalidInstructionData);
                        ix_data[ix_len..ix_len + size].copy_from_slice(&val[..size]);
                        ix_len += size;
                    }
                }
            }

            // Build CPI account lists
            let acct_idx_offset = ix_entry.account_indexes_offset.get() as usize;
            let acct_idx_len = ix_entry.account_indexes_len.get() as usize;
            let acct_indexes = &pool[acct_idx_offset..acct_idx_offset + acct_idx_len];

            require!(
                acct_indexes.len() <= 16,
                ProgramError::InvalidInstructionData
            );

            let mut cpi_ix_accounts: [core::mem::MaybeUninit<InstructionAccount>; 32] =
                unsafe { core::mem::MaybeUninit::uninit().assume_init() };
            let mut cpi_accts: [core::mem::MaybeUninit<CpiAccount>; 32] =
                unsafe { core::mem::MaybeUninit::uninit().assume_init() };

            for (i, &idx) in acct_indexes.iter().enumerate() {
                let idx = idx as usize;
                require!(idx < account_count, ProgramError::NotEnoughAccountKeys);
                let view = unsafe { account_views[idx].assume_init_ref() };
                let acct_def = &acct_entries[idx];

                cpi_ix_accounts[i].write(
                    InstructionAccount::new(view.address(), acct_def.is_writable, acct_def.is_signer),
                );
                cpi_accts[i].write(CpiAccount::from(view));
            }

            let prog_idx = ix_entry.program_account_index as usize;
            require!(prog_idx < account_count, ProgramError::NotEnoughAccountKeys);
            let program = unsafe { account_views[prog_idx].assume_init_ref() };

            let instruction = InstructionView {
                program_id: program.address(),
                accounts: unsafe {
                    core::slice::from_raw_parts(cpi_ix_accounts[0].as_ptr(), acct_indexes.len())
                },
                data: &ix_data[..ix_len],
            };

            let signers = [Signer::from(&vault_seeds[..])];
            unsafe {
                solana_instruction_view::cpi::invoke_signed_unchecked(
                    &instruction,
                    core::slice::from_raw_parts(cpi_accts[0].as_ptr(), acct_indexes.len()),
                    &signers,
                );
            }
        }

        Ok(())
    }
}

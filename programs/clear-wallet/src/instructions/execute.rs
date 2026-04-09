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
        require!(self.wallet.intent_index < u8::MAX, ProgramError::InvalidArgument);
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

        // Block removal if the target intent has open proposals
        let apc_offset = crate::state::intent::INTENT_ACTIVE_PROPOSAL_COUNT_OFFSET;
        let apc_bytes =
            unsafe { core::slice::from_raw_parts(target.data_mut_ptr().add(apc_offset), 2) };
        let active_count = u16::from_le_bytes([apc_bytes[0], apc_bytes[1]]);
        require!(active_count == 0, ProgramError::InvalidArgument);

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

    /// remaining: all accounts referenced by the intent's account definitions,
    /// EXCEPT accounts already declared in the Execute struct (vault,
    /// system_program, wallet, etc.). Those are injected automatically when
    /// a Static or Vault entry's address matches a declared account.
    #[inline(never)]
    fn execute_custom(
        &self,
        bumps: &ExecuteBumps,
        remaining: RemainingAccounts,
    ) -> Result<(), ProgramError> {
        let params_data = self.proposal.params_data();
        let intent = &self.intent;
        let pool = intent.byte_pool();

        // Declared accounts available for injection (quasar rejects
        // remaining accounts that duplicate these).
        let declared: [&AccountView; 3] = [
            self.vault.to_account_view(),
            self.system_program.to_account_view(),
            self.wallet.to_account_view(),
        ];

        // Build account_views by walking intent account entries.
        // Vault entries and Static entries whose address matches a declared
        // account are injected directly; everything else is consumed from
        // remaining_accounts in order.
        let acct_entries = intent.accounts();
        let mut account_views: [core::mem::MaybeUninit<AccountView>; 32] =
            unsafe { core::mem::MaybeUninit::uninit().assume_init() };
        let mut account_count = 0usize;
        let mut remaining_iter = remaining.iter();

        for acct_def in acct_entries {
            require!(account_count < 32, ProgramError::InvalidArgument);
            if acct_def.source_type == AccountSourceType::Vault {
                account_views[account_count].write(self.vault.to_account_view().clone());
            } else if acct_def.source_type == AccountSourceType::Static {
                let po = acct_def.pool_offset.get() as usize;
                let pl = acct_def.pool_len.get() as usize;
                let addr_bytes = pool.get(po..po + pl)
                    .ok_or(ProgramError::InvalidInstructionData)?;
                require!(addr_bytes.len() >= 32, ProgramError::InvalidInstructionData);
                let addr = Address::new_from_array(
                    addr_bytes[..32].try_into()
                        .map_err(|_| ProgramError::InvalidInstructionData)?,
                );
                if let Some(dv) = declared.iter().find(|d| *d.address() == addr) {
                    account_views[account_count].write((*dv).clone());
                } else {
                    let acct = remaining_iter
                        .next()
                        .ok_or(ProgramError::NotEnoughAccountKeys)??;
                    account_views[account_count].write(acct);
                }
            } else {
                let acct = remaining_iter
                    .next()
                    .ok_or(ProgramError::NotEnoughAccountKeys)??;
                account_views[account_count].write(acct);
            }
            account_count += 1;
        }

        // Validate remaining accounts match intent definitions
        validate_remaining_accounts(
            &account_views,
            account_count,
            intent,
            params_data,
            self.vault.address(),
        )?;

        let vault_seeds = self.vault_seeds(bumps);
        execute_cpi_loop(&vault_seeds, intent, params_data, &account_views, account_count)
    }
}

/// CPI execution loop in its own stack frame (ix_data and CPI arrays are large).
#[inline(never)]
fn execute_cpi_loop(
    vault_seeds: &[Seed],
    intent: &Intent<'_>,
    params_data: &[u8],
    account_views: &[core::mem::MaybeUninit<AccountView>; 32],
    account_count: usize,
) -> Result<(), ProgramError> {
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

        let signers = [Signer::from(vault_seeds)];
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

/// Validates that each remaining account matches the address specified by the
/// intent definition's account entries (Static, Param, PdaDerived, HasOne, Vault).
#[inline(never)]
fn validate_remaining_accounts(
    account_views: &[core::mem::MaybeUninit<AccountView>; 32],
    account_count: usize,
    intent: &Intent<'_>,
    params_data: &[u8],
    vault_address: &Address,
) -> Result<(), ProgramError> {
    let acct_entries = intent.accounts();
    let pool = intent.byte_pool();

    require!(
        account_count == acct_entries.len(),
        ProgramError::InvalidArgument
    );

    for (i, acct_def) in acct_entries.iter().enumerate() {
        let current_addr = *unsafe { account_views[i].assume_init_ref() }.address();
        let po = acct_def.pool_offset.get() as usize;
        let pl = acct_def.pool_len.get() as usize;

        match acct_def.source_type {
            AccountSourceType::Static => {
                let pool_data = pool
                    .get(po..po + pl)
                    .ok_or(ProgramError::InvalidInstructionData)?;
                require!(pool_data.len() >= 32, ProgramError::InvalidInstructionData);
                let expected = Address::new_from_array(
                    pool_data[..32]
                        .try_into()
                        .map_err(|_| ProgramError::InvalidInstructionData)?,
                );
                require_keys_eq!(current_addr, expected, ProgramError::InvalidArgument);
            }
            AccountSourceType::Param => {
                let pool_data = pool
                    .get(po..po + pl)
                    .ok_or(ProgramError::InvalidInstructionData)?;
                require!(!pool_data.is_empty(), ProgramError::InvalidInstructionData);
                let addr_bytes = intent.read_param_bytes(params_data, pool_data[0])?;
                require!(addr_bytes.len() >= 32, ProgramError::InvalidInstructionData);
                let expected = Address::new_from_array(
                    addr_bytes[..32]
                        .try_into()
                        .map_err(|_| ProgramError::InvalidInstructionData)?,
                );
                require_keys_eq!(current_addr, expected, ProgramError::InvalidArgument);
            }
            AccountSourceType::PdaDerived => {
                let pool_data = pool
                    .get(po..po + pl)
                    .ok_or(ProgramError::InvalidInstructionData)?;
                validate_pda_account(
                    &current_addr, pool_data, account_views, account_count,
                    intent, params_data,
                )?;
            }
            AccountSourceType::HasOne => {
                let pool_data = pool
                    .get(po..po + pl)
                    .ok_or(ProgramError::InvalidInstructionData)?;
                require!(pool_data.len() >= 3, ProgramError::InvalidInstructionData);
                let acct_idx = pool_data[0] as usize;
                let byte_offset =
                    u16::from_le_bytes([pool_data[1], pool_data[2]]) as usize;

                require!(
                    acct_idx < account_count,
                    ProgramError::NotEnoughAccountKeys
                );
                let ref_view =
                    unsafe { account_views[acct_idx].assume_init_ref() };
                let data_len = ref_view.data_len();
                require!(
                    byte_offset + 32 <= data_len,
                    ProgramError::InvalidInstructionData
                );
                let addr_bytes = unsafe {
                    core::slice::from_raw_parts(
                        ref_view.data_ptr().add(byte_offset),
                        32,
                    )
                };
                let expected = Address::new_from_array(
                    addr_bytes
                        .try_into()
                        .map_err(|_| ProgramError::InvalidInstructionData)?,
                );
                require_keys_eq!(current_addr, expected, ProgramError::InvalidArgument);
            }
            AccountSourceType::Vault => {
                require_keys_eq!(
                    current_addr,
                    *vault_address,
                    ProgramError::InvalidArgument
                );
            }
        }
    }

    Ok(())
}

/// PDA account validation in its own stack frame (seed buffers are large).
#[inline(never)]
fn validate_pda_account(
    current_addr: &Address,
    pool_data: &[u8],
    account_views: &[core::mem::MaybeUninit<AccountView>; 32],
    account_count: usize,
    intent: &Intent<'_>,
    params_data: &[u8],
) -> Result<(), ProgramError> {
    let pool = intent.byte_pool();
    let seed_entries = intent.seeds();
    require!(pool_data.len() >= 5, ProgramError::InvalidInstructionData);
    let prog_acct_idx = pool_data[0] as usize;
    let seeds_start = u16::from_le_bytes([pool_data[1], pool_data[2]]) as usize;
    let seeds_count = u16::from_le_bytes([pool_data[3], pool_data[4]]) as usize;

    require!(prog_acct_idx < account_count, ProgramError::NotEnoughAccountKeys);
    let program_addr = *unsafe { account_views[prog_acct_idx].assume_init_ref() }.address();

    require!(seeds_count <= 16, ProgramError::InvalidInstructionData);
    let mut seed_bufs = [[0u8; 32]; 16];
    let mut seed_lens = [0usize; 16];

    for s in 0..seeds_count {
        let se = seed_entries
            .get(seeds_start + s)
            .ok_or(ProgramError::InvalidInstructionData)?;
        let se_start = se.pool_offset.get() as usize;
        let se_len = se.pool_len.get() as usize;
        let se_pool = pool
            .get(se_start..se_start + se_len)
            .ok_or(ProgramError::InvalidInstructionData)?;

        match se.seed_type {
            SeedType::Literal => {
                require!(se_pool.len() <= 32, ProgramError::InvalidInstructionData);
                seed_bufs[s][..se_pool.len()].copy_from_slice(se_pool);
                seed_lens[s] = se_pool.len();
            }
            SeedType::ParamRef => {
                require!(!se_pool.is_empty(), ProgramError::InvalidInstructionData);
                let val = intent.read_param_bytes(params_data, se_pool[0])?;
                require!(val.len() <= 32, ProgramError::InvalidInstructionData);
                seed_bufs[s][..val.len()].copy_from_slice(val);
                seed_lens[s] = val.len();
            }
            SeedType::AccountRef => {
                require!(!se_pool.is_empty(), ProgramError::InvalidInstructionData);
                let acct_idx = se_pool[0] as usize;
                require!(acct_idx < account_count, ProgramError::NotEnoughAccountKeys);
                let addr = *unsafe { account_views[acct_idx].assume_init_ref() }.address();
                seed_bufs[s].copy_from_slice(addr.as_ref());
                seed_lens[s] = 32;
            }
        }
    }

    let mut seed_refs: [&[u8]; 16] = [&[]; 16];
    for s in 0..seeds_count {
        seed_refs[s] = &seed_bufs[s][..seed_lens[s]];
    }

    let (expected, _) = Address::find_program_address(&seed_refs[..seeds_count], &program_addr);
    require_keys_eq!(*current_addr, expected, ProgramError::InvalidArgument);
    Ok(())
}

//! Builds the flat entry arrays + byte pool that get passed to Intent::set_inner.
//!
//! Usage:
//! ```ignore
//! let mut b = IntentBuilder::new();
//! b.set_governance(2, 1, 3600);
//! b.add_proposer(addr);
//! b.add_approver(addr);
//! b.add_param("amount", ParamType::U64, None);
//! b.add_static_account(system_program, false, false);
//! b.add_vault_account(true, true);
//! b.add_param_account(0, false, true);
//! let ix = b.begin_instruction(0); // program = account 0
//! ix.add_account_index(1);
//! ix.add_literal_segment(&[2, 0, 0, 0]);
//! ix.add_param_segment(0, DataEncoding::LittleEndianU64);
//! b.set_template("transfer {0} lamports");
//! let built = b.build();
//! ```

use std::{string::String, vec::Vec};
use clear_wallet::utils::definition::*;
use quasar_lang::prelude::*;
use solana_address::Address;

/// Packed intent data ready for set_inner.
/// Uses native Rust types (not Pod) since this runs off-chain.
pub struct BuiltIntent {
    pub approval_threshold: u8,
    pub cancellation_threshold: u8,
    pub timelock_seconds: u32,
    pub template_offset: u16,
    pub template_len: u16,
    pub proposers: Vec<Address>,
    pub approvers: Vec<Address>,
    pub params: Vec<ParamEntry>,
    pub accounts: Vec<AccountEntry>,
    pub instructions: Vec<InstructionEntry>,
    pub data_segments: Vec<DataSegmentEntry>,
    pub seeds: Vec<SeedEntry>,
    pub byte_pool: Vec<u8>,
}

impl BuiltIntent {
    /// Get the template string from the byte pool.
    pub fn template_str(&self) -> &str {
        let start = self.template_offset as usize;
        let end = start + self.template_len as usize;
        core::str::from_utf8(&self.byte_pool[start..end]).unwrap_or("")
    }

    /// Get a slice of the byte pool.
    pub fn pool_slice(&self, offset: PodU16, len: PodU16) -> &[u8] {
        let start = offset.get() as usize;
        &self.byte_pool[start..start + len.get() as usize]
    }
}

pub struct IntentBuilder {
    approval_threshold: u8,
    cancellation_threshold: u8,
    timelock_seconds: u32,
    proposers: Vec<Address>,
    approvers: Vec<Address>,
    params: Vec<ParamEntry>,
    accounts: Vec<AccountEntry>,
    instructions: Vec<InstructionEntry>,
    data_segments: Vec<DataSegmentEntry>,
    seeds: Vec<SeedEntry>,
    pool: Vec<u8>,
    template: String,
}

impl Default for IntentBuilder {
    fn default() -> Self { Self::new() }
}

impl IntentBuilder {
    pub fn new() -> Self {
        Self {
            approval_threshold: 1,
            cancellation_threshold: 1,
            timelock_seconds: 0,
            proposers: Vec::new(),
            approvers: Vec::new(),
            params: Vec::new(),
            accounts: Vec::new(),
            instructions: Vec::new(),
            data_segments: Vec::new(),
            seeds: Vec::new(),
            pool: Vec::new(),
            template: String::new(),
        }
    }

    pub fn set_governance(&mut self, approval: u8, cancellation: u8, timelock: u32) -> &mut Self {
        self.approval_threshold = approval;
        self.cancellation_threshold = cancellation;
        self.timelock_seconds = timelock;
        self
    }

    pub fn add_proposer(&mut self, addr: Address) -> &mut Self {
        self.proposers.push(addr);
        self
    }

    pub fn add_approver(&mut self, addr: Address) -> &mut Self {
        self.approvers.push(addr);
        self
    }

    pub fn add_param(&mut self, name: &str, param_type: ParamType, constraint: Option<(ConstraintType, u64)>) -> &mut Self {
        let name_offset = self.pool.len() as u16;
        self.pool.extend_from_slice(name.as_bytes());
        let (ct, cv) = constraint.unwrap_or((ConstraintType::None, 0));
        self.params.push(ParamEntry {
            param_type,
            name_offset: PodU16::from(name_offset),
            name_len: PodU16::from(name.len() as u16),
            constraint_type: ct,
            constraint_value: PodU64::from(cv),
        });
        self
    }

    // --- Account sources ---

    pub fn add_static_account(&mut self, addr: Address, signer: bool, writable: bool) -> &mut Self {
        let offset = self.pool.len() as u16;
        self.pool.extend_from_slice(addr.as_ref());
        self.accounts.push(AccountEntry {
            is_signer: signer,
            is_writable: writable,
            source_type: AccountSourceType::Static,
            pool_offset: PodU16::from(offset),
            pool_len: PodU16::from(32),
        });
        self
    }

    pub fn add_param_account(&mut self, param_index: u8, signer: bool, writable: bool) -> &mut Self {
        let offset = self.pool.len() as u16;
        self.pool.push(param_index);
        self.accounts.push(AccountEntry {
            is_signer: signer,
            is_writable: writable,
            source_type: AccountSourceType::Param,
            pool_offset: PodU16::from(offset),
            pool_len: PodU16::from(1),
        });
        self
    }

    pub fn add_vault_account(&mut self, signer: bool, writable: bool) -> &mut Self {
        self.accounts.push(AccountEntry {
            is_signer: signer,
            is_writable: writable,
            source_type: AccountSourceType::Vault,
            pool_offset: PodU16::from(0),
            pool_len: PodU16::from(0),
        });
        self
    }

    pub fn add_pda_account(
        &mut self,
        program_account_index: u8,
        pda_seeds: &[PdaSeedSpec],
        signer: bool,
        writable: bool,
    ) -> &mut Self {
        let seeds_start = self.seeds.len() as u16;
        for seed in pda_seeds {
            match seed {
                PdaSeedSpec::Literal(data) => {
                    let offset = self.pool.len() as u16;
                    self.pool.extend_from_slice(data);
                    self.seeds.push(SeedEntry {
                        seed_type: SeedType::Literal,
                        pool_offset: PodU16::from(offset),
                        pool_len: PodU16::from(data.len() as u16),
                    });
                }
                PdaSeedSpec::ParamRef(idx) => {
                    let offset = self.pool.len() as u16;
                    self.pool.push(*idx);
                    self.seeds.push(SeedEntry {
                        seed_type: SeedType::ParamRef,
                        pool_offset: PodU16::from(offset),
                        pool_len: PodU16::from(1),
                    });
                }
                PdaSeedSpec::AccountRef(idx) => {
                    let offset = self.pool.len() as u16;
                    self.pool.push(*idx);
                    self.seeds.push(SeedEntry {
                        seed_type: SeedType::AccountRef,
                        pool_offset: PodU16::from(offset),
                        pool_len: PodU16::from(1),
                    });
                }
            }
        }
        let seeds_count = self.seeds.len() as u16 - seeds_start;

        // PdaDerived pool data: [program_account_index, seeds_start LE, seeds_count LE]
        let offset = self.pool.len() as u16;
        self.pool.push(program_account_index);
        self.pool.extend_from_slice(&seeds_start.to_le_bytes());
        self.pool.extend_from_slice(&seeds_count.to_le_bytes());

        self.accounts.push(AccountEntry {
            is_signer: signer,
            is_writable: writable,
            source_type: AccountSourceType::PdaDerived,
            pool_offset: PodU16::from(offset),
            pool_len: PodU16::from(5),
        });
        self
    }

    pub fn add_has_one_account(&mut self, account_index: u8, byte_offset: u16, signer: bool, writable: bool) -> &mut Self {
        let offset = self.pool.len() as u16;
        self.pool.push(account_index);
        self.pool.extend_from_slice(&byte_offset.to_le_bytes());
        self.accounts.push(AccountEntry {
            is_signer: signer,
            is_writable: writable,
            source_type: AccountSourceType::HasOne,
            pool_offset: PodU16::from(offset),
            pool_len: PodU16::from(3),
        });
        self
    }

    // --- Instructions ---

    pub fn begin_instruction(&mut self, program_account_index: u8) -> InstructionBuilder<'_> {
        let segments_start = self.data_segments.len() as u16;
        let account_indexes_offset = self.pool.len() as u16;
        InstructionBuilder {
            parent: self,
            program_account_index,
            account_indexes_offset,
            account_indexes_len: 0,
            segments_start,
        }
    }

    pub fn set_template(&mut self, template: &str) -> &mut Self {
        self.template = template.to_string();
        self
    }

    pub fn build(mut self) -> BuiltIntent {
        let template_offset = self.pool.len() as u16;
        self.pool.extend_from_slice(self.template.as_bytes());
        let template_len = self.template.len() as u16;

        BuiltIntent {
            approval_threshold: self.approval_threshold,
            cancellation_threshold: self.cancellation_threshold,
            timelock_seconds: self.timelock_seconds,
            template_offset,
            template_len,
            proposers: self.proposers,
            approvers: self.approvers,
            params: self.params,
            accounts: self.accounts,
            instructions: self.instructions,
            data_segments: self.data_segments,
            seeds: self.seeds,
            byte_pool: self.pool,
        }
    }
}

pub struct InstructionBuilder<'a> {
    parent: &'a mut IntentBuilder,
    program_account_index: u8,
    account_indexes_offset: u16,
    account_indexes_len: u16,
    segments_start: u16,
}

impl<'a> InstructionBuilder<'a> {
    pub fn add_account_index(&mut self, index: u8) -> &mut Self {
        self.parent.pool.push(index);
        self.account_indexes_len += 1;
        self
    }

    pub fn add_literal_segment(&mut self, data: &[u8]) -> &mut Self {
        let offset = self.parent.pool.len() as u16;
        self.parent.pool.extend_from_slice(data);
        self.parent.data_segments.push(DataSegmentEntry {
            segment_type: SegmentType::Literal,
            pool_offset: PodU16::from(offset),
            pool_len: PodU16::from(data.len() as u16),
        });
        self
    }

    pub fn add_param_segment(&mut self, param_index: u8, encoding: DataEncoding) -> &mut Self {
        let offset = self.parent.pool.len() as u16;
        self.parent.pool.push(param_index);
        self.parent.pool.push(encoding as u8);
        self.parent.data_segments.push(DataSegmentEntry {
            segment_type: SegmentType::Param,
            pool_offset: PodU16::from(offset),
            pool_len: PodU16::from(2),
        });
        self
    }

    /// Finalize this instruction and add it to the builder.
    pub fn finish(self) {
        let segments_count = self.parent.data_segments.len() as u16 - self.segments_start;
        self.parent.instructions.push(InstructionEntry {
            program_account_index: self.program_account_index,
            account_indexes_offset: PodU16::from(self.account_indexes_offset),
            account_indexes_len: PodU16::from(self.account_indexes_len),
            segments_start: PodU16::from(self.segments_start),
            segments_count: PodU16::from(segments_count),
        });
    }
}

impl BuiltIntent {
    /// Serialize to the raw byte layout matching Intent's quasar zero-copy format
    /// (everything AFTER the discriminator byte). This can be written directly
    /// to an Intent account's data starting at offset 1.
    pub fn serialize_body(
        &self,
        wallet: &Address,
        bump: u8,
        intent_index: u8,
        intent_type: u8,
    ) -> Vec<u8> {
        let mut out = Vec::new();

        // Fixed header (46 bytes)
        out.extend_from_slice(wallet.as_ref());    // 32
        out.push(bump);                             // 1
        out.push(intent_index);                     // 1
        out.push(intent_type);                      // 1
        out.push(1u8); // approved                  // 1
        out.push(self.approval_threshold);           // 1
        out.push(self.cancellation_threshold);       // 1
        out.extend_from_slice(&self.timelock_seconds.to_le_bytes()); // 4
        out.extend_from_slice(&self.template_offset.to_le_bytes()); // 2
        out.extend_from_slice(&self.template_len.to_le_bytes());    // 2
        out.extend_from_slice(&0u16.to_le_bytes());                 // 2: active_proposal_count = 0

        // Dynamic fields: u32 LE count prefix + elements
        write_vec_address(&mut out, &self.proposers);
        write_vec_address(&mut out, &self.approvers);
        write_vec_raw(&mut out, &self.params);
        write_vec_raw(&mut out, &self.accounts);
        write_vec_raw(&mut out, &self.instructions);
        write_vec_raw(&mut out, &self.data_segments);
        write_vec_raw(&mut out, &self.seeds);
        // byte_pool: u32 count + raw bytes
        out.extend_from_slice(&(self.byte_pool.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.byte_pool);

        out
    }
}

fn write_vec_address(out: &mut Vec<u8>, addrs: &[Address]) {
    out.extend_from_slice(&(addrs.len() as u32).to_le_bytes());
    for addr in addrs {
        out.extend_from_slice(addr.as_ref());
    }
}

fn write_vec_raw<T: Copy>(out: &mut Vec<u8>, items: &[T]) {
    out.extend_from_slice(&(items.len() as u32).to_le_bytes());
    let bytes = unsafe {
        core::slice::from_raw_parts(items.as_ptr() as *const u8, core::mem::size_of_val(items))
    };
    out.extend_from_slice(bytes);
}

/// Seed specification for PDA accounts.
pub enum PdaSeedSpec {
    Literal(Vec<u8>),
    ParamRef(u8),
    AccountRef(u8),
}

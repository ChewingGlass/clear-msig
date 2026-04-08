use quasar_lang::prelude::*;

use crate::utils::definition::*;

/// Raw byte offsets in the Intent account data for fields that need
/// direct access from remaining_accounts (where quasar casting isn't available).
/// Layout: disc(1) + wallet(32) + bump(1) + intent_index(1) + intent_type(1) + approved(1)
///  + approval_threshold(1) + cancellation_threshold(1) + timelock_seconds(4)
///  + template_offset(2) + template_len(2) + active_proposal_count(2)
pub const INTENT_APPROVED_OFFSET: usize = 1 + 32 + 1 + 1 + 1; // = 36
pub const INTENT_ACTIVE_PROPOSAL_COUNT_OFFSET: usize = 1 + 32 + 1 + 1 + 1 + 1 + 1 + 1 + 4 + 2 + 2; // = 47

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IntentType {
    AddIntent = 0,
    RemoveIntent = 1,
    UpdateIntent = 2,
    Custom = 3,
}

impl IntentType {
    pub fn from_u8(val: u8) -> Result<Self, ProgramError> {
        match val {
            0 => Ok(Self::AddIntent),
            1 => Ok(Self::RemoveIntent),
            2 => Ok(Self::UpdateIntent),
            3 => Ok(Self::Custom),
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }
}

/// The intent account IS the definition. No separate blob — all fields
/// are directly on the account struct, handled zero-copy by quasar.
#[account(discriminator = 2)]
pub struct Intent<'a> {
    // --- Intent identity ---
    pub wallet: Address,
    pub bump: u8,
    pub intent_index: u8,
    pub intent_type: IntentType,
    pub approved: u8,

    // --- Governance ---
    pub approval_threshold: u8,
    pub cancellation_threshold: u8,
    pub timelock_seconds: u32,
    pub template_offset: u16,
    pub template_len: u16,
    /// Number of open (Active or Approved) proposals using this intent.
    /// Prevents intent modification while proposals are in flight.
    pub active_proposal_count: u16,

    // --- Definition (dynamic, zero-copy) ---
    pub proposers: Vec<'a, Address, 16>,
    pub approvers: Vec<'a, Address, 16>,
    pub params: Vec<'a, ParamEntry, 8>,
    pub accounts: Vec<'a, AccountEntry, 32>,
    pub instructions: Vec<'a, InstructionEntry, 12>,
    pub data_segments: Vec<'a, DataSegmentEntry, 32>,
    pub seeds: Vec<'a, SeedEntry, 32>,
    /// Byte pool for variable data: param names, seed literals,
    /// instruction literal data, static addresses, template string.
    pub byte_pool: Vec<'a, u8, 4096>,
}

impl Intent<'_> {
    pub fn is_approved(&self) -> bool {
        self.approved != 0
    }

    pub fn is_proposer(&self, address: &Address) -> bool {
        self.proposers().iter().any(|a| a == address)
    }

    pub fn is_approver(&self, address: &Address) -> bool {
        self.approvers().iter().any(|a| a == address)
    }

    /// Returns the index of this approver in the approvers list, or None.
    pub fn approver_index(&self, address: &Address) -> Option<u8> {
        self.approvers()
            .iter()
            .position(|a| a == address)
            .map(|i| i as u8)
    }

    pub fn template_str(&self) -> Result<&str, ProgramError> {
        let pool = self.byte_pool();
        let offset = self.template_offset.get() as usize;
        let len = self.template_len.get() as usize;
        if offset + len > pool.len() {
            return Err(ProgramError::InvalidInstructionData);
        }
        core::str::from_utf8(&pool[offset..offset + len])
            .map_err(|_| ProgramError::InvalidInstructionData)
    }

    pub fn pool_slice(&self, offset: u16, len: u16) -> Result<&[u8], ProgramError> {
        let pool = self.byte_pool();
        let start = offset as usize;
        let end = start + len as usize;
        pool.get(start..end).ok_or(ProgramError::InvalidInstructionData)
    }

    pub fn param_name(&self, param: &ParamEntry) -> Result<&str, ProgramError> {
        let bytes = self.pool_slice(param.name_offset.get(), param.name_len.get())?;
        core::str::from_utf8(bytes).map_err(|_| ProgramError::InvalidInstructionData)
    }

    pub fn read_param_bytes<'a>(
        &self,
        params_data: &'a [u8],
        param_index: u8,
    ) -> Result<&'a [u8], ProgramError> {
        let params = self.params();
        let mut offset = 0usize;
        for i in 0..param_index as usize {
            let param = params.get(i).ok_or(ProgramError::InvalidInstructionData)?;
            let pt = param.param_type;
            offset += param_byte_size(pt, params_data, offset)?;
        }
        let param = params
            .get(param_index as usize)
            .ok_or(ProgramError::InvalidInstructionData)?;
        let pt = param.param_type;
        let size = param_byte_size(pt, params_data, offset)?;
        params_data
            .get(offset..offset + size)
            .ok_or(ProgramError::InvalidInstructionData)
    }

    pub fn validate_param_constraints(&self, params_data: &[u8]) -> Result<(), ProgramError> {
        let params = self.params();
        let mut offset = 0usize;
        for param in params {
            let pt = param.param_type;
            match pt {
                ParamType::Address => {
                    require!(
                        offset + 32 <= params_data.len(),
                        ProgramError::InvalidInstructionData
                    );
                    offset += 32;
                }
                ParamType::U64 => {
                    require!(
                        offset + 8 <= params_data.len(),
                        ProgramError::InvalidInstructionData
                    );
                    let val = u64::from_le_bytes(
                        params_data[offset..offset + 8]
                            .try_into()
                            .map_err(|_| ProgramError::InvalidInstructionData)?,
                    );
                    if param.constraint_type == ConstraintType::LessThanU64 {
                        require!(
                            val < param.constraint_value.get(),
                            ProgramError::InvalidArgument
                        );
                    } else if param.constraint_type == ConstraintType::GreaterThanU64 {
                        require!(
                            val > param.constraint_value.get(),
                            ProgramError::InvalidArgument
                        );
                    }
                    offset += 8;
                }
                ParamType::I64 => {
                    require!(
                        offset + 8 <= params_data.len(),
                        ProgramError::InvalidInstructionData
                    );
                    offset += 8;
                }
                ParamType::String => {
                    require!(
                        offset < params_data.len(),
                        ProgramError::InvalidInstructionData
                    );
                    let len = params_data[offset] as usize;
                    offset += 1;
                    require!(
                        offset + len <= params_data.len(),
                        ProgramError::InvalidInstructionData
                    );
                    core::str::from_utf8(&params_data[offset..offset + len])
                        .map_err(|_| ProgramError::InvalidInstructionData)?;
                    offset += len;
                }
                ParamType::Bool | ParamType::U8 => {
                    require!(
                        offset + 1 <= params_data.len(),
                        ProgramError::InvalidInstructionData
                    );
                    offset += 1;
                }
                ParamType::U16 => {
                    require!(
                        offset + 2 <= params_data.len(),
                        ProgramError::InvalidInstructionData
                    );
                    offset += 2;
                }
                ParamType::U32 => {
                    require!(
                        offset + 4 <= params_data.len(),
                        ProgramError::InvalidInstructionData
                    );
                    offset += 4;
                }
                ParamType::U128 => {
                    require!(
                        offset + 16 <= params_data.len(),
                        ProgramError::InvalidInstructionData
                    );
                    offset += 16;
                }
            }
        }
        Ok(())
    }
}

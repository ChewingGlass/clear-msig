use quasar_lang::prelude::*;

// --- Enums ---
// All #[repr(u8)] so they're 1 byte, alignment 1 — safe in #[repr(C)] structs.

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    Address = 0,
    U64 = 1,
    I64 = 2,
    String = 3,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintType {
    None = 0,
    LessThanU64 = 1,
    GreaterThanU64 = 2,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountSourceType {
    Static = 0,
    Param = 1,
    PdaDerived = 2,
    HasOne = 3,
    Vault = 4,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentType {
    Literal = 0,
    Param = 1,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedType {
    Literal = 0,
    ParamRef = 1,
    AccountRef = 2,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataEncoding {
    RawAddress = 0,
    LittleEndianU64 = 1,
    LittleEndianI64 = 2,
}

// --- Fixed-size entry types for Intent Vecs ---
//
// All #[repr(C)] with alignment-1 fields (u8, enums, PodU16/PodU64).
// PodU16/PodU64 are quasar's alignment-1 integer wrappers — they store
// multi-byte integers as [u8; N] so the struct stays alignment 1,
// which is required for quasar's zero-copy pointer casting.
//
// Variable-length or variant-specific data lives in the byte_pool.
// Each entry has a type discriminator and pool_offset + pool_len
// pointing to its data in the pool.

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ParamEntry {
    pub param_type: ParamType,
    /// Byte pool offset + length of the param name (UTF-8).
    pub name_offset: PodU16,
    pub name_len: PodU16,
    pub constraint_type: ConstraintType,
    /// Constraint value (only meaningful when constraint_type != None).
    pub constraint_value: PodU64,
}

/// Account source entry. Pool data layout depends on source_type:
///   Static:     [address: 32 bytes]
///   Param:      [param_index: 1 byte]
///   PdaDerived: [program_account_index: 1, seeds_start: 2 LE, seeds_count: 2 LE]
///   HasOne:     [account_index: 1, byte_offset: 2 LE]
///   Vault:      (empty, pool_len = 0)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AccountEntry {
    pub is_signer: bool,
    pub is_writable: bool,
    pub source_type: AccountSourceType,
    pub pool_offset: PodU16,
    pub pool_len: PodU16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InstructionEntry {
    pub program_account_index: u8,
    /// Byte pool range containing one account index byte per account.
    pub account_indexes_offset: PodU16,
    pub account_indexes_len: PodU16,
    /// Index range into the data_segments Vec.
    pub segments_start: PodU16,
    pub segments_count: PodU16,
}

/// Data segment entry. Pool data layout depends on segment_type:
///   Literal: [raw bytes]
///   Param:   [param_index: 1, encoding: 1]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DataSegmentEntry {
    pub segment_type: SegmentType,
    pub pool_offset: PodU16,
    pub pool_len: PodU16,
}

/// PDA seed entry. Pool data layout depends on seed_type:
///   Literal:    [raw bytes]
///   ParamRef:   [param_index: 1]
///   AccountRef: [account_index: 1]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SeedEntry {
    pub seed_type: SeedType,
    pub pool_offset: PodU16,
    pub pool_len: PodU16,
}

// --- Helpers ---

pub fn param_byte_size(param_type: ParamType, params_data: &[u8], offset: usize) -> Result<usize, ProgramError> {
    match param_type {
        ParamType::Address => Ok(32),
        ParamType::U64 | ParamType::I64 => Ok(8),
        ParamType::String => {
            let len = *params_data.get(offset).ok_or(ProgramError::InvalidInstructionData)? as usize;
            Ok(1 + len)
        }
    }
}

impl DataEncoding {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::RawAddress),
            1 => Some(Self::LittleEndianU64),
            2 => Some(Self::LittleEndianI64),
            _ => None,
        }
    }

    pub fn byte_size(self) -> usize {
        match self {
            Self::RawAddress => 32,
            Self::LittleEndianU64 | Self::LittleEndianI64 => 8,
        }
    }
}

pub fn encoding_byte_size(encoding: DataEncoding) -> usize {
    encoding.byte_size()
}

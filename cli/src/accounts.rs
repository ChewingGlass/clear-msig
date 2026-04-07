use crate::error::*;
use clear_wallet::utils::definition::*;
use serde::Serialize;

/// Deserialized ClearWallet account.
#[derive(Debug, Serialize)]
pub struct WalletAccount {
    pub bump: u8,
    pub proposal_index: u64,
    pub intent_index: u8,
    pub name: String,
}

/// Deserialized Intent account.
#[allow(dead_code)]
pub struct IntentAccount {
    pub wallet: String,
    pub bump: u8,
    pub intent_index: u8,
    pub intent_type: u8,
    pub approved: bool,
    pub approval_threshold: u8,
    pub cancellation_threshold: u8,
    pub timelock_seconds: u32,
    pub template_offset: u16,
    pub template_len: u16,
    pub active_proposal_count: u16,
    pub proposers: Vec<String>,
    pub approvers: Vec<String>,
    pub params: Vec<ParamEntry>,
    pub accounts: Vec<AccountEntry>,
    pub instructions: Vec<InstructionEntry>,
    pub data_segments: Vec<DataSegmentEntry>,
    pub seeds: Vec<SeedEntry>,
    pub byte_pool: Vec<u8>,
}

/// Deserialized Proposal account.
#[derive(Debug, Serialize)]
pub struct ProposalAccount {
    pub wallet: String,
    pub intent: String,
    pub proposal_index: u64,
    pub proposer: String,
    pub status: String,
    pub proposed_at: i64,
    pub approved_at: i64,
    pub bump: u8,
    pub approval_bitmap: u16,
    pub cancellation_bitmap: u16,
    pub rent_refund: String,
    pub params_data: Vec<u8>,
}

fn read_u8(data: &[u8], offset: &mut usize) -> Result<u8> {
    let val = *data.get(*offset).ok_or(anyhow!("unexpected end of data at {}", *offset))?;
    *offset += 1;
    Ok(val)
}

fn read_u16_le(data: &[u8], offset: &mut usize) -> Result<u16> {
    let bytes: [u8; 2] = data.get(*offset..*offset + 2)
        .ok_or(anyhow!("unexpected end of data"))?.try_into()?;
    *offset += 2;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32_le(data: &[u8], offset: &mut usize) -> Result<u32> {
    let bytes: [u8; 4] = data.get(*offset..*offset + 4)
        .ok_or(anyhow!("unexpected end of data"))?.try_into()?;
    *offset += 4;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64_le(data: &[u8], offset: &mut usize) -> Result<u64> {
    let bytes: [u8; 8] = data.get(*offset..*offset + 8)
        .ok_or(anyhow!("unexpected end of data"))?.try_into()?;
    *offset += 8;
    Ok(u64::from_le_bytes(bytes))
}

fn read_i64_le(data: &[u8], offset: &mut usize) -> Result<i64> {
    let bytes: [u8; 8] = data.get(*offset..*offset + 8)
        .ok_or(anyhow!("unexpected end of data"))?.try_into()?;
    *offset += 8;
    Ok(i64::from_le_bytes(bytes))
}

fn read_address(data: &[u8], offset: &mut usize) -> Result<String> {
    let bytes = data.get(*offset..*offset + 32)
        .ok_or(anyhow!("unexpected end of data"))?;
    *offset += 32;
    Ok(bs58::encode(bytes).into_string())
}

fn read_vec_addresses(data: &[u8], offset: &mut usize) -> Result<Vec<String>> {
    let count = read_u32_le(data, offset)? as usize;
    let mut addresses = Vec::with_capacity(count);
    for _ in 0..count {
        addresses.push(read_address(data, offset)?);
    }
    Ok(addresses)
}

fn read_vec_raw<T: Copy>(data: &[u8], offset: &mut usize) -> Result<Vec<T>> {
    let count = read_u32_le(data, offset)? as usize;
    let elem_size = core::mem::size_of::<T>();
    let total = count * elem_size;
    let bytes = data.get(*offset..*offset + total)
        .ok_or(anyhow!("unexpected end of data reading vec of {} elements", count))?;
    let items: Vec<T> = (0..count)
        .map(|i| unsafe { core::ptr::read(bytes[i * elem_size..].as_ptr() as *const T) })
        .collect();
    *offset += total;
    Ok(items)
}

fn read_vec_u8(data: &[u8], offset: &mut usize) -> Result<Vec<u8>> {
    let count = read_u32_le(data, offset)? as usize;
    let bytes = data.get(*offset..*offset + count)
        .ok_or(anyhow!("unexpected end of data reading {} bytes", count))?;
    let result = bytes.to_vec();
    *offset += count;
    Ok(result)
}

pub fn parse_wallet(data: &[u8]) -> Result<WalletAccount> {
    if data.is_empty() || data[0] != 1 {
        return Err(anyhow!("not a ClearWallet account (discriminator={})", data.first().unwrap_or(&0)));
    }
    let mut offset = 1;
    let bump = read_u8(data, &mut offset)?;
    let proposal_index = read_u64_le(data, &mut offset)?;
    let intent_index = read_u8(data, &mut offset)?;
    // name is a dynamic String with u32 LE prefix
    let name_len = read_u32_le(data, &mut offset)? as usize;
    let name_bytes = data.get(offset..offset + name_len)
        .ok_or(anyhow!("unexpected end of data reading name"))?;
    let name = String::from_utf8_lossy(name_bytes).to_string();

    Ok(WalletAccount { bump, proposal_index, intent_index, name })
}

pub fn parse_intent(data: &[u8]) -> Result<IntentAccount> {
    if data.is_empty() || data[0] != 2 {
        return Err(anyhow!("not an Intent account (discriminator={})", data.first().unwrap_or(&0)));
    }
    let mut offset = 1;
    let wallet = read_address(data, &mut offset)?;
    let bump = read_u8(data, &mut offset)?;
    let intent_index = read_u8(data, &mut offset)?;
    let intent_type = read_u8(data, &mut offset)?;
    let approved = read_u8(data, &mut offset)? != 0;
    let approval_threshold = read_u8(data, &mut offset)?;
    let cancellation_threshold = read_u8(data, &mut offset)?;
    let timelock_seconds = read_u32_le(data, &mut offset)?;
    let template_offset = read_u16_le(data, &mut offset)?;
    let template_len = read_u16_le(data, &mut offset)?;
    let active_proposal_count = read_u16_le(data, &mut offset)?;

    let proposers = read_vec_addresses(data, &mut offset)?;
    let approvers = read_vec_addresses(data, &mut offset)?;
    let params = read_vec_raw::<ParamEntry>(data, &mut offset)?;
    let accounts = read_vec_raw::<AccountEntry>(data, &mut offset)?;
    let instructions = read_vec_raw::<InstructionEntry>(data, &mut offset)?;
    let data_segments = read_vec_raw::<DataSegmentEntry>(data, &mut offset)?;
    let seeds = read_vec_raw::<SeedEntry>(data, &mut offset)?;
    let byte_pool = read_vec_u8(data, &mut offset)?;

    Ok(IntentAccount {
        wallet, bump, intent_index, intent_type, approved,
        approval_threshold, cancellation_threshold, timelock_seconds,
        template_offset, template_len, active_proposal_count,
        proposers, approvers, params, accounts, instructions,
        data_segments, seeds, byte_pool,
    })
}

pub fn parse_proposal(data: &[u8]) -> Result<ProposalAccount> {
    if data.is_empty() || data[0] != 3 {
        return Err(anyhow!("not a Proposal account (discriminator={})", data.first().unwrap_or(&0)));
    }
    let mut offset = 1;
    let wallet = read_address(data, &mut offset)?;
    let intent = read_address(data, &mut offset)?;
    let proposal_index = read_u64_le(data, &mut offset)?;
    let proposer = read_address(data, &mut offset)?;
    let status_byte = read_u8(data, &mut offset)?;
    let status = match status_byte {
        0 => "Active", 1 => "Approved", 2 => "Executed", 3 => "Cancelled", _ => "Unknown",
    }.to_string();
    let proposed_at = read_i64_le(data, &mut offset)?;
    let approved_at = read_i64_le(data, &mut offset)?;
    let bump = read_u8(data, &mut offset)?;
    let approval_bitmap = read_u16_le(data, &mut offset)?;
    let cancellation_bitmap = read_u16_le(data, &mut offset)?;
    let rent_refund = read_address(data, &mut offset)?;
    let params_data = read_vec_u8(data, &mut offset)?;

    Ok(ProposalAccount {
        wallet, intent, proposal_index, proposer, status,
        proposed_at, approved_at, bump, approval_bitmap, cancellation_bitmap,
        rent_refund, params_data,
    })
}

impl IntentAccount {
    pub fn intent_type_name(&self) -> &str {
        match self.intent_type {
            0 => "AddIntent",
            1 => "RemoveIntent",
            2 => "UpdateIntent",
            3 => "Custom",
            _ => "Unknown",
        }
    }

    pub fn template(&self) -> &str {
        if self.template_len == 0 { return ""; }
        let start = self.template_offset as usize;
        let end = start + self.template_len as usize;
        if end <= self.byte_pool.len() {
            std::str::from_utf8(&self.byte_pool[start..end]).unwrap_or("")
        } else {
            ""
        }
    }
}

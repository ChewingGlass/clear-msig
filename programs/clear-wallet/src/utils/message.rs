use quasar_lang::prelude::*;

use crate::state::intent::Intent;
use crate::utils::{
    base58::encode_base58,
    datetime::format_timestamp,
    definition::{ParamType, param_byte_size},
};

const MSG_BUF_SIZE: usize = 2048;

/// Solana offchain message header.
/// Format: `\xffsolana offchain` (16) + version (1) + format (1) + length (2 LE) = 20 bytes.
/// This enables Ledger hardware wallets to display the human-readable message.
const OFFCHAIN_SIGNING_DOMAIN: &[u8] = b"\xffsolana offchain";
const OFFCHAIN_HEADER_LEN: usize = 20; // domain(16) + version(1) + format(1) + length(2)

/// Common fields present in every signed message.
pub struct MessageContext<'a> {
    pub expiry: i64,
    pub action: &'a str,
    pub wallet_name: &'a str,
    pub proposal_index: u64,
}

/// Stack-allocated message buffer.
///
/// All messages follow:
///   `expires <ts>: <action> <content> | wallet: <name> proposal: <index>`
///
/// The buffer reserves space for the Solana offchain message header at the front.
/// After building the message, call `as_bytes()` to get the full offchain-wrapped message.
pub struct MessageBuilder {
    buf: [u8; MSG_BUF_SIZE],
    len: usize,
}

impl Default for MessageBuilder {
    fn default() -> Self { Self { buf: [0u8; MSG_BUF_SIZE], len: OFFCHAIN_HEADER_LEN } }
}

impl MessageBuilder {
    pub fn new() -> Self { Self::default() }

    /// Returns the full offchain-wrapped message bytes for signature verification.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// Finalize the offchain header. Must be called after the message body is complete.
    pub fn finalize(&mut self) -> Result<(), ProgramError> {
        let message_len = self.len - OFFCHAIN_HEADER_LEN;
        require!(message_len <= u16::MAX as usize, ProgramError::InvalidInstructionData);

        // Write header into the reserved space
        self.buf[..OFFCHAIN_SIGNING_DOMAIN.len()].copy_from_slice(OFFCHAIN_SIGNING_DOMAIN);
        self.buf[16] = 0; // version 0
        self.buf[17] = 0; // format 0 = restricted ASCII
        let len_bytes = (message_len as u16).to_le_bytes();
        self.buf[18] = len_bytes[0];
        self.buf[19] = len_bytes[1];
        Ok(())
    }

    fn push_bytes(&mut self, data: &[u8]) -> Result<(), ProgramError> {
        let new_len = self.len + data.len();
        require!(new_len <= MSG_BUF_SIZE, ProgramError::InvalidInstructionData);
        self.buf[self.len..new_len].copy_from_slice(data);
        self.len = new_len;
        Ok(())
    }

    fn push_str(&mut self, s: &str) -> Result<(), ProgramError> {
        self.push_bytes(s.as_bytes())
    }

    fn push_base58(&mut self, address: &[u8]) -> Result<(), ProgramError> {
        let mut b58 = [0u8; 44];
        let n = encode_base58(address, &mut b58).ok_or(ProgramError::InvalidInstructionData)?;
        self.push_bytes(&b58[..n])
    }

    pub fn push_u64(&mut self, val: u64) -> Result<(), ProgramError> {
        if val == 0 { return self.push_bytes(b"0"); }
        let mut buf = [0u8; 20];
        let mut pos = 20;
        let mut v = val;
        while v > 0 { pos -= 1; buf[pos] = b'0' + (v % 10) as u8; v /= 10; }
        self.push_bytes(&buf[pos..])
    }

    fn push_i64(&mut self, val: i64) -> Result<(), ProgramError> {
        if val < 0 {
            self.push_bytes(b"-")?;
            self.push_u64((val as u64).wrapping_neg())
        } else {
            self.push_u64(val as u64)
        }
    }

    fn push_timestamp(&mut self, ts: i64) -> Result<(), ProgramError> {
        let start = self.len;
        require!(start + 19 <= MSG_BUF_SIZE, ProgramError::InvalidInstructionData);
        format_timestamp(ts, &mut self.buf[start..]).ok_or(ProgramError::InvalidInstructionData)?;
        self.len = start + 19;
        Ok(())
    }

    fn push_hex(&mut self, data: &[u8]) -> Result<(), ProgramError> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for &b in data {
            self.push_bytes(&[HEX[(b >> 4) as usize], HEX[(b & 0x0f) as usize]])?;
        }
        Ok(())
    }

    fn push_header(&mut self, ctx: &MessageContext<'_>) -> Result<(), ProgramError> {
        self.push_str("expires ")?;
        self.push_timestamp(ctx.expiry)?;
        self.push_str(": ")?;
        self.push_str(ctx.action)?;
        self.push_str(" ")
    }

    fn push_suffix(&mut self, ctx: &MessageContext<'_>) -> Result<(), ProgramError> {
        self.push_str(" | wallet: ")?;
        self.push_str(ctx.wallet_name)?;
        self.push_str(" proposal: ")?;
        self.push_u64(ctx.proposal_index)
    }

    /// Build the appropriate message for any intent type (meta or custom).
    /// Includes Solana offchain message header for Ledger compatibility.
    pub fn build_message_for_intent(
        &mut self,
        ctx: &MessageContext<'_>,
        intent: &Intent<'_>,
        params_data: &[u8],
    ) -> Result<(), ProgramError> {
        use crate::state::intent::IntentType;
        match intent.intent_type {
            IntentType::AddIntent => {
                let h = crate::utils::hash::sha256(params_data);
                self.build_add_intent_message(ctx, &h)?;
            }
            IntentType::RemoveIntent => {
                require!(params_data.len() == 1, ProgramError::InvalidInstructionData);
                self.build_remove_intent_message(ctx, params_data[0])?;
            }
            IntentType::UpdateIntent => {
                require!(params_data.len() > 1, ProgramError::InvalidInstructionData);
                let h = crate::utils::hash::sha256(&params_data[1..]);
                self.build_update_intent_message(ctx, params_data[0], &h)?;
            }
            IntentType::Custom => self.build_custom_message(ctx, intent, params_data)?,
        }
        self.finalize()
    }

    // --- Custom intent messages ---

    pub fn build_custom_message(
        &mut self, ctx: &MessageContext<'_>, intent: &Intent<'_>, params_data: &[u8],
    ) -> Result<(), ProgramError> {
        self.push_header(ctx)?;
        self.render_template(intent, params_data)?;
        self.push_suffix(ctx)
    }

    // --- Meta-intent messages ---

    pub fn build_add_intent_message(
        &mut self, ctx: &MessageContext<'_>, definition_hash: &[u8; 32],
    ) -> Result<(), ProgramError> {
        self.push_header(ctx)?;
        self.push_str("add intent definition_hash: ")?;
        self.push_hex(definition_hash)?;
        self.push_suffix(ctx)
    }

    pub fn build_remove_intent_message(
        &mut self, ctx: &MessageContext<'_>, intent_index: u8,
    ) -> Result<(), ProgramError> {
        self.push_header(ctx)?;
        self.push_str("remove intent ")?;
        self.push_u64(intent_index as u64)?;
        self.push_suffix(ctx)
    }

    pub fn build_update_intent_message(
        &mut self, ctx: &MessageContext<'_>, intent_index: u8, definition_hash: &[u8; 32],
    ) -> Result<(), ProgramError> {
        self.push_header(ctx)?;
        self.push_str("update intent ")?;
        self.push_u64(intent_index as u64)?;
        self.push_str(" definition_hash: ")?;
        self.push_hex(definition_hash)?;
        self.push_suffix(ctx)
    }

    // --- Template rendering ---

    fn render_template(&mut self, intent: &Intent<'_>, params_data: &[u8]) -> Result<(), ProgramError> {
        let template = intent.template_str()?;
        let bytes = template.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'{' {
                let start = i + 1;
                let end = bytes[start..].iter().position(|&b| b == b'}')
                    .ok_or(ProgramError::InvalidInstructionData)? + start;
                let idx = parse_usize(&template[start..end])? as u8;
                self.render_param(intent, params_data, idx)?;
                i = end + 1;
            } else {
                self.push_bytes(&bytes[i..i + 1])?;
                i += 1;
            }
        }
        Ok(())
    }

    fn render_param(&mut self, intent: &Intent<'_>, data: &[u8], idx: u8) -> Result<(), ProgramError> {
        let param = intent.params().get(idx as usize).ok_or(ProgramError::InvalidInstructionData)?;
        let offset = param_offset(intent, data, idx)?;
        match param.param_type {
            ParamType::Address => self.push_base58(&data[offset..offset + 32]),
            ParamType::U64 => {
                let v = u64::from_le_bytes(data[offset..offset+8].try_into().map_err(|_| ProgramError::InvalidInstructionData)?);
                self.push_u64(v)
            }
            ParamType::I64 => {
                let v = i64::from_le_bytes(data[offset..offset+8].try_into().map_err(|_| ProgramError::InvalidInstructionData)?);
                self.push_i64(v)
            }
            ParamType::String => {
                let len = data[offset] as usize;
                let s = core::str::from_utf8(&data[offset+1..offset+1+len]).map_err(|_| ProgramError::InvalidInstructionData)?;
                self.push_str(s)
            }
        }
    }
}

fn param_offset(intent: &Intent<'_>, data: &[u8], target: u8) -> Result<usize, ProgramError> {
    let params = intent.params();
    let mut off = 0usize;
    for i in 0..target as usize {
        let p = params.get(i).ok_or(ProgramError::InvalidInstructionData)?;
        off += param_byte_size(p.param_type, data, off)?;
    }
    Ok(off)
}

fn parse_usize(s: &str) -> Result<usize, ProgramError> {
    let mut r = 0usize;
    for &b in s.as_bytes() {
        require!(b.is_ascii_digit(), ProgramError::InvalidInstructionData);
        r = r.checked_mul(10).and_then(|r| r.checked_add((b - b'0') as usize))
            .ok_or(ProgramError::InvalidInstructionData)?;
    }
    Ok(r)
}

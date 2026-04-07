use crate::intent_builder::{IntentBuilder, BuiltIntent, PdaSeedSpec};
use clear_wallet::utils::definition::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentDefinitionJson {
    pub proposers: Vec<String>,
    pub approvers: Vec<String>,
    #[serde(default = "default_one")]
    pub approval_threshold: u8,
    #[serde(default = "default_one")]
    pub cancellation_threshold: u8,
    #[serde(default)]
    pub timelock_seconds: u32,
    #[serde(default)]
    pub params: Vec<ParamDefJson>,
    #[serde(default)]
    pub accounts: Vec<AccountDefJson>,
    #[serde(default)]
    pub instructions: Vec<InstructionDefJson>,
    #[serde(default)]
    pub template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamDefJson {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: ParamTypeJson,
    #[serde(default)]
    pub constraint: Option<ConstraintJson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamTypeJson { Address, U64, I64, String }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintJson { LessThan(u64), GreaterThan(u64) }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountDefJson {
    pub source: AccountSourceJson,
    #[serde(default)]
    pub signer: bool,
    #[serde(default)]
    pub writable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountSourceJson {
    Static(String),
    Param(u8),
    Pda(PdaSourceJson),
    HasOne(HasOneJson),
    Vault,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdaSourceJson {
    pub program_account_index: u8,
    pub seeds: Vec<SeedJson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedJson { Literal(Vec<u8>), ParamRef(u8), AccountRef(u8) }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HasOneJson { pub account_index: u8, pub byte_offset: u16 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionDefJson {
    pub program_account_index: u8,
    pub account_indexes: Vec<u8>,
    pub data_segments: Vec<DataSegmentJson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSegmentJson {
    Literal(Vec<u8>),
    Param(ParamSegmentJson),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamSegmentJson { pub param_index: u8, pub encoding: DataEncodingJson }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataEncodingJson { RawAddress, LeU64, LeI64 }

/// Transaction-only intent definition (no governance fields).
/// Used by CLI `intent add` / `intent update` where governance comes from flags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentTransactionJson {
    #[serde(default)]
    pub params: Vec<ParamDefJson>,
    #[serde(default)]
    pub accounts: Vec<AccountDefJson>,
    #[serde(default)]
    pub instructions: Vec<InstructionDefJson>,
    #[serde(default)]
    pub template: String,
}

impl IntentTransactionJson {
    /// Convert to a full IntentDefinitionJson by injecting governance fields.
    pub fn with_governance(
        self,
        proposers: Vec<String>,
        approvers: Vec<String>,
        approval_threshold: u8,
        cancellation_threshold: u8,
        timelock_seconds: u32,
    ) -> IntentDefinitionJson {
        IntentDefinitionJson {
            proposers,
            approvers,
            approval_threshold,
            cancellation_threshold,
            timelock_seconds,
            params: self.params,
            accounts: self.accounts,
            instructions: self.instructions,
            template: self.template,
        }
    }
}

fn default_one() -> u8 { 1 }

// --- Conversion ---

impl IntentDefinitionJson {
    pub fn to_built(&self) -> Result<BuiltIntent, String> {
        let mut b = IntentBuilder::new();
        b.set_governance(self.approval_threshold, self.cancellation_threshold, self.timelock_seconds);

        for addr_str in &self.proposers {
            b.add_proposer(parse_address(addr_str)?);
        }
        for addr_str in &self.approvers {
            b.add_approver(parse_address(addr_str)?);
        }
        for param in &self.params {
            let pt = match param.param_type {
                ParamTypeJson::Address => ParamType::Address,
                ParamTypeJson::U64 => ParamType::U64,
                ParamTypeJson::I64 => ParamType::I64,
                ParamTypeJson::String => ParamType::String,
            };
            let constraint = match &param.constraint {
                None => None,
                Some(ConstraintJson::LessThan(v)) => Some((ConstraintType::LessThanU64, *v)),
                Some(ConstraintJson::GreaterThan(v)) => Some((ConstraintType::GreaterThanU64, *v)),
            };
            b.add_param(&param.name, pt, constraint);
        }
        for acct in &self.accounts {
            match &acct.source {
                AccountSourceJson::Static(addr) => { b.add_static_account(parse_address(addr)?, acct.signer, acct.writable); }
                AccountSourceJson::Param(idx) => { b.add_param_account(*idx, acct.signer, acct.writable); }
                AccountSourceJson::Vault => { b.add_vault_account(acct.signer, acct.writable); }
                AccountSourceJson::HasOne(h) => { b.add_has_one_account(h.account_index, h.byte_offset, acct.signer, acct.writable); }
                AccountSourceJson::Pda(pda) => {
                    let seeds: Vec<PdaSeedSpec> = pda.seeds.iter().map(|s| match s {
                        SeedJson::Literal(d) => PdaSeedSpec::Literal(d.clone()),
                        SeedJson::ParamRef(i) => PdaSeedSpec::ParamRef(*i),
                        SeedJson::AccountRef(i) => PdaSeedSpec::AccountRef(*i),
                    }).collect();
                    b.add_pda_account(pda.program_account_index, &seeds, acct.signer, acct.writable);
                }
            }
        }
        for ix in &self.instructions {
            let mut ix_b = b.begin_instruction(ix.program_account_index);
            for &idx in &ix.account_indexes {
                ix_b.add_account_index(idx);
            }
            for seg in &ix.data_segments {
                match seg {
                    DataSegmentJson::Literal(data) => { ix_b.add_literal_segment(data); }
                    DataSegmentJson::Param(p) => {
                        let enc = match p.encoding {
                            DataEncodingJson::RawAddress => DataEncoding::RawAddress,
                            DataEncodingJson::LeU64 => DataEncoding::LittleEndianU64,
                            DataEncodingJson::LeI64 => DataEncoding::LittleEndianI64,
                        };
                        ix_b.add_param_segment(p.param_index, enc);
                    }
                }
            }
            ix_b.finish();
        }
        b.set_template(&self.template);
        Ok(b.build())
    }
}

pub fn json_to_built(json_str: &str) -> Result<BuiltIntent, String> {
    let json: IntentDefinitionJson =
        serde_json::from_str(json_str).map_err(|e| format!("JSON parse error: {e}"))?;
    json.to_built()
}

fn parse_address(s: &str) -> Result<solana_address::Address, String> {
    let bytes = bs58::decode(s)
        .into_vec()
        .map_err(|e| format!("invalid base58 address '{s}': {e}"))?;
    if bytes.len() != 32 {
        return Err(format!("address '{s}' decoded to {} bytes, expected 32", bytes.len()));
    }
    // Length validated above, conversion is infallible
    let arr: [u8; 32] = bytes.try_into().map_err(|_| format!("address '{s}' not 32 bytes"))?;
    Ok(solana_address::Address::new_from_array(arr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_intent_roundtrip() {
        let addr = "11111111111111111111111111111111";
        let json = serde_json::json!({
            "proposers": [addr],
            "approvers": [addr],
            "template": ""
        });
        let built = serde_json::from_value::<IntentDefinitionJson>(json)
            .unwrap().to_built().unwrap();
        assert_eq!(built.proposers.len(), 1);
        assert_eq!(built.approvers.len(), 1);
        assert!(built.params.is_empty());
    }

    #[test]
    fn test_transfer_intent() {
        let sys = "11111111111111111111111111111111";
        let json = serde_json::json!({
            "proposers": [sys],
            "approvers": [sys],
            "approval_threshold": 1,
            "params": [
                { "name": "destination", "type": "address" },
                { "name": "amount", "type": "u64", "constraint": { "less_than": 1000000000 } }
            ],
            "accounts": [
                { "source": { "static": sys }, "signer": false, "writable": false },
                { "source": "vault", "signer": true, "writable": true },
                { "source": { "param": 0 }, "signer": false, "writable": true }
            ],
            "instructions": [{
                "program_account_index": 0,
                "account_indexes": [1, 2],
                "data_segments": [
                    { "literal": [2, 0, 0, 0] },
                    { "param": { "param_index": 1, "encoding": "le_u64" } }
                ]
            }],
            "template": "transfer {1} lamports to {0}"
        });

        let built = serde_json::from_value::<IntentDefinitionJson>(json)
            .unwrap().to_built().unwrap();
        assert_eq!(built.params.len(), 2);
        assert_eq!(built.accounts.len(), 3);
        assert_eq!(built.instructions.len(), 1);
        assert_eq!(built.data_segments.len(), 2);

        assert_eq!(built.template_str(), "transfer {1} lamports to {0}");
    }

    #[test]
    fn test_pda_and_vault() {
        let json = serde_json::json!({
            "proposers": [], "approvers": [],
            "params": [{ "name": "owner", "type": "address" }],
            "accounts": [
                { "source": { "static": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA" }, "signer": false, "writable": false },
                { "source": "vault", "signer": true, "writable": true },
                { "source": { "pda": { "program_account_index": 0, "seeds": [{ "account_ref": 1 }, { "param_ref": 0 }] }}, "signer": false, "writable": true }
            ],
            "template": ""
        });
        let built = serde_json::from_value::<IntentDefinitionJson>(json)
            .unwrap().to_built().unwrap();
        assert_eq!(built.accounts[1].source_type, AccountSourceType::Vault);
        assert_eq!(built.accounts[2].source_type, AccountSourceType::PdaDerived);
        assert_eq!(built.seeds.len(), 2);
    }

    #[test]
    fn test_invalid_address() {
        let json = r#"{ "proposers": ["bad!"], "approvers": [], "template": "" }"#;
        assert!(json_to_built(json).is_err());
    }

    #[test]
    fn test_defaults() {
        let json = r#"{ "proposers": [], "approvers": [], "template": "hi" }"#;
        let built = json_to_built(json).unwrap();
        assert_eq!(built.approval_threshold, 1);
        assert_eq!(built.cancellation_threshold, 1);
        assert_eq!(built.timelock_seconds, 0);
    }
}

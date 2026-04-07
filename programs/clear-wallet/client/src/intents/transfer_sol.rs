use clear_wallet::utils::definition::*;
use crate::intent_builder::{IntentBuilder, BuiltIntent};
use solana_address::Address;

const SYSTEM_PROGRAM: Address = solana_address::address!("11111111111111111111111111111111");

pub struct IntentConfig<'a> {
    pub proposers: &'a [Address],
    pub approvers: &'a [Address],
    pub approval_threshold: u8,
    pub cancellation_threshold: u8,
    pub timelock_seconds: u32,
}

/// Build a SOL transfer intent.
///
/// Params: {0} = destination (address), {1} = amount (u64)
/// Message: "transfer {1} lamports to {0}"
pub fn build(config: &IntentConfig<'_>) -> BuiltIntent {
    let mut b = IntentBuilder::new();

    b.set_governance(config.approval_threshold, config.cancellation_threshold, config.timelock_seconds);

    for p in config.proposers { b.add_proposer(*p); }
    for a in config.approvers { b.add_approver(*a); }

    b.add_param("destination", ParamType::Address, None);
    b.add_param("amount", ParamType::U64, None);

    // account 0: System program
    b.add_static_account(SYSTEM_PROGRAM, false, false);
    // account 1: Vault (PDA signer)
    b.add_vault_account(true, true);
    // account 2: Destination (from param 0)
    b.add_param_account(0, false, true);

    // System program transfer: discriminator [2,0,0,0] + amount u64 LE
    let mut ix = b.begin_instruction(0);
    ix.add_account_index(1); // vault (from)
    ix.add_account_index(2); // destination (to)
    ix.add_literal_segment(&[2, 0, 0, 0]);
    ix.add_param_segment(1, DataEncoding::LittleEndianU64);
    ix.finish();

    b.set_template("transfer {1} lamports to {0}");
    b.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build() {
        let proposer = Address::new_from_array([1u8; 32]);
        let approver = Address::new_from_array([2u8; 32]);

        let built = build(&IntentConfig {
            proposers: &[proposer],
            approvers: &[approver],
            approval_threshold: 1,
            cancellation_threshold: 1,
            timelock_seconds: 3600,
        });

        assert_eq!(built.proposers, vec![proposer]);
        assert_eq!(built.approvers, vec![approver]);
        assert_eq!(built.approval_threshold, 1);
        assert_eq!(built.timelock_seconds, 3600);
        assert_eq!(built.params.len(), 2);
        assert_eq!(built.accounts.len(), 3);
        assert_eq!(built.instructions.len(), 1);
        assert_eq!(built.data_segments.len(), 2);

        assert_eq!(built.template_str(), "transfer {1} lamports to {0}");

        // Verify vault account source
        assert_eq!(built.accounts[1].source_type, AccountSourceType::Vault);
    }
}

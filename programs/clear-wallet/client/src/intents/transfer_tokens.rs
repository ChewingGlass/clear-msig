use clear_wallet::utils::definition::*;
use crate::intent_builder::{IntentBuilder, BuiltIntent, PdaSeedSpec};
use solana_address::Address;

pub use super::transfer_sol::IntentConfig;

const TOKEN_PROGRAM: Address = solana_address::address!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const ATA_PROGRAM: Address = solana_address::address!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const SYSTEM_PROGRAM: Address = solana_address::address!("11111111111111111111111111111111");

/// Build a SPL token transfer intent with idempotent ATA creation.
///
/// Params: {0} = destination (address), {1} = mint (address), {2} = amount (u64)
/// Message: "transfer {2} of mint {1} to {0}"
pub fn build(config: &IntentConfig<'_>) -> BuiltIntent {
    let mut b = IntentBuilder::new();

    b.set_governance(config.approval_threshold, config.cancellation_threshold, config.timelock_seconds);

    for p in config.proposers { b.add_proposer(*p); }
    for a in config.approvers { b.add_approver(*a); }

    b.add_param("destination", ParamType::Address, None);
    b.add_param("mint", ParamType::Address, None);
    b.add_param("amount", ParamType::U64, None);

    // account 0: Token Program
    b.add_static_account(TOKEN_PROGRAM, false, false);
    // account 1: ATA Program
    b.add_static_account(ATA_PROGRAM, false, false);
    // account 2: System Program
    b.add_static_account(SYSTEM_PROGRAM, false, false);
    // account 3: Vault (PDA signer, pays for ATA creation)
    b.add_vault_account(true, true);
    // account 4: Destination wallet (param 0)
    b.add_param_account(0, false, false);
    // account 5: Mint (param 1)
    b.add_param_account(1, false, false);
    // account 6: Source ATA — PDA([vault, token_program, mint], ata_program)
    b.add_pda_account(1, &[
        PdaSeedSpec::AccountRef(3), // vault
        PdaSeedSpec::AccountRef(0), // token program
        PdaSeedSpec::AccountRef(5), // mint
    ], false, true);
    // account 7: Destination ATA — PDA([destination, token_program, mint], ata_program)
    b.add_pda_account(1, &[
        PdaSeedSpec::AccountRef(4), // destination
        PdaSeedSpec::AccountRef(0), // token program
        PdaSeedSpec::AccountRef(5), // mint
    ], false, true);

    // Instruction 0: Create destination ATA (idempotent)
    // ATA program, data = [1]
    let mut ix0 = b.begin_instruction(1); // ATA program = account 1
    ix0.add_account_index(3); // funding = vault
    ix0.add_account_index(7); // ata = dest_ata
    ix0.add_account_index(4); // wallet = destination
    ix0.add_account_index(5); // mint
    ix0.add_account_index(2); // system_program
    ix0.add_account_index(0); // token_program
    ix0.add_literal_segment(&[1]);
    ix0.finish();

    // Instruction 1: SPL Token transfer
    // Token program, data = [3] + amount u64 LE
    let mut ix1 = b.begin_instruction(0); // Token program = account 0
    ix1.add_account_index(6); // source = source_ata
    ix1.add_account_index(7); // dest = dest_ata
    ix1.add_account_index(3); // authority = vault
    ix1.add_literal_segment(&[3]); // Transfer discriminator
    ix1.add_param_segment(2, DataEncoding::LittleEndianU64);
    ix1.finish();

    b.set_template("transfer {2} of mint {1} to {0}");
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
            timelock_seconds: 0,
        });

        assert_eq!(built.params.len(), 3);
        assert_eq!(built.accounts.len(), 8);
        assert_eq!(built.instructions.len(), 2);
        assert_eq!(built.seeds.len(), 6); // 3 seeds per PDA × 2 PDAs

        // Verify vault account
        assert_eq!(built.accounts[3].source_type, AccountSourceType::Vault);

        // Verify PDA accounts
        assert_eq!(built.accounts[6].source_type, AccountSourceType::PdaDerived);
        assert_eq!(built.accounts[7].source_type, AccountSourceType::PdaDerived);

        assert_eq!(built.template_str(), "transfer {2} of mint {1} to {0}");
    }

    #[test]
    fn test_instruction_structure() {
        let built = build(&IntentConfig {
            proposers: &[],
            approvers: &[],
            approval_threshold: 0,
            cancellation_threshold: 0,
            timelock_seconds: 0,
        });

        // Instruction 0: ATA create idempotent
        let ix0 = &built.instructions[0];
        assert_eq!(ix0.program_account_index, 1);
        let acct_indexes_0 = built.pool_slice(ix0.account_indexes_offset, ix0.account_indexes_len);
        assert_eq!(acct_indexes_0, &[3, 7, 4, 5, 2, 0]);

        // Instruction 1: SPL transfer
        let ix1 = &built.instructions[1];
        assert_eq!(ix1.program_account_index, 0);
        let acct_indexes_1 = built.pool_slice(ix1.account_indexes_offset, ix1.account_indexes_len);
        assert_eq!(acct_indexes_1, &[6, 7, 3]);
    }
}

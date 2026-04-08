extern crate std;

use alloc::vec::Vec;
use {
    alloc::vec,
    crate::clear_wallet::cpi::*,
    clear_wallet_client::{
        intents,
        pda::{
            compute_name_hash, find_intent_address, find_proposal_address, find_vault_address,
            find_wallet_address,
        },
    },
    ed25519_dalek::Signer as DalekSigner,
    quasar_lang::client::{DynBytes, DynVec, TailBytes},
    quasar_svm::{Account, Instruction, Pubkey, QuasarSvm},
    sha2::{Digest, Sha256},
    solana_instruction::AccountMeta,
    std::{format, println, string::String},
};

// =========================================================================
// Helpers
// =========================================================================

fn setup() -> QuasarSvm {
    let elf = std::fs::read("../../target/deploy/clear_wallet.so").unwrap();
    QuasarSvm::new().with_program(&crate::ID, &elf)
}

fn setup_with_tokens() -> QuasarSvm {
    let elf = std::fs::read("../../target/deploy/clear_wallet.so").unwrap();
    QuasarSvm::new()
        .with_program(&crate::ID, &elf)
        .with_token_program()
        .with_associated_token_program()
}

fn funded_account(address: Pubkey) -> Account {
    quasar_svm::token::create_keyed_system_account(&address, 10_000_000_000)
}

fn empty_account(address: Pubkey) -> Account {
    Account { address, lamports: 0, data: vec![], owner: quasar_svm::system_program::ID, executable: false }
}

fn new_keypair() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::generate(&mut rand::thread_rng())
}

fn pubkey_of(key: &ed25519_dalek::SigningKey) -> Pubkey {
    Pubkey::from(key.verifying_key().to_bytes())
}

fn pubkey_bytes(key: &ed25519_dalek::SigningKey) -> [u8; 32] {
    key.verifying_key().to_bytes()
}

fn sign_message(key: &ed25519_dalek::SigningKey, msg: &[u8]) -> [u8; 64] {
    key.sign(msg).to_bytes()
}

fn sha256_hash(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

fn format_timestamp(ts: i64) -> String {
    let secs_per_day: i64 = 86400;
    let mut days = ts / secs_per_day;
    let day_secs = ((ts % secs_per_day) + secs_per_day) % secs_per_day;
    if ts < 0 && day_secs > 0 { days -= 1; }
    let (hour, min, sec) = (day_secs / 3600, (day_secs % 3600) / 60, day_secs % 60);
    let adj = days + 719468;
    let era = if adj >= 0 { adj } else { adj - 146096 } / 146097;
    let doe = adj - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02} {hour:02}:{min:02}:{sec:02}")
}

fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

fn message_suffix(wallet_name: &str, proposal_index: u64) -> String {
    format!(" | wallet: {wallet_name} proposal: {proposal_index}")
}

const DEFAULT_EXPIRY: i64 = 1_000_000_000;

type MessageFn = dyn Fn(&str, i64, &str, u64, &[u8]) -> Vec<u8>;

// =========================================================================
// Message builders (must match on-chain format exactly)
// =========================================================================

/// Wraps a plain-text message body with the Solana offchain message header.
/// Format: `\xffsolana offchain` (16) + version(1) + format(1) + length(2 LE)
fn wrap_offchain(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(20 + body.len());
    out.extend_from_slice(b"\xffsolana offchain");
    out.push(0); // version
    out.push(0); // format (restricted ASCII)
    out.extend_from_slice(&(body.len() as u16).to_le_bytes());
    out.extend_from_slice(body);
    out
}

fn add_intent_msg(action: &str, expiry: i64, wallet_name: &str, proposal_index: u64, data: &[u8]) -> Vec<u8> {
    let body = format!(
        "expires {}: {action} add intent definition_hash: {}{}",
        format_timestamp(expiry), hex_encode(&sha256_hash(data)), message_suffix(wallet_name, proposal_index),
    );
    wrap_offchain(body.as_bytes())
}

fn remove_intent_msg(action: &str, expiry: i64, wallet_name: &str, proposal_index: u64, intent_index: u8) -> Vec<u8> {
    let body = format!(
        "expires {}: {action} remove intent {intent_index}{}",
        format_timestamp(expiry), message_suffix(wallet_name, proposal_index),
    );
    wrap_offchain(body.as_bytes())
}

// =========================================================================
// Instruction builder helpers
// =========================================================================

fn create_wallet_ix(
    payer: Pubkey, name: &str, proposers: &[Pubkey], approvers: &[Pubkey], threshold: u8,
) -> (Instruction, Vec<Account>) {
    let name_hash = Pubkey::from(compute_name_hash(name));
    let (wallet, _) = find_wallet_address(name, &crate::ID);
    let (add_intent, _) = find_intent_address(&wallet, 0, &crate::ID);
    let (remove_intent, _) = find_intent_address(&wallet, 1, &crate::ID);
    let (update_intent, _) = find_intent_address(&wallet, 2, &crate::ID);

    let instruction: Instruction = CreateWalletInstruction {
        payer, name_hash, wallet, add_intent, remove_intent, update_intent,
        system_program: quasar_svm::system_program::ID,
        name: DynBytes::new(name.as_bytes().to_vec()),
        approval_threshold: threshold, cancellation_threshold: 1, timelock_seconds: 0,
        proposers: DynVec::new(proposers.iter().map(|p| p.to_bytes()).collect()),
        approvers: DynVec::new(approvers.iter().map(|a| a.to_bytes()).collect()),
    }.into();

    let accounts = vec![funded_account(payer), empty_account(name_hash), empty_account(wallet),
        empty_account(add_intent), empty_account(remove_intent), empty_account(update_intent)];
    (instruction, accounts)
}

struct ProposeArgs {
    payer: Pubkey,
    wallet: Pubkey,
    intent: Pubkey,
    proposal_index: u64,
    expiry: i64,
    proposer_pubkey: [u8; 32],
    signature: [u8; 64],
    params_data: Vec<u8>,
}

fn build_propose_ix(args: ProposeArgs) -> Instruction {
    let (proposal, _) = find_proposal_address(&args.intent, args.proposal_index, &crate::ID);
    ProposeInstruction {
        payer: args.payer, wallet: args.wallet, intent: args.intent, proposal,
        system_program: quasar_svm::system_program::ID,
        expiry: args.expiry, proposer_pubkey: args.proposer_pubkey,
        signature: args.signature, params_data: TailBytes(args.params_data),
    }.into()
}

fn build_approve_ix(wallet: Pubkey, intent: Pubkey, proposal: Pubkey,
    expiry: i64, approver_index: u8, signature: [u8; 64],
) -> Instruction {
    ApproveInstruction { wallet, intent, proposal, expiry, approver_index, signature }.into()
}

fn build_cancel_ix(wallet: Pubkey, intent: Pubkey, proposal: Pubkey,
    expiry: i64, canceller_index: u8, signature: [u8; 64],
) -> Instruction {
    CancelInstruction { wallet, intent, proposal, expiry, canceller_index, signature }.into()
}

fn build_execute_ix(wallet: Pubkey, intent: Pubkey, proposal: Pubkey,
    remaining: Vec<AccountMeta>,
) -> (Instruction, Pubkey) {
    let (vault, _) = find_vault_address(&wallet, &crate::ID);
    let instruction: Instruction = ExecuteInstruction {
        wallet, vault, intent, proposal,
        system_program: quasar_svm::system_program::ID,
        remaining_accounts: remaining,
    }.into();
    (instruction, vault)
}

fn get_proposal_address(intent: Pubkey, index: u64) -> Pubkey {
    find_proposal_address(&intent, index, &crate::ID).0
}

/// Full propose → approve → execute flow.
struct ProposeApproveExecuteArgs<'a> {
    svm: &'a mut QuasarSvm,
    payer: Pubkey,
    wallet: Pubkey,
    wallet_name: &'a str,
    intent: Pubkey,
    proposal_index: u64,
    proposer: &'a ed25519_dalek::SigningKey,
    approver: &'a ed25519_dalek::SigningKey,
    params_data: Vec<u8>,
    msg_fn: &'a MessageFn,
    execute_remaining: Vec<AccountMeta>,
    execute_extra_accounts: Vec<Account>,
}

fn propose_approve_execute(args: ProposeApproveExecuteArgs<'_>) -> Pubkey {
    let proposal_address = get_proposal_address(args.intent, args.proposal_index);

    // Propose
    let msg = (args.msg_fn)("propose", DEFAULT_EXPIRY, args.wallet_name, args.proposal_index, &args.params_data);
    let instruction = build_propose_ix(ProposeArgs {
        payer: args.payer, wallet: args.wallet, intent: args.intent,
        proposal_index: args.proposal_index, expiry: DEFAULT_EXPIRY,
        proposer_pubkey: pubkey_bytes(args.proposer),
        signature: sign_message(args.proposer, &msg),
        params_data: args.params_data.clone(),
    });
    let result = args.svm.process_instruction(&instruction, &[funded_account(args.payer), empty_account(proposal_address)]);
    assert!(result.is_ok(), "propose failed: {:?}", result.raw_result);

    // Approve (approver is always at index 0)
    let msg = (args.msg_fn)("approve", DEFAULT_EXPIRY, args.wallet_name, args.proposal_index, &args.params_data);
    let instruction = build_approve_ix(args.wallet, args.intent, proposal_address, DEFAULT_EXPIRY, 0, sign_message(args.approver, &msg));
    let result = args.svm.process_instruction(&instruction, &[]);
    assert!(result.is_ok(), "approve failed: {:?}", result.raw_result);

    // Execute — vault is already in SVM state, don't overwrite it with empty
    let (instruction, _vault) = build_execute_ix(args.wallet, args.intent, proposal_address, args.execute_remaining);
    let all_accounts = args.execute_extra_accounts;
    let result = args.svm.process_instruction(&instruction, &all_accounts);
    assert!(result.is_ok(), "execute failed: {:?}", result.raw_result);
    println!("  EXECUTE CU: {}", result.compute_units_consumed);

    proposal_address
}

// =========================================================================
// Tests
// =========================================================================

#[test]
fn test_create_wallet() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let (instruction, accounts) = create_wallet_ix(payer, "treasury", &[Pubkey::new_unique()], &[Pubkey::new_unique()], 1);
    let result = svm.process_instruction(&instruction, &accounts);
    assert!(result.is_ok(), "create failed: {:?}", result.raw_result);

    let (wallet, _) = find_wallet_address("treasury", &crate::ID);
    assert_eq!(result.account(&wallet).unwrap().data[0], 1);
    for index in 0..3u8 {
        let (intent_address, _) = find_intent_address(&wallet, index, &crate::ID);
        assert_eq!(result.account(&intent_address).unwrap().data[0], 2);
    }
    println!("  CREATE CU: {}", result.compute_units_consumed);
}

#[test]
fn test_create_wallet_wrong_wallet_address_fails() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = Pubkey::new_unique();
    let approver = Pubkey::new_unique();
    let (wallet, _) = find_wallet_address("wrong-name", &crate::ID);
    let (add_intent, _) = find_intent_address(&wallet, 0, &crate::ID);
    let (remove_intent, _) = find_intent_address(&wallet, 1, &crate::ID);
    let (update_intent, _) = find_intent_address(&wallet, 2, &crate::ID);

    let wrong_name_hash = Pubkey::from([0u8; 32]);
    let instruction: Instruction = CreateWalletInstruction {
        payer, name_hash: wrong_name_hash, wallet, add_intent, remove_intent, update_intent,
        system_program: quasar_svm::system_program::ID,
        name: DynBytes::new(b"actual-name".to_vec()),
        approval_threshold: 1, cancellation_threshold: 1, timelock_seconds: 0,
        proposers: DynVec::new(vec![proposer.to_bytes()]),
        approvers: DynVec::new(vec![approver.to_bytes()]),
    }.into();

    let result = svm.process_instruction(&instruction, &[
        funded_account(payer), empty_account(wrong_name_hash), empty_account(wallet),
        empty_account(add_intent), empty_account(remove_intent), empty_account(update_intent),
    ]);
    assert!(result.is_err(), "wrong wallet address should fail PDA check");
}

#[test]
fn test_create_wallet_bad_threshold_fails() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let (instruction, accounts) = create_wallet_ix(payer, "bad", &[Pubkey::new_unique()], &[Pubkey::new_unique()], 2);
    assert!(svm.process_instruction(&instruction, &accounts).is_err());
}

#[test]
fn test_propose_add_intent() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver = new_keypair();
    let wallet_name = "prop-test";

    let (instruction, accounts) = create_wallet_ix(payer, wallet_name, &[pubkey_of(&proposer)], &[pubkey_of(&approver)], 1);
    assert!(svm.process_instruction(&instruction, &accounts).is_ok());

    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (add_intent, _) = find_intent_address(&wallet, 0, &crate::ID);

    let built = intents::transfer_sol::build(&intents::transfer_sol::IntentConfig {
        proposers: &[pubkey_of(&proposer)], approvers: &[pubkey_of(&approver)],
        approval_threshold: 1, cancellation_threshold: 1, timelock_seconds: 0,
    });
    let params_data = built.serialize_body(&wallet, 0, 3, 3);

    let msg = add_intent_msg("propose", DEFAULT_EXPIRY, wallet_name, 0, &params_data);
    let instruction = build_propose_ix(ProposeArgs {
        payer, wallet, intent: add_intent, proposal_index: 0, expiry: DEFAULT_EXPIRY,
        proposer_pubkey: pubkey_bytes(&proposer), signature: sign_message(&proposer, &msg),
        params_data,
    });
    let proposal_address = get_proposal_address(add_intent, 0);

    let result = svm.process_instruction(&instruction, &[funded_account(payer), empty_account(proposal_address)]);
    assert!(result.is_ok(), "propose failed: {:?}", result.raw_result);
    println!("  PROPOSE CU: {}", result.compute_units_consumed);
}

#[test]
fn test_propose_and_approve_add_intent() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver = new_keypair();
    let wallet_name = "approve-test";

    let (instruction, accounts) = create_wallet_ix(payer, wallet_name, &[pubkey_of(&proposer)], &[pubkey_of(&approver)], 1);
    assert!(svm.process_instruction(&instruction, &accounts).is_ok());

    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (add_intent, _) = find_intent_address(&wallet, 0, &crate::ID);

    let built = intents::transfer_sol::build(&intents::transfer_sol::IntentConfig {
        proposers: &[pubkey_of(&proposer)], approvers: &[pubkey_of(&approver)],
        approval_threshold: 1, cancellation_threshold: 1, timelock_seconds: 0,
    });
    let params_data = built.serialize_body(&wallet, 0, 3, 3);
    let proposal_address = get_proposal_address(add_intent, 0);

    // Propose
    let msg = add_intent_msg("propose", DEFAULT_EXPIRY, wallet_name, 0, &params_data);
    let instruction = build_propose_ix(ProposeArgs {
        payer, wallet, intent: add_intent, proposal_index: 0, expiry: DEFAULT_EXPIRY,
        proposer_pubkey: pubkey_bytes(&proposer), signature: sign_message(&proposer, &msg),
        params_data: params_data.clone(),
    });
    assert!(svm.process_instruction(&instruction, &[funded_account(payer), empty_account(proposal_address)]).is_ok());

    // Approve
    let msg = add_intent_msg("approve", DEFAULT_EXPIRY, wallet_name, 0, &params_data);
    let instruction = build_approve_ix(wallet, add_intent, proposal_address, DEFAULT_EXPIRY, 0, sign_message(&approver, &msg));
    let result = svm.process_instruction(&instruction, &[]);
    assert!(result.is_ok(), "approve failed: {:?}", result.raw_result);

    // Verify Approved status (byte offset 105)
    assert_eq!(svm.get_account(&proposal_address).unwrap().data[105], 1, "status should be Approved(1)");
    println!("  APPROVE CU: {}", result.compute_units_consumed);
}

#[test]
fn test_cancel_overrides_approval() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver1 = new_keypair();
    let approver2 = new_keypair();
    let wallet_name = "cancel-test";

    let (instruction, accounts) = create_wallet_ix(payer, wallet_name,
        &[pubkey_of(&proposer)], &[pubkey_of(&approver1), pubkey_of(&approver2)], 2);
    assert!(svm.process_instruction(&instruction, &accounts).is_ok());

    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (add_intent, _) = find_intent_address(&wallet, 0, &crate::ID);

    let built = intents::transfer_sol::build(&intents::transfer_sol::IntentConfig {
        proposers: &[pubkey_of(&proposer)], approvers: &[pubkey_of(&approver1), pubkey_of(&approver2)],
        approval_threshold: 2, cancellation_threshold: 1, timelock_seconds: 0,
    });
    let params_data = built.serialize_body(&wallet, 0, 3, 3);
    let proposal_address = get_proposal_address(add_intent, 0);

    // Propose
    let msg = add_intent_msg("propose", DEFAULT_EXPIRY, wallet_name, 0, &params_data);
    svm.process_instruction(
        &build_propose_ix(ProposeArgs {
            payer, wallet, intent: add_intent, proposal_index: 0, expiry: DEFAULT_EXPIRY,
            proposer_pubkey: pubkey_bytes(&proposer), signature: sign_message(&proposer, &msg),
            params_data: params_data.clone(),
        }),
        &[funded_account(payer), empty_account(proposal_address)],
    ).unwrap();

    // Approver 1 approves
    let msg = add_intent_msg("approve", DEFAULT_EXPIRY, wallet_name, 0, &params_data);
    svm.process_instruction(&build_approve_ix(wallet, add_intent, proposal_address, DEFAULT_EXPIRY, 0, sign_message(&approver1, &msg)), &[]).unwrap();

    // Approver 1 switches to cancel
    let cancel_msg = wrap_offchain(format!("expires {}: cancel add intent definition_hash: {}{}",
        format_timestamp(DEFAULT_EXPIRY), hex_encode(&sha256_hash(&params_data)), message_suffix(wallet_name, 0)).as_bytes());
    svm.process_instruction(&build_cancel_ix(wallet, add_intent, proposal_address, DEFAULT_EXPIRY, 0, sign_message(&approver1, &cancel_msg)), &[]).unwrap();

    assert_eq!(svm.get_account(&proposal_address).unwrap().data[105], 3, "status should be Cancelled(3)");
}

#[test]
fn test_wrong_signer_propose_fails() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver = new_keypair();
    let wrong_key = new_keypair();
    let wallet_name = "wrong-signer";

    let (instruction, accounts) = create_wallet_ix(payer, wallet_name, &[pubkey_of(&proposer)], &[pubkey_of(&approver)], 1);
    assert!(svm.process_instruction(&instruction, &accounts).is_ok());

    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (add_intent, _) = find_intent_address(&wallet, 0, &crate::ID);

    let params_data = vec![0u8; 10];
    let msg = add_intent_msg("propose", DEFAULT_EXPIRY, wallet_name, 0, &params_data);
    let instruction = build_propose_ix(ProposeArgs {
        payer, wallet, intent: add_intent, proposal_index: 0, expiry: DEFAULT_EXPIRY,
        proposer_pubkey: pubkey_bytes(&wrong_key), signature: sign_message(&wrong_key, &msg),
        params_data,
    });
    let proposal_address = get_proposal_address(add_intent, 0);
    assert!(svm.process_instruction(&instruction, &[funded_account(payer), empty_account(proposal_address)]).is_err());
}

#[test]
fn test_expired_signature_fails() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver = new_keypair();
    let wallet_name = "expired-sig";

    let (instruction, accounts) = create_wallet_ix(payer, wallet_name, &[pubkey_of(&proposer)], &[pubkey_of(&approver)], 1);
    assert!(svm.process_instruction(&instruction, &accounts).is_ok());

    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (add_intent, _) = find_intent_address(&wallet, 0, &crate::ID);

    let params_data = vec![0u8; 10];
    let expired = -1i64;
    let msg = add_intent_msg("propose", expired, wallet_name, 0, &params_data);
    let instruction = build_propose_ix(ProposeArgs {
        payer, wallet, intent: add_intent, proposal_index: 0, expiry: expired,
        proposer_pubkey: pubkey_bytes(&proposer), signature: sign_message(&proposer, &msg),
        params_data,
    });
    let proposal_address = get_proposal_address(add_intent, 0);
    assert!(svm.process_instruction(&instruction, &[funded_account(payer), empty_account(proposal_address)]).is_err());
}

#[test]
fn test_propose_remove_intent() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver = new_keypair();
    let wallet_name = "remove-test";

    let (instruction, accounts) = create_wallet_ix(payer, wallet_name, &[pubkey_of(&proposer)], &[pubkey_of(&approver)], 1);
    assert!(svm.process_instruction(&instruction, &accounts).is_ok());

    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (remove_intent, _) = find_intent_address(&wallet, 1, &crate::ID);

    let params_data = vec![0u8]; // target_index = 0
    let msg = remove_intent_msg("propose", DEFAULT_EXPIRY, wallet_name, 0, 0);
    let instruction = build_propose_ix(ProposeArgs {
        payer, wallet, intent: remove_intent, proposal_index: 0, expiry: DEFAULT_EXPIRY,
        proposer_pubkey: pubkey_bytes(&proposer), signature: sign_message(&proposer, &msg),
        params_data,
    });
    let proposal_address = get_proposal_address(remove_intent, 0);

    let result = svm.process_instruction(&instruction, &[funded_account(payer), empty_account(proposal_address)]);
    assert!(result.is_ok(), "propose remove failed: {:?}", result.raw_result);
    println!("  PROPOSE_REMOVE CU: {}", result.compute_units_consumed);
}

#[test]
fn test_duplicate_approval_fails() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver = new_keypair();
    let wallet_name = "dup-approve";

    let (instruction, accounts) = create_wallet_ix(payer, wallet_name, &[pubkey_of(&proposer)], &[pubkey_of(&approver)], 1);
    assert!(svm.process_instruction(&instruction, &accounts).is_ok());

    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (add_intent, _) = find_intent_address(&wallet, 0, &crate::ID);
    let proposal_address = get_proposal_address(add_intent, 0);

    let params_data = vec![0u8; 10];
    let msg = add_intent_msg("propose", DEFAULT_EXPIRY, wallet_name, 0, &params_data);
    svm.process_instruction(
        &build_propose_ix(ProposeArgs {
            payer, wallet, intent: add_intent, proposal_index: 0, expiry: DEFAULT_EXPIRY,
            proposer_pubkey: pubkey_bytes(&proposer), signature: sign_message(&proposer, &msg),
            params_data: params_data.clone(),
        }),
        &[funded_account(payer), empty_account(proposal_address)],
    ).unwrap();

    let msg = add_intent_msg("approve", DEFAULT_EXPIRY, wallet_name, 0, &params_data);
    let signature = sign_message(&approver, &msg);
    assert!(svm.process_instruction(&build_approve_ix(wallet, add_intent, proposal_address, DEFAULT_EXPIRY, 0, signature), &[]).is_ok());
    assert!(svm.process_instruction(&build_approve_ix(wallet, add_intent, proposal_address, DEFAULT_EXPIRY, 0, signature), &[]).is_err(),
        "duplicate approval should fail");
}

// =========================================================================
// Execute lifecycle tests
// =========================================================================

#[test]
fn test_execute_add_intent() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver = new_keypair();
    let wallet_name = "exec-add";

    let (instruction, accounts) = create_wallet_ix(payer, wallet_name, &[pubkey_of(&proposer)], &[pubkey_of(&approver)], 1);
    assert!(svm.process_instruction(&instruction, &accounts).is_ok());

    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (add_intent, _) = find_intent_address(&wallet, 0, &crate::ID);
    let (new_intent_address, _) = find_intent_address(&wallet, 3, &crate::ID);

    let built = intents::transfer_sol::build(&intents::transfer_sol::IntentConfig {
        proposers: &[pubkey_of(&proposer)], approvers: &[pubkey_of(&approver)],
        approval_threshold: 1, cancellation_threshold: 1, timelock_seconds: 0,
    });
    let params_data = built.serialize_body(&wallet, 0, 3, 3);

    propose_approve_execute(ProposeApproveExecuteArgs {
        svm: &mut svm, payer, wallet, wallet_name, intent: add_intent,
        proposal_index: 0, proposer: &proposer, approver: &approver,
        params_data, msg_fn: &add_intent_msg,
        execute_remaining: vec![AccountMeta::new(payer, true), AccountMeta::new(new_intent_address, false)],
        execute_extra_accounts: vec![funded_account(payer), empty_account(new_intent_address)],
    });

    let intent_data = svm.get_account(&new_intent_address).unwrap();
    assert_eq!(intent_data.data[0], 2, "new intent discriminator");
    assert_eq!(intent_data.owner, crate::ID, "new intent owned by program");
}

#[test]
fn test_execute_remove_intent() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver = new_keypair();
    let wallet_name = "exec-remove";

    let (instruction, accounts) = create_wallet_ix(payer, wallet_name, &[pubkey_of(&proposer)], &[pubkey_of(&approver)], 1);
    assert!(svm.process_instruction(&instruction, &accounts).is_ok());

    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (remove_intent, _) = find_intent_address(&wallet, 1, &crate::ID);
    let (add_intent, _) = find_intent_address(&wallet, 0, &crate::ID);

    propose_approve_execute(ProposeApproveExecuteArgs {
        svm: &mut svm, payer, wallet, wallet_name, intent: remove_intent,
        proposal_index: 0, proposer: &proposer, approver: &approver,
        params_data: vec![0u8],
        msg_fn: &|action, expiry, wallet_name, proposal_index, data|
            remove_intent_msg(action, expiry, wallet_name, proposal_index, data[0]),
        execute_remaining: vec![AccountMeta::new(add_intent, false)],
        execute_extra_accounts: vec![],
    });

    assert_eq!(svm.get_account(&add_intent).unwrap().data[36], 0, "intent should be deactivated");
}

#[test]
fn test_removed_intent_cannot_be_used() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver = new_keypair();
    let wallet_name = "removed-fail";

    let (instruction, accounts) = create_wallet_ix(payer, wallet_name, &[pubkey_of(&proposer)], &[pubkey_of(&approver)], 1);
    assert!(svm.process_instruction(&instruction, &accounts).is_ok());

    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (remove_intent, _) = find_intent_address(&wallet, 1, &crate::ID);
    let (add_intent, _) = find_intent_address(&wallet, 0, &crate::ID);

    // Remove AddIntent
    propose_approve_execute(ProposeApproveExecuteArgs {
        svm: &mut svm, payer, wallet, wallet_name, intent: remove_intent,
        proposal_index: 0, proposer: &proposer, approver: &approver,
        params_data: vec![0u8],
        msg_fn: &|action, expiry, wallet_name, proposal_index, data|
            remove_intent_msg(action, expiry, wallet_name, proposal_index, data[0]),
        execute_remaining: vec![AccountMeta::new(add_intent, false)],
        execute_extra_accounts: vec![],
    });

    // Try to propose via the removed AddIntent — should fail
    let dummy_params = vec![0u8; 10];
    let msg = add_intent_msg("propose", DEFAULT_EXPIRY, wallet_name, 1, &dummy_params);
    let instruction = build_propose_ix(ProposeArgs {
        payer, wallet, intent: add_intent, proposal_index: 1, expiry: DEFAULT_EXPIRY,
        proposer_pubkey: pubkey_bytes(&proposer), signature: sign_message(&proposer, &msg),
        params_data: dummy_params,
    });
    let proposal_address = get_proposal_address(add_intent, 1);
    assert!(svm.process_instruction(&instruction, &[funded_account(payer), empty_account(proposal_address)]).is_err());
}

// =========================================================================
// Comprehensive tests
// =========================================================================

#[test]
fn test_timelock_enforcement() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver = new_keypair();
    let wallet_name = "timelock-test";

    let name_hash = Pubkey::from(compute_name_hash(wallet_name));
    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (add_intent, _) = find_intent_address(&wallet, 0, &crate::ID);
    let (remove_intent, _) = find_intent_address(&wallet, 1, &crate::ID);
    let (update_intent, _) = find_intent_address(&wallet, 2, &crate::ID);

    let instruction: Instruction = CreateWalletInstruction {
        payer, name_hash, wallet, add_intent, remove_intent, update_intent,
        system_program: quasar_svm::system_program::ID,
        name: DynBytes::new(wallet_name.as_bytes().to_vec()),
        approval_threshold: 1, cancellation_threshold: 1,
        timelock_seconds: 3600,
        proposers: DynVec::new(vec![pubkey_of(&proposer).to_bytes()]),
        approvers: DynVec::new(vec![pubkey_of(&approver).to_bytes()]),
    }.into();

    svm.process_instruction(&instruction, &[
        funded_account(payer), empty_account(name_hash), empty_account(wallet),
        empty_account(add_intent), empty_account(remove_intent), empty_account(update_intent),
    ]).unwrap();

    let params_data = vec![0u8];
    let proposal_address = get_proposal_address(remove_intent, 0);

    // Propose + approve
    let msg = remove_intent_msg("propose", DEFAULT_EXPIRY, wallet_name, 0, 0);
    svm.process_instruction(
        &build_propose_ix(ProposeArgs {
            payer, wallet, intent: remove_intent, proposal_index: 0, expiry: DEFAULT_EXPIRY,
            proposer_pubkey: pubkey_bytes(&proposer), signature: sign_message(&proposer, &msg),
            params_data: params_data.clone(),
        }),
        &[funded_account(payer), empty_account(proposal_address)],
    ).unwrap();

    let msg = remove_intent_msg("approve", DEFAULT_EXPIRY, wallet_name, 0, 0);
    svm.process_instruction(&build_approve_ix(wallet, remove_intent, proposal_address, DEFAULT_EXPIRY, 0, sign_message(&approver, &msg)), &[]).unwrap();

    // Execute immediately should fail (clock=0, timelock=3600)
    let (instruction, vault) = build_execute_ix(wallet, remove_intent, proposal_address, vec![AccountMeta::new(add_intent, false)]);
    assert!(svm.process_instruction(&instruction, &[empty_account(vault)]).is_err());
    println!("  TIMELOCK: correctly blocked execution");
}

#[test]
fn test_execute_not_approved_fails() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver = new_keypair();
    let wallet_name = "not-approved";

    let (instruction, accounts) = create_wallet_ix(payer, wallet_name, &[pubkey_of(&proposer)], &[pubkey_of(&approver)], 1);
    assert!(svm.process_instruction(&instruction, &accounts).is_ok());

    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (remove_intent, _) = find_intent_address(&wallet, 1, &crate::ID);
    let (add_intent, _) = find_intent_address(&wallet, 0, &crate::ID);

    // Propose but don't approve
    let params_data = vec![0u8];
    let proposal_address = get_proposal_address(remove_intent, 0);
    let msg = remove_intent_msg("propose", DEFAULT_EXPIRY, wallet_name, 0, 0);
    svm.process_instruction(
        &build_propose_ix(ProposeArgs {
            payer, wallet, intent: remove_intent, proposal_index: 0, expiry: DEFAULT_EXPIRY,
            proposer_pubkey: pubkey_bytes(&proposer), signature: sign_message(&proposer, &msg),
            params_data,
        }),
        &[funded_account(payer), empty_account(proposal_address)],
    ).unwrap();

    let (instruction, vault) = build_execute_ix(wallet, remove_intent, proposal_address, vec![AccountMeta::new(add_intent, false)]);
    assert!(svm.process_instruction(&instruction, &[empty_account(vault)]).is_err());
}

#[test]
fn test_multi_approver_threshold() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver1 = new_keypair();
    let approver2 = new_keypair();
    let approver3 = new_keypair();
    let wallet_name = "multi-approve";

    let (instruction, accounts) = create_wallet_ix(payer, wallet_name,
        &[pubkey_of(&proposer)], &[pubkey_of(&approver1), pubkey_of(&approver2), pubkey_of(&approver3)], 2);
    assert!(svm.process_instruction(&instruction, &accounts).is_ok());

    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (remove_intent, _) = find_intent_address(&wallet, 1, &crate::ID);
    let proposal_address = get_proposal_address(remove_intent, 0);

    let params_data = vec![0u8];
    let msg = remove_intent_msg("propose", DEFAULT_EXPIRY, wallet_name, 0, 0);
    svm.process_instruction(
        &build_propose_ix(ProposeArgs {
            payer, wallet, intent: remove_intent, proposal_index: 0, expiry: DEFAULT_EXPIRY,
            proposer_pubkey: pubkey_bytes(&proposer), signature: sign_message(&proposer, &msg),
            params_data: params_data.clone(),
        }),
        &[funded_account(payer), empty_account(proposal_address)],
    ).unwrap();

    // First approval — not enough
    let msg = remove_intent_msg("approve", DEFAULT_EXPIRY, wallet_name, 0, 0);
    svm.process_instruction(&build_approve_ix(wallet, remove_intent, proposal_address, DEFAULT_EXPIRY, 0, sign_message(&approver1, &msg)), &[]).unwrap();
    assert_eq!(svm.get_account(&proposal_address).unwrap().data[105], 0, "should still be Active");

    // Second approval — threshold met
    svm.process_instruction(&build_approve_ix(wallet, remove_intent, proposal_address, DEFAULT_EXPIRY, 1, sign_message(&approver2, &msg)), &[]).unwrap();
    assert_eq!(svm.get_account(&proposal_address).unwrap().data[105], 1, "should be Approved");
    println!("  MULTI_APPROVE: 2-of-3 threshold works");
}

#[test]
fn test_cancel_reverts_approved_to_active() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver1 = new_keypair();
    let approver2 = new_keypair();
    let wallet_name = "revert-test";

    let name_hash = Pubkey::from(compute_name_hash(wallet_name));
    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (add_intent, _) = find_intent_address(&wallet, 0, &crate::ID);
    let (remove_intent, _) = find_intent_address(&wallet, 1, &crate::ID);
    let (update_intent, _) = find_intent_address(&wallet, 2, &crate::ID);

    let instruction: Instruction = CreateWalletInstruction {
        payer, name_hash, wallet, add_intent, remove_intent, update_intent,
        system_program: quasar_svm::system_program::ID,
        name: DynBytes::new(wallet_name.as_bytes().to_vec()),
        approval_threshold: 2, cancellation_threshold: 2,
        timelock_seconds: 0,
        proposers: DynVec::new(vec![pubkey_of(&proposer).to_bytes()]),
        approvers: DynVec::new(vec![pubkey_of(&approver1).to_bytes(), pubkey_of(&approver2).to_bytes()]),
    }.into();
    svm.process_instruction(&instruction, &[
        funded_account(payer), empty_account(name_hash), empty_account(wallet),
        empty_account(add_intent), empty_account(remove_intent), empty_account(update_intent),
    ]).unwrap();

    let params_data = vec![0u8];
    let proposal_address = get_proposal_address(remove_intent, 0);

    let msg = remove_intent_msg("propose", DEFAULT_EXPIRY, wallet_name, 0, 0);
    svm.process_instruction(
        &build_propose_ix(ProposeArgs {
            payer, wallet, intent: remove_intent, proposal_index: 0, expiry: DEFAULT_EXPIRY,
            proposer_pubkey: pubkey_bytes(&proposer), signature: sign_message(&proposer, &msg),
            params_data: params_data.clone(),
        }),
        &[funded_account(payer), empty_account(proposal_address)],
    ).unwrap();

    // Both approve
    let approve_msg = remove_intent_msg("approve", DEFAULT_EXPIRY, wallet_name, 0, 0);
    svm.process_instruction(&build_approve_ix(wallet, remove_intent, proposal_address, DEFAULT_EXPIRY, 0, sign_message(&approver1, &approve_msg)), &[]).unwrap();
    svm.process_instruction(&build_approve_ix(wallet, remove_intent, proposal_address, DEFAULT_EXPIRY, 1, sign_message(&approver2, &approve_msg)), &[]).unwrap();
    assert_eq!(svm.get_account(&proposal_address).unwrap().data[105], 1, "should be Approved");

    // approver1 switches to cancel
    let cancel_msg = wrap_offchain(format!("expires {}: cancel remove intent 0{}",
        format_timestamp(DEFAULT_EXPIRY), message_suffix(wallet_name, 0)).as_bytes());
    svm.process_instruction(&build_cancel_ix(wallet, remove_intent, proposal_address, DEFAULT_EXPIRY, 0, sign_message(&approver1, &cancel_msg)), &[]).unwrap();

    assert_eq!(svm.get_account(&proposal_address).unwrap().data[105], 0, "should revert to Active");
    println!("  REVERT: Approved → Active after vote switch");
}

#[test]
fn test_non_approver_approve_fails() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver = new_keypair();
    let random_key = new_keypair();
    let wallet_name = "non-approver";

    let (instruction, accounts) = create_wallet_ix(payer, wallet_name, &[pubkey_of(&proposer)], &[pubkey_of(&approver)], 1);
    assert!(svm.process_instruction(&instruction, &accounts).is_ok());

    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (remove_intent, _) = find_intent_address(&wallet, 1, &crate::ID);
    let proposal_address = get_proposal_address(remove_intent, 0);

    let params_data = vec![0u8];
    let msg = remove_intent_msg("propose", DEFAULT_EXPIRY, wallet_name, 0, 0);
    svm.process_instruction(
        &build_propose_ix(ProposeArgs {
            payer, wallet, intent: remove_intent, proposal_index: 0, expiry: DEFAULT_EXPIRY,
            proposer_pubkey: pubkey_bytes(&proposer), signature: sign_message(&proposer, &msg),
            params_data,
        }),
        &[funded_account(payer), empty_account(proposal_address)],
    ).unwrap();

    let msg = remove_intent_msg("approve", DEFAULT_EXPIRY, wallet_name, 0, 0);
    assert!(svm.process_instruction(
        &build_approve_ix(wallet, remove_intent, proposal_address, DEFAULT_EXPIRY, 99, sign_message(&random_key, &msg)), &[]).is_err());
}

#[test]
fn test_full_add_then_remove_lifecycle() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver = new_keypair();
    let wallet_name = "full-lifecycle";

    let (instruction, accounts) = create_wallet_ix(payer, wallet_name, &[pubkey_of(&proposer)], &[pubkey_of(&approver)], 1);
    assert!(svm.process_instruction(&instruction, &accounts).is_ok());

    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (add_intent, _) = find_intent_address(&wallet, 0, &crate::ID);
    let (remove_intent, _) = find_intent_address(&wallet, 1, &crate::ID);
    let (new_intent_address, _) = find_intent_address(&wallet, 3, &crate::ID);

    // 1. Add a transfer intent
    let built = intents::transfer_sol::build(&intents::transfer_sol::IntentConfig {
        proposers: &[pubkey_of(&proposer)], approvers: &[pubkey_of(&approver)],
        approval_threshold: 1, cancellation_threshold: 1, timelock_seconds: 0,
    });
    let params_data = built.serialize_body(&wallet, 0, 3, 3);

    propose_approve_execute(ProposeApproveExecuteArgs {
        svm: &mut svm, payer, wallet, wallet_name, intent: add_intent,
        proposal_index: 0, proposer: &proposer, approver: &approver,
        params_data, msg_fn: &add_intent_msg,
        execute_remaining: vec![AccountMeta::new(payer, true), AccountMeta::new(new_intent_address, false)],
        execute_extra_accounts: vec![funded_account(payer), empty_account(new_intent_address)],
    });
    assert_eq!(svm.get_account(&new_intent_address).unwrap().data[0], 2, "new intent created");

    // 2. Remove the new intent
    propose_approve_execute(ProposeApproveExecuteArgs {
        svm: &mut svm, payer, wallet, wallet_name, intent: remove_intent,
        proposal_index: 1, proposer: &proposer, approver: &approver,
        params_data: vec![3u8],
        msg_fn: &|action, expiry, wallet_name, proposal_index, data|
            remove_intent_msg(action, expiry, wallet_name, proposal_index, data[0]),
        execute_remaining: vec![AccountMeta::new(new_intent_address, false)],
        execute_extra_accounts: vec![],
    });

    assert_eq!(svm.get_account(&new_intent_address).unwrap().data[36], 0, "intent deactivated");

    // 3. Try to propose using deactivated intent — should fail
    let dummy_params = vec![0u8; 10];
    let msg = add_intent_msg("propose", DEFAULT_EXPIRY, wallet_name, 2, &dummy_params);
    let instruction = build_propose_ix(ProposeArgs {
        payer, wallet, intent: new_intent_address, proposal_index: 2, expiry: DEFAULT_EXPIRY,
        proposer_pubkey: pubkey_bytes(&proposer), signature: sign_message(&proposer, &msg),
        params_data: dummy_params,
    });
    let proposal_address = get_proposal_address(new_intent_address, 2);
    assert!(svm.process_instruction(&instruction, &[funded_account(payer), empty_account(proposal_address)]).is_err());
    println!("  FULL_LIFECYCLE: add → remove → reject all passed");
}

#[test]
fn test_remove_add_intent_blocks_future_adds() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver = new_keypair();
    let wallet_name = "block-adds";

    let (instruction, accounts) = create_wallet_ix(payer, wallet_name, &[pubkey_of(&proposer)], &[pubkey_of(&approver)], 1);
    assert!(svm.process_instruction(&instruction, &accounts).is_ok());

    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (add_intent, _) = find_intent_address(&wallet, 0, &crate::ID);
    let (remove_intent, _) = find_intent_address(&wallet, 1, &crate::ID);

    // Remove AddIntent itself
    propose_approve_execute(ProposeApproveExecuteArgs {
        svm: &mut svm, payer, wallet, wallet_name, intent: remove_intent,
        proposal_index: 0, proposer: &proposer, approver: &approver,
        params_data: vec![0u8],
        msg_fn: &|action, expiry, wallet_name, proposal_index, data|
            remove_intent_msg(action, expiry, wallet_name, proposal_index, data[0]),
        execute_remaining: vec![AccountMeta::new(add_intent, false)],
        execute_extra_accounts: vec![],
    });

    // Now try to add an intent — AddIntent is deactivated
    let built = intents::transfer_sol::build(&intents::transfer_sol::IntentConfig {
        proposers: &[pubkey_of(&proposer)], approvers: &[pubkey_of(&approver)],
        approval_threshold: 1, cancellation_threshold: 1, timelock_seconds: 0,
    });
    let params_data = built.serialize_body(&wallet, 0, 3, 3);
    let msg = add_intent_msg("propose", DEFAULT_EXPIRY, wallet_name, 1, &params_data);
    let proposal_address = get_proposal_address(add_intent, 1);
    let instruction = build_propose_ix(ProposeArgs {
        payer, wallet, intent: add_intent, proposal_index: 1, expiry: DEFAULT_EXPIRY,
        proposer_pubkey: pubkey_bytes(&proposer), signature: sign_message(&proposer, &msg),
        params_data,
    });
    assert!(svm.process_instruction(&instruction, &[funded_account(payer), empty_account(proposal_address)]).is_err());
    println!("  BLOCK_ADDS: removing AddIntent blocks future additions");
}

#[test]
#[ignore] // quasar-svm returns UnbalancedInstruction on close; works on real validator
fn test_cleanup_executed_proposal() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver = new_keypair();
    let wallet_name = "cleanup-test";

    let (instruction, accounts) = create_wallet_ix(payer, wallet_name, &[pubkey_of(&proposer)], &[pubkey_of(&approver)], 1);
    assert!(svm.process_instruction(&instruction, &accounts).is_ok());

    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (remove_intent, _) = find_intent_address(&wallet, 1, &crate::ID);
    let (add_intent, _) = find_intent_address(&wallet, 0, &crate::ID);

    let proposal_address = propose_approve_execute(ProposeApproveExecuteArgs {
        svm: &mut svm, payer, wallet, wallet_name, intent: remove_intent,
        proposal_index: 0, proposer: &proposer, approver: &approver,
        params_data: vec![0u8],
        msg_fn: &|action, expiry, wallet_name, proposal_index, data|
            remove_intent_msg(action, expiry, wallet_name, proposal_index, data[0]),
        execute_remaining: vec![AccountMeta::new(add_intent, false)],
        execute_extra_accounts: vec![],
    });

    assert_eq!(svm.get_account(&proposal_address).unwrap().data[105], 2, "should be Executed");

    let instruction: Instruction = CleanupProposalInstruction {
        proposal: proposal_address, rent_refund: payer,
    }.into();
    let result = svm.process_instruction(&instruction, &[]);
    assert!(result.is_ok(), "cleanup failed: {:?}", result.raw_result);

    let account = svm.get_account(&proposal_address);
    assert!(account.is_none_or(|a| a.data.is_empty() || a.lamports == 0), "proposal should be closed");
    println!("  CLEANUP: proposal closed successfully");
}

#[test]
fn test_cleanup_active_proposal_fails() {
    let mut svm = setup();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver = new_keypair();
    let wallet_name = "cleanup-fail";

    let (instruction, accounts) = create_wallet_ix(payer, wallet_name, &[pubkey_of(&proposer)], &[pubkey_of(&approver)], 1);
    assert!(svm.process_instruction(&instruction, &accounts).is_ok());

    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (remove_intent, _) = find_intent_address(&wallet, 1, &crate::ID);

    let params_data = vec![0u8];
    let proposal_address = get_proposal_address(remove_intent, 0);
    let msg = remove_intent_msg("propose", DEFAULT_EXPIRY, wallet_name, 0, 0);
    svm.process_instruction(
        &build_propose_ix(ProposeArgs {
            payer, wallet, intent: remove_intent, proposal_index: 0, expiry: DEFAULT_EXPIRY,
            proposer_pubkey: pubkey_bytes(&proposer), signature: sign_message(&proposer, &msg),
            params_data,
        }),
        &[funded_account(payer), empty_account(proposal_address)],
    ).unwrap();

    let instruction: Instruction = CleanupProposalInstruction {
        proposal: proposal_address, rent_refund: payer,
    }.into();
    assert!(svm.process_instruction(&instruction, &[funded_account(payer)]).is_err());
}

// =========================================================================
// SPL Token transfer test — exercises the full CPI execution engine
// =========================================================================

#[test]
fn test_execute_spl_token_transfer() {
    use quasar_svm::token::{create_keyed_mint_account, create_keyed_token_account, Mint, TokenAccount};
    use quasar_svm::{SPL_TOKEN_PROGRAM_ID, SPL_ASSOCIATED_TOKEN_PROGRAM_ID};
    use spl_token::state::AccountState;
    use spl_token::solana_program::program_pack::Pack;

    let mut svm = setup_with_tokens();
    let payer = Pubkey::new_unique();
    let proposer = new_keypair();
    let approver = new_keypair();
    let wallet_name = "token-transfer";
    let transfer_amount = 500_000u64;

    // 1. Create the wallet
    let (instruction, accounts) = create_wallet_ix(
        payer, wallet_name, &[pubkey_of(&proposer)], &[pubkey_of(&approver)], 1,
    );
    svm.process_instruction(&instruction, &accounts).unwrap();

    let (wallet, _) = find_wallet_address(wallet_name, &crate::ID);
    let (add_intent, _) = find_intent_address(&wallet, 0, &crate::ID);
    let (vault, _) = find_vault_address(&wallet, &crate::ID);

    // 2. Add a transfer_tokens intent
    let built_intent = intents::transfer_tokens::build(&intents::transfer_sol::IntentConfig {
        proposers: &[pubkey_of(&proposer)],
        approvers: &[pubkey_of(&approver)],
        approval_threshold: 1,
        cancellation_threshold: 1,
        timelock_seconds: 0,
    });
    let intent_body = built_intent.serialize_body(&wallet, 0, 3, 3);
    let (new_intent_address, _) = find_intent_address(&wallet, 3, &crate::ID);

    propose_approve_execute(ProposeApproveExecuteArgs {
        svm: &mut svm,
        payer,
        wallet,
        wallet_name,
        intent: add_intent,
        proposal_index: 0,
        proposer: &proposer,
        approver: &approver,
        params_data: intent_body,
        msg_fn: &add_intent_msg,
        execute_remaining: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(new_intent_address, false),
        ],
        execute_extra_accounts: vec![funded_account(payer), empty_account(new_intent_address)],
    });
    assert_eq!(svm.get_account(&new_intent_address).unwrap().data[0], 2, "intent created");

    // 3. Set up token accounts
    let mint_address = Pubkey::new_unique();
    let destination_wallet = Pubkey::new_unique();
    let decimals = 6u8;
    let initial_supply = 1_000_000u64;

    // Create mint
    let mint_account = create_keyed_mint_account(&mint_address, &Mint {
        decimals,
        supply: initial_supply,
        is_initialized: true,
        ..Default::default()
    });

    // Derive ATAs
    let (source_ata, _) = Pubkey::find_program_address(
        &[vault.as_ref(), SPL_TOKEN_PROGRAM_ID.as_ref(), mint_address.as_ref()],
        &SPL_ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    let (dest_ata, _) = Pubkey::find_program_address(
        &[destination_wallet.as_ref(), SPL_TOKEN_PROGRAM_ID.as_ref(), mint_address.as_ref()],
        &SPL_ASSOCIATED_TOKEN_PROGRAM_ID,
    );

    // Create source ATA with tokens
    let source_token_account = create_keyed_token_account(&source_ata, &TokenAccount {
        mint: mint_address,
        owner: vault,
        amount: initial_supply,
        state: AccountState::Initialized,
        ..Default::default()
    });

    // Load token accounts into SVM
    svm.set_account(mint_account);
    svm.set_account(source_token_account);

    // Fund the vault with SOL for ATA creation rent via system transfer
    let fund_vault_ix = solana_instruction::Instruction {
        program_id: quasar_svm::system_program::ID,
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(vault, false),
        ],
        data: {
            let mut d = vec![2, 0, 0, 0]; // Transfer instruction
            d.extend_from_slice(&10_000_000_000u64.to_le_bytes());
            d
        },
    };
    svm.process_instruction(&fund_vault_ix, &[funded_account(payer), empty_account(vault)]).unwrap();

    // 4. Build params_data for the token transfer proposal
    // The transfer_tokens intent params are: destination(address), mint(address), amount(u64)
    let mut params_data = Vec::new();
    params_data.extend_from_slice(destination_wallet.as_ref()); // param 0: destination (32 bytes)
    params_data.extend_from_slice(mint_address.as_ref());        // param 1: mint (32 bytes)
    params_data.extend_from_slice(&transfer_amount.to_le_bytes()); // param 2: amount (8 bytes)

    // 5. Build the human-readable message for this custom intent
    // Template: "transfer {2} of mint {1} to {0}"
    // This needs to match what the on-chain message builder produces.
    let rendered_template = format!(
        "transfer {transfer_amount} of mint {} to {}",
        bs58::encode(mint_address.as_ref()).into_string(),
        bs58::encode(destination_wallet.as_ref()).into_string(),
    );
    let propose_msg = wrap_offchain(format!(
        "expires {}: propose {rendered_template}{}",
        format_timestamp(DEFAULT_EXPIRY),
        message_suffix(wallet_name, 1), // proposal_index = 1 (we already used 0 for add intent)
    ).as_bytes());
    let approve_msg = wrap_offchain(format!(
        "expires {}: approve {rendered_template}{}",
        format_timestamp(DEFAULT_EXPIRY),
        message_suffix(wallet_name, 1),
    ).as_bytes());

    let proposal_address = get_proposal_address(new_intent_address, 1);

    // 6. Propose the token transfer
    let instruction = build_propose_ix(ProposeArgs {
        payer,
        wallet,
        intent: new_intent_address,
        proposal_index: 1,
        expiry: DEFAULT_EXPIRY,
        proposer_pubkey: pubkey_bytes(&proposer),
        signature: sign_message(&proposer, &propose_msg),
        params_data: params_data.clone(),
    });
    let result = svm.process_instruction(&instruction, &[funded_account(payer), empty_account(proposal_address)]);
    assert!(result.is_ok(), "propose token transfer failed: {:?}", result.raw_result);
    println!("  TOKEN PROPOSE CU: {}", result.compute_units_consumed);

    // 7. Approve the token transfer
    let instruction = build_approve_ix(
        wallet, new_intent_address, proposal_address,
        DEFAULT_EXPIRY, 0, sign_message(&approver, &approve_msg),
    );
    let result = svm.process_instruction(&instruction, &[]);
    assert!(result.is_ok(), "approve token transfer failed: {:?}", result.raw_result);
    println!("  TOKEN APPROVE CU: {}", result.compute_units_consumed);

    // 8. Execute the token transfer
    // The transfer_tokens intent defines these accounts:
    //   0: Token Program, 1: ATA Program, 2: System Program,
    //   3: Vault, 4: Destination wallet, 5: Mint,
    //   6: Source ATA (PDA), 7: Dest ATA (PDA)
    let (execute_instruction, _execute_vault) = build_execute_ix(
        wallet, new_intent_address, proposal_address,
        vec![
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(SPL_ASSOCIATED_TOKEN_PROGRAM_ID, false),
            // system_program and vault are NOT passed — they're injected from
            // declared Execute accounts (quasar rejects duplicate remaining accounts)
            AccountMeta::new_readonly(destination_wallet, false),
            AccountMeta::new_readonly(mint_address, false),
            AccountMeta::new(source_ata, false),
            AccountMeta::new(dest_ata, false),
        ],
    );
    let result = svm.process_instruction(&execute_instruction, &[
        empty_account(destination_wallet),
        empty_account(dest_ata),
    ]);
    assert!(result.is_ok(), "execute token transfer failed: {:?}", result.raw_result);
    println!("  TOKEN EXECUTE CU: {}", result.compute_units_consumed);

    // 9. Verify the transfer happened
    let dest_account_data = svm.get_account(&dest_ata).unwrap();
    assert_eq!(dest_account_data.owner, SPL_TOKEN_PROGRAM_ID, "dest ATA should be owned by token program");

    // Parse the token account to check amount
    let dest_token: TokenAccount = TokenAccount::unpack(&dest_account_data.data).unwrap();
    assert_eq!(dest_token.amount, transfer_amount, "dest should have received tokens");
    assert_eq!(dest_token.owner, destination_wallet, "dest ATA should be owned by destination wallet");
    assert_eq!(dest_token.mint, mint_address, "dest ATA should have correct mint");

    // Check source was debited
    let source_account_data = svm.get_account(&source_ata).unwrap();
    let source_token: TokenAccount = TokenAccount::unpack(&source_account_data.data).unwrap();
    assert_eq!(source_token.amount, initial_supply - transfer_amount, "source should be debited");

    println!("  TOKEN_TRANSFER: {transfer_amount} tokens transferred successfully!");
}

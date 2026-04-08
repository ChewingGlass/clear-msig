use crate::accounts::IntentAccount;
use crate::error::*;
use crate::rpc;
use clear_wallet::utils::definition::*;
use solana_client::rpc_client::RpcClient;
use solana_instruction::AccountMeta;
use solana_pubkey::Pubkey;

/// Resolve all remaining accounts needed for `execute` based on intent type.
///
/// For meta-intents (AddIntent, RemoveIntent, UpdateIntent), the remaining
/// accounts are deterministic from the wallet + proposal.
///
/// For custom intents, each AccountEntry is resolved in order — later entries
/// can reference earlier ones (AccountRef seeds, HasOne).
pub fn resolve_remaining_accounts(
    rpc: &RpcClient,
    intent: &IntentAccount,
    wallet: &Pubkey,
    vault: &Pubkey,
    params_data: &[u8],
    payer: &Pubkey,
) -> Result<Vec<AccountMeta>> {
    match intent.intent_type {
        // AddIntent: remaining = [payer(mut,signer), new_intent(mut)]
        0 => {
            let next_index = {
                let wallet_data = rpc::fetch_account(rpc, wallet)?;
                let w = crate::accounts::parse_wallet(&wallet_data)?;
                w.intent_index + 1
            };
            let program_id = crate::instructions::program_id();
            let (new_intent, _) = Pubkey::find_program_address(
                &[b"intent", wallet.as_ref(), &[next_index]],
                &program_id,
            );
            Ok(vec![
                AccountMeta::new(*payer, true),
                AccountMeta::new(new_intent, false),
            ])
        }
        // RemoveIntent: remaining = [target_intent(mut)]
        1 => {
            if params_data.is_empty() {
                return Err(anyhow!("RemoveIntent params must be 1 byte"));
            }
            let target_index = params_data[0];
            let program_id = crate::instructions::program_id();
            let (target_intent, _) = Pubkey::find_program_address(
                &[b"intent", wallet.as_ref(), &[target_index]],
                &program_id,
            );
            Ok(vec![AccountMeta::new(target_intent, false)])
        }
        // UpdateIntent: remaining = [payer(mut,signer), target_intent(mut)]
        2 => {
            if params_data.is_empty() {
                return Err(anyhow!("UpdateIntent params must be >1 byte"));
            }
            let target_index = params_data[0];
            let program_id = crate::instructions::program_id();
            let (target_intent, _) = Pubkey::find_program_address(
                &[b"intent", wallet.as_ref(), &[target_index]],
                &program_id,
            );
            Ok(vec![
                AccountMeta::new(*payer, true),
                AccountMeta::new(target_intent, false),
            ])
        }
        // Custom: resolve each AccountEntry
        3 => resolve_custom_accounts(rpc, intent, wallet, vault, params_data),
        _ => Err(anyhow!("unknown intent type {}", intent.intent_type)),
    }
}

fn resolve_custom_accounts(
    rpc: &RpcClient,
    intent: &IntentAccount,
    wallet: &Pubkey,
    vault: &Pubkey,
    params_data: &[u8],
) -> Result<Vec<AccountMeta>> {
    let pool = &intent.byte_pool;
    let mut resolved: Vec<Pubkey> = Vec::new();

    for entry in &intent.accounts {
        let address = resolve_account_source(
            rpc, entry, pool, &intent.seeds, &intent.params,
            params_data, wallet, vault, &resolved,
        )?;
        resolved.push(address);
    }

    // All remaining accounts are passed as non-signers from the client.
    // The program handles CPI signing via invoke_signed with vault PDA seeds.
    Ok(intent
        .accounts
        .iter()
        .zip(resolved.iter())
        .map(|(entry, addr)| {
            if entry.is_writable {
                AccountMeta::new(*addr, false)
            } else {
                AccountMeta::new_readonly(*addr, false)
            }
        })
        .collect())
}

fn resolve_account_source(
    rpc: &RpcClient,
    entry: &AccountEntry,
    pool: &[u8],
    seeds: &[SeedEntry],
    params: &[ParamEntry],
    params_data: &[u8],
    _wallet: &Pubkey,
    vault: &Pubkey,
    resolved: &[Pubkey],
) -> Result<Pubkey> {
    let offset = entry.pool_offset.get() as usize;
    let len = entry.pool_len.get() as usize;
    let pool_data = pool.get(offset..offset + len).unwrap_or(&[]);

    match entry.source_type {
        AccountSourceType::Static => {
            if pool_data.len() != 32 {
                return Err(anyhow!("static account needs 32 bytes, got {}", pool_data.len()));
            }
            Ok(Pubkey::new_from_array(pool_data.try_into().unwrap()))
        }
        AccountSourceType::Param => {
            if pool_data.is_empty() {
                return Err(anyhow!("param account source missing index"));
            }
            let param_idx = pool_data[0] as usize;
            let param_offset = compute_param_offset(params, params_data, param_idx)?;
            let addr_bytes: [u8; 32] = params_data[param_offset..param_offset + 32]
                .try_into()
                .with_context(|| "not enough param data for address")?;
            Ok(Pubkey::new_from_array(addr_bytes))
        }
        AccountSourceType::Vault => Ok(*vault),
        AccountSourceType::PdaDerived => {
            if pool_data.len() < 5 {
                return Err(anyhow!("PdaDerived pool data too short"));
            }
            let program_account_index = pool_data[0] as usize;
            let seeds_start = u16::from_le_bytes([pool_data[1], pool_data[2]]) as usize;
            let seeds_count = u16::from_le_bytes([pool_data[3], pool_data[4]]) as usize;

            let program_id = resolved.get(program_account_index)
                .ok_or(anyhow!("PDA program account index {} not yet resolved", program_account_index))?;

            let mut seed_bufs: Vec<Vec<u8>> = Vec::new();
            for i in seeds_start..seeds_start + seeds_count {
                let seed_entry = seeds.get(i)
                    .ok_or(anyhow!("seed index {i} out of bounds"))?;
                let seed_offset = seed_entry.pool_offset.get() as usize;
                let seed_len = seed_entry.pool_len.get() as usize;
                let seed_data = &pool[seed_offset..seed_offset + seed_len];

                let buf = match seed_entry.seed_type {
                    SeedType::Literal => seed_data.to_vec(),
                    SeedType::ParamRef => {
                        let param_idx = seed_data[0] as usize;
                        let param_off = compute_param_offset(params, params_data, param_idx)?;
                        let param = &params[param_idx];
                        let size = param_byte_size(param.param_type, params_data, param_off)?;
                        params_data[param_off..param_off + size].to_vec()
                    }
                    SeedType::AccountRef => {
                        let acct_idx = seed_data[0] as usize;
                        let acct = resolved.get(acct_idx)
                            .ok_or(anyhow!("AccountRef {acct_idx} not yet resolved"))?;
                        acct.to_bytes().to_vec()
                    }
                };
                seed_bufs.push(buf);
            }

            let seed_refs: Vec<&[u8]> = seed_bufs.iter().map(|s| s.as_slice()).collect();
            let (pda, _) = Pubkey::find_program_address(&seed_refs, program_id);
            Ok(pda)
        }
        AccountSourceType::HasOne => {
            if pool_data.len() < 3 {
                return Err(anyhow!("HasOne pool data too short"));
            }
            let account_index = pool_data[0] as usize;
            let byte_offset = u16::from_le_bytes([pool_data[1], pool_data[2]]) as usize;

            let source_addr = resolved.get(account_index)
                .ok_or(anyhow!("HasOne references account {account_index} not yet resolved"))?;
            let account_data = rpc::fetch_account(rpc, source_addr)?;
            let addr_bytes: [u8; 32] = account_data
                .get(byte_offset..byte_offset + 32)
                .ok_or(anyhow!("HasOne byte_offset {} out of bounds", byte_offset))?
                .try_into()?;
            Ok(Pubkey::new_from_array(addr_bytes))
        }
    }
}

fn compute_param_offset(params: &[ParamEntry], params_data: &[u8], target: usize) -> Result<usize> {
    let mut offset = 0usize;
    for i in 0..target {
        let param = params.get(i).ok_or(anyhow!("param index {i} out of bounds"))?;
        offset += param_byte_size(param.param_type, params_data, offset)
            .map_err(|_| anyhow!("error computing param size at index {i}"))?;
    }
    Ok(offset)
}

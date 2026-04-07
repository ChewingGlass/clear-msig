use crate::accounts::IntentAccount;
use crate::error::*;
use clear_wallet::utils::definition::ParamType;
use std::collections::HashMap;

/// Encode --param key=value pairs into params_data bytes,
/// ordered by the intent's param definitions.
pub fn encode_params(intent: &IntentAccount, raw_params: &[String]) -> Result<Vec<u8>> {
    // Parse key=value pairs
    let mut param_map: HashMap<String, String> = HashMap::new();
    for raw in raw_params {
        let (key, value) = raw.split_once('=')
            .ok_or(anyhow!("invalid param format: '{raw}', expected key=value"))?;
        param_map.insert(key.to_string(), value.to_string());
    }

    let mut data = Vec::new();
    let pool = &intent.byte_pool;

    for param in &intent.params {
        let name_start = param.name_offset.get() as usize;
        let name_end = name_start + param.name_len.get() as usize;
        let name = std::str::from_utf8(&pool[name_start..name_end])
            .with_context(|| "invalid param name in intent")?;

        let value = param_map.get(name)
            .ok_or(anyhow!("missing required param: {name}"))?;

        match param.param_type {
            ParamType::Address => {
                let bytes = bs58::decode(value).into_vec()
                    .with_context(|| format!("invalid address for param {name}: {value}"))?;
                if bytes.len() != 32 {
                    return Err(anyhow!("address for {name} must be 32 bytes, got {}", bytes.len()));
                }
                data.extend_from_slice(&bytes);
            }
            ParamType::U64 => {
                let val: u64 = value.parse()
                    .with_context(|| format!("invalid u64 for param {name}: {value}"))?;
                data.extend_from_slice(&val.to_le_bytes());
            }
            ParamType::I64 => {
                let val: i64 = value.parse()
                    .with_context(|| format!("invalid i64 for param {name}: {value}"))?;
                data.extend_from_slice(&val.to_le_bytes());
            }
            ParamType::String => {
                let bytes = value.as_bytes();
                if bytes.len() > 255 {
                    return Err(anyhow!("string param {name} too long (max 255 bytes)"));
                }
                data.push(bytes.len() as u8);
                data.extend_from_slice(bytes);
            }
        }
    }

    // Warn about extra params
    for key in param_map.keys() {
        let name_bytes = key.as_bytes();
        let found = intent.params.iter().any(|p| {
            let start = p.name_offset.get() as usize;
            let end = start + p.name_len.get() as usize;
            pool.get(start..end) == Some(name_bytes)
        });
        if !found {
            eprintln!("warning: unknown param '{key}' (not defined in intent)");
        }
    }

    Ok(data)
}

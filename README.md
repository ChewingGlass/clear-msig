# clear-msig

A clear-sign multisig wallet for Solana. Signers approve human-readable messages via ed25519 signatures instead of signing opaque transactions.

Built with [Quasar](https://github.com/blueshift-gg/quasar).

## How It Works

**Wallets** hold a set of **intents** — pre-configured transaction blueprints that define what the wallet can do. Each intent specifies its own proposers, approvers, thresholds, and timelock.

Three meta-intents are created with every wallet:
- **AddIntent** (index 0) — add new intents
- **RemoveIntent** (index 1) — disable existing intents
- **UpdateIntent** (index 2) — replace an intent's definition

Custom intents define parameters, accounts, instructions, and a human-readable template. For example, a SOL transfer intent with template `"transfer {1} lamports to {0}"` produces messages like:

```
expires 2030-01-01 00:00:00: approve transfer 1000000000 lamports to 9abc... | wallet: treasury proposal: 42
```

Signers see exactly what they're approving.

## Architecture

```
Wallet (PDA: ["clear_wallet", sha256(name)])
  └── Vault (PDA: ["vault", wallet]) — holds funds, signs CPIs
  └── Intent 0: AddIntent
  └── Intent 1: RemoveIntent
  └── Intent 2: UpdateIntent
  └── Intent 3+: Custom intents (transfer SOL, transfer tokens, etc.)

Proposal (PDA: ["proposal", intent, index_le_bytes])
  └── params_data: encoded parameter values
  └── approval_bitmap / cancellation_bitmap: u16 bitmaps over approver list
  └── rent_refund: address to receive rent on cleanup
```

### Proposal Lifecycle

1. **Propose** — a proposer signs a human-readable message and submits it with parameters
2. **Approve** — approvers sign the same message; bitmap tracks who approved
3. **Execute** — once threshold is met and timelock elapsed, anyone can execute
4. **Cleanup** — reclaim rent from executed/cancelled proposals

Vote switching is supported: approving clears your cancellation, and vice versa.

## Project Structure

```
programs/clear-wallet/          # On-chain program (Quasar)
  src/
    state/                      # Wallet, Intent, Proposal accounts
    instructions/               # create_wallet, propose, approve, cancel, execute, cleanup
    utils/                      # Message building, base58, datetime, sha256
  client/                       # Off-chain helpers (PDA derivation, intent builder, JSON parsing)
cli/                            # CLI tool (clear-msig)
examples/intents/               # Example intent JSON files
```

## Prerequisites

- Rust
- [Quasar CLI](https://github.com/blueshift-gg/quasar)
- Agave (Solana validator) **v3.1+** — required for the SBPFv2 r2 data pointer feature

```bash
agave-install init 3.1.12
```

## Build

```bash
# Build the on-chain program
cd programs/clear-wallet
quasar build

# Build the CLI
cargo build -p clear-msig-cli
```

## Test

```bash
# Run all tests (23 on-chain + 8 client)
cargo test
```

## Deploy to Localnet

```bash
# Start a local validator
solana-test-validator --reset &

# Build (from the program directory)
cd programs/clear-wallet
quasar build

# Deploy (from the workspace root, where target/deploy/ lives)
cd ../..
quasar deploy -u http://localhost:8899 --skip-build \
  --program-keypair target/deploy/clear_wallet-keypair.json

# Point CLI at localnet
clear-msig config set --url http://localhost:8899
clear-msig config set --signer ~/.config/solana/id.json
```

## CLI Usage

### Create a Wallet

```bash
clear-msig wallet create \
  --name "treasury" \
  --proposers <addr1>,<addr2> \
  --approvers <addr1>,<addr2> \
  --threshold 2 \
  --cancellation-threshold 1 \
  --timelock 3600
```

### Add a Custom Intent

Intent definitions are JSON files with parameters, accounts, instructions, and a template. Governance (proposers, approvers, threshold) comes from CLI flags.

```bash
clear-msig intent add \
  --wallet "treasury" \
  --file examples/intents/transfer_sol.json \
  --proposers <addr1> \
  --approvers <addr1>,<addr2> \
  --threshold 2
```

This creates a proposal via AddIntent. Approve and execute it to activate.

### Propose, Approve, Execute

```bash
# Create a proposal against a custom intent
clear-msig proposal create \
  --wallet "treasury" \
  --intent-index 3 \
  --param destination=<address> \
  --param amount=1000000000

# Approve it
clear-msig proposal approve \
  --wallet "treasury" \
  --proposal <proposal-address>

# Execute once threshold is met
clear-msig proposal execute \
  --wallet "treasury" \
  --proposal <proposal-address>
```

### Other Commands

```bash
clear-msig wallet show --name "treasury"
clear-msig intent list --wallet "treasury"
clear-msig proposal list --wallet "treasury"
clear-msig proposal show --proposal <address>
clear-msig proposal cleanup --proposal <address>
clear-msig config show
```

All commands output JSON to stdout.

## End-to-End Example

Full localnet walkthrough — create a wallet, add a SOL transfer intent, transfer 1 SOL from the vault:

```bash
# Setup
SELF=$(solana address)
clear-msig config set --url http://localhost:8899
clear-msig config set --signer ~/.config/solana/id.json

# 1. Create wallet
clear-msig wallet create \
  --name "demo" \
  --proposers "$SELF" \
  --approvers "$SELF" \
  --threshold 1

# 2. Add a SOL transfer intent (proposes via AddIntent)
clear-msig intent add \
  --wallet "demo" \
  --file examples/intents/transfer_sol.json \
  --proposers "$SELF" \
  --approvers "$SELF" \
  --threshold 1
# Note the proposal address from the output

# 3. Approve and execute the add-intent proposal
clear-msig proposal approve --wallet "demo" --proposal <add-proposal>
clear-msig proposal execute --wallet "demo" --proposal <add-proposal>

# 4. Verify the new intent (index 3)
clear-msig intent list --wallet "demo"

# 5. Fund the vault
VAULT=$(clear-msig wallet show --name "demo" | jq -r .vault)
solana transfer "$VAULT" 2 --allow-unfunded-recipient

# 6. Create a transfer proposal
clear-msig proposal create \
  --wallet "demo" \
  --intent-index 3 \
  --param "destination=<recipient-address>" \
  --param "amount=1000000000"

# 7. Approve and execute the transfer
clear-msig proposal approve --wallet "demo" --proposal <transfer-proposal>
clear-msig proposal execute --wallet "demo" --proposal <transfer-proposal>

# 8. Verify
solana balance <recipient-address>  # Should show 1 SOL
```

## Intent JSON Format

Intent files define the transaction blueprint without governance fields:

```json
{
  "params": [
    { "name": "destination", "type": "address" },
    { "name": "amount", "type": "u64" }
  ],
  "accounts": [
    { "source": { "static": "11111111111111111111111111111111" }, "signer": false, "writable": false },
    { "source": "vault", "signer": true, "writable": true },
    { "source": { "param": 0 }, "signer": false, "writable": true }
  ],
  "instructions": [
    {
      "program_account_index": 0,
      "account_indexes": [1, 2],
      "data_segments": [
        { "literal": [2, 0, 0, 0] },
        { "param": { "param_index": 1, "encoding": "le_u64" } }
      ]
    }
  ],
  "template": "transfer {1} lamports to {0}"
}
```

### Account Sources

| Source | Description |
|--------|-------------|
| `{ "static": "<address>" }` | Hardcoded address (e.g., system program) |
| `{ "param": <index> }` | Address from a parameter |
| `"vault"` | The wallet's vault PDA |
| `{ "pda": { "program_account_index": N, "seeds": [...] } }` | Derived PDA |
| `{ "has_one": { "account_index": N, "byte_offset": M } }` | Read address from another account's data |

### Parameter Types

`address`, `u64`, `i64`, `string`

### Data Encodings

`raw_address`, `le_u64`, `le_i64`

See `examples/intents/transfer_sol.json` and `examples/intents/transfer_tokens.json` for complete examples.

## Two-Identity Model

The CLI manages two distinct identities:

- **Payer** — standard Solana keypair that signs transactions and pays fees
- **Signer** — ed25519 identity for multisig message signing (proposer/approver)

These can be the same keypair (default) or different — e.g., a relayer pays gas while a hardware wallet holder signs messages.

```bash
clear-msig config set --keypair ~/payer.json
clear-msig config set --signer ~/signer.json
```

## Known Issues

- `proposal cleanup` fails on localnet due to a quasar framework issue with `close` attribute. Works conceptually but blocked by quasar-svm's `UnbalancedInstruction` error in tests and a `MissingRequiredSignature` on the real validator.
- Requires Agave v3.1+ for the SBPFv2 r2 data pointer. Earlier versions crash with `Access violation at address 0xfffffffffffffff8`.

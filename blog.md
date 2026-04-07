# Beyond Blind Signing

The @DriftProtocol exploit has everyone justifiably scared. Multisig signers had their computers compromised. Even though they were using hardware wallets, they were tricked into signing malicious transactions. Bybit had a similar issue — their multisig UI displayed a safe transaction but routed an unsafe one to the hardware wallet. The signer's computer wasn't even compromised; the UI serving the application was.

We've been pitching hardware wallets as the solution for years, but they clearly leave a lot to be desired.

## The Real Problem

Nobody actually knows what they're signing.

Thousands of experts have since weighed in saying Drift should be using dedicated signing laptops. Sure, that would have helped. But take a step back — why can I, a human in the loop, not know for a fact what I'm signing on my hardware wallet? What's the point of a hardware wallet if context only makes sense on a dedicated laptop? How stupid is it that you need a hardware wallet AND a dedicated laptop AND you still can't trust the web UI on that laptop?

It's easy to blame @Ledger for not having clear signing, but remember: these are multisig transactions. Even with clear signing, all the Ledger can show you is an on-chain transaction ID. The actual transaction definition lives on-chain. Do you know how to read that? Do you trust the UI reading it for you? Bybit and Drift both proved you can't trust your computer, and you can't trust the UI. You certainly can't trust humans to read raw Solana transactions.

## The Solution

Solana transactions were never meant to be human-readable, and there is only pain in trying to make them so. But Ledger and other hardware wallets support `signMessage`. On-chain we have `brine-ed25519`. Why not create a new smart wallet primitive that executes explicit actions written by humans, for humans?

Instead of an illegible hash, sign:

```
expires 2026-04-01 10:00:00: approve transfer 1000000 lamports to 9abc...
```

Or:

```
expires 2026-04-01 10:00:00: propose add new market for mint: <xxx>
```

It literally doesn't matter how compromised your laptop is. The message on your Ledger is what happens on-chain.

## Intents

Every protocol multisig should have an explicit list of actions it can perform, with thresholds and signers for each. Those actions should be encodable as a human-readable string — an "intent" to perform that action. A DeFi protocol might have intents to add markets, adjust circuit breakers, or rotate admin keys. A program multisig might have an intent to swap the program buffer.

Adding new intents carries risk. An intent is defined in the language of Solana transactions — accounts, data, CPIs. That part requires the existing level of paranoia. But you only have to do it once. Once you have your list of intents, you can remove the ability to add new ones. Now you have an extremely constrained multisig that can only execute clear-text actions. You can actually trust your hardware wallet again. And with explicit expiry dates built into every signature, you no longer need durable nonces.

## Single-Signer Custody

This multisig with a signer count of one is effectively an off-chain signer with configurable expiry. Standardize on an immutable program like this and every custody provider can use it directly.

## Proof of Concept

Talk is cheap. Pointing fingers is useless. It's more productive to eliminate footguns by building better software, so I built something.

<link to repo>

This was also a good chance to try out [Quasar](https://github.com/blueshift-gg/quasar) from @blueshift. If this gets enough interest, maybe we can get it audited, formally verified, and made immutable. If not, maybe another team picks up the idea and puts it into a more complete multisig product.

## Disclaimer

This code is unaudited. It's on devnet only because Quasar isn't on mainnet yet. While it can turn into something great, don't use it to secure your protocol just yet.

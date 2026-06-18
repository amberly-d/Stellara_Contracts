# Deployment Guide

## Testnet Deployment Steps

### 1. Prepare Environment

```bash
# Install Stellar CLI if not already installed
curl --proto '=https' --tlsv1.2 -sSf https://install.stellar.org | sh

# Verify installation
stellar version
```

### 2. Set Up Network Configuration

```bash
# Configure testnet
stellar config network add \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015" \
  testnet

# Set testnet as active network
stellar config network set testnet
```

### 3. Create Funded Account

```bash
# Generate new keypair
stellar keys generate my-account

# Fund account using testnet faucet
stellar network use testnet
# Visit: https://friendbot.stellar.org/?addr=GXXXXXX
```

### 4. Build WASM Binaries

```bash
# Build all contracts
cargo build --release --target wasm32-unknown-unknown

# Binaries located at:
# target/wasm32-unknown-unknown/release/trading_contract.wasm
# target/wasm32-unknown-unknown/release/academy_contract.wasm
# target/wasm32-unknown-unknown/release/social_rewards_contract.wasm
# target/wasm32-unknown-unknown/release/messaging_contract.wasm
```

### 5. Deploy Contracts

```bash
# Deploy trading contract
TRADING_ID=$(stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/trading_contract.wasm \
  --source my-account \
  --network testnet \
  --no-wait)

# Deploy academy contract
ACADEMY_ID=$(stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/academy_contract.wasm \
  --source my-account \
  --network testnet \
  --no-wait)

# Deploy social rewards contract
REWARDS_ID=$(stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/social_rewards_contract.wasm \
  --source my-account \
  --network testnet \
  --no-wait)

# Deploy messaging contract
MESSAGING_ID=$(stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/messaging_contract.wasm \
  --source my-account \
  --network testnet \
  --no-wait)
```

### 6. Initialize Contracts

```bash
# Initialize each contract
stellar contract invoke \
  --id $TRADING_ID \
  --source my-account \
  --network testnet \
  -- init

stellar contract invoke \
  --id $ACADEMY_ID \
  --source my-account \
  --network testnet \
  -- init

stellar contract invoke \
  --id $REWARDS_ID \
  --source my-account \
  --network testnet \
  -- init

stellar contract invoke \
  --id $MESSAGING_ID \
  --source my-account \
  --network testnet \
  -- init
```

> **⚠️ Initializer Protection**: Each contract can only be initialized **once**.
> Calling `init` a second time will be rejected. This is enforced by the
> `upgradeability` module which sets an `"init"` flag in persistent storage
> on the first call. See [UPGRADEABILITY.md](./UPGRADEABILITY.md) for details.

### 7. Verify Initializer Protection

After initializing each contract, verify that re-initialization is blocked:

```bash
# Attempt to re-initialize trading contract (MUST FAIL)
stellar contract invoke \
  --id $TRADING_ID \
  --source my-account \
  --network testnet \
  -- init 2>&1 && echo "FAIL: re-init allowed!" || echo "PASS: re-init blocked"

# Attempt to re-initialize messaging contract (MUST FAIL)
stellar contract invoke \
  --id $MESSAGING_ID \
  --source my-account \
  --network testnet \
  -- init 2>&1 && echo "FAIL: re-init allowed!" || echo "PASS: re-init blocked"
```

If any contract allows re-initialization, **do not proceed** — this indicates
a security vulnerability. Check that the contract uses the `upgradeability`
module's `initializer_guard()` or equivalent storage-key check.

You can also run the automated deployment script which performs these checks:

```bash
./scripts/deploy/deploy_upgradeable.sh --network testnet
```

### 8. Verify Deployment

```bash
# Check contract exists
stellar contract info --id $TRADING_ID --network testnet

# Test a function
stellar contract invoke \
  --id $TRADING_ID \
  --source my-account \
  --network testnet \
  -- get_stats
```

## Contract Addresses (Testnet)

Update these after deployment:

```
Trading Contract:     [DEPLOYED_ADDRESS]
Academy Contract:     [DEPLOYED_ADDRESS]
Social Rewards:       [DEPLOYED_ADDRESS]
Messaging Contract:   [DEPLOYED_ADDRESS]
```

## Mainnet Migration

When ready for mainnet:

1. Replace testnet RPC URLs with mainnet
2. Use mainnet account credentials
3. Re-deploy using mainnet network configuration
4. Update all contract addresses in frontend code
5. **Verify initializer protection** on all deployed contracts

## Troubleshooting

### Build Issues

```bash
# Clean build
cargo clean
cargo build --release --target wasm32-unknown-unknown

# Check dependencies
cargo check
```

### Deployment Failures

```bash
# Verify account balance
stellar account info --source my-account --network testnet

# Check contract logs
stellar contract logs --id $CONTRACT_ID --network testnet
```

### Initializer Protection Issues

If a contract allows re-initialization:

1. **Check the `init` function** — it must check for the `"init"` storage key
   before proceeding:
   ```rust
   if env.storage().persistent().has(&symbol_short!("init")) {
       return Err(ContractError::Unauthorized);
   }
   env.storage().persistent().set(&symbol_short!("init"), &true);
   ```
2. **Use the `upgradeability` crate** — import `upgradeability::initializer_guard`
   for a standardized, tested guard:
   ```rust
   use upgradeability;
   
   pub fn init(env: Env, ...) {
       upgradeability::initializer_guard(&env);
       // ... rest of init
   }
   ```
3. **Run integration tests** to verify:
   ```bash
   cargo test -p integration-tests -- initializer_protection --test-threads=1
   ```

## Gas Estimation

Typical costs on testnet:
- Contract deployment: ~1000 stroops
- Function invocation: ~100-500 stroops
- Storage operations: Variable

## Security Checklist

Before deploying to any network:

- [ ] All contracts use initializer protection (`"init"` storage key guard)
- [ ] `cargo test -p upgradeability` passes (7 tests)
- [ ] `cargo test -p integration-tests -- initializer_protection` passes
- [ ] Double-init is verified blocked on deployed contracts
- [ ] Governance roles are correctly assigned
- [ ] Multi-sig signers are configured

## Further Resources

- [Soroban Documentation](https://developers.stellar.org/soroban)
- [Stellar CLI Reference](https://developers.stellar.org/cli)
- [Testnet Faucet](https://friendbot.stellar.org/)
- [UPGRADEABILITY.md](./UPGRADEABILITY.md) — Detailed upgradeability design

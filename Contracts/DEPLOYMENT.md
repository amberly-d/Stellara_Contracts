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

### 7. Verify Deployment

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

## Upgrade Safety

### Storage Layout Gaps

`MultisigTreasury` includes a `uint256[50] private __gap` at the end of its storage layout. This is the standard OpenZeppelin pattern for upgradeable contracts: it reserves 50 storage slots so that future state variables can be appended without shifting the positions of existing variables, which would corrupt proxy storage.

**Rules when upgrading:**
- New state variables must be added **before** `__gap`, and `__gap` size must be reduced by the number of slots added.
- Never reorder or remove existing state variables.
- Run storage layout checks (`hardhat-upgrades` or `slither`) before deploying an upgraded implementation.

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

## Gas Estimation

Typical costs on testnet:
- Contract deployment: ~1000 stroops
- Function invocation: ~100-500 stroops
- Storage operations: Variable

## Further Resources

- [Soroban Documentation](https://developers.stellar.org/soroban)
- [Stellar CLI Reference](https://developers.stellar.org/cli)
- [Testnet Faucet](https://friendbot.stellar.org/)

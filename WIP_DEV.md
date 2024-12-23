# Saya SNOS dev WIP

This little guide is meant to help to dev on Saya / Katana / SNOS compatibility.

## Setup.

First, Dojo must be cloned on the `katana/dev-snos` branch.
Then run a local katana:
```bash
cargo run --bin katana -r -- --dev
```

## Make a simple transfer to include a transaction in the block.

To ease the transaction execution, go into the `dojo/examples/simple` project, and run:

```bash
sozo execute 0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d transfer \
-c 0x13d9ee239f33fea4f8785b9e3870ade909e20a9599ae7cd62c1c292b73af1b7,u256:100000
```

This will transfer `STRK` tokens from address `0x127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec` to `0x13d9ee239f33fea4f8785b9e3870ade909e20a9599ae7cd62c1c292b73af1b7`.

For now, we don't use a full dojo project to prove a very simple block. Then, the `simple` project will be used to prove Dojo related blocks.

## To prove the block, you'll need:

- SNOS repository on `saya-snos-felt` [branch](https://github.com/cartridge-gg/snos/tree/saya-snos-felt)
- Saya repository on the `feat/katana-snos` [branch](https://github.com/dojoengine/saya/tree/feat/katana-snos)

Once pulling Saya, you must pull the large files from git lfs to run it:

```bash
git lfs install
git lfs pull
```

Then you can run Saya like so:

```bash
RUST_BACKTRACE=1 cargo run -- \
--prover-url https://staging.api.herodotus.cloud \
--private-key <HERODOTUS_PRIVATE_KEY> \
--rpc-url http://0.0.0.0:5050 --signer-address 0x4ba5ae775eb7da75f092b3b30b03bce15c3476337ef5f9e3cdf18db7a7534bd \
--signer-key 0x02a61a292ec9b72fe2ab6a3a11c731952ea352760aa6da8ffa9ec6e3b7f85b78 \
--settlement-contract 0x02d7e279f4cc935bd3ac65251091b5578c640c1b846d3e6586f5929081558de3
```

## Some notes in the code:

1. In `crates/rpc-client/src/pathfinder/proofs.rs`, the `verify_proof` function is returns `Ok(())` instead of actually verifying the proof. Must be removed if working on proofs.
2. A tons of `dbg!` are scattered around the codebase. Remove them as you wish.
3. The `Cargo.toml` files are pointing to local dependencies, with the branches mentioned above.

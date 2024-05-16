// Copyright 2024 ADM Contributors
// SPDX-License-Identifier: Apache-2.0, MIT

use bytes::Bytes;
use clap::{Args, Subcommand};
use clap_stdin::FileOrStdin;
use fendermint_actor_machine::WriteAccess;
use fendermint_crypto::SecretKey;
use fendermint_vm_message::query::FvmQueryHeight;
use fvm_shared::address::Address;
use serde_json::json;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};

use adm_provider::{
    json_rpc::JsonRpcProvider,
    util::{parse_address, parse_query_height},
};
use adm_sdk::{
    machine::{accumulator::Accumulator, Machine},
    TxParams,
};
use adm_signer::{key::parse_secret_key, AccountKind, Wallet};

use crate::{get_rpc_url, get_subnet_id, print_json, BroadcastMode, Cli, TxArgs};

#[derive(Clone, Debug, Args)]
pub struct AccumulatorArgs {
    #[command(subcommand)]
    command: AccumulatorCommands,
}

#[derive(Clone, Debug, Subcommand)]
enum AccumulatorCommands {
    /// Create a new accumulator.
    Create(AccumulatorCreateArgs),
    /// Push a value.
    Push(AccumulatorPushArgs),
    /// Get leaf at a given index and height.
    Leaf(AccumulatorLeafArgs),
    /// Get leaf count at a given height.
    Count(AccumulatorAddressArgs),
    /// Get peaks at a given height.
    Peaks(AccumulatorAddressArgs),
    /// Get root at a given height.
    Root(AccumulatorAddressArgs),
}

#[derive(Clone, Debug, Args)]
struct AccumulatorCreateArgs {
    /// Wallet private key (ECDSA, secp256k1) for signing transactions.
    #[arg(short, long, env, value_parser = parse_secret_key)]
    private_key: SecretKey,
    /// Allow public write access to the accumulator.
    #[arg(long, default_value_t = false)]
    public_write: bool,
    #[command(flatten)]
    tx_args: TxArgs,
}

#[derive(Clone, Debug, Args)]
struct AccumulatorPushArgs {
    /// Wallet private key (ECDSA, secp256k1) for signing transactions.
    #[arg(short, long, env, value_parser = parse_secret_key)]
    private_key: SecretKey,
    /// Accumulator machine address.
    #[arg(short, long, value_parser = parse_address)]
    address: Address,
    /// Input file (or stdin) containing the value to push.
    #[clap(default_value = "-")]
    input: FileOrStdin,
    /// Broadcast mode for the transaction.
    #[arg(short, long, value_enum, env, default_value_t = BroadcastMode::Commit)]
    broadcast_mode: BroadcastMode,
    #[command(flatten)]
    tx_args: TxArgs,
}

#[derive(Clone, Debug, Args)]
struct AccumulatorAddressArgs {
    /// Accumulator machine address.
    #[arg(short, long, value_parser = parse_address)]
    address: Address,
    /// Query height.
    #[arg(long, value_parser = parse_query_height, default_value = "committed")]
    height: FvmQueryHeight,
}

#[derive(Clone, Debug, Args)]
struct AccumulatorLeafArgs {
    /// Leaf index.
    index: u64,
    #[command(flatten)]
    address: AccumulatorAddressArgs,
}

pub async fn handle_accumulator(cli: Cli, args: &AccumulatorArgs) -> anyhow::Result<()> {
    let provider = JsonRpcProvider::new_http(get_rpc_url(&cli)?, None)?;
    let subnet_id = get_subnet_id(&cli)?;

    match &args.command {
        AccumulatorCommands::Create(AccumulatorCreateArgs {
            private_key,
            public_write,
            tx_args,
        }) => {
            let TxParams {
                sequence,
                gas_params,
            } = tx_args.to_tx_params();
            let mut signer =
                Wallet::new_secp256k1(private_key.clone(), AccountKind::Ethereum, subnet_id)?;
            signer.set_sequence(sequence, &provider).await?;

            let write_access = if public_write.clone() {
                WriteAccess::Public
            } else {
                WriteAccess::OnlyOwner
            };
            let (store, tx) =
                Accumulator::new(&provider, &mut signer, write_access, gas_params).await?;

            print_json(&json!({"address": store.address().to_string(), "tx": &tx}))
        }
        AccumulatorCommands::Push(AccumulatorPushArgs {
            private_key,
            address,
            input,
            broadcast_mode,
            tx_args,
        }) => {
            let TxParams {
                gas_params,
                sequence,
            } = tx_args.to_tx_params();
            let mut signer =
                Wallet::new_secp256k1(private_key.clone(), AccountKind::Ethereum, subnet_id)?;
            signer.set_sequence(sequence, &provider).await?;

            let machine = Accumulator::attach(address.clone());

            let mut reader = input.into_async_reader().await?;
            let mut buf = Vec::new();
            reader.read_to_end(&mut buf).await?;
            let payload = Bytes::from(buf);

            let broadcast_mode = broadcast_mode.get();
            let tx = machine
                .push(&provider, &mut signer, payload, broadcast_mode, gas_params)
                .await?;

            print_json(&tx)
        }
        AccumulatorCommands::Leaf(args) => {
            let machine = Accumulator::attach(args.address.address);
            let leaf = machine
                .leaf(&provider, args.index, args.address.height)
                .await?;

            let mut stdout = io::stdout();
            stdout.write_all(&leaf).await?;
            Ok(())
        }
        AccumulatorCommands::Count(args) => {
            let machine = Accumulator::attach(args.address);
            let count = machine.count(&provider, args.height).await?;

            print_json(&json!({"count": count}))
        }
        AccumulatorCommands::Peaks(args) => {
            let machine = Accumulator::attach(args.address);
            let peaks = machine.peaks(&provider, args.height).await?;

            print_json(&json!({"peaks": peaks}))
        }
        AccumulatorCommands::Root(args) => {
            let machine = Accumulator::attach(args.address);
            let root = machine.root(&provider, args.height).await?;

            print_json(&json!({"root": root.to_string()}))
        }
    }
}

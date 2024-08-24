extern crate core;

use std::{sync::Arc, time::Instant};

use alloy::{
    primitives::{B256, U256},
    providers::{IpcConnect, Provider, ProviderBuilder},
};
use clap::Parser;

use crate::{meme, search};

#[derive(Debug, Parser)]
pub struct Args {
    tx: B256,

    #[arg(long, env = "ETH_RPC_URL")]
    ipc_url: String,
}

pub async fn run(args: Args) {
    tracing_subscriber::fmt().init();

    let provider = ProviderBuilder::new()
        .on_ipc(IpcConnect::new(args.ipc_url))
        .await
        .unwrap();

    let provider: Arc<dyn Provider<_>> = Arc::new(provider);

    let tx = provider
        .get_transaction_by_hash(args.tx)
        .await
        .expect("failed to get transaction")
        .expect("transaction not found");

    let buy = meme::Buy::try_from(&tx).expect("not a buy tx");

    let block = provider
        .get_block_by_number(tx.block_number.unwrap().into(), false)
        .await
        .expect("failed to get block")
        .expect("block not found");

    let block_number = block.header.number.unwrap();

    let token_info = meme::get_token_info(Arc::clone(&provider), buy.token, block_number.into())
        .await
        .expect("fail to get token info");

    let fee_rate = meme::get_fee_rate(provider.as_ref(), block_number.into())
        .await
        .expect("fail to get fee rate");

    let min_fee = meme::get_min_fee(provider.as_ref(), block_number.into())
        .await
        .expect("fail to get min fee");

    let context = search::Context {
        token_info,
        fee_rate,
        min_fee,
        buy,
        token_balance: U256::ZERO,
    };

    let start = Instant::now();
    let solution = search::go(context).expect("cannot find solution");
    let optimal_elapsed = start.elapsed();

    println!("profit: {}", solution.profit);
    println!("time: {:?}", optimal_elapsed);

    todo!("use eth_callBundle to simulate and validate result")
}

use std::sync::Arc;

use alloy::{
    primitives::B256,
    providers::{IpcConnect, Provider, ProviderBuilder},
    pubsub::PubSubFrontend,
    signers::{local::PrivateKeySigner, Signer},
};
use burberry::{
    collector::{FullBlockCollector, MempoolCollector},
    map_collector, Engine,
};
use clap::Parser;

use crate::strategy::{Config, Event, Strategy};

#[derive(Debug, Parser)]
pub struct Args {
    #[arg(long, env = "ETH_RPC_URL")]
    pub ipc_url: String,

    #[arg(long)]
    pub private_key: B256,

    #[command(flatten)]
    pub config: Config,
}

pub async fn run(args: Args) {
    tracing_subscriber::fmt::init();

    let provider = ProviderBuilder::new()
        .on_ipc(IpcConnect::new(args.ipc_url))
        .await
        .unwrap();
    let provider: Arc<dyn Provider<PubSubFrontend>> = Arc::from(provider);

    let chain_id = provider.get_chain_id().await.expect("fail to get chain id");

    let signer = PrivateKeySigner::from_bytes(&args.private_key)
        .expect("fail to parse private key")
        .with_chain_id(Some(chain_id));

    let attacker = signer.address();

    let mut engine = Engine::default();

    engine.add_collector(map_collector!(
        MempoolCollector::new(provider.clone()),
        Event::PendingTx
    ));
    engine.add_collector(map_collector!(
        FullBlockCollector::new(provider.clone()),
        Event::FullBlock
    ));

    let strategy = Strategy::new(args.config.clone().into(), attacker, Arc::clone(&provider));

    engine.add_strategy(Box::new(strategy));

    // engine.add_executor(map_executor!(
    //     todo!("implement your our bundle executor to send bundles"),
    //     Action::SendBundle
    // ));

    engine.run_and_join().await.unwrap();
}

use std::{collections::HashSet, num::NonZeroUsize, sync::Arc, time::Instant};

use alloy::{
    eips::BlockNumberOrTag,
    network::TransactionBuilder,
    primitives::{Address, Bytes, B256, U256},
    providers::Provider,
    rpc::types::{Block, Transaction, TransactionRequest},
    sol_types::SolInterface,
    transports::Transport,
};
use burberry::ActionSubmitter;
use clap::Parser;
use eyre::{ContextCompat, WrapErr};
use lru::LruCache;
use tracing::{debug, error, info, instrument};

use crate::{
    meme::{
        self, find_sandwich_bot,
        FourMeme::{purchaseTokenAMAPCall, saleTokenCall, FourMemeCalls},
        FOUR_MEME,
        IERC20::{approveCall, IERC20Calls},
    },
    search,
};

#[derive(Debug, Clone, Parser)]
pub struct Config {
    #[arg(long, default_value = "1000000000", help = "Gas price in wei")]
    gas_price: u128,
}

pub struct Strategy<T> {
    sender: Address,
    provider: Arc<dyn Provider<T>>,
    config: Arc<Config>,

    /// Current block number
    block: Block,
    bots: HashSet<Address>,
    visited_tx: LruCache<B256, ()>,
}

#[derive(Clone, Debug)]
pub enum Event {
    FullBlock(Block),
    PendingTx(Transaction),
}

#[derive(Clone, Debug)]
pub enum SignedOrUnsignedTx {
    Unsigned(TransactionRequest),
    Signed(Bytes),
}

#[derive(Clone, Debug)]
pub struct Bundle {
    txs: Vec<SignedOrUnsignedTx>,
    block: u64,
}

#[derive(Debug, Clone)]
pub enum Action {
    SendBundle(Bundle),
}

impl<T: Transport + Clone> Strategy<T> {
    pub fn new(config: Arc<Config>, sender: Address, provider: Arc<dyn Provider<T>>) -> Self {
        Self {
            sender,
            provider,
            config,
            block: Block::default(),
            bots: Default::default(),
            visited_tx: LruCache::new(NonZeroUsize::new(1000).unwrap()),
        }
    }

    pub async fn on_pending_tx(&mut self, tx: Transaction, submitter: Arc<dyn ActionSubmitter<Action>>) {
        debug!(tx = %tx.hash, block = self.block.header.number.unwrap(), "received new transaction");

        let raw_tx = match self
            .provider
            .root()
            .raw_request::<_, Bytes>("eth_getRawTransactionByHash".into(), (tx.hash,))
            .await
        {
            Ok(raw_tx) => raw_tx,
            Err(err) => {
                error!(tx = %tx.hash, %err, "failed to get raw transaction");
                return;
            }
        };

        self.handle_tx(tx, raw_tx, submitter).await;
    }

    pub async fn handle_tx(&mut self, tx: Transaction, raw_tx: Bytes, submitter: Arc<dyn ActionSubmitter<Action>>) {
        if self.should_skip_tx(&tx) {
            return;
        }

        let buy = match meme::Buy::try_from(&tx) {
            Ok(buy) => buy,
            Err(_) => return,
        };

        info!(
            tx = %tx.hash,
            token = ?buy.token,
            amount = %buy.amount,
            min_received = %buy.min_received,
            "found new buy");

        self.handle_buy_optimal(tx, raw_tx, buy, submitter).await;
    }

    pub fn tx_visited(&mut self, tx: &Transaction) -> bool {
        if self.visited_tx.contains(&tx.hash) {
            return true;
        }

        self.visited_tx.push(tx.hash, ());

        false
    }

    pub fn should_skip_tx(&mut self, tx: &Transaction) -> bool {
        if tx.block_number.is_some() || tx.from == self.sender {
            return true;
        }

        if self.bots.contains(&tx.from) {
            info!(tx = %tx.hash, "skip bot tx");
            return true;
        }

        false
    }

    pub async fn get_search_context(&self, buy: meme::Buy, user: Address) -> eyre::Result<search::Context> {
        let block = self.block.header.number.unwrap();

        let token_info = meme::get_token_info(Arc::clone(&self.provider), buy.token, block.into())
            .await
            .context("fail to get token info")?;

        let token_balance = meme::get_balance(Arc::clone(&self.provider), buy.token, user, block.into())
            .await
            .context("fail to get token balance")?;

        let fee_rate = meme::get_fee_rate(self.provider.as_ref(), block.into())
            .await
            .context("fail to get fee rate")?;

        let min_fee = meme::get_min_fee(self.provider.as_ref(), block.into())
            .await
            .context("fail to get min fee")?;

        Ok(search::Context {
            token_info,
            fee_rate,
            min_fee,
            buy,
            token_balance,
        })
    }

    #[instrument(skip_all, fields(tx = %tx.hash))]
    pub async fn handle_buy_optimal(
        &mut self,
        tx: Transaction,
        raw_tx: Bytes,
        buy: meme::Buy,
        submitter: Arc<dyn ActionSubmitter<Action>>,
    ) {
        let block = self.block.header.number.unwrap();
        let context = match self.get_search_context(buy.clone(), tx.from).await {
            Ok(c) => c,
            Err(err) => {
                error!("fail to build search context: {err:#}");
                return;
            }
        };

        info!(?context);

        let start = Instant::now();
        let solution = match search::go(context) {
            Some(s) if !s.profit.is_zero() => s,
            Some(_) => {
                info!(elapsed = ?start.elapsed(), "no profit");
                return;
            }
            None => {
                error!(elapsed = ?start.elapsed(), "failed to find optimal solution");
                return;
            }
        };

        info!(
            tx = %tx.hash,
            profit = %solution.profit,
            to_buy = %solution.token_bought,
            spent = %solution.ether_spent,
            token_sold = %solution.token_sold,
            elapsed = ?start.elapsed(), "found solution");

        let allowance = match meme::get_allowance(
            Arc::clone(&self.provider),
            buy.token,
            self.sender,
            meme::FOUR_MEME,
            block.into(),
        )
        .await
        {
            Ok(a) => a,
            Err(err) => {
                error!("fail to get allowance: {err:#}");
                return;
            }
        };

        info!(%allowance);

        let buy_tx = TransactionRequest::default()
            .with_from(self.sender)
            .with_to(meme::FOUR_MEME)
            .with_input(
                FourMemeCalls::purchaseTokenAMAP {
                    0: purchaseTokenAMAPCall {
                        tokenAddress: buy.token,
                        funds: solution.token_bought,
                        minAmount: Default::default(),
                    },
                }
                .abi_encode(),
            )
            .with_value(solution.ether_spent)
            .with_gas_limit(150000)
            .with_gas_price(self.config.gas_price);

        let sell_tx = TransactionRequest::default()
            .with_from(self.sender)
            .with_to(meme::FOUR_MEME)
            .with_input(
                FourMemeCalls::saleToken {
                    0: saleTokenCall {
                        tokenAddress: buy.token,
                        amount: solution.token_sold,
                    },
                }
                .abi_encode(),
            )
            .with_gas_limit(130000)
            .with_gas_price(self.config.gas_price);

        let approve_tx = if allowance < solution.token_sold {
            TransactionRequest::default()
                .with_from(self.sender)
                .with_to(buy.token)
                .with_input(
                    IERC20Calls::approve {
                        0: approveCall {
                            spender: FOUR_MEME,
                            allowance: U256::MAX,
                        },
                    }
                    .abi_encode(),
                )
                .with_gas_limit(50000)
                .with_gas_price(self.config.gas_price)
                .into()
        } else {
            None
        };

        let cost = self.calculate_cost(&buy_tx, &sell_tx, &approve_tx);

        let mut profit = U256::from(solution.profit);
        if profit < cost {
            info!("profit {profit} is less than cost {cost}");
            return;
        }

        profit -= cost;

        // Bribe?

        let bundle = self.build_bundle(raw_tx, buy_tx, sell_tx, approve_tx);

        submitter.submit(Action::SendBundle(bundle));
    }

    fn build_bundle(
        &mut self,
        raw_tx: Bytes,
        buy_tx: TransactionRequest,
        sell_tx: TransactionRequest,
        approve_tx: Option<TransactionRequest>,
    ) -> Bundle {
        let mut bundle = Bundle {
            txs: vec![
                SignedOrUnsignedTx::Unsigned(buy_tx.with_gas_price(self.config.gas_price)),
                SignedOrUnsignedTx::Signed(raw_tx),
            ],
            block: self.block.header.number.unwrap(),
        };

        if let Some(tx) = approve_tx {
            let tx = SignedOrUnsignedTx::Unsigned(tx.with_gas_price(self.config.gas_price));
            bundle.txs.push(tx);
        }

        bundle.txs.push(SignedOrUnsignedTx::Unsigned(
            sell_tx.with_gas_price(self.config.gas_price),
        ));

        for (i, tx) in bundle.txs.iter().enumerate() {
            debug!(index = i, "tx: {tx:?}");
        }

        bundle
    }

    fn calculate_cost(
        &mut self,
        buy_tx: &TransactionRequest,
        sell_tx: &TransactionRequest,
        approve_tx: &Option<TransactionRequest>,
    ) -> U256 {
        let total_gas =
            buy_tx.gas.unwrap() + approve_tx.as_ref().and_then(|t| t.gas).unwrap_or_default() + sell_tx.gas.unwrap();

        U256::from(total_gas) * U256::from(self.config.gas_price)
    }

    pub async fn on_new_block(&mut self, block: Block) {
        let bots = find_sandwich_bot(&block);
        info!(block = block.header.number.unwrap(), "new block, bots: {:?}", bots);

        for bot in bots {
            if bot == self.sender {
                continue;
            }

            if self.bots.contains(&bot) {
                continue;
            }

            self.bots.insert(bot);
            info!(%bot, "found new bot");
        }

        self.block = block;
    }
}

#[burberry::async_trait]
impl<T: Transport + Clone> burberry::Strategy<Event, Action> for Strategy<T> {
    fn name(&self) -> &str {
        "arbitrage"
    }

    async fn sync_state(&mut self, _submitter: Arc<dyn ActionSubmitter<Action>>) -> eyre::Result<()> {
        self.block = self
            .provider
            .get_block_by_number(BlockNumberOrTag::Latest, false)
            .await
            .context("fail to get latest block")?
            .context("latest block not found")?;

        Ok(())
    }

    async fn process_event(&mut self, event: Event, submitter: Arc<dyn ActionSubmitter<Action>>) {
        match event {
            Event::FullBlock(block) => self.on_new_block(block).await,
            Event::PendingTx(tx) if !self.tx_visited(&tx) => self.on_pending_tx(tx, submitter).await,
            _ => {}
        }
    }
}

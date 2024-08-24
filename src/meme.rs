use std::{collections::HashSet, sync::Arc};

use alloy::{
    eips::BlockId,
    hex,
    primitives::{address, Address, U256},
    providers::Provider,
    rpc::types::{Block, Transaction},
    sol,
    transports::Transport,
    uint,
};
use eyre::eyre;

pub const FOUR_MEME: Address = address!("ec4549cadce5da21df6e6422d448034b5233bfbc");

sol! {
    #[sol(rpc)]
    FourMeme,
    "src/meme.json"
}

sol! {
    #[sol(rpc)]
    interface IERC20 {
        function balanceOf(address) external view returns (uint256);
        function approve(address spender, uint256 allowance) external;
        function allowance(address, address) external view returns (uint256);
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct Buy {
    pub token: Address,
    pub tx_value: U256,
    pub amount: U256,
    pub min_received: U256,
}

impl TryFrom<&Transaction> for Buy {
    type Error = ();

    fn try_from(tx: &Transaction) -> Result<Self, Self::Error> {
        if !tx.to.map(|to| to == FOUR_MEME).unwrap_or(false) {
            return Err(());
        }

        if !tx.input.starts_with(&hex!("3deec419")) || tx.input.len() != 4 + 32 * 3 {
            return Err(());
        }

        Ok(Buy {
            tx_value: tx.value,
            token: Address::from_slice(&tx.input[16..36]),
            amount: U256::from_be_slice(&tx.input[36..68]),
            min_received: U256::from_be_slice(&tx.input[68..100]),
        })
    }
}

#[derive(Debug, Hash, PartialEq, Eq)]
pub struct Sell {
    pub token: Address,
    pub amount: U256,
}

impl TryFrom<&Transaction> for Sell {
    type Error = ();

    fn try_from(tx: &Transaction) -> Result<Self, Self::Error> {
        if !tx.to.map(|to| to == FOUR_MEME).unwrap_or(false) {
            return Err(());
        }

        if !tx.input.starts_with(&hex!("9b911b5e")) || tx.input.len() != 4 + 32 * 2 {
            return Err(());
        }

        Ok(Self {
            token: Address::from_slice(&tx.input[16..36]),
            amount: U256::from_be_slice(&tx.input[36..68]),
        })
    }
}

#[derive(Debug, Copy, Clone)]
pub struct TokenInfo {
    pub k: U256,
    pub t: U256,
    pub offer: U256,
    pub ether: U256,
}

pub async fn get_fee_rate<T: Transport + Clone>(provider: &dyn Provider<T>, block_id: BlockId) -> eyre::Result<U256> {
    provider
        .get_storage_at(FOUR_MEME, uint!(0x163_U256))
        .block_id(block_id)
        .await
        .map_err(|err| eyre!("{err:#}"))
}

pub async fn get_min_fee<T: Transport + Clone>(provider: &dyn Provider<T>, block_id: BlockId) -> eyre::Result<U256> {
    provider
        .get_storage_at(FOUR_MEME, uint!(0x164_U256))
        .block_id(block_id)
        .await
        .map_err(|err| eyre!("{err:#}"))
}

pub async fn get_balance<T: Transport + Clone>(
    provider: Arc<dyn Provider<T>>,
    token: Address,
    account: Address,
    block: BlockId,
) -> eyre::Result<U256> {
    let c = IERC20::new(token, provider.root().clone());

    c.balanceOf(account)
        .block(block)
        .call()
        .await
        .map(|r| r._0)
        .map_err(|err| eyre!("fail to call balanceOf(): {err:#}"))
}

pub async fn get_allowance<T: Transport + Clone>(
    provider: Arc<dyn Provider<T>>,
    token: Address,
    owner: Address,
    spender: Address,
    block: BlockId,
) -> eyre::Result<U256> {
    let c = IERC20::new(token, provider.root().clone());

    c.allowance(owner, spender)
        .block(block)
        .call()
        .await
        .map(|r| r._0)
        .map_err(|err| eyre!("fail to call allowance(): {err:#}"))
}

pub async fn get_token_info<T: Transport + Clone>(
    provider: Arc<dyn Provider<T>>,
    token: Address,
    block_id: BlockId,
) -> eyre::Result<TokenInfo> {
    let contract = FourMeme::new(FOUR_MEME, provider.root().clone());
    let infos = contract._tokenInfos(token).block(block_id).call().await?;

    Ok(TokenInfo {
        k: infos.K,
        t: infos.T,
        offer: infos.offers,
        ether: infos.ethers,
    })
}

pub const ETHER: U256 = uint!(1_000_000_000_000_000_000_U256);
pub const GWEI: U256 = uint!(1_000_000_000_U256);

pub fn calc_max_buy(token: &TokenInfo) -> Option<U256> {
    (token.k * ETHER)
        .checked_div(token.t.checked_sub(token.offer)?)?
        .checked_sub((token.k * ETHER).checked_div(token.t)?)
}

pub fn calc_actual_buy(expect_buy: U256, token: &TokenInfo) -> Option<U256> {
    let v0 = (token.k * ETHER).checked_div(token.t)?;
    let v1 = (token.k * ETHER).checked_div(expect_buy.checked_add(v0)?)?;
    Some(token.t.checked_sub(v1)? / GWEI * GWEI)
}

pub fn calc_buy(actual_buy: U256, token: &TokenInfo) -> Option<U256> {
    let v0 = (token.k * ETHER).checked_div(token.t.checked_sub(actual_buy)?)?;
    let v1 = (token.k * ETHER).checked_div(token.t)?;
    Some(v0 - v1)
}

pub fn calc_sell(token_amount: U256, token: &TokenInfo) -> Option<U256> {
    let v0 = (token.k * ETHER).checked_div(token.t)?;
    let v1 = (token.k * ETHER).checked_div(token_amount + token.t)?;
    Some(v0 - v1)
}

/// FeeRate on slot 0x163
/// MinFee on slot 0x164
pub fn calc_fee(ether: U256, fee_rate: U256, min_fee: U256) -> U256 {
    let fee = fee_rate * ether / uint!(10_000_U256);
    fee.max(min_fee)
}

pub fn post_buy_update_status(token_info: &mut TokenInfo, actual_buy: &U256, buy_cost: &U256) -> Option<()> {
    token_info.t = token_info.t.checked_sub(*actual_buy)?;
    token_info.offer = token_info.offer.checked_sub(*actual_buy)?;
    token_info.ether += buy_cost; // unlikely to overflow because ether is BNB received
    Some(())
}

pub fn find_sandwich_bot(block: &Block) -> Vec<Address> {
    let txs = match block.transactions.as_transactions() {
        None => return vec![],
        Some(tx) => tx,
    };

    let all_token_buyers = txs
        .iter()
        .filter_map(|tx| Buy::try_from(tx).map(|buy| (tx.from, buy.token)).ok())
        .collect::<HashSet<_>>();

    let all_token_sellers = txs
        .iter()
        .filter_map(|tx| Sell::try_from(tx).map(|sell| (tx.from, sell.token)).ok())
        .collect::<HashSet<_>>();

    let common = all_token_buyers.intersection(&all_token_sellers);

    common.map(|(user, _)| *user).collect()
}

use alloy::{primitives::U256, uint};
use tracing::trace;

use crate::meme;

#[derive(Debug, Clone, Default)]
pub struct Solution {
    pub profit: U256,
    pub ether_spent: U256,
    pub token_bought: U256,
    pub token_sold: U256,
}

#[derive(Debug)]
pub struct Context {
    pub token_info: meme::TokenInfo,
    pub fee_rate: U256,
    pub min_fee: U256,
    pub buy: meme::Buy,
    pub token_balance: U256,
}

pub fn go(mut context: Context) -> Option<Solution> {
    trace!(?context);

    let mut upper_bound = meme::calc_max_buy(&context.token_info)?;
    let mut lower_bound = U256::from(1_000_000_000); // 1 gwei
    let mut solution = Solution::default();

    for _ in 0..100 {
        let m = (upper_bound - lower_bound) / U256::from(3);

        let s1 = trial_ultimate(&mut context, lower_bound + m).unwrap_or_default();
        let s2 = trial_ultimate(&mut context, lower_bound + m + m).unwrap_or_default();

        if s1.profit >= s2.profit {
            upper_bound = lower_bound + m + m;
        } else {
            lower_bound += m;
        }

        if s1.profit > solution.profit {
            solution = s1;
        }

        if s2.profit > solution.profit {
            solution = s2
        }
    }

    Some(solution)
}

fn trial_ultimate(context: &mut Context, to_buy: U256) -> Option<Solution> {
    let my_token_received = meme::calc_actual_buy(to_buy, &context.token_info)?;
    let my_buy_cost = meme::calc_buy(my_token_received, &context.token_info)?;
    meme::post_buy_update_status(&mut context.token_info, &my_token_received, &my_buy_cost)?;

    // Victim buy
    let victim_actual_buy = meme::calc_actual_buy(context.buy.amount, &context.token_info)?;
    let enough_value_to_buy = victim_actual_buy > context.buy.min_received;
    let dont_over_buy = victim_actual_buy + meme::ETHER <= context.token_info.offer;
    // trace!(%my_token_received, %my_buy_cost, %victim_actual_buy);

    if !(enough_value_to_buy && dont_over_buy) {
        // trace!(
        //     enough_value_to_buy,
        //     dont_over_buy,
        //     v.buy = %victim_actual_buy,
        //     v.min = %buy.min_received,
        //     t.offer = %token_info.offer,
        //     "trial stop"
        // );
        return None;
    }

    let victim_buy_cost = meme::calc_buy(victim_actual_buy, &context.token_info)?;
    let victim_buy_fee = meme::calc_fee(victim_buy_cost, context.fee_rate, context.min_fee);
    if victim_buy_cost + victim_buy_fee > context.buy.tx_value {
        // trace!(%victim_buy_cost, %victim_buy_fee, "trial stop");
        return None;
    }

    meme::post_buy_update_status(&mut context.token_info, &victim_actual_buy, &victim_buy_cost)?;

    // We sell
    let amount_sold = my_token_received + context.token_balance - uint!(1_000_000_000_U256);
    let ether_sold = meme::calc_sell(amount_sold, &context.token_info)?;
    let sold_fee = meme::calc_fee(ether_sold, context.fee_rate, context.min_fee);
    let ether_received = ether_sold.checked_sub(sold_fee)?;
    let my_buy_fee = meme::calc_fee(my_buy_cost, context.fee_rate, context.min_fee);
    let ether_spent = my_buy_cost + my_buy_fee;

    if ether_received <= ether_spent {
        // trace!(%ether_received, %ether_spent, "trial stop");
        return None;
    }

    Some(Solution {
        profit: ether_received - ether_spent,
        ether_spent: my_buy_cost + my_buy_fee,
        token_sold: amount_sold,
        token_bought: to_buy,
    })
}

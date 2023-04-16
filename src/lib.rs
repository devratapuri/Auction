#![allow(unused_variables)]

#[macro_use]
extern crate pbc_contract_codegen;

use std::collections::BTreeMap;

use create_type_spec_derive::CreateTypeSpec;
use pbc_contract_common::address::{Address, AddressType, Shortname};
use pbc_contract_common::context::{CallbackContext, ContractContext};
use pbc_contract_common::events::EventGroup;
use read_write_rpc_derive::{ReadRPC, WriteRPC};
use read_write_state_derive::ReadWriteState;

mod tests;
#[derive(ReadRPC, WriteRPC, ReadWriteState, CreateTypeSpec)]
#[cfg_attr(test, derive(PartialEq, Eq, Clone, Debug))]
pub struct Bid {
    bidder: Address,
    amount: u128,
}

#[derive(ReadWriteState, CreateTypeSpec)]
#[cfg_attr(test, derive(PartialEq, Eq, Clone, Debug))]
pub struct TokenClaim {
    tokens_for_bidding: u128,
    tokens_for_sale: u128,
}


type ContractStatus = u8;
const CREATION: ContractStatus = 0;
const BIDDING: ContractStatus = 1;
const ENDED: ContractStatus = 2;
const CANCELLED: ContractStatus = 3;

/// Token contract actions
#[inline]
fn token_contract_transfer() -> Shortname {
    Shortname::from_u32(0x01)
}

#[inline]
fn token_contract_transfer_from() -> Shortname {
    Shortname::from_u32(0x03)
}
#[state]
#[cfg_attr(test, derive(Clone, PartialEq, Eq, Debug))]
pub struct AuctionContractState {
    contract_owner: Address,
    start_time_millis: i64,
    end_time_millis: i64,
    token_amount_for_sale: u128,
    token_for_sale: Address,
    token_for_bidding: Address,
    highest_bidder: Bid,
    reserve_price: u128,
    min_increment: u128,
    claim_map: BTreeMap<Address, TokenClaim>,
    status: ContractStatus,
}

impl AuctionContractState {
    fn add_to_claim_map(&mut self, bidder: Address, additional_claim: TokenClaim) {
        let mut entry = self.claim_map.entry(bidder).or_insert(TokenClaim {
            tokens_for_bidding: 0,
            tokens_for_sale: 0,
        });
        entry.tokens_for_bidding += additional_claim.tokens_for_bidding;
        entry.tokens_for_sale += additional_claim.tokens_for_sale;
    }
}


#[init]
pub fn initialize(
    ctx: ContractContext,
    token_amount_for_sale: u128,
    token_for_sale: Address,
    token_for_bidding: Address,
    reserve_price: u128,
    min_increment: u128,
    auction_duration_hours: u32,
) -> (AuctionContractState, Vec<EventGroup>) {
    if token_for_sale.address_type != AddressType::PublicContract {
        panic!("Tried to create a contract selling a non publicContract token");
    }
    if token_for_bidding.address_type != AddressType::PublicContract {
        panic!("Tried to create a contract buying a non publicContract token");
    }
    let duration_millis = i64::from(auction_duration_hours) * 60 * 60 * 1000;
    let end_time_millis = ctx.block_production_time + duration_millis;
    let state = AuctionContractState {
        contract_owner: ctx.sender,
        start_time_millis: ctx.block_production_time,
        end_time_millis,
        token_amount_for_sale,
        token_for_sale,
        token_for_bidding,
        highest_bidder: Bid {
            bidder: ctx.sender,
            amount: 0,
        },
        reserve_price,
        min_increment,
        claim_map: BTreeMap::new(),
        status: CREATION,
    };

    (state, vec![])
}

#[action(shortname = 0x01)]
pub fn start(
    context: ContractContext,
    state: AuctionContractState,
) -> (AuctionContractState, Vec<EventGroup>) {
    if context.sender != state.contract_owner {
        panic!("Start can only be called by the creator of the contract");
    }
    if state.status != CREATION {
        panic!("Start should only be called while setting up the contract");
    }
   

    let mut event_group = EventGroup::builder();

    event_group.with_callback(SHORTNAME_START_CALLBACK).done();

    event_group
        .call(state.token_for_sale, token_contract_transfer_from())
        .argument(context.sender)
        .argument(context.contract_address)
        .argument(state.token_amount_for_sale)
        .done();

    (state, vec![event_group.build()])
}


#[callback(shortname = 0x02)]
pub fn start_callback(
    ctx: ContractContext,
    callback_ctx: CallbackContext,
    state: AuctionContractState,
) -> (AuctionContractState, Vec<EventGroup>) {
    let mut new_state = state;
    if !callback_ctx.success {
        panic!("Transfer event did not succeed for start");
    }
    new_state.status = BIDDING;
    (new_state, vec![])
}


#[action(shortname = 0x03)]
pub fn bid(
    context: ContractContext,
    state: AuctionContractState,
    bid_amount: u128,
) -> (AuctionContractState, Vec<EventGroup>) {
    // Potential new bid, create the transfer event
    // transfer(auctionContract, bid_amount)

    let bid: Bid = Bid {
        bidder: context.sender,
        amount: bid_amount,
    };

    let mut event_group = EventGroup::builder();
    event_group
        .call(state.token_for_bidding, token_contract_transfer_from())
        .argument(context.sender)
        .argument(context.contract_address)
        .argument(bid_amount)
        .done();
    event_group
        .with_callback(SHORTNAME_BID_CALLBACK)
        .argument(bid)
        .done();
    (state, vec![event_group.build()])
}

#[callback(shortname = 0x04)]
pub fn bid_callback(
    ctx: ContractContext,
    callback_ctx: CallbackContext,
    state: AuctionContractState,
    bid: Bid,
) -> (AuctionContractState, Vec<EventGroup>) {
    let mut new_state = state;
    if !callback_ctx.success {
        panic!("Transfer event did not succeed for bid");
    } else if new_state.status != BIDDING
        || ctx.block_production_time >= new_state.end_time_millis
        || bid.amount < new_state.highest_bidder.amount + new_state.min_increment
        || bid.amount < new_state.reserve_price
    {

        new_state.add_to_claim_map(
            bid.bidder,
            TokenClaim {
                tokens_for_bidding: bid.amount,
                tokens_for_sale: 0,
            },
        );
    } else {
        let prev_highest_bidder = new_state.highest_bidder;

        new_state.highest_bidder = bid;
        new_state.add_to_claim_map(
            prev_highest_bidder.bidder,
            TokenClaim {
                tokens_for_bidding: prev_highest_bidder.amount,
                tokens_for_sale: 0,
            },
        );
    }
    (new_state, vec![])
}
#[action(shortname = 0x05)]
pub fn claim(
    context: ContractContext,
    state: AuctionContractState,
) -> (AuctionContractState, Vec<EventGroup>) {
    let mut new_state = state;
    let opt_claimable = new_state.claim_map.get(&context.sender);
    match opt_claimable {
        None => (new_state, vec![]),
        Some(claimable) => {
            let mut event_group = EventGroup::builder();
            if claimable.tokens_for_bidding > 0 {
                event_group
                    .call(new_state.token_for_bidding, token_contract_transfer())
                    .argument(context.sender)
                    .argument(claimable.tokens_for_bidding)
                    .done();
            }
            if claimable.tokens_for_sale > 0 {
                event_group
                    .call(new_state.token_for_sale, token_contract_transfer())
                    .argument(context.sender)
                    .argument(claimable.tokens_for_sale)
                    .done();
            }
            new_state.claim_map.insert(
                context.sender,
                TokenClaim {
                    tokens_for_bidding: 0,
                    tokens_for_sale: 0,
                },
            );
            (new_state, vec![event_group.build()])
        }
    }
}
#[action(shortname = 0x06)]
pub fn execute(
    context: ContractContext,
    state: AuctionContractState,
) -> (AuctionContractState, Vec<EventGroup>) {
    let mut new_state = state;
    if context.block_production_time < new_state.end_time_millis {
        panic!("Tried to execute the auction before auction end block time");
    } else if new_state.status != BIDDING {
        panic!("Tried to execute the auction when the status isn't Bidding");
    } else {
        new_state.status = ENDED;
        new_state.add_to_claim_map(
            new_state.contract_owner,
            TokenClaim {
                tokens_for_bidding: new_state.highest_bidder.amount,
                tokens_for_sale: 0,
            },
        );
        new_state.add_to_claim_map(
            new_state.highest_bidder.bidder,
            TokenClaim {
                tokens_for_bidding: 0,
                tokens_for_sale: new_state.token_amount_for_sale,
            },
        );
        (new_state, vec![])
    }
}
#[action(shortname = 0x07)]
pub fn cancel(
    context: ContractContext,
    state: AuctionContractState,
) -> (AuctionContractState, Vec<EventGroup>) {
    let mut new_state = state;
    if context.sender != new_state.contract_owner {
        panic!("Only the contract owner can cancel the auction");
    } else if context.block_production_time >= new_state.end_time_millis {
        panic!("Tried to cancel the auction after auction end block time");
    } else if new_state.status != BIDDING {
        panic!("Tried to cancel the auction when the status isn't Bidding");
    } else {
        new_state.status = CANCELLED;
        new_state.add_to_claim_map(
            new_state.highest_bidder.bidder,
            TokenClaim {
                tokens_for_bidding: new_state.highest_bidder.amount,
                tokens_for_sale: 0,
            },
        );
        new_state.add_to_claim_map(
            new_state.contract_owner,
            TokenClaim {
                tokens_for_bidding: 0,
                tokens_for_sale: new_state.token_amount_for_sale,
            },
        );
        (new_state, vec![])
    }
}

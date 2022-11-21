# meta-dao

Meta DAO is a fork of Sputnik DAO with bounties and delegation logic removed. The point was to add the Price Ascension mechanism described in https://a16zcrypto.com/paying-people-to-participate-in-governance/ to an existing DAO.

The contract has been deployed on Testnet to: https://explorer.testnet.near.org/transactions/CiPyFYost4apKbfd2L7BCgh1bi9DY7oD5C1anvd2Y19P

# Quick Start

`cd contracts && cargo test`

# Summary

![alt text](https://github.com/pmespresso/direct-revelation-dao/diagram.jpeg?raw=true)

## Inspiration

https://a16zcrypto.com/paying-people-to-participate-in-governance/

## What it does

Every time there is a proposal up for vote, we take the `proposal_period` and chunk it up into `N` discrete buckets. At the end of each bucket, the offered reward for voting steps up to the next bracket. At the end of the voting period, some random `threshold_block` is chosen at which point all further votes will be counted toward the final vote, but not rewarded directly. Hence it is in the voters' interest to: a) carefully analyze their real cost of voting and b) submit a vote as close to when the the proposed reward by the DAO surpasses it.

There is seemingly the incentive to delay indefinitely in order to maximize rewards, but the caveat is that they run the risk of delaying past the threshold block (which is determined retroactively) and hence being frozen out of rewards entirely.

## Challenges we ran into

- provably fair method of picking `N`

## Accomplishments that we're proud of

- Groking and implementing the research article in just a few days while also learning how NEAR works from scratch
- Actually writing some Rust code that works (after being scared away by Substrate years ago)

## What we learned

- All the `near_sdk` types and blockchian specific primitives to use out of the box
- All the testing tools provided by `near_sdk`
- How a NEAR DAO is structured (with the Sputnik example)

## What's next for Meta DAO

- pick `N` retroactively in a provably fair / random way (maybe like [Polkadot's Candle Auctions] (https://polkadot.network/blog/research-update-the-case-for-candle-auctions/))
- build a UI and a community around it to play with it

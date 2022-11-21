use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::{LazyOption, LookupMap};
use near_sdk::json_types::{Base58CryptoHash, U128};
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::{
    env, ext_contract, near_bindgen, AccountId, Balance, BorshStorageKey, CryptoHash,
    PanicOnDefault, Promise, PromiseResult,
};

pub use crate::policy::{
    default_policy, Policy, RoleKind, RolePermission, VersionedPolicy, VotePolicy,
};
use crate::proposals::VersionedProposal;
pub use crate::proposals::{Proposal, ProposalInput, ProposalKind, ProposalStatus};
pub use crate::types::{Action, Config, OldAccountId, OLD_BASE_TOKEN};
use crate::upgrade::{internal_get_factory_info, internal_set_factory_info, FactoryInfo};
pub use crate::views::{ProposalOutput};

mod policy;
mod proposals;
mod types;
mod upgrade;
pub mod views;

#[derive(BorshStorageKey, BorshSerialize)]
pub enum StorageKeys {
    Config,
    Policy,
    Delegations,
    Proposals,
    Bounties,
    BountyClaimers,
    BountyClaimCounts,
    Blobs,
}

/// After payouts, allows a callback
#[ext_contract(ext_self)]
pub trait ExtSelf {
    /// Callback after proposal execution.
    fn on_proposal_callback(&mut self, proposal_id: u64) -> PromiseOrValue<()>;
}

#[near_bindgen]
#[derive(BorshSerialize, BorshDeserialize, PanicOnDefault)]
pub struct Contract {
    /// DAO configuration.
    pub config: LazyOption<Config>,
    /// Voting and permissions policy.
    pub policy: LazyOption<VersionedPolicy>,
    /// Last available id for the proposals.
    pub last_proposal_id: u64,
    /// Proposal map from ID to proposal information.
    pub proposals: LookupMap<u64, VersionedProposal>,
}

#[near_bindgen]
impl Contract {
    #[init]
    pub fn new(config: Config, policy: VersionedPolicy) -> Self {
        let this = Self {
            config: LazyOption::new(StorageKeys::Config, Some(&config)),
            policy: LazyOption::new(StorageKeys::Policy, Some(&policy.upgrade())),
            last_proposal_id: 0,
            proposals: LookupMap::new(StorageKeys::Proposals),
        };
        internal_set_factory_info(&FactoryInfo {
            factory_id: env::predecessor_account_id(),
            auto_update: true,
        });
        this
    }

    /// Returns factory information, including if auto update is allowed.
    pub fn get_factory_info(&self) -> FactoryInfo {
        internal_get_factory_info()
    }
}

#[cfg(test)]
mod tests {
    use near_sdk::test_utils::{accounts, VMContextBuilder};
    use near_sdk::testing_env;
    use near_sdk_sim::to_yocto;

    use crate::proposals::ProposalStatus;

    use super::*;

    fn create_proposal(context: &mut VMContextBuilder, contract: &mut Contract) -> u64 {
        testing_env!(context.attached_deposit(to_yocto("1")).build());
        contract.add_proposal(ProposalInput {
            description: "test".to_string(),
            kind: ProposalKind::Transfer {
                token_id: String::from(OLD_BASE_TOKEN),
                receiver_id: accounts(2).into(),
                amount: U128(to_yocto("100")),
                msg: None,
            },
        })
    }

    #[test]
    fn test_basics() {
        let mut context = VMContextBuilder::new();
        testing_env!(context.predecessor_account_id(accounts(1)).build());
        let mut contract = Contract::new(
            Config::test_config(),
            VersionedPolicy::Default(vec![accounts(1).into(), accounts(2).into()]),
        );
        let id = create_proposal(&mut context, &mut contract);
        assert_eq!(contract.get_proposal(id).proposal.description, "test");
        assert_eq!(contract.get_proposals(0, 10).len(), 1);

        let id = create_proposal(&mut context, &mut contract);
        contract.act_proposal(id, Action::VoteApprove, None);
        assert_eq!(
            contract.get_proposal(id).proposal.status,
            ProposalStatus::Approved
        );

        let id = create_proposal(&mut context, &mut contract);
        // proposal expired, finalize.
        testing_env!(context
            .block_timestamp(1_000_000_000 * 24 * 60 * 60 * 8)
            .build());
        contract.act_proposal(id, Action::Finalize, None);
        assert_eq!(
            contract.get_proposal(id).proposal.status,
            ProposalStatus::Expired
        );
    }
}

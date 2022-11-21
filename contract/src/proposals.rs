use std::collections::HashMap;

use near_contract_standards::fungible_token::core_impl::ext_fungible_token;
use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::json_types::{Base64VecU8, U128, U64};
use near_sdk::{log, AccountId, Balance, Gas, PromiseOrValue, BlockHeight};

use crate::types::{
    convert_old_to_new_token, Action, Config, OldAccountId, GAS_FOR_FT_TRANSFER, OLD_BASE_TOKEN,
    ONE_YOCTO_NEAR,
};
use crate::upgrade::{upgrade_remote, upgrade_using_factory};
use crate::policy::UserInfo;
use crate::*;

/// Status of a proposal.
#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(crate = "near_sdk::serde")]
pub enum ProposalStatus {
    InProgress,
    /// If quorum voted yes, this proposal is successfully approved.
    Approved,
    /// If quorum voted no, this proposal is rejected. Bond is returned.
    Rejected,
    /// If quorum voted to remove (e.g. spam), this proposal is rejected and bond is not returned.
    /// Interfaces shouldn't show removed proposals.
    Removed,
    /// Expired after period of time.
    Expired,
    /// If proposal was moved to Hub or somewhere else.
    Moved,
    /// If proposal has failed when finalizing. Allowed to re-finalize again to either expire or approved.
    Failed,
}

/// Function call arguments.
#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Clone, Debug))]
#[serde(crate = "near_sdk::serde")]
pub struct ActionCall {
    method_name: String,
    args: Base64VecU8,
    deposit: U128,
    gas: U64,
}

/// Function call arguments.
#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Clone, Debug))]
#[serde(crate = "near_sdk::serde")]
pub struct PolicyParameters {
    // pub proposal_bond: Option<U128>,
    pub proposal_period: Option<U64>,
}

/// Kinds of proposals, doing different action.
#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Clone, Debug))]
#[serde(crate = "near_sdk::serde")]
pub enum ProposalKind {
    /// Change the DAO config.
    ChangeConfig { config: Config },
    /// Change the full policy.
    ChangePolicy { policy: VersionedPolicy },
    /// Calls `receiver_id` with list of method names in a single promise.
    /// Allows this contract to execute any arbitrary set of actions in other contracts.
    FunctionCall {
        receiver_id: AccountId,
        actions: Vec<ActionCall>,
    },
    /// Upgrade this contract with given hash from blob store.
    UpgradeSelf { hash: Base58CryptoHash },
    /// Upgrade another contract, by calling method with the code from given hash from blob store.
    UpgradeRemote {
        receiver_id: AccountId,
        method_name: String,
        hash: Base58CryptoHash,
    },
    /// Transfers given amount of `token_id` from this DAO to `receiver_id`.
    /// If `msg` is not None, calls `ft_transfer_call` with given `msg`. Fails if this base token.
    /// For `ft_transfer` and `ft_transfer_call` `memo` is the `description` of the proposal.
    Transfer {
        /// Can be "" for $NEAR or a valid account id.
        token_id: OldAccountId,
        receiver_id: AccountId,
        amount: U128,
        msg: Option<String>,
    },
    /// Just a signaling vote, with no execution.
    Vote,
}

impl ProposalKind {
    /// Returns label of policy for given type of proposal.
    pub fn to_policy_label(&self) -> &str {
        match self {
            ProposalKind::ChangeConfig { .. } => "config",
            ProposalKind::ChangePolicy { .. } => "policy",
            ProposalKind::FunctionCall { .. } => "call",
            ProposalKind::UpgradeSelf { .. } => "upgrade_self",
            ProposalKind::UpgradeRemote { .. } => "upgrade_remote",
            ProposalKind::Transfer { .. } => "transfer",
            ProposalKind::Vote => "vote",
        }
    }
}

/// Votes recorded in the proposal.
#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Clone, Debug)]
#[serde(crate = "near_sdk::serde")]
pub enum Vote {
    Approve = 0x0,
    Reject = 0x1,
    Remove = 0x2,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Debug))]
#[serde(crate = "near_sdk::serde")]
pub struct VoteWithTimestamp {
    pub vote: Vote,
    pub blocknumber: BlockHeight,
}

impl From<Action> for Vote {
    fn from(action: Action) -> Self {
        match action {
            Action::VoteApprove => Vote::Approve,
            Action::VoteReject => Vote::Reject,
            Action::VoteRemove => Vote::Remove,
            _ => unreachable!(),
        }
    }
}

/// Proposal that are sent to this DAO.
#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Debug))]
#[serde(crate = "near_sdk::serde")]
pub struct Proposal {
    /// Original proposer.
    pub proposer: AccountId,
    /// Description of this proposal.
    pub description: String,
    /// Kind of proposal with relevant information.
    pub kind: ProposalKind,
    /// Current status of the proposal.
    pub status: ProposalStatus,
    /// Count of votes per role per decision: yes / no / spam.
    pub vote_counts: HashMap<String, [Balance; 3]>,
    /// Map of who voted and how.
    pub votes: HashMap<AccountId, VoteWithTimestamp>,
    /// The cutoff for when a submitted vote will be rewarded
    pub threshold_block: Option<BlockHeight>,
    /// Submission time (for voting period).
    pub submission_time: U64,
    /// FIXME: wanted to use mapping but Borsch cannot serialize so gotta implement manually.
    pub reward_group_1: Vec<AccountId>,
    pub reward_group_2: Vec<AccountId>,
    pub reward_group_3: Vec<AccountId>,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Debug))]
#[serde(crate = "near_sdk::serde")]
pub enum VersionedProposal {
    Default(Proposal),
}

impl From<VersionedProposal> for Proposal {
    fn from(v: VersionedProposal) -> Self {
        match v {
            VersionedProposal::Default(p) => p,
        }
    }
}

impl Proposal {
    /// Adds vote of the given user If user already voted, fails.
      pub fn update_votes(
        &mut self,
        account_id: &AccountId,
        roles: &[String],
        vote: Vote,
        policy: &Policy,
    ) {
        for role in roles {
            let amount = if policy.is_token_weighted(role, &self.kind.to_policy_label().to_string())
            {
                1
            } else {
                1
            };
            self.vote_counts.entry(role.clone()).or_insert([0u128; 3])[vote.clone() as usize] +=
                amount;
        }
        assert!(
            self.votes.insert(account_id.clone(), VoteWithTimestamp { vote: vote, blocknumber: env::block_height() }).is_none(),
            "ERR_ALREADY_VOTED"
        );
    }
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct ProposalInput {
    /// Description of this proposal.
    pub description: String,
    /// Kind of proposal with relevant information.
    pub kind: ProposalKind,
}

impl From<ProposalInput> for Proposal {
    fn from(input: ProposalInput) -> Self {
        Self {
            proposer: env::predecessor_account_id(),
            description: input.description,
            kind: input.kind,
            status: ProposalStatus::InProgress,
            vote_counts: HashMap::default(),
            votes: HashMap::default(),
            threshold_block: None,
            submission_time: U64::from(env::block_timestamp()),
            reward_group_1: vec![],
            reward_group_2: vec![],
            reward_group_3: vec![],
        }
    }
}

impl Contract {
    /// Execute payout of given token to given user.
    pub(crate) fn internal_payout(
        &mut self,
        token_id: &Option<AccountId>,
        receiver_id: &AccountId,
        amount: Balance,
        memo: String,
        msg: Option<String>,
    ) -> PromiseOrValue<()> {
        if token_id.is_none() {
            Promise::new(receiver_id.clone()).transfer(amount).into()
        } else {
            if let Some(msg) = msg {
                ext_fungible_token::ft_transfer_call(
                    receiver_id.clone(),
                    U128(amount),
                    Some(memo),
                    msg,
                    token_id.as_ref().unwrap().clone(),
                    ONE_YOCTO_NEAR,
                    GAS_FOR_FT_TRANSFER,
                )
            } else {
                ext_fungible_token::ft_transfer(
                    receiver_id.clone(),
                    U128(amount),
                    Some(memo),
                    token_id.as_ref().unwrap().clone(),
                    ONE_YOCTO_NEAR,
                    GAS_FOR_FT_TRANSFER,
                )
            }
            .into()
        }
    }
  
    /// Executes given proposal and updates the contract's state.
    fn internal_execute_proposal(
        &mut self,
        policy: &Policy,
        proposal: &mut Proposal,
        proposal_id: u64,
    ) -> PromiseOrValue<()> {

        let mut payout = ONE_YOCTO_NEAR;
        let mut payout_token = None;

        // 1. Decide on N (threshold)
        // This should be done in a more encrypted way, but for now we just hardcode buckets
        let n:u64 = policy.proposal_period.0 / 4;

        // 2. Bucket rewards groups
        proposal.votes.iter()
            .map(|(account_id, vote_with_timestamp)| 
                if vote_with_timestamp.blocknumber > proposal.submission_time.0 + n {
                    proposal.reward_group_1.push(account_id.clone());
                } else if vote_with_timestamp.blocknumber > proposal.submission_time.0 + 2 * n {
                    proposal.reward_group_2.push(account_id.clone());
                } else {
                    proposal.reward_group_3.push(account_id.clone());
                }
            );
            
        // 3. Make payouts with memo
        proposal.reward_group_1.iter().for_each(|account_id| {
            self.internal_payout(
                &payout_token,
                account_id,
                payout * 4,
                format!("Proposal #{} reward", proposal_id),
                None,
            );
        });

        proposal.reward_group_2.iter().for_each(|account_id| {
            self.internal_payout(
                &payout_token,
                account_id,
                payout * 3,
                format!("Proposal #{} reward", proposal_id),
                None,
            );
        });

        proposal.reward_group_3.iter().for_each(|account_id| {
            self.internal_payout(
                &payout_token,
                account_id,
                payout * 2,
                format!("Proposal #{} reward", proposal_id),
                None,
            );
        });
    
        let result = match &proposal.kind {
            ProposalKind::ChangeConfig { config } => {
                self.config.set(config);
                PromiseOrValue::Value(())
            }
            ProposalKind::ChangePolicy { policy } => {
                self.policy.set(policy);
                PromiseOrValue::Value(())
            }
            ProposalKind::FunctionCall {
                receiver_id,
                actions,
            } => {
                let mut promise = Promise::new(receiver_id.clone().into());
                for action in actions {
                    promise = promise.function_call(
                        action.method_name.clone().into(),
                        action.args.clone().into(),
                        action.deposit.0,
                        Gas(action.gas.0),
                    )
                }
                promise.into()
            }
            ProposalKind::UpgradeSelf { hash } => {
                upgrade_using_factory(hash.clone());
                PromiseOrValue::Value(())
            }
            ProposalKind::UpgradeRemote {
                receiver_id,
                method_name,
                hash,
            } => {
                upgrade_remote(&receiver_id, method_name, &CryptoHash::from(hash.clone()));
                PromiseOrValue::Value(())
            }
            ProposalKind::Transfer {
                token_id,
                receiver_id,
                amount,
                msg,
            } => self.internal_payout(
                &convert_old_to_new_token(token_id),
                &receiver_id,
                amount.0,
                proposal.description.clone(),
                msg.clone(),
            ),
            ProposalKind::Vote => PromiseOrValue::Value(()),
        };
        match result {
            PromiseOrValue::Promise(promise) => promise
                .then(ext_self::on_proposal_callback(
                    proposal_id,
                    env::current_account_id(),
                    0,
                    GAS_FOR_FT_TRANSFER,
                ))
                .into(),
            PromiseOrValue::Value(()) => PromiseOrValue::Value(()),
        }
    }
    pub(crate) fn internal_callback_proposal_success(
        &mut self,
        proposal: &mut Proposal,
    ) -> PromiseOrValue<()> {
        // let policy = self.policy.get().unwrap().to_policy();
        proposal.status = ProposalStatus::Approved;
        PromiseOrValue::Value(())
    }

    pub(crate) fn internal_callback_proposal_fail(
        &mut self,
        proposal: &mut Proposal,
    ) -> PromiseOrValue<()> {
        proposal.status = ProposalStatus::Failed;
        PromiseOrValue::Value(())
    }

    pub(crate) fn internal_user_info(&self) -> UserInfo {
        let account_id = env::predecessor_account_id();
        UserInfo {
            amount: 1,
            account_id,
        }
    }
}

#[near_bindgen]
impl Contract {
    /// Add proposal to this DAO.
    #[payable]
    pub fn add_proposal(&mut self, proposal: ProposalInput) -> u64 {
        // 0. validate bond attached.
        // TODO: consider bond in the token of this DAO.
        let policy = self.policy.get().unwrap().to_policy();

        assert_eq!(
            env::attached_deposit(),
            policy.proposal_bond.0,
            "ERR_MIN_BOND"
        );

        // 1. Validate proposal.
        match &proposal.kind {
            ProposalKind::ChangePolicy { policy } => match policy {
                VersionedPolicy::Current(_) => {}
                _ => panic!("ERR_INVALID_POLICY"),
            },
            ProposalKind::Transfer { token_id, msg, .. } => {
                assert!(
                    !(token_id == OLD_BASE_TOKEN) || msg.is_none(),
                    "ERR_BASE_TOKEN_NO_MSG"
                );
            }
            _ => {}
        };
        // 3. Actually add proposal to the current list of proposals.
        let id = self.last_proposal_id;
        self.proposals
            .insert(&id, &VersionedProposal::Default(proposal.into()));
        self.last_proposal_id += 1;
        // self.locked_amount += env::attached_deposit();
        id
    }

    /// Act on given proposal by id, if permissions allow.
    /// Memo is logged but not stored in the state. Can be used to leave notes or explain the action.
    pub fn act_proposal(&mut self, id: u64, action: Action, memo: Option<String>) {
        let mut proposal: Proposal = self.proposals.get(&id).expect("ERR_NO_PROPOSAL").into();
        let policy = self.policy.get().unwrap().to_policy();
        // Check permissions for the given action.
        let (roles, allowed) =
            policy.can_execute_action(self.internal_user_info(), &proposal.kind, &action);
        assert!(allowed, "ERR_PERMISSION_DENIED");
        let sender_id = env::predecessor_account_id();
        // Update proposal given action. Returns true if should be updated in storage.
        let update = match action {
            Action::AddProposal => env::panic_str("ERR_WRONG_ACTION"),
            Action::RemoveProposal => {
                self.proposals.remove(&id);
                false
            }
            Action::VoteApprove | Action::VoteReject | Action::VoteRemove => {
                assert!(
                    matches!(proposal.status, ProposalStatus::InProgress),
                    "ERR_PROPOSAL_NOT_READY_FOR_VOTE"
                );
                proposal.update_votes(
                    &sender_id,
                    &roles,
                    Vote::from(action),
                    &policy,
                );
               // Updates proposal status with new votes using the policy.
                proposal.status =
                    policy.proposal_status(&proposal, roles);
                println!("proposal status after VoteApprove {:?}", proposal.status);

                if proposal.status == ProposalStatus::Approved {
                    self.internal_execute_proposal(&policy, &mut proposal, id);
                    true
                } else if proposal.status == ProposalStatus::Removed {
                    // self.internal_reject_proposal(&policy, &proposal, false);
                    self.proposals.remove(&id);
                    false
                } else if proposal.status == ProposalStatus::Rejected {
                    // self.internal_reject_proposal(&policy, &proposal, true);
                    true
                } else {
                    // Still in progress or expired.
                    true
                }
            }
            // There are two cases when proposal must be finalized manually: expired or failed.
            // In case of failed, we just recompute the status and if it still approved, we re-execute the proposal.
            // In case of expired, we reject the proposal and return the bond.
            // Corner cases:
            //  - if proposal expired during the failed state - it will be marked as expired.
            //  - if the number of votes in the group has changed (new members has been added) -
            //      the proposal can loose it's approved state. In this case new proposal needs to be made, this one can only expire.
             Action::Finalize => {
                proposal.status = policy.proposal_status(
                    &proposal,
                    policy.roles.iter().map(|r| r.name.clone()).collect(),
                );
                match proposal.status {
                    ProposalStatus::Approved => {
                        self.internal_execute_proposal(&policy, &mut proposal, id);
                    }
                    ProposalStatus::Expired => {
                        println!("{:?} proposal expired", proposal.status)
                    }
                    _ => {
                        env::panic_str("ERR_PROPOSAL_NOT_EXPIRED_OR_FAILED");
                    }
                }
                true
            }
            Action::MoveToHub => false,
        };
        if update {
            self.proposals
                .insert(&id, &VersionedProposal::Default(proposal));
        }
        if let Some(memo) = memo {
            log!("Memo: {}", memo);
        }
    }

    /// Receiving callback after the proposal has been finalized.
    /// If successful, returns bond money to the proposal originator.
    /// If the proposal execution failed (funds didn't transfer or function call failure),
    /// move proposal to "Failed" state.
    #[private]
    pub fn on_proposal_callback(&mut self, proposal_id: u64) -> PromiseOrValue<()> {
        let mut proposal: Proposal = self
            .proposals
            .get(&proposal_id)
            .expect("ERR_NO_PROPOSAL")
            .into();
        assert_eq!(
            env::promise_results_count(),
            1,
            "ERR_UNEXPECTED_CALLBACK_PROMISES"
        );
        let result: PromiseOrValue<()> = match env::promise_result(0) {
            PromiseResult::NotReady => unreachable!(),
            PromiseResult::Successful(_) => self.internal_callback_proposal_success(&mut proposal),
            PromiseResult::Failed => self.internal_callback_proposal_fail(&mut proposal),
        };
        self.proposals
            .insert(&proposal_id, &VersionedProposal::Default(proposal.into()));
        result
    }
}

// Copyright (C) 2021 Subspace Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Primitives for executor pallet.

#![cfg_attr(not(feature = "std"), no_std)]

pub mod bundle_election;

use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;
use schnorrkel::vrf::{VRF_OUTPUT_LENGTH, VRF_PROOF_LENGTH};
use sp_consensus_slots::Slot;
use sp_core::crypto::KeyTypeId;
use sp_core::H256;
use sp_runtime::traits::{
    BlakeTwo256, Block as BlockT, Hash as HashT, Header as HeaderT, NumberFor,
};
use sp_runtime::transaction_validity::{InvalidTransaction, TransactionValidity};
use sp_runtime::OpaqueExtrinsic;
use sp_std::borrow::Cow;
use sp_std::vec::Vec;
use sp_trie::StorageProof;
use subspace_core_primitives::{Blake2b256Hash, BlockNumber, Randomness};
use subspace_runtime_primitives::AccountId;

/// Key type for Executor.
const KEY_TYPE: KeyTypeId = KeyTypeId(*b"exec");

mod app {
    use super::KEY_TYPE;
    use sp_application_crypto::{app_crypto, sr25519};

    app_crypto!(sr25519, KEY_TYPE);
}

/// An executor authority signature.
pub type ExecutorSignature = app::Signature;

/// An executor authority keypair. Necessarily equivalent to the schnorrkel public key used in
/// the main executor module. If that ever changes, then this must, too.
#[cfg(feature = "std")]
pub type ExecutorPair = app::Pair;

/// An executor authority identifier.
pub type ExecutorPublicKey = app::Public;

/// A type that implements `BoundToRuntimeAppPublic`, used for executor signing key.
pub struct ExecutorKey;

impl sp_runtime::BoundToRuntimeAppPublic for ExecutorKey {
    type Public = ExecutorPublicKey;
}

/// Stake weight in the domain bundle election.
///
/// Derived from the Balance and can't be smaller than u128.
pub type StakeWeight = u128;

/// Unique identifier of a domain.
#[derive(
    Clone, Copy, Debug, Hash, Default, Eq, PartialEq, Ord, PartialOrd, Encode, Decode, TypeInfo,
)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
pub struct DomainId(u32);

impl From<u32> for DomainId {
    fn from(x: u32) -> Self {
        Self(x)
    }
}

impl From<DomainId> for u32 {
    fn from(domain_id: DomainId) -> Self {
        domain_id.0
    }
}

impl core::ops::Add<u32> for DomainId {
    type Output = Self;

    fn add(self, other: u32) -> Self {
        Self(self.0 + other)
    }
}

impl core::ops::Sub<u32> for DomainId {
    type Output = Self;

    fn sub(self, other: u32) -> Self {
        Self(self.0 - other)
    }
}

// TODO: confirm the range of different domain types.
const CORE_DOMAIN_ID_START: u32 = 100;
const OPEN_DOMAIN_ID_START: u32 = 1000;

impl DomainId {
    pub const SYSTEM: Self = Self::new(0);

    /// Creates a [`DomainId`].
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    /// Returns `true` if a domain is a system domain.
    pub fn is_system(&self) -> bool {
        self.0 < CORE_DOMAIN_ID_START
    }

    /// Returns `true` if a domain is a core domain.
    pub fn is_core(&self) -> bool {
        self.0 >= CORE_DOMAIN_ID_START && self.0 < OPEN_DOMAIN_ID_START
    }

    /// Returns `true` if a domain is an open domain.
    pub fn is_open(&self) -> bool {
        self.0 >= OPEN_DOMAIN_ID_START
    }

    /// Converts the inner integer to little-endian bytes.
    pub fn to_le_bytes(&self) -> [u8; 4] {
        self.0.to_le_bytes()
    }
}

/// Domain configuration.
#[derive(Debug, Encode, Decode, TypeInfo, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
pub struct DomainConfig<Hash, Balance, Weight> {
    /// Hash of the domain wasm runtime blob.
    pub wasm_runtime_hash: Hash,

    // May be supported later.
    //pub upgrade_keys: Vec<AccountId>,
    // TODO: elaborate this field.
    pub bundle_frequency: u32,

    /// Maximum domain bundle size in bytes.
    pub max_bundle_size: u32,

    /// Maximum domain bundle weight.
    pub max_bundle_weight: Weight,

    /// Minimum executor stake value to be an operator on this domain.
    pub min_operator_stake: Balance,
}

/// Custom invalid validity code for the extrinsics in pallet-executor.
#[repr(u8)]
pub enum InvalidTransactionCode {
    BundleEquivicationProof = 101,
    TrasactionProof = 102,
    ExecutionReceipt = 103,
    Bundle = 104,
    FraudProof = 105,
}

impl From<InvalidTransactionCode> for InvalidTransaction {
    fn from(invalid_code: InvalidTransactionCode) -> Self {
        InvalidTransaction::Custom(invalid_code as u8)
    }
}

impl From<InvalidTransactionCode> for TransactionValidity {
    fn from(invalid_code: InvalidTransactionCode) -> Self {
        InvalidTransaction::Custom(invalid_code as u8).into()
    }
}

/// Header of transaction bundle.
#[derive(Debug, Decode, Encode, TypeInfo, PartialEq, Eq, Clone)]
pub struct BundleHeader<Hash> {
    /// The hash of primary block at which the bundle was created.
    pub primary_hash: Hash,
    /// The slot number.
    pub slot_number: u64,
    /// The merkle root of the extrinsics.
    pub extrinsics_root: H256,
}

impl<Hash: Encode> BundleHeader<Hash> {
    /// Returns the hash of this header.
    pub fn hash(&self) -> H256 {
        BlakeTwo256::hash_of(self)
    }
}

#[derive(Debug, Decode, Encode, TypeInfo, PartialEq, Eq, Clone)]
pub struct ProofOfElection<SecondaryHash> {
    /// Domain id.
    pub domain_id: DomainId,
    /// VRF output.
    pub vrf_output: [u8; VRF_OUTPUT_LENGTH],
    /// VRF proof.
    pub vrf_proof: [u8; VRF_PROOF_LENGTH],
    /// VRF public key.
    pub executor_public_key: ExecutorPublicKey,
    /// Global challenge.
    pub global_challenge: Blake2b256Hash,
    /// State root corresponding to the storage proof above.
    pub state_root: SecondaryHash,
    /// Storage proof for the bundle election state.
    pub storage_proof: StorageProof,
    /// Number of the secondary block at which the proof of election was created.
    pub block_number: BlockNumber,
    /// Block hash corresponding to the `block_number` above.
    pub block_hash: SecondaryHash,
}

impl<SecondaryHash: Default> ProofOfElection<SecondaryHash> {
    #[cfg(feature = "std")]
    pub fn with_public_key(executor_public_key: ExecutorPublicKey) -> Self {
        Self {
            domain_id: DomainId::default(),
            vrf_output: [0u8; VRF_OUTPUT_LENGTH],
            vrf_proof: [0u8; VRF_PROOF_LENGTH],
            executor_public_key,
            global_challenge: Blake2b256Hash::default(),
            state_root: Default::default(),
            storage_proof: StorageProof::empty(),
            block_number: Default::default(),
            block_hash: Default::default(),
        }
    }
}

/// Transaction bundle
#[derive(Debug, Decode, Encode, TypeInfo, PartialEq, Eq, Clone)]
pub struct Bundle<Extrinsic, Number, Hash, SecondaryHash> {
    /// The bundle header.
    pub header: BundleHeader<Hash>,
    /// Expected receipts by the primay chain when the bundle was created.
    pub receipts: Vec<ExecutionReceipt<Number, Hash, SecondaryHash>>,
    /// The accompanying extrinsics.
    pub extrinsics: Vec<Extrinsic>,
}

impl<Extrinsic, Number, Hash: Encode, SecondaryHash>
    Bundle<Extrinsic, Number, Hash, SecondaryHash>
{
    /// Returns the hash of this bundle.
    pub fn hash(&self) -> H256 {
        self.header.hash()
    }
}

/// Bundle with opaque extrinsics.
pub type OpaqueBundle<Number, Hash, SecondaryHash> =
    Bundle<OpaqueExtrinsic, Number, Hash, SecondaryHash>;

impl<Extrinsic: Encode, Number, Hash, SecondaryHash>
    Bundle<Extrinsic, Number, Hash, SecondaryHash>
{
    /// Convert a bundle with generic extrinsic to a bundle with opaque extrinsic.
    pub fn into_opaque_bundle(self) -> OpaqueBundle<Number, Hash, SecondaryHash> {
        let Bundle {
            header,
            receipts,
            extrinsics,
        } = self;
        let opaque_extrinsics = extrinsics
            .into_iter()
            .map(|xt| {
                OpaqueExtrinsic::from_bytes(&xt.encode())
                    .expect("We have just encoded a valid extrinsic; qed")
            })
            .collect();
        OpaqueBundle {
            header,
            receipts,
            extrinsics: opaque_extrinsics,
        }
    }
}

/// Signed version of [`Bundle`].
#[derive(Debug, Decode, Encode, TypeInfo, PartialEq, Eq, Clone)]
pub struct SignedBundle<Extrinsic, Number, Hash, SecondaryHash> {
    /// The bundle header.
    pub bundle: Bundle<Extrinsic, Number, Hash, SecondaryHash>,
    /// Proof of bundle election.
    pub proof_of_election: ProofOfElection<SecondaryHash>,
    /// Signature of the bundle.
    pub signature: ExecutorSignature,
}

/// [`SignedBundle`] with opaque extrinsic.
pub type SignedOpaqueBundle<Number, Hash, SecondaryHash> =
    SignedBundle<OpaqueExtrinsic, Number, Hash, SecondaryHash>;

impl<Extrinsic: Encode, Number: Encode, Hash: Encode, SecondaryHash: Encode>
    SignedBundle<Extrinsic, Number, Hash, SecondaryHash>
{
    /// Returns the hash of signed bundle.
    pub fn hash(&self) -> H256 {
        BlakeTwo256::hash_of(self)
    }

    /// Returns the domain_id of this bundle.
    pub fn domain_id(&self) -> DomainId {
        self.proof_of_election.domain_id
    }
}

impl<Extrinsic: Encode, Number, Hash, SecondaryHash>
    SignedBundle<Extrinsic, Number, Hash, SecondaryHash>
{
    /// Convert a signed bundle with generic extrinsic to a signed bundle with opaque extrinsic.
    pub fn into_signed_opaque_bundle(self) -> SignedOpaqueBundle<Number, Hash, SecondaryHash> {
        SignedOpaqueBundle {
            bundle: self.bundle.into_opaque_bundle(),
            proof_of_election: self.proof_of_election,
            signature: self.signature,
        }
    }
}

/// Receipt of state execution.
#[derive(Debug, Decode, Encode, TypeInfo, PartialEq, Eq, Clone)]
pub struct ExecutionReceipt<Number, Hash, SecondaryHash> {
    /// Primary block number.
    pub primary_number: Number,
    /// Primary block hash.
    pub primary_hash: Hash,
    /// Secondary block hash.
    pub secondary_hash: SecondaryHash,
    /// List of storage roots collected during the block execution.
    pub trace: Vec<SecondaryHash>,
    /// The merkle root of `trace`.
    pub trace_root: Blake2b256Hash,
}

impl<Number: Encode, Hash: Encode, SecondaryHash: Encode>
    ExecutionReceipt<Number, Hash, SecondaryHash>
{
    /// Returns the hash of this execution receipt.
    pub fn hash(&self) -> H256 {
        BlakeTwo256::hash_of(self)
    }
}

/// Execution phase along with an optional encoded call data.
///
/// Each execution phase has a different method for the runtime call.
#[derive(Debug, Decode, Encode, TypeInfo, PartialEq, Eq, Clone)]
pub enum ExecutionPhase {
    /// Executes the `initialize_block` hook.
    InitializeBlock { call_data: Vec<u8> },
    /// Executes some extrinsic.
    /// TODO: maybe optimized to not include the whole extrinsic blob in the future.
    ApplyExtrinsic { call_data: Vec<u8> },
    /// Executes the `finalize_block` hook.
    FinalizeBlock,
}

impl ExecutionPhase {
    /// Returns the method for generating the proof.
    pub fn proving_method(&self) -> &'static str {
        match self {
            // TODO: Replace `SecondaryApi_initialize_block_with_post_state_root` with `Core_initalize_block`
            // Should be a same issue with https://github.com/paritytech/substrate/pull/10922#issuecomment-1068997467
            Self::InitializeBlock { .. } => "SecondaryApi_initialize_block_with_post_state_root",
            Self::ApplyExtrinsic { .. } => "BlockBuilder_apply_extrinsic",
            Self::FinalizeBlock => "BlockBuilder_finalize_block",
        }
    }

    /// Returns the method for verifying the proof.
    ///
    /// The difference with [`Self::proving_method`] is that the return value of verifying method
    /// must contain the post state root info so that it can be used to compare whether the
    /// result of execution reported in [`FraudProof`] is expected or not.
    pub fn verifying_method(&self) -> &'static str {
        match self {
            Self::InitializeBlock { .. } => "SecondaryApi_initialize_block_with_post_state_root",
            Self::ApplyExtrinsic { .. } => "SecondaryApi_apply_extrinsic_with_post_state_root",
            Self::FinalizeBlock => "BlockBuilder_finalize_block",
        }
    }

    /// Returns the call data used to generate and verify the proof.
    pub fn call_data(&self) -> &[u8] {
        match self {
            Self::InitializeBlock { call_data } | Self::ApplyExtrinsic { call_data } => call_data,
            Self::FinalizeBlock => Default::default(),
        }
    }

    /// Returns the post state root for the given execution result.
    pub fn decode_execution_result<Header: HeaderT>(
        &self,
        execution_result: Vec<u8>,
    ) -> Result<Header::Hash, VerificationError> {
        match self {
            ExecutionPhase::InitializeBlock { .. } | ExecutionPhase::ApplyExtrinsic { .. } => {
                let encoded_storage_root = Vec::<u8>::decode(&mut execution_result.as_slice())
                    .map_err(VerificationError::InitializeBlockOrApplyExtrinsicDecode)?;
                Header::Hash::decode(&mut encoded_storage_root.as_slice())
                    .map_err(VerificationError::StorageRootDecode)
            }
            ExecutionPhase::FinalizeBlock => {
                let new_header = Header::decode(&mut execution_result.as_slice())
                    .map_err(VerificationError::HeaderDecode)?;
                Ok(*new_header.state_root())
            }
        }
    }
}

/// Error type of fraud proof verification on primary node.
#[derive(Debug)]
#[cfg_attr(feature = "thiserror", derive(thiserror::Error))]
pub enum VerificationError {
    /// Failed to pass the execution proof check.
    #[cfg_attr(
        feature = "thiserror",
        error("Failed to pass the execution proof check")
    )]
    BadProof(sp_std::boxed::Box<dyn sp_state_machine::Error>),
    /// The `post_state_root` calculated by farmer does not match the one declared in [`FraudProof`].
    #[cfg_attr(
        feature = "thiserror",
        error("`post_state_root` mismatches, expected: {expected}, got: {got}")
    )]
    BadPostStateRoot { expected: H256, got: H256 },
    /// Failed to decode the return value of `initialize_block` and `apply_extrinsic`.
    #[cfg_attr(
        feature = "thiserror",
        error(
            "Failed to decode the return value of `initialize_block` and `apply_extrinsic`: {0}"
        )
    )]
    InitializeBlockOrApplyExtrinsicDecode(parity_scale_codec::Error),
    /// Failed to decode the storage root produced by verifying `initialize_block` or `apply_extrinsic`.
    #[cfg_attr(
        feature = "thiserror",
        error(
            "Failed to decode the storage root from verifying `initialize_block` and `apply_extrinsic`: {0}"
        )
    )]
    StorageRootDecode(parity_scale_codec::Error),
    /// Failed to decode the header produced by `finalize_block`.
    #[cfg_attr(
        feature = "thiserror",
        error("Failed to decode the header from verifying `finalize_block`: {0}")
    )]
    HeaderDecode(parity_scale_codec::Error),
    /// Runtime api error.
    #[cfg(feature = "std")]
    #[cfg_attr(feature = "thiserror", error("Runtime api error: {0}"))]
    RuntimeApi(#[from] sp_api::ApiError),
}

/// Fraud proof for the state computation.
#[derive(Debug, Decode, Encode, TypeInfo, PartialEq, Eq, Clone)]
pub struct FraudProof {
    /// Hash of the signed bundle in which an invalid state transition occurred.
    pub bad_signed_bundle_hash: H256,
    /// Parent number.
    pub parent_number: BlockNumber,
    /// Parent hash of the block at which the invalid execution occurred.
    ///
    /// Runtime code for this block's execution is retrieved on top of the parent block.
    pub parent_hash: H256,
    /// State root before the fraudulent transaction.
    pub pre_state_root: H256,
    /// State root after the fraudulent transaction.
    pub post_state_root: H256,
    /// Proof recorded during the computation.
    pub proof: StorageProof,
    /// Execution phase.
    pub execution_phase: ExecutionPhase,
}

/// Represents a bundle equivocation proof. An equivocation happens when an executor
/// produces more than one bundle on the same slot. The proof of equivocation
/// are the given distinct bundle headers that were signed by the validator and which
/// include the slot number.
#[derive(Clone, Debug, Decode, Encode, PartialEq, TypeInfo)]
pub struct BundleEquivocationProof<Hash> {
    /// The authority id of the equivocator.
    pub offender: AccountId,
    /// The slot at which the equivocation happened.
    pub slot: Slot,
    /// The first header involved in the equivocation.
    pub first_header: BundleHeader<Hash>,
    /// The second header involved in the equivocation.
    pub second_header: BundleHeader<Hash>,
}

impl<Hash: Clone + Default + Encode> BundleEquivocationProof<Hash> {
    /// Returns the hash of this bundle equivocation proof.
    pub fn hash(&self) -> H256 {
        BlakeTwo256::hash_of(self)
    }

    // TODO: remove this later.
    /// Constructs a dummy bundle equivocation proof.
    pub fn dummy_at(slot_number: u64) -> Self {
        let dummy_header = BundleHeader {
            primary_hash: Hash::default(),
            slot_number,
            extrinsics_root: H256::default(),
        };
        Self {
            offender: AccountId::decode(&mut sp_runtime::traits::TrailingZeroInput::zeroes())
                .expect("Failed to create zero account"),
            slot: Slot::default(),
            first_header: dummy_header.clone(),
            second_header: dummy_header,
        }
    }
}

/// Represents an invalid transaction proof.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, TypeInfo)]
pub struct InvalidTransactionProof;

/// List of [`OpaqueBundle`].
pub type OpaqueBundles<Block, SecondaryHash> =
    Vec<OpaqueBundle<NumberFor<Block>, <Block as BlockT>::Hash, SecondaryHash>>;

/// List of [`SignedOpaqueBundle`].
pub type SignedOpaqueBundles<Block, SecondaryHash> =
    Vec<SignedOpaqueBundle<NumberFor<Block>, <Block as BlockT>::Hash, SecondaryHash>>;

sp_api::decl_runtime_apis! {
    /// API necessary for executor pallet.
    pub trait ExecutorApi<SecondaryHash: Encode + Decode> {
        /// Submits the transaction bundle via an unsigned extrinsic.
        fn submit_bundle_unsigned(opaque_bundle: SignedOpaqueBundle<NumberFor<Block>, Block::Hash, SecondaryHash>);

        /// Submits the fraud proof via an unsigned extrinsic.
        fn submit_fraud_proof_unsigned(fraud_proof: FraudProof);

        /// Submits the bundle equivocation proof via an unsigned extrinsic.
        fn submit_bundle_equivocation_proof_unsigned(
            bundle_equivocation_proof: BundleEquivocationProof<Block::Hash>,
        );

        /// Submits the invalid transaction proof via an unsigned extrinsic.
        fn submit_invalid_transaction_proof_unsigned(
            invalid_transaction_proof: InvalidTransactionProof,
        );

        /// Extract the system bundles from the given extrinsics.
        fn extract_system_bundles(
            extrinsics: Vec<Block::Extrinsic>,
        ) -> (OpaqueBundles<Block, SecondaryHash>, SignedOpaqueBundles<Block, SecondaryHash>);

        /// Extract the receipts from the given extrinsics.
        fn extract_receipts(
            extrinsics: Vec<Block::Extrinsic>,
        ) -> Vec<ExecutionReceipt<NumberFor<Block>, Block::Hash, SecondaryHash>>;

        /// Extract the fraud proofs from the given extrinsics.
        fn extract_fraud_proofs(extrinsics: Vec<Block::Extrinsic>) -> Vec<FraudProof>;

        /// Generates a randomness seed for extrinsics shuffling.
        fn extrinsics_shuffling_seed(header: Block::Header) -> Randomness;

        /// WASM bundle for execution runtime.
        fn execution_wasm_bundle() -> Cow<'static, [u8]>;

        /// Returns the best execution chain number.
        fn best_execution_chain_number() -> NumberFor<Block>;

        /// Returns the block number of oldest execution receipt.
        fn oldest_receipt_number() -> NumberFor<Block>;

        /// Returns the maximum receipt drift.
        fn maximum_receipt_drift() -> NumberFor<Block>;
    }
}

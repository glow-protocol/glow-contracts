use cosmwasm_bignumber::Uint256;
use cosmwasm_std::{StdError, Uint128};
use cw0::Expiration;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("Invalid instantiation deposit amount: {0}")]
    InvalidDepositInstantiation(Uint128),

    #[error("The contract is paused")]
    ContractPaused {},

    #[error("Invalid boost config. Base multiplier must be less than or equal to max multiplier")]
    InvalidBoostConfig {},

    #[error("Cannot register contracts twice")]
    AlreadyRegistered {},

    #[error("Contract have not been registered yet")]
    NotRegistered {},

    #[error("Invalid deposit amount")]
    ZeroDepositAmount {},

    #[error("Insufficient deposit amount for {0} tickets")]
    InsufficientDepositAmount(u64),

    #[error("Sequence must be 6 digits between 0-f but instead it was: {0}")]
    InvalidSequence(String),

    #[error("Invalid encoded tickets. Could not decode.")]
    InvalidEncodedTickets {},

    #[error("The ticket max holder limit has been reached for the following ticket: {0}")]
    InvalidHolderSequence(String),

    #[error("Gift tickets to oneself is not allowed")]
    GiftToSelf {},

    #[error("Gift ticket amount must be greater than zero")]
    ZeroGiftAmount {},

    #[error("Insufficient gift deposit amount for {0} tickets")]
    InsufficientGiftDepositAmount(u64),

    #[error("Sponsorship amount must be greater than zero")]
    ZeroSponsorshipAmount {},

    #[error("Lottery already in progress, wait until the next one begins")]
    LotteryAlreadyStarted {},

    #[error("Lottery is not ready to undergo execution yet, please wait until next_lottery_time: {next_lottery_time:?}")]
    LotteryNotReady { next_lottery_time: Expiration },

    #[error("The depositor doesn't have any savings aust so there is nothing to withdraw")]
    NoDepositorSavingsAustToWithdraw {},

    #[error("The depositor specified to withdraw zero funds which is too small")]
    SpecifiedWithdrawAmountIsZero {},

    #[error("The depositor specified to withdraw more funds ({amount:?}) than they have to withdraw ({depositor_balance:?})")]
    SpecifiedWithdrawAmountTooBig {
        amount: Uint128,
        depositor_balance: Uint256,
    },

    #[error("The number of tickets to be withdrawn ({withdrawn_tickets}) is more tickets than the depositor owns ({num_depositor_tickets})")]
    WithdrawingTooManyTickets {
        withdrawn_tickets: u128,
        num_depositor_tickets: u128,
    },

    #[error("There are no enough funds in the contract for that operation. Amount to send: {to_send}. Available balance: {available_balance}")]
    InsufficientFunds {
        to_send: Uint128,
        available_balance: Uint256,
    },

    #[error("The sponsor doesn't have any lottery deposits so there is nothing to withdraw")]
    NoSponsorLotteryDeposit {},

    #[error("The lottery pool ({pool_value}) is smaller than total lottery deposits ({total_lottery_deposits}), no redeem stable allowed")]
    InsufficientPoolFunds {
        pool_value: Uint256,
        total_lottery_deposits: Uint256,
    },

    #[error("There are not enough funds to run the lottery")]
    InsufficientLotteryFunds {},

    #[error("Max number of concurrent unbonding claims for this users has been reached")]
    MaxUnbondingClaims {},

    #[error("Lottery claim is invalid, as lottery #{0} has not being awarded yet")]
    InvalidClaimLotteryNotAwarded(u64),

    #[error("Lottery claim is invalid, as prize has already been claimed for lottery #")]
    InvalidClaimPrizeAlreadyClaimed(u64),

    #[error("There not enough claimable funds for the given user")]
    InsufficientClaimableFunds {},

    #[error("Invalid prize distribution config")]
    InvalidPrizeDistribution {},

    #[error("Invalid reserve factor config")]
    InvalidReserveFactor {},

    #[error("Invalid split factor config")]
    InvalidSplitFactor {},

    #[error("Invalid instant withdrawal fee config")]
    InvalidWithdrawalFee {},

    #[error("Invalid unbonding period config")]
    InvalidUnbondingPeriod {},

    #[error("Invalid first lottery execution time")]
    InvalidFirstLotteryExec {},

    #[error("Invalid epoch interval config")]
    InvalidEpochInterval {},

    #[error("Invalid max holders config, outside bounds")]
    InvalidMaxHoldersOutsideBounds {},

    #[error("Invalid max holders config, can only increase max holders, not decrease")]
    InvalidMaxHoldersAttemptedDecrease {},

    #[error("Invalid lottery interval config")]
    InvalidLotteryInterval {},

    #[error("Invalid lottery next time")]
    InvalidLotteryNextTime {},

    #[error("Invalid execution of the lottery. No sent funds allowed.")]
    InvalidLotteryExecutionFunds {},

    #[error("Invalid execution of the lottery. No tickets in the lotto.")]
    InvalidLotteryExecutionTickets {},

    #[error("Invalid execution of the lottery prize. The lottery must be executed first.")]
    InvalidLotteryPrizeExecution {},

    #[error("Invalid execution of the lottery prize. Block time has not expired yet.")]
    InvalidLotteryPrizeExecutionExpired {},

    #[error("Invalid execution of the lottery prize. Sent funds not allowed.")]
    InvalidLotteryPrizeExecutionFunds {},

    #[error("Invalid execute epochs execution")]
    InvalidEpochExecution {},

    #[error("Max tickets per depositor exceeded. Max tickets per depositor: {max_tickets_per_depositor}. Post transaction num depositor tickets: {post_transaction_num_depositor_tickets}")]
    MaxTicketsPerDepositorExceeded {
        max_tickets_per_depositor: u64,
        post_transaction_num_depositor_tickets: u64,
    },

    #[error("Unauthorized")]
    Unauthorized {},
}

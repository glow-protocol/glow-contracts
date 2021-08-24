use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("Invalid instantiation deposit amount")]
    InvalidDepositInstantiation {},

    #[error("Cannot register contracts twice")]
    AlreadyRegistered {},

    #[error("Invalid deposit amount")]
    InvalidDepositAmount {},

    #[error("Insufficient deposit amount for {0} tickets")]
    InsufficientDepositAmount(u64),

    #[error("Sequence must be 5 digits between 0-9")]
    InvalidSequence {},

    #[error("Gift tickets to oneself is not allowed")]
    InvalidGift {},

    #[error("Gift ticket amount must be greater than zero")]
    InvalidGiftAmount {},

    #[error("Insufficient gift deposit amount for {0} tickets")]
    InsufficientGiftDepositAmount(u64),

    #[error("Gift ticket amount must be greater than zero")]
    InvalidSponsorshipAmount {},

    #[error("Lottery already in progress, wait until the next one begins")]
    LotteryAlreadyStarted {},

    #[error("Lottery still in progress, wait until next lottery time")]
    LotteryInProgress {},

    #[error("There are no deposits to withdraw")]
    InvalidWithdraw {},

    #[error("There are no deposits to withdraw")]
    InsufficientFunds {},

    #[error("There are no funds to run the lottery")]
    InsufficientLotteryFunds {},

    #[error("Gift ticket amount must be greater than zero")]
    InvalidClaimAmount {},

    #[error("Invalid prize distribution config")]
    InvalidPrizeDistribution {},

    #[error("Invalid reserve factor config")]
    InvalidReserveFactor {},

    #[error("Invalid reserve factor config")]
    InvalidSplitFactor {},

    #[error("Invalid unbonding period config")]
    InvalidUnbondingPeriod {},

    #[error("Invalid execution of the lottery. Funds cannot be sent.")]
    InvalidLotteryExecution {},

    #[error("Unauthorized")]
    Unauthorized {},
}

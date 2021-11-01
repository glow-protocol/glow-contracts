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

    #[error("The sender already owns the ticket or the ticket max holder has been reached")]
    InvalidHolderSequence {},

    #[error("Gift tickets to oneself is not allowed")]
    InvalidGift {},

    #[error("Gift ticket amount must be greater than zero")]
    InvalidGiftAmount {},

    #[error("Insufficient gift deposit amount for {0} tickets")]
    InsufficientGiftDepositAmount(u64),

    #[error("Sponsorship amount must be greater than zero")]
    InvalidSponsorshipAmount {},

    #[error("Lottery already in progress, wait until the next one begins")]
    LotteryAlreadyStarted {},

    #[error("Lottery still in progress, wait until next lottery time")]
    LotteryInProgress {},

    #[error("There are no deposits to withdraw")]
    InvalidWithdraw {},

    #[error("There are no enough funds in the contract for that operation")]
    InsufficientFunds {},

    #[error("There are no sponsor deposits to withdraw")]
    InvalidSponsorWithdraw {},

    #[error("The Anchor Sponsor Pool is smaller than total sponsors, no withdraws allowed")]
    InsufficientSponsorFunds {},

    #[error("The Anchor Pool is smaller than total deposits, no withdraws allowed")]
    InsufficientPoolFunds {},

    #[error("There are no funds to run the lottery")]
    InsufficientLotteryFunds {},

    #[error("Invalid claim amount")]
    InvalidClaimAmount {},

    #[error("Max number of concurrent unbonding claims for this users has been reached")]
    MaxUnbondingClaims {},

    #[error("Lottery claim is invalid, as lottery has not still being awarded")]
    InvalidLotteryClaim {},

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

    #[error("Invalid execution of the lottery. Funds cannot be sent.")]
    InvalidLotteryFundsExecution {},

    #[error("Invalid execution of the lottery. There are no playing tickets.")]
    InvalidLotteryTicketsExecution {},

    #[error("Invalid execution of the lottery. Execute lottery already been called.")]
    InvalidLotteryExecution {},

    #[error("Invalid execution of the lottery prize. The lottery must be executed first.")]
    InvalidLotteryPrizeExecution {},

    #[error("Invalid execute epochs execution")]
    InvalidEpochExecution {},

    #[error("Unauthorized")]
    Unauthorized {},
}

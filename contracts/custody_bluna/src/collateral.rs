use crate::error::ContractError;
use crate::state::{
    read_borrower_info, read_borrowers, read_config, remove_borrower_info, store_borrower_info,
    BorrowerInfo, Config,
};

use cosmwasm_bignumber::Uint256;
use cosmwasm_std::{
    attr, to_binary, Addr, CosmosMsg, Deps, DepsMut, MessageInfo, Response, StdResult, WasmMsg,
};
use cw20::Cw20ExecuteMsg;
use moneymarket::custody::{BorrowerResponse, BorrowersResponse};
use moneymarket::liquidation::Cw20HookMsg as LiquidationCw20HookMsg;
use terra_cosmwasm::TerraMsgWrapper;

/// Deposit new collateral
/// Executor: bAsset token contract
pub fn deposit_collateral(
    deps: DepsMut,
    borrower: Addr,
    amount: Uint256,
) -> Result<Response<TerraMsgWrapper>, ContractError> {
    let mut borrower_info: BorrowerInfo = read_borrower_info(deps.storage, &borrower);

    // increase borrower collateral
    borrower_info.balance += amount;
    borrower_info.spendable += amount;

    store_borrower_info(deps.storage, &borrower, &borrower_info)?;

    Ok(Response::new().add_attributes(vec![
        attr("action", "deposit_collateral"),
        attr("borrower", borrower.as_str()),
        attr("amount", amount.to_string()),
    ]))
}

/// Withdraw spendable collateral or a specified amount of collateral
/// Executor: borrower
pub fn withdraw_collateral(
    deps: DepsMut,
    info: MessageInfo,
    amount: Option<Uint256>,
) -> Result<Response<TerraMsgWrapper>, ContractError> {
    let config: Config = read_config(deps.storage)?;

    let borrower = info.sender;
    let mut borrower_info: BorrowerInfo = read_borrower_info(deps.storage, &borrower);

    // Check spendable balance
    let amount = amount.unwrap_or(borrower_info.spendable);
    if borrower_info.spendable < amount {
        return Err(ContractError::WithdrawAmountExceedsSpendable(
            borrower_info.spendable.into(),
        ));
    }

    // decrease borrower collateral
    borrower_info.balance = borrower_info.balance - amount;
    borrower_info.spendable = borrower_info.spendable - amount;

    if borrower_info.balance == Uint256::zero() {
        remove_borrower_info(deps.storage, &borrower);
    } else {
        store_borrower_info(deps.storage, &borrower, &borrower_info)?;
    }

    Ok(Response::new()
        .add_message(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.collateral_token.to_string(),
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Transfer {
                recipient: borrower.to_string(),
                amount: amount.into(),
            })?,
        }))
        .add_attributes(vec![
            attr("action", "withdraw_collateral"),
            attr("borrower", borrower.as_str()),
            attr("amount", amount.to_string()),
        ]))
}

/// Decrease spendable collateral to lock
/// specified amount of collateral token
/// Executor: overseer
pub fn lock_collateral(
    deps: DepsMut,
    info: MessageInfo,
    borrower: Addr,
    amount: Uint256,
) -> Result<Response<TerraMsgWrapper>, ContractError> {
    let config: Config = read_config(deps.storage)?;
    if info.sender != config.overseer_contract {
        return Err(ContractError::Unauthorized {});
    }

    let mut borrower_info: BorrowerInfo = read_borrower_info(deps.storage, &borrower);
    if amount > borrower_info.spendable {
        return Err(ContractError::LockAmountExceedsSpendable(
            borrower_info.spendable.into(),
        ));
    }

    borrower_info.spendable = borrower_info.spendable - amount;
    store_borrower_info(deps.storage, &borrower, &borrower_info)?;
    Ok(Response::new().add_attributes(vec![
        attr("action", "lock_collateral"),
        attr("borrower", borrower),
        attr("amount", amount),
    ]))
}

/// Increase spendable collateral to unlock
/// specified amount of collateral token
/// Executor: overseer
pub fn unlock_collateral(
    deps: DepsMut,
    info: MessageInfo,
    borrower: Addr,
    amount: Uint256,
) -> Result<Response<TerraMsgWrapper>, ContractError> {
    let config: Config = read_config(deps.storage)?;
    if info.sender != config.overseer_contract {
        return Err(ContractError::Unauthorized {});
    }

    let mut borrower_info: BorrowerInfo = read_borrower_info(deps.storage, &borrower);
    let borrowed_amt = borrower_info.balance - borrower_info.spendable;
    if amount > borrowed_amt {
        return Err(ContractError::UnlockAmountExceedsLocked(
            borrowed_amt.into(),
        ));
    }

    borrower_info.spendable += amount;
    store_borrower_info(deps.storage, &borrower, &borrower_info)?;

    Ok(Response::new().add_attributes(vec![
        attr("action", "unlock_collateral"),
        attr("borrower", borrower),
        attr("amount", amount),
    ]))
}

pub fn liquidate_collateral(
    deps: DepsMut,
    info: MessageInfo,
    liquidator: Addr,
    borrower: Addr,
    amount: Uint256,
) -> Result<Response<TerraMsgWrapper>, ContractError> {
    let config: Config = read_config(deps.storage)?;
    if info.sender != config.overseer_contract {
        return Err(ContractError::Unauthorized {});
    }
    let mut borrower_info: BorrowerInfo = read_borrower_info(deps.storage, &borrower);
    let borrowed_amt = borrower_info.balance - borrower_info.spendable;
    if amount > borrowed_amt {
        return Err(ContractError::LiquidationAmountExceedsLocked(
            borrowed_amt.into(),
        ));
    }

    borrower_info.balance = borrower_info.balance - amount;
    store_borrower_info(deps.storage, &borrower, &borrower_info)?;

    Ok(Response::new()
        .add_message(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: config.collateral_token.to_string(),
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Send {
                contract: config.liquidation_contract.to_string(),
                amount: amount.into(),
                msg: to_binary(&LiquidationCw20HookMsg::ExecuteBid {
                    liquidator: liquidator.to_string(),
                    fee_address: Some(config.overseer_contract.to_string()),
                    repay_address: Some(config.market_contract.to_string()),
                })?,
            })?,
        }))
        .add_attributes(vec![
            attr("action", "liquidate_collateral"),
            attr("liquidator", liquidator),
            attr("borrower", borrower),
            attr("amount", amount),
        ]))
}

pub fn query_borrower(deps: Deps, borrower: Addr) -> StdResult<BorrowerResponse> {
    let borrower_info: BorrowerInfo = read_borrower_info(deps.storage, &borrower);
    Ok(BorrowerResponse {
        borrower: borrower.to_string(),
        balance: borrower_info.balance,
        spendable: borrower_info.spendable,
    })
}

pub fn query_borrowers(
    deps: Deps,
    start_after: Option<Addr>,
    limit: Option<u32>,
) -> StdResult<BorrowersResponse> {
    let borrowers = read_borrowers(deps, start_after, limit)?;
    Ok(BorrowersResponse { borrowers })
}

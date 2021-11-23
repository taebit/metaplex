use spl_token::instruction::{set_authority, AuthorityType};

use crate::{
    errors::AuctionError,
    processor::{AuctionData, AuctionDataExtended, BidderMetadata, BidderPot},
    utils::{
        assert_derivation, assert_initialized, assert_owned_by, assert_signer,
        assert_token_program_matches_package, create_or_allocate_account_raw, spl_token_transfer,
        TokenTransferParams,
    },
    EXTENDED, PREFIX,
};

use {
    borsh::{BorshDeserialize, BorshSerialize},
    solana_program::{
        account_info::{next_account_info, AccountInfo},
        entrypoint::ProgramResult,
        msg,
        program::invoke_signed,
        program_error::ProgramError,
        program_pack::Pack,
        pubkey::Pubkey,
        system_instruction,
        sysvar::{clock::Clock, Sysvar},
    },
    spl_token::state::Account,
};

#[repr(C)]
#[derive(Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub struct DelegateArgs {
    pub escrow_nonce: u8,
}

struct Accounts<'a, 'b: 'a> {
    token_creator: &'a AccountInfo<'b>,
    mint: &'a AccountInfo<'b>,
    escrow: &'a AccountInfo<'b>,
    token_account: &'a AccountInfo<'b>,
    token_program: &'a AccountInfo<'b>,
}

fn parse_accounts<'a, 'b: 'a>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'b>],
) -> Result<Accounts<'a, 'b>, ProgramError> {
    let account_iter = &mut accounts.iter();
    let accounts = Accounts {
        token_creator: next_account_info(account_iter)?,
        mint: next_account_info(account_iter)?,
        escrow: next_account_info(account_iter)?,
        token_account: next_account_info(account_iter)?,
        token_program: next_account_info(account_iter)?,
    };

    assert_signer(accounts.token_creator)?;
    assert_owned_by(accounts.mint, &spl_token::id())?;
    assert_token_program_matches_package(accounts.token_program)?;
    if *accounts.token_program.key != spl_token::id() {
        return Err(AuctionError::InvalidTokenProgram.into());
    }

    Ok(accounts)
}

pub fn delegate(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    args: DelegateArgs,
) -> ProgramResult {
    let accounts = parse_accounts(program_id, accounts)?;

    // check escrow validity and derive escrow seeds
    let escrow_seeds = &[b"escrow", accounts.token_creator.key.as_ref(), &[args.escrow_nonce]];
    let escrow_pubkey = Pubkey::create_program_address(
        escrow_seeds, program_id)?;
    if escrow_pubkey != *accounts.escrow.key {
        msg!("TokenAuthMismatch");
        return Err(AuctionError::InvalidDelegate.into());
    }

    // change mint and freeze authority
    msg!("Setting mint authority");
    invoke_signed(
        &set_authority(
            accounts.token_program.key,
            accounts.mint.key,
            Some(accounts.escrow.key),
            AuthorityType::MintTokens,
            accounts.token_creator.key,
            &[&accounts.token_creator.key],
        )
        .unwrap(),
        &[
            accounts.token_creator.clone(),
            accounts.mint.clone(),
            accounts.token_program.clone(),
            accounts.escrow.clone(),
        ],
        &[escrow_seeds],
    )?;
    msg!("Setting freeze authority");
    invoke_signed(
        &set_authority(
            accounts.token_program.key,
            accounts.mint.key,
            Some(accounts.escrow.key),
            AuthorityType::FreezeAccount,
            accounts.token_creator.key,
            &[&accounts.token_creator.key],
        )
        .unwrap(),
        &[
            accounts.token_creator.clone(),
            accounts.mint.clone(),
            accounts.token_program.clone(),
            accounts.escrow.clone(),
        ],
        &[escrow_seeds],
    )?;

    // change ownership of the target token account
    msg!("Setting owner authority");
    invoke_signed(
        &set_authority(
            accounts.token_program.key,
            accounts.token_account.key,
            Some(accounts.escrow.key),
            AuthorityType::AccountOwner,
            accounts.token_creator.key,
            &[&accounts.token_creator.key],
        )
        .unwrap(),
        &[
            accounts.token_creator.clone(),
            accounts.token_account.clone(),
            accounts.token_program.clone(),
            accounts.escrow.clone(),
        ],
        &[escrow_seeds],
    )?;
    Ok(())
}

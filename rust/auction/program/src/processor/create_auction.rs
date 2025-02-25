use mem::size_of;

use crate::{
    errors::AuctionError,
    processor::{
        AuctionData, AuctionDataExtended, AuctionName, AuctionState, Bid, BidState, PriceFloor,
        WinnerLimit, BASE_AUCTION_DATA_SIZE, MAX_AUCTION_DATA_EXTENDED_SIZE,
    },
    utils::{
        assert_derivation,
        assert_owned_by,
        create_or_allocate_account_raw,
        spl_token_create_account,// ADDED
        TokenCreateAccount,// ADDED
        spl_token_transfer,// ADDED
        TokenTransferParams,// ADDED
    },
    EXTENDED, PREFIX,
};

use {
    borsh::{BorshDeserialize, BorshSerialize},
    solana_program::{
        account_info::{next_account_info, AccountInfo},
        clock::UnixTimestamp,
        entrypoint::ProgramResult,
        msg,
        program_error::ProgramError,
        pubkey::Pubkey,
    },
    std::mem,
};

#[repr(C)]
#[derive(Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub struct CreateAuctionArgs {
    /// How many winners are allowed for this auction. See AuctionData.
    pub winners: WinnerLimit,
    /// End time is the cut-off point that the auction is forced to end by. See AuctionData.
    pub end_auction_at: Option<UnixTimestamp>,
    /// Gap time is how much time after the previous bid where the auction ends. See AuctionData.
    pub end_auction_gap: Option<UnixTimestamp>,
    /// Token mint for the SPL token used for bidding.
    pub token_mint: Pubkey,
    /// Authority
    pub authority: Pubkey,
    /// The resource being auctioned. See AuctionData.
    pub resource: Pubkey,
    /// Set a price floor.
    pub price_floor: PriceFloor,
    /// Add a tick size increment
    pub tick_size: Option<u64>,
    /// Add a minimum percentage increase each bid must meet.
    pub gap_tick_size_percentage: Option<u8>,
    /// Target nft mint of auction
    pub nft_mint: Pubkey,// ADDED
    /// amount of nft token for selling
    pub nft_selling_amount: u64,// ADDED
}

struct Accounts<'a, 'b: 'a> {
    auction: &'a AccountInfo<'b>,
    auction_extended: &'a AccountInfo<'b>,
    nft_mint: &'a AccountInfo<'b>,// ADDED
    seller_nft_token_account: &'a AccountInfo<'b>,// ADDED
    escrow: &'a AccountInfo<'b>,// ADDED
    auction_nft_account: &'a AccountInfo<'b>,// ADDED
    token_program: &'a AccountInfo<'b>,// ADDED
    payer: &'a AccountInfo<'b>,
    rent: &'a AccountInfo<'b>,
    system: &'a AccountInfo<'b>,
}

fn parse_accounts<'a, 'b: 'a>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'b>],
) -> Result<Accounts<'a, 'b>, ProgramError> {
    let account_iter = &mut accounts.iter();
    let accounts = Accounts {
        payer: next_account_info(account_iter)?,
        auction: next_account_info(account_iter)?,
        auction_extended: next_account_info(account_iter)?,
        nft_mint: next_account_info(account_iter)?,// ADDED
        seller_nft_token_account: next_account_info(account_iter)?,// ADDED
        escrow: next_account_info(account_iter)?,// ADDED
        auction_nft_account: next_account_info(account_iter)?,// ADDED
        token_program: next_account_info(account_iter)?,// ADDED
        rent: next_account_info(account_iter)?,
        system: next_account_info(account_iter)?,
    };
    Ok(accounts)
}

pub fn create_auction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    args: CreateAuctionArgs,
    instant_sale_price: Option<u64>,
    name: Option<AuctionName>,
) -> ProgramResult {
    msg!("+ Processing CreateAuction");
    let accounts = parse_accounts(program_id, accounts)?;

    let auction_path = [
        PREFIX.as_bytes(),
        program_id.as_ref(),
        &args.resource.to_bytes(),
    ];

    // Derive the address we'll store the auction in, and confirm it matches what we expected the
    // user to provide.
    let (auction_key, bump) = Pubkey::find_program_address(&auction_path, program_id);
    if auction_key != *accounts.auction.key {
        return Err(AuctionError::InvalidAuctionAccount.into());
    }
    // The data must be large enough to hold at least the number of winners.
    let auction_size = match args.winners {
        WinnerLimit::Capped(n) => {
            mem::size_of::<Bid>() * BidState::max_array_size_for(n) + BASE_AUCTION_DATA_SIZE
        }
        WinnerLimit::Unlimited(_) => BASE_AUCTION_DATA_SIZE,
    };

    let bid_state = match args.winners {
        WinnerLimit::Capped(n) => BidState::new_english(n),
        WinnerLimit::Unlimited(_) => BidState::new_open_edition(),
    };

    if let Some(gap_tick) = args.gap_tick_size_percentage {
        if gap_tick > 100 {
            return Err(AuctionError::InvalidGapTickSizePercentage.into());
        }
    }

    // Create auction account with enough space for a winner tracking.
    create_or_allocate_account_raw(
        *program_id,
        accounts.auction,
        accounts.rent,
        accounts.system,
        accounts.payer,
        auction_size,
        &[
            PREFIX.as_bytes(),
            program_id.as_ref(),
            &args.resource.to_bytes(),
            &[bump],
        ],
    )?;
    // ADDED START: create auction's token account for target nft
    // Derive the address we'll store the auction in, and confirm it matches what we expected the
    // user to provide.
    let (auction_nft_account_key, nft_account_bump) = Pubkey::find_program_address(&[b"nft", auction_key.as_ref()], program_id);
    if auction_nft_account_key != *accounts.auction_nft_account.key {
        return Err(AuctionError::InvalidAuctionAccount.into());
    }
    spl_token_create_account(TokenCreateAccount {
        payer: accounts.payer.clone(),
        mint: accounts.nft_mint.clone(),
        account: accounts.auction_nft_account.clone(),
        authority: accounts.auction.clone(),
        authority_seeds:
        &[
            PREFIX.as_bytes(),
            program_id.as_ref(),
            &args.resource.to_bytes(),
            &[bump],
        ],
        token_program: accounts.token_program.clone(),
        rent: accounts.rent.clone(),
    })?;

    // check escrow validity and derive escrow seeds
    let (escrow_key, escrow_seeds) = Pubkey::find_program_address(&[b"escrow", accounts.payer.key.as_ref()], program_id);
    if escrow_key != *accounts.escrow.key {
        return Err(AuctionError::InvalidAuctionAccount.into());
    }
    spl_token_transfer(TokenTransferParams {
        source: accounts.seller_nft_token_account.clone(),
        destination: accounts.auction_nft_account.clone(),
        authority: accounts.escrow.clone(),
        authority_signer_seeds:
        &[
            b"escrow",
            accounts.payer.key.as_ref(),
            &[escrow_seeds]
        ],
        token_program: accounts.token_program.clone(),
        amount: args.nft_selling_amount,
    })?;
    // ADDED END

    let auction_ext_bump = assert_derivation(
        program_id,
        accounts.auction_extended,
        &[
            PREFIX.as_bytes(),
            program_id.as_ref(),
            &args.resource.to_bytes(),
            EXTENDED.as_bytes(),
        ],
    )?;

    create_or_allocate_account_raw(
        *program_id,
        accounts.auction_extended,
        accounts.rent,
        accounts.system,
        accounts.payer,
        MAX_AUCTION_DATA_EXTENDED_SIZE,
        &[
            PREFIX.as_bytes(),
            program_id.as_ref(),
            &args.resource.to_bytes(),
            EXTENDED.as_bytes(),
            &[auction_ext_bump],
        ],
    )?;

    // Configure extended
    AuctionDataExtended {
        nft_mint: args.nft_mint,// ADDED
        nft_amount: args.nft_selling_amount,// ADDED
        total_uncancelled_bids: 0,
        tick_size: args.tick_size,
        gap_tick_size_percentage: args.gap_tick_size_percentage,
        instant_sale_price,
        name,
    }
    .serialize(&mut *accounts.auction_extended.data.borrow_mut())?;

    // Configure Auction.
    AuctionData {
        authority: args.authority,
        bid_state: bid_state,
        end_auction_at: args.end_auction_at,
        end_auction_gap: args.end_auction_gap,
        ended_at: None,
        last_bid: None,
        price_floor: args.price_floor,
        state: AuctionState::create(),
        token_mint: args.token_mint,
    }
    .serialize(&mut *accounts.auction.data.borrow_mut())?;

    Ok(())
}

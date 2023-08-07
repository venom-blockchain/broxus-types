use bitflags::bitflags;

use crate::cell::*;
use crate::error::Error;
use crate::models::currency::CurrencyCollection;
use crate::models::message::LazyMessage;

/// Out actions list reverse iterator.
pub struct OutActionsRevIter<'a> {
    slice: CellSlice<'a>,
}

impl<'a> OutActionsRevIter<'a> {
    /// Creates a new output actions list iterator from the list rev head.
    pub fn new(slice: CellSlice<'a>) -> Self {
        Self { slice }
    }
}

impl<'a> Iterator for OutActionsRevIter<'a> {
    type Item = Result<OutAction, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let prev_cell = match self.slice.load_reference() {
            Ok(cell) => cell,
            Err(_) => {
                return if self.slice.is_data_empty() && self.slice.is_refs_empty() {
                    None
                } else {
                    Some(Err(Error::InvalidData))
                }
            }
        };

        let action = match OutAction::load_from(&mut self.slice) {
            Ok(action) => action,
            Err(e) => return Some(Err(e)),
        };
        self.slice = match prev_cell.as_slice() {
            Ok(slice) => slice,
            Err(e) => return Some(Err(e)),
        };
        Some(Ok(action))
    }
}

bitflags! {
    /// Mode flags for `SendMsg` output action.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct SendMsgFlags: u8 {
        /// The sender will pay transfer fees separately.
        const PAY_FEE_SEPARATELY = 1;
        /// Any errors arising while processing this message during
        /// the action phase should be ignored.
        const IGNORE_ERROR = 2;
        /// The current account must be destroyed if its resulting balance is zero.
        const DELETE_IF_EMPTY = 32;
        /// Message will carry all the remaining value of the inbound message
        /// in addition to the value initially indicated in the new message
        /// (if bit 0 is not set, the gas fees are deducted from this amount).
        const WITH_REMAINING_BALANCE = 64;
        /// Message will carry all the remaining balance of the current smart contract
        /// (instead of the value originally indicated in the message).
        const ALL_BALANCE = 128;
    }
}

impl Store for SendMsgFlags {
    fn store_into(&self, builder: &mut CellBuilder, _: &mut dyn Finalizer) -> Result<(), Error> {
        builder.store_u8(self.bits())
    }
}

impl<'a> Load<'a> for SendMsgFlags {
    fn load_from(slice: &mut CellSlice<'a>) -> Result<Self, Error> {
        Ok(Self::from_bits_retain(ok!(slice.load_u8())))
    }
}

bitflags! {
    /// Mode flags for `ReserveCurrency` output action.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct ReserveCurrencyFlags: u8 {
        /// Output action will reserve all but x nanograms.
        const ALL_BUT = 1;
        /// The external action does not fail if the specified amount cannot bereserved.
        /// Instead, all remaining balance is reserve.
        const IGNORE_ERROR = 2;
        /// x is increased by the original balance of the current account (before the
        /// compute phase), including all extra currencies, before performing any
        /// other checks and actions.
        const WITH_ORIGINAL_BALANCE = 4;
        /// `x = −x` before performing any further action.
        const REVERSE = 8;
    }
}

impl Store for ReserveCurrencyFlags {
    fn store_into(&self, builder: &mut CellBuilder, _: &mut dyn Finalizer) -> Result<(), Error> {
        builder.store_u8(self.bits())
    }
}

impl<'a> Load<'a> for ReserveCurrencyFlags {
    #[inline]
    fn load_from(slice: &mut CellSlice<'a>) -> Result<Self, Error> {
        Ok(Self::from_bits_retain(ok!(slice.load_u8())))
    }
}

bitflags! {
    /// Mode flags for `ChangeLibrary` output action.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct ChangeLibraryFlags: u8 {
        /// Output action will reserve all but x nanograms.
        const REMOVE = 1;
        /// Adds library as private.
        const PRIVATE = 2 + 1;
        /// Adds library as public.
        const PUBLIC = 4 + 1;
    }
}

impl Store for ChangeLibraryFlags {
    fn store_into(&self, builder: &mut CellBuilder, _: &mut dyn Finalizer) -> Result<(), Error> {
        builder.store_u8(self.bits())
    }
}

impl<'a> Load<'a> for ChangeLibraryFlags {
    #[inline]
    fn load_from(slice: &mut CellSlice<'a>) -> Result<Self, Error> {
        Ok(Self::from_bits_retain(ok!(slice.load_u8())))
    }
}

/// Output action.
pub enum OutAction {
    /// Sends a raw message contained in cell.
    SendMsg {
        /// Behavior flags.
        mode: SendMsgFlags,
        /// A cell with a message.
        out_msg: LazyMessage,
    },
    /// Creates an output action that would change this smart contract code
    /// to that given by cell.
    SetCode {
        /// A cell with new code.
        new_code: Cell,
    },
    /// Creates an output action which would reserve exactly some balance.
    ReserveCurrency {
        /// Behavior flags.
        mode: ReserveCurrencyFlags,
        /// Reserved value.
        value: CurrencyCollection,
    },
    /// Creates an output action that would modify the collection of this
    /// smart contract libraries by adding or removing library with code given in cell.
    ChangeLibrary {
        /// Behavior flags.
        mode: ChangeLibraryFlags,
        /// Cell library hash.
        hash: Option<HashBytes>,
        /// Cell library code.
        code: Option<Cell>,
    },
    /// Copyleft action.
    CopyLeft {
        /// License number.
        license: u8,
        /// Owner address.
        address: HashBytes,
    },
}

impl OutAction {
    const TAG_SEND_MSG: u32 = 0x0ec3c86d;
    const TAG_SET_CODE: u32 = 0xad4de08e;
    const TAG_RESERVE: u32 = 0x36e6b809;
    const TAG_CHANGE_LIB: u32 = 0x26fa1dd4;
    const TAG_COPYLEFT: u32 = 0x24486f7a;
}

impl Store for OutAction {
    fn store_into(
        &self,
        builder: &mut CellBuilder,
        finalizer: &mut dyn Finalizer,
    ) -> Result<(), Error> {
        match self {
            Self::SendMsg { mode, out_msg } => {
                ok!(builder.store_u32(Self::TAG_SEND_MSG));
                ok!(builder.store_u8(mode.bits()));
                builder.store_reference(out_msg.inner().clone())
            }
            Self::SetCode { new_code } => {
                ok!(builder.store_u32(Self::TAG_SET_CODE));
                builder.store_reference(new_code.clone())
            }
            Self::ReserveCurrency { mode, value } => {
                ok!(builder.store_u32(Self::TAG_RESERVE));
                ok!(builder.store_u8(mode.bits()));
                value.store_into(builder, finalizer)
            }
            Self::ChangeLibrary { mode, code, hash } => {
                ok!(builder.store_u32(Self::TAG_CHANGE_LIB));
                ok!(builder.store_u8(mode.bits()));
                if let Some(hash) = hash {
                    ok!(builder.store_u256(hash));
                }
                if let Some(code) = code {
                    ok!(builder.store_reference(code.clone()))
                }
                Ok(())
            }
            Self::CopyLeft { license, address } => {
                ok!(builder.store_u32(Self::TAG_COPYLEFT));
                ok!(builder.store_u8(*license));
                builder.store_u256(address)
            }
        }
    }
}

impl<'a> Load<'a> for OutAction {
    fn load_from(slice: &mut CellSlice<'a>) -> Result<Self, Error> {
        let tag = ok!(slice.load_u32());
        Ok(match tag {
            Self::TAG_SEND_MSG => Self::SendMsg {
                mode: ok!(SendMsgFlags::load_from(slice)),
                out_msg: ok!(LazyMessage::load_from(slice)),
            },
            Self::TAG_SET_CODE => Self::SetCode {
                new_code: ok!(slice.load_reference_cloned()),
            },
            Self::TAG_RESERVE => Self::ReserveCurrency {
                mode: ok!(ReserveCurrencyFlags::load_from(slice)),
                value: ok!(CurrencyCollection::load_from(slice)),
            },
            Self::TAG_CHANGE_LIB => {
                let mode = ok!(ChangeLibraryFlags::load_from(slice));
                let load_hash = mode.is_empty();
                Self::ChangeLibrary {
                    mode,
                    hash: if load_hash {
                        Some(ok!(slice.load_u256()))
                    } else {
                        None
                    },
                    code: if !load_hash {
                        Some(ok!(slice.load_reference_cloned()))
                    } else {
                        None
                    },
                }
            }
            Self::TAG_COPYLEFT => Self::CopyLeft {
                license: ok!(slice.load_u8()),
                address: ok!(slice.load_u256()),
            },
            _ => return Err(Error::InvalidTag),
        })
    }
}
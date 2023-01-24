#![no_main]
use libfuzzer_sys::{fuzz_target, Corpus};

use everscale_types::cell::Load;
use everscale_types::models::Message;
use everscale_types::{Boc, RcCellFamily};

fuzz_target!(|data: &[u8]| -> Corpus {
    if let Ok(cell) = Boc::<RcCellFamily>::decode(data) {
        if Message::<RcCellFamily>::load_from(&mut cell.as_slice()).is_some() {
            return Corpus::Keep;
        }
    }
    Corpus::Reject
});
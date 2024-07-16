//! Dictionary implementation.

pub use self::aug::*;
pub use self::ops::*;
pub use self::raw::*;
pub use self::typed::*;

use crate::cell::*;
use crate::error::Error;

mod aug;
mod raw;
mod typed;

mod ops {
    pub use self::build::{build_aug_dict_from_sorted_iter, build_dict_from_sorted_iter};
    pub use self::find::{dict_find_bound, dict_find_bound_owned, dict_find_owned};
    pub use self::get::{dict_get, dict_get_owned, dict_get_subdict};
    pub use self::insert::{aug_dict_insert, dict_insert, dict_insert_owned};
    pub use self::remove::{aug_dict_remove_owned, dict_remove_bound_owned, dict_remove_owned};
    pub use self::split_merge::{dict_merge, dict_split_by_prefix};

    mod build;
    mod find;
    mod get;
    mod insert;
    mod remove;
    mod split_merge;
}

/// Type which can be used as a dictionary key.
pub trait DictKey: Sized {
    /// Length in bits for a dictionary key.
    const BITS: u16;

    /// Creates a key from a raw builder data.
    fn from_raw_data(raw_data: &[u8; 128]) -> Option<Self>;
}

macro_rules! impl_dict_key {
    ($($ty:ty => $bits:literal => |$raw_data:ident| $expr:expr),*,) => {
        $(impl DictKey for $ty {
            const BITS: u16 = $bits;

            #[inline]
            fn from_raw_data($raw_data: &[u8; 128]) -> Option<Self> {
                Some($expr)
            }
        })*
    };
}

impl_dict_key! {
    bool => 1 => |d| d[0] & 0x80 != 0,
    u8 => 8 => |d| d[0],
    i8 => 8 => |d| d[0] as i8,
    u16 => 16 => |d| u16::from_be_bytes([d[0], d[1]]),
    i16 => 16 => |d| i16::from_be_bytes([d[0], d[1]]),
    u32 => 32 => |d| u32::from_be_bytes(d[..4].try_into().unwrap()),
    i32 => 32 => |d| i32::from_be_bytes(d[..4].try_into().unwrap()),
    u64 => 64 => |d| u64::from_be_bytes(d[..8].try_into().unwrap()),
    i64 => 64 => |d| i64::from_be_bytes(d[..8].try_into().unwrap()),
    u128 => 128 => |d| u128::from_be_bytes(d[..16].try_into().unwrap()),
    i128 => 128 => |d| i128::from_be_bytes(d[..16].try_into().unwrap()),
    [u8; 16] => 128 => |d| d[..16].try_into().unwrap(),
    [u8; 20] => 160 => |d| d[..20].try_into().unwrap(),
    [u8; 32] => 256 => |d| d[..32].try_into().unwrap(),
    HashBytes => 256 => |d| HashBytes(d[..32].try_into().unwrap()),
     (u64, u32) => 96 => |d| {
        (u64::from_be_bytes(d[..8].try_into().unwrap()), u32::from_be_bytes(d[8..12].try_into().unwrap()))
    },
}

/// Dictionary insertion mode.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
pub enum SetMode {
    /// Sets the value associated with the key in the dictionary.
    Set = 0b11,
    /// Sets the value associated with the key in the dictionary
    /// only if the key was already present in it.
    Replace = 0b01,
    /// Sets the value associated with key in dictionary,
    /// but only if it is not already present.
    Add = 0b10,
}

impl SetMode {
    /// Returns `true` if the new value can replace the old value for the same key.
    #[inline]
    pub const fn can_replace(self) -> bool {
        self as u8 & 0b01 != 0
    }

    /// Returns `true` if inserting a value can add a new key to the dictionary.
    #[inline]
    pub const fn can_add(self) -> bool {
        self as u8 & 0b10 != 0
    }
}

/// A type for a comparator function for [`AugDict`].
///
/// ## Args
/// - `left` - a left branch data.
/// - `right` - a right branch data.
/// - `builder` - a builder to write the result.
/// - `context` - a cell context.
pub type AugDictFn =
    fn(&mut CellSlice, &mut CellSlice, &mut CellBuilder, &mut dyn CellContext) -> Result<(), Error>;

/// Creates a leaf node
fn make_leaf(
    key: &CellSlice,
    key_bit_len: u16,
    value: &dyn Store,
    context: &mut dyn CellContext,
) -> Result<Cell, Error> {
    let mut builder = CellBuilder::new();
    ok!(write_label(key, key_bit_len, &mut builder));
    ok!(value.store_into(&mut builder, context));
    builder.build_ext(context)
}

/// Creates a leaf node with extra value
fn make_leaf_with_extra(
    key: &CellSlice,
    key_bit_len: u16,
    extra: &dyn Store,
    value: &dyn Store,
    context: &mut dyn CellContext,
) -> Result<Cell, Error> {
    let mut builder = CellBuilder::new();
    ok!(write_label(key, key_bit_len, &mut builder));
    ok!(extra.store_into(&mut builder, context));
    ok!(value.store_into(&mut builder, context));
    builder.build_ext(context)
}

/// Splits an edge or leaf
fn split_edge(
    data: &CellSlice,
    prefix: &mut CellSlice,
    lcp: &CellSlice,
    key: &mut CellSlice,
    value: &dyn Store,
    context: &mut dyn CellContext,
) -> Result<Cell, Error> {
    // Advance the key
    let prev_key_bit_len = key.remaining_bits();
    if !key.try_advance(lcp.remaining_bits() + 1, 0) {
        return Err(Error::CellUnderflow);
    }

    // Read the next bit from the data
    prefix.try_advance(lcp.remaining_bits(), 0);
    let old_to_right = ok!(prefix.load_bit());

    // Create a leaf for the old value
    let mut left = ok!(make_leaf(prefix, key.remaining_bits(), data, context));
    // Create a leaf for the right value
    let mut right = ok!(make_leaf(key, key.remaining_bits(), value, context));

    // The part that starts with 1 goes to the right cell
    if old_to_right {
        std::mem::swap(&mut left, &mut right);
    }

    // Create fork
    let mut builder = CellBuilder::new();
    ok!(write_label(lcp, prev_key_bit_len, &mut builder));
    ok!(builder.store_reference(left));
    ok!(builder.store_reference(right));
    builder.build_ext(context)
}

#[allow(clippy::too_many_arguments)]
fn split_aug_edge(
    data: &mut CellSlice,
    prefix: &mut CellSlice,
    lcp: &CellSlice,
    key: &mut CellSlice,
    extra: &dyn Store,
    value: &dyn Store,
    comparator: AugDictFn,
    context: &mut dyn CellContext,
) -> Result<Cell, Error> {
    // Advance the key
    let prev_key_bit_len = key.remaining_bits();
    if !key.try_advance(lcp.remaining_bits() + 1, 0) {
        return Err(Error::CellUnderflow);
    }

    // Read the next bit from the data
    prefix.try_advance(lcp.remaining_bits(), 0);
    let old_to_right = ok!(prefix.load_bit());

    // Create a leaf for the old value
    let mut left = ok!(make_leaf(prefix, key.remaining_bits(), data, context));
    // Create a leaf for the new value
    let mut right = ok!(make_leaf_with_extra(
        key,
        key.remaining_bits(),
        extra,
        value,
        context
    ));
    // The part that starts with 1 goes to the right cell
    if old_to_right {
        std::mem::swap(&mut left, &mut right);
    }

    let left_slice = &mut ok!(left.as_slice());
    let right_slice = &mut ok!(right.as_slice());
    ok!(read_label(left_slice, key.remaining_bits()));
    ok!(read_label(right_slice, key.remaining_bits()));

    // Create fork edge
    let mut builder = CellBuilder::new();
    ok!(write_label(lcp, prev_key_bit_len, &mut builder));
    ok!(builder.store_reference(left.clone()));
    ok!(builder.store_reference(right.clone()));
    ok!(comparator(left_slice, right_slice, &mut builder, context));
    builder.build_ext(context)
}

/// Type alias for a pair of key and value as cell slice parts.
pub type DictOwnedEntry = (CellBuilder, CellSliceParts);

/// Dictionary bound.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub enum DictBound {
    /// The lowest dictionary key.
    Min,
    /// The largest dictionary key.
    Max,
}

impl DictBound {
    fn update_direction(
        self,
        prefix: &CellSlice<'_>,
        signed: bool,
        direction: &mut Option<Branch>,
    ) -> Branch {
        match direction {
            // Compute direction by the first part
            None => {
                let mut branch = *direction.insert(self.into_branch());
                // Invert first bit for signed keys if starting from the empty part
                if signed && prefix.is_data_empty() {
                    branch = branch.reversed();
                }
                branch
            }
            // Use the same direction for all remaining parts
            Some(direction) => *direction,
        }
    }

    fn into_branch(self) -> Branch {
        match self {
            Self::Min => Branch::Left,
            Self::Max => Branch::Right,
        }
    }
}

/// Loads a non-empty dictionary from the root cell.
pub fn dict_load_from_root(
    slice: &mut CellSlice<'_>,
    key_bit_len: u16,
    context: &mut dyn CellContext,
) -> Result<Cell, Error> {
    let mut root = *slice;

    let label = ok!(read_label(slice, key_bit_len));
    if label.remaining_bits() != key_bit_len {
        if !slice.try_advance(0, 2) {
            return Err(Error::CellUnderflow);
        }
        let root_bits = root.remaining_bits() - slice.remaining_bits();
        let root_refs = root.remaining_refs() - slice.remaining_refs();
        root = root.get_prefix(root_bits, root_refs)
    } else {
        slice.load_remaining();
    }

    let mut builder = CellBuilder::new();
    ok!(builder.store_slice(root));
    builder.build_ext(context)
}

fn rebuild_dict_from_stack(
    mut segments: Vec<Segment<'_>>,
    mut leaf: Cell,
    context: &mut dyn CellContext,
) -> Result<Cell, Error> {
    // Rebuild the tree starting from leaves
    while let Some(last) = segments.pop() {
        // Load the opposite branch
        let (left, right) = match last.next_branch {
            Branch::Left => match last.data.reference_cloned(1) {
                Some(cell) => (leaf, cell),
                None => return Err(Error::CellUnderflow),
            },
            Branch::Right => match last.data.reference_cloned(0) {
                Some(cell) => (cell, leaf),
                None => return Err(Error::CellUnderflow),
            },
        };

        let mut builder = CellBuilder::new();
        ok!(builder.store_cell_data(last.data));
        ok!(builder.store_reference(left));
        ok!(builder.store_reference(right));
        leaf = ok!(builder.build_ext(context));
    }

    Ok(leaf)
}

fn rebuild_aug_dict_from_stack(
    mut segments: Vec<Segment<'_>>,
    mut leaf: Cell,
    comparator: AugDictFn,
    context: &mut dyn CellContext,
) -> Result<Cell, Error> {
    // Rebuild the tree starting from leaves
    while let Some(last) = segments.pop() {
        // Load the opposite branch
        let (left, right) = match last.next_branch {
            Branch::Left => match last.data.reference_cloned(1) {
                Some(cell) => (leaf, cell),
                None => return Err(Error::CellUnderflow),
            },
            Branch::Right => match last.data.reference_cloned(0) {
                Some(cell) => (cell, leaf),
                None => return Err(Error::CellUnderflow),
            },
        };

        let last_data_slice = ok!(last.data.as_slice());
        let last_label = ok!(read_label(&mut last_data_slice.clone(), last.key_bit_len));

        let child_key_bit_len = last.key_bit_len - last_label.remaining_bits() - 1;

        let left_slice = &mut left.as_slice()?;
        let right_slice = &mut right.as_slice()?;
        ok!(read_label(left_slice, child_key_bit_len));
        ok!(read_label(right_slice, child_key_bit_len));

        let mut builder = CellBuilder::new();
        ok!(write_label(&last_label, last.key_bit_len, &mut builder));
        ok!(builder.store_reference(left.clone()));
        ok!(builder.store_reference(right.clone()));
        ok!(comparator(left_slice, right_slice, &mut builder, context));
        leaf = ok!(builder.build_ext(context));
    }

    Ok(leaf)
}

#[derive(Clone, Copy)]
struct Segment<'a> {
    data: &'a DynCell,
    next_branch: Branch,
    key_bit_len: u16,
}

impl Segment<'_> {
    // Returns the new leaf and the removed leaf
    fn rebuild_as_removed(
        self,
        key: &CellSlice<'_>,
        prev_key_bit_len: u16,
        context: &mut dyn CellContext,
    ) -> Result<(Cell, Cell), Error> {
        let index = self.next_branch as u8;

        // Load value branch
        let Some(value) = self.data.reference_cloned(index) else {
            return Err(Error::CellUnderflow);
        };
        // NOTE: do not use gas here as it was accounted while loading `child` in previous block.
        // TODO: change mode to `LoadMode::Noop` if copy-on-write for libraries is not ok.
        let value = ok!(context.load_cell(value, LoadMode::Resolve));

        // Load parent label
        let pfx = {
            // SAFETY: `self.data` was already checked for pruned branch access.
            let mut parent = unsafe { self.data.as_slice_unchecked() };
            ok!(read_label(&mut parent, prev_key_bit_len))
        };

        // Load the opposite branch
        let mut opposite = match self.data.reference(1 - index) {
            // TODO: change mode to `LoadMode::UseGas` if copy-on-write for libraries is not ok.
            Some(cell) => ok!(context
                .load_dyn_cell(cell, LoadMode::Full)
                .and_then(CellSlice::new)),
            None => return Err(Error::CellUnderflow),
        };
        let rem = ok!(read_label(&mut opposite, key.remaining_bits()));

        // Build an edge cell
        let mut builder = CellBuilder::new();
        ok!(write_label_parts(
            &pfx,
            index == 0,
            &rem,
            prev_key_bit_len,
            &mut builder
        ));
        ok!(builder.store_slice(opposite));
        let leaf = ok!(builder.build_ext(context));

        Ok((leaf, value))
    }
}

fn write_label(key: &CellSlice, key_bit_len: u16, label: &mut CellBuilder) -> Result<(), Error> {
    if key_bit_len == 0 || key.is_data_empty() {
        return write_hml_empty(label);
    }

    let bits_for_len = (16 - key_bit_len.leading_zeros()) as u16;

    let remaining_bits = key.remaining_bits();

    let hml_short_len = 2 + 2 * remaining_bits;
    let hml_long_len = 2 + bits_for_len + remaining_bits;
    let hml_same_len = 3 + bits_for_len;

    if hml_same_len < hml_long_len && hml_same_len < hml_short_len {
        if let Some(bit) = key.test_uniform() {
            ok!(write_hml_same(bit, remaining_bits, bits_for_len, label));
            return Ok(());
        }
    }

    if hml_short_len <= MAX_BIT_LEN && hml_short_len <= hml_long_len {
        ok!(write_hml_short_tag(remaining_bits, label));
    } else if hml_long_len <= MAX_BIT_LEN {
        ok!(write_hml_long_tag(remaining_bits, bits_for_len, label));
    } else {
        return Err(Error::InvalidData);
    }

    ok!(label.store_slice_data(key));
    Ok(())
}

fn write_label_parts(
    pfx: &CellSlice,
    bit: bool,
    rem: &CellSlice,
    key_bit_len: u16,
    label: &mut CellBuilder,
) -> Result<(), Error> {
    if key_bit_len == 0 {
        return write_hml_empty(label);
    }

    let bits_for_len = (16 - key_bit_len.leading_zeros()) as u16;

    let remaining_bits = pfx.remaining_bits() + 1 + rem.remaining_bits();

    let hml_short_len = 2 + 2 * remaining_bits;
    let hml_long_len = 2 + bits_for_len + remaining_bits;
    let hml_same_len = 3 + bits_for_len;

    if hml_same_len < hml_long_len && hml_same_len < hml_short_len {
        if let Some(pfx_bit) = pfx.test_uniform() {
            if pfx_bit == bit {
                if let Some(rem_bit) = rem.test_uniform() {
                    if rem_bit == bit {
                        return write_hml_same(bit, remaining_bits, bits_for_len, label);
                    }
                }
            }
        }
    }

    if hml_short_len <= MAX_BIT_LEN && hml_short_len <= hml_long_len {
        ok!(write_hml_short_tag(remaining_bits, label));
    } else if hml_long_len <= MAX_BIT_LEN {
        ok!(write_hml_long_tag(remaining_bits, bits_for_len, label));
    } else {
        return Err(Error::InvalidData);
    }
    ok!(label.store_slice_data(pfx));
    ok!(label.store_bit(bit));
    label.store_slice_data(rem)
}

fn read_label<'a>(label: &mut CellSlice<'a>, key_bit_len: u16) -> Result<CellSlice<'a>, Error> {
    let bits_for_len = (16 - key_bit_len.leading_zeros()) as u16;

    if bits_for_len == 0 && label.is_data_empty() {
        Ok(label.get_prefix(0, 0))
    } else if !ok!(label.load_bit()) {
        read_hml_short(label)
    } else if !ok!(label.load_bit()) {
        read_hml_long(label, bits_for_len)
    } else {
        read_hml_same(label, bits_for_len)
    }
}

fn write_hml_empty(label: &mut CellBuilder) -> Result<(), Error> {
    label.store_zeros(2)
}

fn write_hml_short_tag(len: u16, label: &mut CellBuilder) -> Result<(), Error> {
    ok!(label.store_bit_zero());

    for _ in 0..len / 32 {
        ok!(label.store_u32(u32::MAX));
    }

    let rem = len % 32;
    if rem != 0 {
        ok!(label.store_uint(u64::MAX, rem));
    }
    label.store_bit_zero()
}

fn read_hml_short<'a>(label: &mut CellSlice<'a>) -> Result<CellSlice<'a>, Error> {
    let mut len = 0;
    while ok!(label.load_bit()) {
        len += 1;
    }
    let result = *label;
    if label.try_advance(len, 0) {
        Ok(result.get_prefix(len, 0))
    } else {
        Err(Error::CellUnderflow)
    }
}

fn write_hml_long_tag(len: u16, bits_for_len: u16, label: &mut CellBuilder) -> Result<(), Error> {
    ok!(label.store_bit_one());
    ok!(label.store_bit_zero());
    label.store_uint(len as u64, bits_for_len)
}

fn read_hml_long<'a>(label: &mut CellSlice<'a>, bits_for_len: u16) -> Result<CellSlice<'a>, Error> {
    let len = ok!(label.load_uint(bits_for_len)) as u16;
    let result = *label;
    if label.try_advance(len, 0) {
        Ok(result.get_prefix(len, 0))
    } else {
        Err(Error::CellUnderflow)
    }
}

fn write_hml_same(
    bit: bool,
    len: u16,
    bits_for_len: u16,
    label: &mut CellBuilder,
) -> Result<(), Error> {
    ok!(label.store_small_uint(0b110 | bit as u8, 3));
    label.store_uint(len as u64, bits_for_len)
}

fn read_hml_same<'a>(label: &mut CellSlice<'a>, bits_for_len: u16) -> Result<CellSlice<'a>, Error> {
    let cell = match ok!(label.load_bit()) {
        false => Cell::all_zeros_ref(),
        true => Cell::all_ones_ref(),
    };
    let len = ok!(label.load_uint(bits_for_len)) as u16;

    // SAFETY: cell is a static ordinary cell
    let slice = unsafe { cell.as_slice_unchecked() };
    Ok(slice.get_prefix(len, 0))
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum Branch {
    // Branch for a key part that starts with bit 0
    Left = 0,
    // Branch for a key part that starts with bit 1
    Right = 1,
}

impl Branch {
    fn into_bit(self) -> bool {
        self == Self::Right
    }

    fn reversed(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

impl From<bool> for Branch {
    fn from(value: bool) -> Self {
        if value {
            Self::Right
        } else {
            Self::Left
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_cell<F: FnOnce(&mut CellBuilder) -> Result<(), Error>>(f: F) -> Cell {
        let mut builder = CellBuilder::new();
        f(&mut builder).unwrap();
        builder.build().unwrap()
    }

    #[test]
    fn labels() -> anyhow::Result<()> {
        let key_bit_len = 6;

        // Build key
        let key = {
            let mut builder = CellBuilder::new();
            builder.store_zeros(5)?;
            builder.store_bit_one()?;
            builder.build()?
        };

        // Build label
        let label = {
            let mut builder = CellBuilder::new();
            write_label(&key.as_slice()?, key_bit_len, &mut builder)?;
            builder.build().unwrap()
        };

        // Parse label
        let parsed_key = read_label(&mut label.as_slice()?, key_bit_len)?;
        let parsed_key = {
            let mut builder = CellBuilder::new();
            builder.store_slice(parsed_key)?;
            builder.build()?
        };

        // Parsed key should be equal to the original
        assert_eq!(key.as_ref(), parsed_key.as_ref());

        let label = CellBuilder::from_raw_data(&[0xcc, 0x40], 9)
            .unwrap()
            .build()
            .unwrap();
        let prefix = read_label(&mut label.as_slice()?, 32).unwrap();

        println!("{}", build_cell(|b| b.store_slice(prefix)).display_tree());
        assert_eq!(prefix.test_uniform(), Some(false));

        Ok(())
    }

    #[test]
    fn build_from_array() {
        for entries in [
            &[(0u32, 1u32), (1, 2), (2, 4), (2, 3), (3, 4), (4, 5)][..],
            &[
                (534837844, 3117028142),
                (1421713188, 3155891450),
                (1526242096, 2789399854),
                (1971086295, 1228713494),
                (4258889371, 3256452222),
            ],
        ] {
            let result = build_dict_from_sorted_iter(
                entries.iter().copied(),
                32,
                &mut Cell::empty_context(),
            )
            .unwrap();

            let mut dict = Dict::<u32, u32>::new();
            for (k, v) in entries {
                dict.add(k, v).unwrap();
            }

            println!("{}", result.as_ref().unwrap().display_tree());
            println!(
                "BOC: {}",
                crate::boc::BocRepr::encode_base64(&result).unwrap()
            );

            assert_eq!(result, dict.root);
        }
    }

    #[test]
    fn build_from_any_array() {
        for _ in 0..100 {
            let n = 1 + rand::random::<usize>() % 1000;
            let mut entries = (0..n)
                .map(|_| (rand::random::<u32>(), rand::random::<u32>()))
                .collect::<Vec<_>>();
            entries.sort_by_key(|(k, _)| *k);

            let built_from_dict = build_dict_from_sorted_iter(
                entries.iter().copied(),
                32,
                &mut Cell::empty_context(),
            )
            .unwrap();

            let mut dict = Dict::<u32, u32>::new();
            for (k, v) in entries {
                dict.add(k, v).unwrap();
            }

            // println!("{}", built_from_dict.as_ref().unwrap().display_tree());

            assert_eq!(built_from_dict, dict.root);
        }
    }
}

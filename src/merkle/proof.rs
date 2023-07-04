use std::collections::HashMap;
use std::hash::BuildHasher;

use super::{make_pruned_branch, FilterAction, MerkleFilter};
use crate::cell::*;
use crate::error::Error;

/// Non-owning parsed Merkle proof representation.
///
/// NOTE: Serialized into `MerkleProof` cell.
#[derive(Debug, Clone)]
pub struct MerkleProofRef<'a> {
    /// Representation hash of the original cell.
    pub hash: HashBytes,
    /// Representation depth of the origin cell.
    pub depth: u16,
    /// Partially pruned tree with the contents of the original cell.
    pub cell: &'a DynCell,
}

impl Eq for MerkleProofRef<'_> {}

impl PartialEq for MerkleProofRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash && self.depth == other.depth && self.cell == other.cell
    }
}

impl Default for MerkleProofRef<'_> {
    fn default() -> Self {
        Self {
            hash: *EMPTY_CELL_HASH,
            depth: 0,
            cell: Cell::empty_cell_ref(),
        }
    }
}

impl<'a> Load<'a> for MerkleProofRef<'a> {
    fn load_from(s: &mut CellSlice<'a>) -> Result<Self, Error> {
        if !s.has_remaining(MerkleProof::BITS, MerkleProof::REFS) {
            return Err(Error::CellUnderflow);
        }

        if ok!(s.get_u8(0)) != CellType::MerkleProof.to_byte() {
            return Err(Error::InvalidCell);
        }

        let res = Self {
            hash: ok!(s.get_u256(8)),
            depth: ok!(s.get_u16(8 + 256)),
            cell: ok!(s.get_reference(0)),
        };
        if res.cell.hash(0) == &res.hash
            && res.cell.depth(0) == res.depth
            && s.try_advance(MerkleProof::BITS, MerkleProof::REFS)
        {
            Ok(res)
        } else {
            Err(Error::InvalidCell)
        }
    }
}

/// Parsed Merkle proof representation.
///
/// NOTE: Serialized into `MerkleProof` cell.
#[derive(Debug, Clone)]
pub struct MerkleProof {
    /// Representation hash of the original cell.
    pub hash: HashBytes,
    /// Representation depth of the origin cell.
    pub depth: u16,
    /// Partially pruned tree with the contents of the original cell.
    pub cell: Cell,
}

impl Eq for MerkleProof {}

impl PartialEq for MerkleProof {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
            && self.depth == other.depth
            && self.cell.as_ref() == other.cell.as_ref()
    }
}

impl Default for MerkleProof {
    fn default() -> Self {
        Self {
            hash: *EMPTY_CELL_HASH,
            depth: 0,
            cell: Cell::empty_cell(),
        }
    }
}

impl Load<'_> for MerkleProof {
    fn load_from(s: &mut CellSlice) -> Result<Self, Error> {
        if !s.has_remaining(Self::BITS, Self::REFS) {
            return Err(Error::CellUnderflow);
        }

        if ok!(s.get_u8(0)) != CellType::MerkleProof.to_byte() {
            return Err(Error::InvalidCell);
        }

        let res = Self {
            hash: ok!(s.get_u256(8)),
            depth: ok!(s.get_u16(8 + 256)),
            cell: ok!(s.get_reference_cloned(0)),
        };
        if res.cell.as_ref().hash(0) == &res.hash
            && res.cell.as_ref().depth(0) == res.depth
            && s.try_advance(Self::BITS, Self::REFS)
        {
            Ok(res)
        } else {
            Err(Error::InvalidCell)
        }
    }
}

impl Store for MerkleProof {
    fn store_into(&self, b: &mut CellBuilder, _: &mut dyn Finalizer) -> Result<(), Error> {
        if !b.has_capacity(Self::BITS, Self::REFS) {
            return Err(Error::CellOverflow);
        }

        let level_mask = self.cell.as_ref().level_mask();
        b.set_level_mask(level_mask.virtualize(1));
        b.set_exotic(true);
        ok!(b.store_u8(CellType::MerkleProof.to_byte()));
        ok!(b.store_u256(&self.hash));
        ok!(b.store_u16(self.depth));
        b.store_reference(self.cell.clone())
    }
}

impl MerkleProof {
    /// The number of data bits that the Merkle proof occupies.
    pub const BITS: u16 = 8 + 256 + 16;
    /// The number of references that the Merkle proof occupies.
    pub const REFS: u8 = 1;

    /// Starts building a Merkle proof for the specified root,
    /// using cells determined by filter.
    pub fn create<'a, F>(root: &'a DynCell, f: F) -> MerkleProofBuilder<'a, F>
    where
        F: MerkleFilter + 'a,
    {
        MerkleProofBuilder::new(root, f)
    }

    /// Create a Merkle proof for the single cell with the specified
    /// representation hash.
    ///
    /// Only ancestors of the first occurrence are included in the proof.
    ///
    /// Proof creation will fail if the specified child is not found.
    pub fn create_for_cell<'a>(
        root: &'a DynCell,
        child_hash: &'a HashBytes,
    ) -> MerkleProofBuilder<'a, impl MerkleFilter + 'a> {
        struct RootOrChild<'a> {
            cells: ahash::HashSet<&'a HashBytes>,
            child_hash: &'a HashBytes,
        }

        impl MerkleFilter for RootOrChild<'_> {
            fn check(&self, cell: &HashBytes) -> FilterAction {
                if self.cells.contains(cell) || cell == self.child_hash {
                    FilterAction::Include
                } else {
                    FilterAction::Skip
                }
            }
        }

        let mut stack = vec![root.references()];
        while let Some(last_cells) = stack.last_mut() {
            match last_cells.next() {
                Some(child) if child.repr_hash() == child_hash => break,
                Some(child) => stack.push(child.references()),
                None => {
                    stack.pop();
                }
            }
        }

        let mut cells = ahash::HashSet::with_capacity_and_hasher(stack.len(), Default::default());
        for item in stack {
            cells.insert(item.cell().repr_hash());
        }

        MerkleProofBuilder::new(root, RootOrChild { cells, child_hash })
    }
}

/// Helper struct to build a Merkle proof.
pub struct MerkleProofBuilder<'a, F> {
    root: &'a DynCell,
    filter: F,
}

impl<'a, F> MerkleProofBuilder<'a, F>
where
    F: MerkleFilter,
{
    /// Creates a new Merkle proof builder for the tree with the specified root,
    /// using cells determined by filter.
    pub fn new(root: &'a DynCell, f: F) -> Self {
        Self { root, filter: f }
    }

    /// Extends the builder to additionally save all hashes
    /// of cells not included in Merkle proof.
    pub fn track_pruned_branches(self) -> MerkleProofExtBuilder<'a, F> {
        MerkleProofExtBuilder {
            root: self.root,
            filter: self.filter,
        }
    }

    /// Builds a Merkle proof using the specified finalizer.
    pub fn build_ext(self, finalizer: &mut dyn Finalizer) -> Result<MerkleProof, Error> {
        let root = self.root;
        let cell = ok!(self.build_raw_ext(finalizer));
        Ok(MerkleProof {
            hash: *root.repr_hash(),
            depth: root.repr_depth(),
            cell,
        })
    }

    /// Builds a Merkle proof child cell using the specified finalizer.
    pub fn build_raw_ext(self, finalizer: &mut dyn Finalizer) -> Result<Cell, Error> {
        BuilderImpl::<ahash::RandomState> {
            root: self.root,
            filter: &self.filter,
            cells: Default::default(),
            pruned_branches: None,
            finalizer,
        }
        .build()
    }
}

impl<'a, F> MerkleProofBuilder<'a, F>
where
    F: MerkleFilter,
{
    /// Builds a Merkle proof using the default finalizer.
    pub fn build(self) -> Result<MerkleProof, Error> {
        self.build_ext(&mut Cell::default_finalizer())
    }
}

/// Helper struct to build a Merkle proof and keep track of all pruned cells.
pub struct MerkleProofExtBuilder<'a, F> {
    root: &'a DynCell,
    filter: F,
}

impl<'a, F> MerkleProofExtBuilder<'a, F>
where
    F: MerkleFilter,
{
    /// Builds a Merkle proof child cell using the specified finalizer.
    pub fn build_raw_ext(
        self,
        finalizer: &mut dyn Finalizer,
    ) -> Result<(Cell, ahash::HashMap<&'a HashBytes, bool>), Error> {
        let mut pruned_branches = Default::default();
        let mut builder = BuilderImpl {
            root: self.root,
            filter: &self.filter,
            cells: Default::default(),
            pruned_branches: Some(&mut pruned_branches),
            finalizer,
        };
        let cell = ok!(builder.build());
        Ok((cell, pruned_branches))
    }
}

struct BuilderImpl<'a, 'b, S = ahash::RandomState> {
    root: &'a DynCell,
    filter: &'b dyn MerkleFilter,
    cells: HashMap<&'a HashBytes, Cell, S>,
    pruned_branches: Option<&'b mut HashMap<&'a HashBytes, bool, S>>,
    finalizer: &'b mut dyn Finalizer,
}

impl<'a, 'b, S> BuilderImpl<'a, 'b, S>
where
    S: BuildHasher + Default,
{
    fn build(&mut self) -> Result<Cell, Error> {
        struct Node<'a> {
            references: RefsIter<'a>,
            descriptor: CellDescriptor,
            merkle_depth: u8,
            children: CellRefsBuilder,
        }

        if self.filter.check(self.root.repr_hash()) == FilterAction::Skip {
            return Err(Error::EmptyProof);
        }

        let mut stack = Vec::with_capacity(self.root.repr_depth() as usize);

        // Push root node
        let root_descriptor = self.root.descriptor();
        stack.push(Node {
            references: self.root.references(),
            descriptor: root_descriptor,
            merkle_depth: root_descriptor.is_merkle() as u8,
            children: CellRefsBuilder::default(),
        });

        while let Some(last) = stack.last_mut() {
            if let Some(child) = last.references.next() {
                // Process children if they are left

                let child_repr_hash = child.repr_hash();
                let child = if let Some(child) = self.cells.get(child_repr_hash) {
                    // Reused processed cells
                    child.clone()
                } else {
                    // Fetch child descriptor
                    let descriptor = child.descriptor();

                    // Check if child is in a tree
                    match self.filter.check(child_repr_hash) {
                        // Included subtrees are used as is
                        FilterAction::IncludeSubtree => {
                            last.references.peek_prev_cloned().expect("mut not fail")
                        }
                        // Replace all skipped subtrees with pruned branch cells
                        FilterAction::Skip if descriptor.reference_count() > 0 => {
                            // Create pruned branch
                            let child = ok!(make_pruned_branch_cold(
                                child,
                                last.merkle_depth,
                                self.finalizer
                            ));

                            // Insert pruned branch for the current cell
                            if let Some(pruned_branch) = &mut self.pruned_branches {
                                pruned_branch.insert(child_repr_hash, false);
                            }

                            // Use new pruned branch as a child
                            child
                        }
                        // All other cells will be included in a different branch
                        _ => {
                            // Add merkle offset to the current merkle depth
                            let merkle_depth = last.merkle_depth + descriptor.is_merkle() as u8;

                            // Push child node and start processing its references
                            stack.push(Node {
                                references: child.references(),
                                descriptor,
                                merkle_depth,
                                children: CellRefsBuilder::default(),
                            });
                            continue;
                        }
                    }
                };

                // Add child to the references builder
                _ = last.children.store_reference(child);
            } else if let Some(last) = stack.pop() {
                // Build a new cell if there are no child nodes left to process

                let cell = last.references.cell();

                // Compute children mask
                let children_mask =
                    last.descriptor.level_mask() | last.children.compute_level_mask();
                let merkle_offset = last.descriptor.is_merkle() as u8;

                // Build the cell
                let mut builder = CellBuilder::new();
                builder.set_exotic(last.descriptor.is_exotic());
                builder.set_level_mask(children_mask.virtualize(merkle_offset));
                _ = builder.store_cell_data(cell);
                builder.set_references(last.children);
                let proof_cell = ok!(builder.build_ext(self.finalizer));

                // Save this cell as processed cell
                self.cells.insert(cell.repr_hash(), proof_cell.clone());

                match stack.last_mut() {
                    // Append this cell to the ancestor
                    Some(last) => {
                        _ = last.children.store_reference(proof_cell);
                    }
                    // Or return it as a result (for the root node)
                    None => return Ok(proof_cell),
                }
            }
        }

        // Something is wrong if we are here
        Err(Error::EmptyProof)
    }
}

#[cold]
fn make_pruned_branch_cold(
    cell: &DynCell,
    merkle_depth: u8,
    finalizer: &mut dyn Finalizer,
) -> Result<Cell, Error> {
    make_pruned_branch(cell, merkle_depth, finalizer)
}

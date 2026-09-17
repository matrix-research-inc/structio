//! An insertion-ordered map with `String` keys.
//!
//! A JSON object is a sequence of members, and the order a document lists them
//! in is part of what it looks like. A sorted map loses that: read a document
//! and write it back and the members come out alphabetised, which makes the
//! result impossible to diff against anything that kept the order, including
//! serde_json and Glaze's own `glz::generic`. [`OrderedMap`] is the map that
//! keeps it. It is the Rust counterpart of Glaze's `glz::ordered_small_map`,
//! built for the same shape of data: many small objects, a few large ones, and
//! lookups that must not pay for the large case in the small one.
//!
//! # Order is the storage
//!
//! Entries live in one contiguous `Vec<(String, V)>` in insertion order, and
//! that vector *is* the map. Iteration, indexing by position, and everything a
//! writer emits come off it directly, with no ordering step and no side table
//! to consult first. [`insert`](OrderedMap::insert) appends; re-inserting an
//! existing key replaces its value where it already sits rather than moving it
//! to the end, so the order a document was read in survives being edited.
//! [`remove`](OrderedMap::remove) shifts the entries above it down, which costs
//! `O(n)` and is the price of not disturbing everything else.
//!
//! # Linear below eight, indexed above
//!
//! Below `LINEAR_MAX` entries there is no index at all: a lookup compares keys
//! one after another, straight down a vector that is already in cache. The
//! threshold is eight because that is where the overwhelming majority of JSON
//! objects live, and because below it the linear scan is not a fallback but the
//! faster answer — hashing a key costs more than the handful of
//! length-then-bytes comparisons it would save, and an index would be a second
//! allocation for a map that fits in one cache line's worth of pointers.
//!
//! Above the threshold a lazily built index of `(hash, position)` pairs, sorted
//! by hash, turns a lookup into a binary search. The hash is 32 bits, so it
//! narrows rather than decides: entries sharing a hash form a run, and a lookup
//! walks that run comparing whole keys. A 32-bit hash is a filter, never an
//! answer, and the index lookup is written so that a collision costs a few
//! extra comparisons and can never return the wrong entry.
//!
//! # The index covers a prefix
//!
//! An index built over `n` entries is invalidated by appending the `n+1`th, and
//! rebuilding on every append would make filling a map quadratic. So, as in
//! Glaze, the index covers only the first `covered` entries; anything appended
//! past that is found by scanning the tail. The tail is bounded:
//! [`insert`](OrderedMap::insert) rebuilds the index whenever the tail would
//! grow past `TAIL_MAX`, so a lookup is at worst a binary search plus eight
//! comparisons, and filling a map pays for a sort once every eight inserts
//! rather than once per insert.
//!
//! Rebuilding happens only in `&mut self` methods. [`get`](OrderedMap::get),
//! [`get_mut`](OrderedMap::get_mut) and
//! [`contains_key`](OrderedMap::contains_key) take `&self` and mutate nothing:
//! there is no interior mutability here, no `Cell`, and no `unsafe`. Glaze can
//! keep its index `mutable` and refresh it from a `const` lookup; the Rust
//! equivalent would be a `RefCell` on the hot path, and a bounded tail scan is
//! cheaper than the borrow flag it would cost. The one thing that grows a map
//! is `insert`, which already holds `&mut self`, so nothing is lost by putting
//! the work there.
//!
//! # Equality ignores order
//!
//! Two maps are equal when they hold the same keys mapped to equal values,
//! whatever order they were built in. This matches `IndexMap` and serde_json
//! under `preserve_order`, and it is what makes an equality test on a document
//! a statement about what the document *means* rather than about how it was
//! assembled. Order is preserved because it is information about the source
//! text; it is not part of the value.
//!
//! [`sort_keys`](OrderedMap::sort_keys) is there for callers who want the
//! sorted behaviour a `BTreeMap` would have given them: it reorders the entries
//! once, explicitly, rather than on every read.

use crate::keymap::full_hash;
use std::iter::FusedIterator;

/// Entry count at or below which no index is built and lookups scan.
///
/// See the module docs: eight is where a scan of a contiguous vector still
/// beats hashing plus a binary search, and it covers the great majority of real
/// JSON objects.
const LINEAR_MAX: usize = 8;

/// How far the uncovered tail may grow before [`OrderedMap::insert`] rebuilds
/// the index.
///
/// This is the whole of the amortisation: rebuilding sorts `n` entries, so
/// doing it every `TAIL_MAX` inserts costs `O(log n)` per insert, while a
/// lookup pays at most this many key comparisons after its binary search. Eight
/// keeps both small; it is deliberately the same size as [`LINEAR_MAX`], since
/// a tail scan is exactly the linear search that a map this size would have
/// used anyway.
const TAIL_MAX: usize = 8;

/// Seed for [`full_hash`]. Any odd constant will do - it is the multiplier in
/// the final `bitmix`, and multiplying by an odd number is a bijection, so no
/// input bits are lost. This is a different constant from the one the
/// compile-time key search starts at, only so that the two are visibly
/// independent choices.
const HASH_SEED: u64 = 0xA076_1D64_78BD_642F;

/// Widest map the index can address, since a slot stores its position as a
/// `u32`.
///
/// A map this large is not a thing any document produces - it would need tens
/// of gigabytes of keys - but the bound has to be stated somewhere, and stating
/// it here means the degenerate case loses its index and keeps its correctness
/// rather than truncating a position.
const MAX_INDEXED: usize = u32::MAX as usize;

/// Hash of a key, narrowed to the 32 bits a slot stores.
///
/// The high half, though the low one would do as well. [`full_hash`] ends in
/// `bitmix`, whose last step is `h ^ h.rotate_right(49)`, and that fold puts
/// high bits into low ones: the two halves measure alike. Over several key
/// shapes of 200,000 to 300,000 keys -- short numeric keys, `user_N`, path-like
/// keys sharing a prefix, long hex keys -- both halves collide within a few of
/// the birthday expectation for a uniform 32-bit hash, and neither is
/// consistently ahead.
///
/// What does cost collisions is [`full_hash`] itself, on one family of keys.
/// [`key_digest`](crate::keymap) folds whole 8-byte chunks and then re-reads
/// the final 8 bytes as its tail, and it mixes in no length, so two keys
/// sharing a 16-byte prefix and an 8-byte suffix digest identically however
/// they differ in between: `field_name_10500_value` and
/// `field_name_105000_value` are one such pair. Those collide in all 64 bits,
/// so no choice of half separates them. It costs a few extra key comparisons
/// here and nothing else, because every candidate is confirmed against the
/// stored key before it is accepted.
#[inline]
fn hash_key(key: &str) -> u32 {
    let bytes = key.as_bytes();
    (full_hash(bytes, bytes.len(), HASH_SEED) >> 32) as u32
}

/// One index slot: where an entry is, and enough of its hash to skip it
/// cheaply.
#[derive(Clone, Copy)]
struct Slot {
    hash: u32,
    pos: u32,
}

/// The sorted hash index over a prefix of the entries.
///
/// Kept as a named type rather than a bare `Vec` in the map so that the things
/// an index might later carry have somewhere to live. Glaze's version carries a
/// bloom filter beside the sorted slots, to answer "this key is certainly new"
/// without a search and so skip the duplicate check that every insert otherwise
/// pays for. It is deliberately left out here: a bloom filter is a cache,
/// trading 128 bytes per map for time, and this repository's rule is that such
/// a trade is justified by measurement rather than by argument. Adding one
/// later means a field here, a line in [`HashIndex::rebuild`], and a test in
/// [`OrderedMap::insert`] - not a change to anything around them.
#[derive(Clone, Default)]
struct HashIndex {
    /// Slots for entries `0..covered`, sorted by hash. Ties are in no
    /// particular order, which is why a lookup walks the whole run.
    slots: Vec<Slot>,
}

impl HashIndex {
    /// How many leading entries the index accounts for. Every slot describes
    /// exactly one covered entry, so the count is the coverage; there is no
    /// second counter to keep in step.
    #[inline]
    fn covered(&self) -> usize {
        self.slots.len()
    }

    #[inline]
    fn clear(&mut self) {
        self.slots.clear();
    }

    /// Re-hash every entry and sort, so that the index covers all of `data`.
    fn rebuild<V>(&mut self, data: &[(String, V)]) {
        self.slots.clear();
        if data.len() > MAX_INDEXED {
            // No position would fit in a slot. Coverage stays at zero and every
            // lookup scans, which is slow and correct.
            return;
        }
        self.slots.reserve(data.len());
        for (pos, (key, _)) in data.iter().enumerate() {
            self.slots.push(Slot {
                hash: hash_key(key),
                pos: pos as u32,
            });
        }
        // Keys are unique, so equal hashes are collisions with nothing to order
        // them by and no reason to keep their relative order.
        self.slots.sort_unstable_by_key(|slot| slot.hash);
    }

    /// Update the index for the entry at `pos` having been removed and
    /// everything above it having shifted down one.
    ///
    /// Only positions move, so the slots stay sorted by hash: dropping one slot
    /// and decrementing the positions above it preserves both the ordering and
    /// the one-slot-per-covered-entry invariant, without a re-sort.
    fn retire(&mut self, pos: usize) {
        if pos >= self.covered() {
            // The removed entry was in the uncovered tail. Coverage is unchanged
            // and no slot refers to a position at or above it.
            return;
        }
        let pos = pos as u32;
        self.slots.retain(|slot| slot.pos != pos);
        for slot in &mut self.slots {
            if slot.pos > pos {
                slot.pos -= 1;
            }
        }
    }

    /// Position of `key` among the covered entries, or `None`.
    ///
    /// The search is a lower bound, so it lands on the *first* slot whose hash
    /// is at least the key's, which is the head of the run of equal hashes when
    /// there is one. Walking forward from there to the end of the run therefore
    /// sees every candidate, and each is confirmed by a full key comparison
    /// before it is accepted. A 32-bit hash collides; this is the only place
    /// that matters, and it is why nothing here trusts a hash on its own.
    fn find<V>(&self, data: &[(String, V)], key: &str) -> Option<usize> {
        if self.slots.is_empty() {
            return None;
        }
        let hash = hash_key(key);
        let start = self.slots.partition_point(|slot| slot.hash < hash);
        for slot in &self.slots[start..] {
            if slot.hash != hash {
                break;
            }
            let pos = slot.pos as usize;
            if data[pos].0 == key {
                return Some(pos);
            }
        }
        None
    }
}

/// A map from `String` to `V` that remembers the order its keys arrived in.
///
/// ```
/// use structio::OrderedMap;
///
/// let mut map = OrderedMap::new();
/// map.insert("zeta".to_string(), 1);
/// map.insert("alpha".to_string(), 2);
/// map.insert("zeta".to_string(), 3); // replaces, keeps position
///
/// assert_eq!(map.get("zeta"), Some(&3));
/// assert_eq!(
///     map.keys().map(String::as_str).collect::<Vec<_>>(),
///     ["zeta", "alpha"]
/// );
/// ```
///
/// See the [module docs](self) for how lookups work and what the ordering does
/// and does not extend to.
pub struct OrderedMap<V> {
    /// The map itself, in insertion order.
    entries: Vec<(String, V)>,
    /// Lookup acceleration for `entries[..index.covered()]`. Empty for a small
    /// map, and never consulted for an answer without a key comparison to
    /// confirm it.
    index: HashIndex,
}

impl<V> OrderedMap<V> {
    /// An empty map. Allocates nothing.
    #[inline]
    pub fn new() -> Self {
        OrderedMap {
            entries: Vec::new(),
            index: HashIndex::default(),
        }
    }

    /// An empty map with room for `capacity` entries.
    ///
    /// The index is not allocated here: a map that stays small never builds
    /// one, and one that grows past the linear-search threshold sizes it from
    /// the entries it actually holds.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        OrderedMap {
            entries: Vec::with_capacity(capacity),
            index: HashIndex::default(),
        }
    }

    /// How many entries the map can hold before it reallocates.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.entries.capacity()
    }

    /// Reserve room for `additional` more entries.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.entries.reserve(additional);
    }

    /// The number of entries.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the map holds nothing.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drop every entry, keeping the allocated capacity.
    #[inline]
    pub fn clear(&mut self) {
        self.entries.clear();
        self.index.clear();
    }

    /// Position of `key`, by whichever route is cheaper for this map's size.
    ///
    /// The index answers for the covered prefix; the tail beyond it is scanned.
    /// The tail is at most [`TAIL_MAX`] long, because that is the bound
    /// [`Self::insert`] maintains, and for a small map coverage is zero and
    /// this is a plain linear search.
    #[inline]
    fn find(&self, key: &str) -> Option<usize> {
        if let Some(pos) = self.index.find(&self.entries, key) {
            return Some(pos);
        }
        let covered = self.index.covered();
        self.entries[covered..]
            .iter()
            .position(|(k, _)| k.as_str() == key)
            .map(|offset| covered + offset)
    }

    /// A reference to the value `key` maps to.
    #[inline]
    pub fn get(&self, key: &str) -> Option<&V> {
        self.find(key).map(|pos| &self.entries[pos].1)
    }

    /// A mutable reference to the value `key` maps to.
    ///
    /// The key itself is never handed out mutably: changing it would move the
    /// entry to a different place in the index without moving the slot that
    /// points at it.
    #[inline]
    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.find(key).map(|pos| &mut self.entries[pos].1)
    }

    /// The stored key and its value.
    ///
    /// The key comes back as `&str` rather than `&String`, as it does from
    /// every other accessor here; only [`Self::iter`] hands out `&String`,
    /// because the writers take their entries that way.
    #[inline]
    pub fn get_key_value(&self, key: &str) -> Option<(&str, &V)> {
        self.find(key).map(|pos| {
            let (k, v) = &self.entries[pos];
            (k.as_str(), v)
        })
    }

    /// Whether `key` is present.
    #[inline]
    pub fn contains_key(&self, key: &str) -> bool {
        self.find(key).is_some()
    }

    /// Insert a key and value, returning the previous value if the key was
    /// already present.
    ///
    /// A duplicate key keeps the position it already had: the map records where
    /// a key first appeared, and writing to it again is not a reason to move
    /// it. This is the one difference from `BTreeMap::insert`, which has no
    /// position to keep.
    pub fn insert(&mut self, key: String, value: V) -> Option<V> {
        match self.find(&key) {
            Some(pos) => Some(std::mem::replace(&mut self.entries[pos].1, value)),
            None => {
                self.push_new(key, value);
                None
            }
        }
    }

    /// Append an entry whose key is known to be absent, and keep the index's
    /// tail within bounds.
    ///
    /// Returns the new entry's position.
    fn push_new(&mut self, key: String, value: V) -> usize {
        self.entries.push((key, value));
        let len = self.entries.len();
        if len > LINEAR_MAX && len - self.index.covered() > TAIL_MAX {
            self.index.rebuild(&self.entries);
        }
        len - 1
    }

    /// Remove `key`, returning its value.
    ///
    /// Entries after it shift down to close the gap, which is `O(n)` and is
    /// what keeps the order intact. A swap with the last entry would be `O(1)`
    /// and would scramble exactly the thing this map exists to preserve.
    #[inline]
    pub fn remove(&mut self, key: &str) -> Option<V> {
        self.remove_entry(key).map(|(_, value)| value)
    }

    /// Remove `key`, returning the stored key along with its value.
    pub fn remove_entry(&mut self, key: &str) -> Option<(String, V)> {
        let pos = self.find(key)?;
        Some(self.remove_at(pos))
    }

    /// Remove the entry at `pos`, shifting the rest down.
    fn remove_at(&mut self, pos: usize) -> (String, V) {
        let removed = self.entries.remove(pos);
        self.index.retire(pos);
        removed
    }

    /// Rebuild or discard the index after the entries have been reordered or
    /// thinned wholesale.
    ///
    /// Used where positions change in ways no slot-by-slot fixup can follow. A
    /// small map simply drops its index and goes back to scanning.
    fn reindex(&mut self) {
        if self.entries.len() > LINEAR_MAX {
            self.index.rebuild(&self.entries);
        } else {
            self.index.clear();
        }
    }

    /// Keep only the entries `f` returns `true` for, in order.
    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&str, &mut V) -> bool,
    {
        let before = self.entries.len();
        self.entries
            .retain_mut(|(key, value)| f(key.as_str(), value));
        if self.entries.len() != before {
            self.reindex();
        }
    }

    /// Reorder the entries by key.
    ///
    /// The escape hatch for a caller who wants a `BTreeMap`'s output: sort
    /// once, deliberately, instead of paying for an ordering on every read. The
    /// order the map was built in is gone afterwards.
    pub fn sort_keys(&mut self) {
        // Keys are unique, so no two entries compare equal and stability buys
        // nothing.
        self.entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        self.reindex();
    }

    /// The key and value at position `index`, in insertion order.
    #[inline]
    pub fn get_index(&self, index: usize) -> Option<(&str, &V)> {
        self.entries.get(index).map(|(k, v)| (k.as_str(), v))
    }

    /// The key and mutable value at position `index`, in insertion order.
    #[inline]
    pub fn get_index_mut(&mut self, index: usize) -> Option<(&str, &mut V)> {
        self.entries
            .get_mut(index)
            .map(|(k, v)| (k.as_str(), &mut *v))
    }

    /// The first entry, or `None` if the map is empty.
    #[inline]
    pub fn first(&self) -> Option<(&str, &V)> {
        self.get_index(0)
    }

    /// The last entry, or `None` if the map is empty.
    #[inline]
    pub fn last(&self) -> Option<(&str, &V)> {
        self.entries.last().map(|(k, v)| (k.as_str(), v))
    }

    /// The entry for `key`, occupied or vacant.
    ///
    /// Takes an owned key because a vacant entry will store it, and looking up
    /// by `&str` first only to allocate the same string afterwards is the cost
    /// this avoids.
    pub fn entry(&mut self, key: String) -> Entry<'_, V> {
        match self.find(&key) {
            Some(pos) => Entry::Occupied(OccupiedEntry { map: self, pos }),
            None => Entry::Vacant(VacantEntry { map: self, key }),
        }
    }

    /// The entries, in insertion order.
    ///
    /// Yields `(&String, &V)` rather than `&(String, V)`, which is the shape
    /// the JSON and BEVE writers take their keyed entries in, so a map goes
    /// straight into one.
    #[inline]
    pub fn iter(&self) -> Iter<'_, V> {
        Iter {
            inner: self.entries.iter(),
        }
    }

    /// The entries with mutable values, in insertion order.
    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_, V> {
        IterMut {
            inner: self.entries.iter_mut(),
        }
    }

    /// The keys, in insertion order.
    #[inline]
    pub fn keys(&self) -> Keys<'_, V> {
        Keys {
            inner: self.entries.iter(),
        }
    }

    /// The values, in insertion order.
    #[inline]
    pub fn values(&self) -> Values<'_, V> {
        Values {
            inner: self.entries.iter(),
        }
    }

    /// The values, mutably, in insertion order.
    #[inline]
    pub fn values_mut(&mut self) -> ValuesMut<'_, V> {
        ValuesMut {
            inner: self.entries.iter_mut(),
        }
    }
}

// ---------------------------------------------------------------------------
// Trait impls
//
// Written out rather than derived, so that the bounds are the ones the operation
// needs: a derived `Default` would demand `V: Default` for an empty map, and a
// derived `Clone` would put a `Clone` bound on the type rather than on the
// method.
// ---------------------------------------------------------------------------

impl<V> Default for OrderedMap<V> {
    #[inline]
    fn default() -> Self {
        OrderedMap::new()
    }
}

impl<V: Clone> Clone for OrderedMap<V> {
    fn clone(&self) -> Self {
        OrderedMap {
            entries: self.entries.clone(),
            // The clone holds the same entries at the same positions, so the
            // index describes it exactly as well as it describes the original.
            index: self.index.clone(),
        }
    }
}

impl<V: core::fmt::Debug> core::fmt::Debug for OrderedMap<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

/// Equality is over the entries, not over their order: see the [module
/// docs](self).
impl<V: PartialEq> PartialEq for OrderedMap<V> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .all(|(key, value)| other.get(key).is_some_and(|theirs| value == theirs))
    }
}

/// Sound because keys are unique: equal lengths plus a value-for-value match in
/// one direction is a bijection, so the relation is symmetric and transitive.
impl<V: Eq> Eq for OrderedMap<V> {}

impl<V> core::ops::Index<&str> for OrderedMap<V> {
    type Output = V;

    /// # Panics
    ///
    /// If the key is not present. Use [`OrderedMap::get`] for a map that may
    /// not have it.
    #[inline]
    fn index(&self, key: &str) -> &V {
        self.get(key)
            .expect("structio: no entry found for key in OrderedMap")
    }
}

impl<V> FromIterator<(String, V)> for OrderedMap<V> {
    fn from_iter<I: IntoIterator<Item = (String, V)>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let mut map = OrderedMap::with_capacity(iter.size_hint().0);
        map.extend(iter);
        map
    }
}

impl<V> Extend<(String, V)> for OrderedMap<V> {
    fn extend<I: IntoIterator<Item = (String, V)>>(&mut self, iter: I) {
        for (key, value) in iter {
            self.insert(key, value);
        }
    }
}

impl<V, const N: usize> From<[(String, V); N]> for OrderedMap<V> {
    fn from(entries: [(String, V); N]) -> Self {
        OrderedMap::from_iter(entries)
    }
}

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

/// A place in the map, whether or not something is there yet.
///
/// Returned by [`OrderedMap::entry`].
pub enum Entry<'a, V> {
    /// The key is present.
    Occupied(OccupiedEntry<'a, V>),
    /// The key is absent, and the entry owns the key that would go there.
    Vacant(VacantEntry<'a, V>),
}

impl<'a, V> Entry<'a, V> {
    /// The key this entry is for.
    pub fn key(&self) -> &str {
        match self {
            Entry::Occupied(e) => e.key(),
            Entry::Vacant(e) => e.key(),
        }
    }

    /// The value, inserting `default` first if the key was absent.
    pub fn or_insert(self, default: V) -> &'a mut V {
        match self {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => e.insert(default),
        }
    }

    /// The value, inserting what `default` returns if the key was absent.
    ///
    /// `default` is not called for a key that is already there.
    pub fn or_insert_with<F: FnOnce() -> V>(self, default: F) -> &'a mut V {
        match self {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => e.insert(default()),
        }
    }

    /// Run `f` on the value if the key is present, then hand the entry back.
    #[must_use]
    pub fn and_modify<F: FnOnce(&mut V)>(self, f: F) -> Self {
        match self {
            Entry::Occupied(mut e) => {
                f(e.get_mut());
                Entry::Occupied(e)
            }
            Entry::Vacant(e) => Entry::Vacant(e),
        }
    }
}

impl<'a, V: Default> Entry<'a, V> {
    /// The value, inserting `V::default()` if the key was absent.
    pub fn or_default(self) -> &'a mut V {
        self.or_insert_with(V::default)
    }
}

/// A key that is already in the map. See [`Entry`].
pub struct OccupiedEntry<'a, V> {
    map: &'a mut OrderedMap<V>,
    pos: usize,
}

impl<'a, V> OccupiedEntry<'a, V> {
    /// The key.
    #[inline]
    pub fn key(&self) -> &str {
        self.map.entries[self.pos].0.as_str()
    }

    /// The value.
    #[inline]
    pub fn get(&self) -> &V {
        &self.map.entries[self.pos].1
    }

    /// The value, mutably.
    #[inline]
    pub fn get_mut(&mut self) -> &mut V {
        &mut self.map.entries[self.pos].1
    }

    /// The value, mutably, for as long as the map is borrowed.
    #[inline]
    pub fn into_mut(self) -> &'a mut V {
        &mut self.map.entries[self.pos].1
    }

    /// Replace the value, returning the old one. The entry keeps its position.
    #[inline]
    pub fn insert(&mut self, value: V) -> V {
        std::mem::replace(self.get_mut(), value)
    }

    /// Remove the entry, returning its value. Order is preserved, as in
    /// [`OrderedMap::remove`].
    #[inline]
    pub fn remove(self) -> V {
        self.remove_entry().1
    }

    /// Remove the entry, returning its key and value.
    #[inline]
    pub fn remove_entry(self) -> (String, V) {
        self.map.remove_at(self.pos)
    }
}

/// A key that is not in the map yet. See [`Entry`].
pub struct VacantEntry<'a, V> {
    map: &'a mut OrderedMap<V>,
    key: String,
}

impl<'a, V> VacantEntry<'a, V> {
    /// The key that would be inserted.
    #[inline]
    pub fn key(&self) -> &str {
        self.key.as_str()
    }

    /// Take the key back out without inserting anything.
    #[inline]
    pub fn into_key(self) -> String {
        self.key
    }

    /// Insert `value` under this entry's key, at the end of the map.
    pub fn insert(self, value: V) -> &'a mut V {
        // The lookup that produced this entry already established the key is
        // absent, so this appends without searching again.
        let pos = self.map.push_new(self.key, value);
        &mut self.map.entries[pos].1
    }
}

// ---------------------------------------------------------------------------
// Iterators
//
// Each is a thin wrapper over the entry vector's own iterator, so they are all
// exact-sized, double-ended and fused for free, and iteration is a walk down
// contiguous memory rather than a tree traversal.
// ---------------------------------------------------------------------------

/// Iterator over a map's entries, in insertion order. See [`OrderedMap::iter`].
pub struct Iter<'a, V> {
    inner: core::slice::Iter<'a, (String, V)>,
}

impl<V> Clone for Iter<'_, V> {
    fn clone(&self) -> Self {
        Iter {
            inner: self.inner.clone(),
        }
    }
}

impl<'a, V> Iterator for Iter<'a, V> {
    type Item = (&'a String, &'a V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(k, v)| (k, v))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<V> DoubleEndedIterator for Iter<'_, V> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(k, v)| (k, v))
    }
}

impl<V> ExactSizeIterator for Iter<'_, V> {
    #[inline]
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<V> FusedIterator for Iter<'_, V> {}

/// Iterator over a map's entries with mutable values. See
/// [`OrderedMap::iter_mut`].
pub struct IterMut<'a, V> {
    inner: core::slice::IterMut<'a, (String, V)>,
}

impl<'a, V> Iterator for IterMut<'a, V> {
    type Item = (&'a String, &'a mut V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        // The key is reborrowed immutably: an entry whose key changed would sit
        // in a place the index no longer describes.
        self.inner.next().map(|(k, v)| (&*k, v))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<V> DoubleEndedIterator for IterMut<'_, V> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(k, v)| (&*k, v))
    }
}

impl<V> ExactSizeIterator for IterMut<'_, V> {
    #[inline]
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<V> FusedIterator for IterMut<'_, V> {}

/// Owning iterator over a map's entries. See [`OrderedMap::into_iter`].
pub struct IntoIter<V> {
    inner: std::vec::IntoIter<(String, V)>,
}

impl<V> Iterator for IntoIter<V> {
    type Item = (String, V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<V> DoubleEndedIterator for IntoIter<V> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back()
    }
}

impl<V> ExactSizeIterator for IntoIter<V> {
    #[inline]
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<V> FusedIterator for IntoIter<V> {}

/// Iterator over a map's keys. See [`OrderedMap::keys`].
pub struct Keys<'a, V> {
    inner: core::slice::Iter<'a, (String, V)>,
}

impl<V> Clone for Keys<'_, V> {
    fn clone(&self) -> Self {
        Keys {
            inner: self.inner.clone(),
        }
    }
}

impl<'a, V> Iterator for Keys<'a, V> {
    type Item = &'a String;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(k, _)| k)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<V> DoubleEndedIterator for Keys<'_, V> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(k, _)| k)
    }
}

impl<V> ExactSizeIterator for Keys<'_, V> {
    #[inline]
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<V> FusedIterator for Keys<'_, V> {}

/// Iterator over a map's values. See [`OrderedMap::values`].
pub struct Values<'a, V> {
    inner: core::slice::Iter<'a, (String, V)>,
}

impl<V> Clone for Values<'_, V> {
    fn clone(&self) -> Self {
        Values {
            inner: self.inner.clone(),
        }
    }
}

impl<'a, V> Iterator for Values<'a, V> {
    type Item = &'a V;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(_, v)| v)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<V> DoubleEndedIterator for Values<'_, V> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(_, v)| v)
    }
}

impl<V> ExactSizeIterator for Values<'_, V> {
    #[inline]
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<V> FusedIterator for Values<'_, V> {}

/// Iterator over a map's values, mutably. See [`OrderedMap::values_mut`].
pub struct ValuesMut<'a, V> {
    inner: core::slice::IterMut<'a, (String, V)>,
}

impl<'a, V> Iterator for ValuesMut<'a, V> {
    type Item = &'a mut V;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(_, v)| v)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<V> DoubleEndedIterator for ValuesMut<'_, V> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(_, v)| v)
    }
}

impl<V> ExactSizeIterator for ValuesMut<'_, V> {
    #[inline]
    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl<V> FusedIterator for ValuesMut<'_, V> {}

impl<V> IntoIterator for OrderedMap<V> {
    type Item = (String, V);
    type IntoIter = IntoIter<V>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            inner: self.entries.into_iter(),
        }
    }
}

impl<'a, V> IntoIterator for &'a OrderedMap<V> {
    type Item = (&'a String, &'a V);
    type IntoIter = Iter<'a, V>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, V> IntoIterator for &'a mut OrderedMap<V> {
    type Item = (&'a String, &'a mut V);
    type IntoIter = IterMut<'a, V>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Distinct keys from a counter, cheaply. `format!` would do, but the
    /// collision search below wants hundreds of thousands of them.
    fn nth_key(n: usize) -> String {
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
        let mut n = n;
        let mut key = String::from("key_");
        loop {
            key.push(ALPHABET[n % ALPHABET.len()] as char);
            n /= ALPHABET.len();
            if n == 0 {
                break;
            }
        }
        key
    }

    fn map_of(n: usize) -> OrderedMap<usize> {
        let mut map = OrderedMap::new();
        for i in 0..n {
            map.insert(nth_key(i), i);
        }
        map
    }

    /// Every invariant the lookup depends on, checked against the entries.
    fn assert_index_consistent<V>(map: &OrderedMap<V>) {
        let covered = map.index.covered();
        assert!(covered <= map.len(), "index covers more than the map holds");
        for (slot_pos, slot) in map.index.slots.iter().enumerate() {
            let pos = slot.pos as usize;
            assert!(pos < covered, "slot {slot_pos} points outside the prefix");
            assert_eq!(
                slot.hash,
                hash_key(&map.entries[pos].0),
                "slot {slot_pos} holds a stale hash"
            );
            if slot_pos > 0 {
                assert!(
                    map.index.slots[slot_pos - 1].hash <= slot.hash,
                    "index is not sorted by hash"
                );
            }
        }
        // Exactly one slot per covered entry.
        let mut seen = vec![false; covered];
        for slot in &map.index.slots {
            assert!(
                !std::mem::replace(&mut seen[slot.pos as usize], true),
                "two slots point at the same entry"
            );
        }
        // The tail a lookup has to scan stays bounded.
        if map.len() > LINEAR_MAX && map.len() <= MAX_INDEXED {
            assert!(
                map.len() - covered <= TAIL_MAX,
                "uncovered tail grew past its bound"
            );
        }
    }

    #[test]
    fn empty_map_answers_nothing() {
        let map: OrderedMap<i32> = OrderedMap::new();
        assert!(map.is_empty());
        assert_eq!(map.len(), 0);
        assert_eq!(map.get("a"), None);
        assert!(!map.contains_key("a"));
        assert_eq!(map.first(), None);
        assert_eq!(map.last(), None);
        assert_eq!(map.iter().next(), None);
    }

    #[test]
    fn insertion_order_survives_every_size() {
        for n in [0usize, 1, 7, 8, 9, 63, 64, 65, 300, 1000] {
            let map = map_of(n);
            assert_eq!(map.len(), n);
            assert_index_consistent(&map);

            let expected: Vec<String> = (0..n).map(nth_key).collect();
            let actual: Vec<String> = map.keys().cloned().collect();
            assert_eq!(actual, expected, "order wrong at n = {n}");

            for i in 0..n {
                assert_eq!(map.get(&nth_key(i)), Some(&i), "missing key at n = {n}");
                assert_eq!(map.get_index(i), Some((nth_key(i).as_str(), &i)));
            }
            assert_eq!(map.get("key_not_present"), None, "phantom key at n = {n}");
            assert_eq!(map.get(""), None);
        }
    }

    #[test]
    fn duplicate_insert_replaces_in_place() {
        for n in [1usize, 8, 9, 40, 200] {
            let mut map = map_of(n);
            let mut targets = vec![0usize, n / 2, n - 1];
            targets.dedup();
            for target in targets {
                let key = nth_key(target);
                let old = map.insert(key.clone(), 10_000 + target);
                assert_eq!(old, Some(target), "old value not returned at n = {n}");
                assert_eq!(map.len(), n, "duplicate insert changed the length");
                assert_eq!(map.get(&key), Some(&(10_000 + target)));
                assert_eq!(
                    map.get_index(target).map(|(k, _)| k.to_string()),
                    Some(key),
                    "duplicate insert moved the entry"
                );
            }
            assert_index_consistent(&map);
        }
    }

    // --- engineered hash collisions ----------------------------------------

    /// Pairs of distinct keys whose 32-bit hashes are equal.
    ///
    /// Found rather than hard-coded: a hard-coded pair would silently stop
    /// being a collision if the seed or the hash ever changed, and the test
    /// would go on passing while testing nothing.
    fn colliding_pairs(wanted: usize) -> Vec<(String, String)> {
        let mut first_seen: std::collections::HashMap<u32, usize> =
            std::collections::HashMap::new();
        let mut pairs = Vec::new();
        for i in 0..2_000_000usize {
            let key = nth_key(i);
            let hash = hash_key(&key);
            match first_seen.get(&hash) {
                Some(&j) => {
                    pairs.push((nth_key(j), key));
                    if pairs.len() == wanted {
                        return pairs;
                    }
                }
                None => {
                    first_seen.insert(hash, i);
                }
            }
        }
        panic!("no 32-bit hash collision found; the search bound is too low");
    }

    #[test]
    fn colliding_keys_are_all_found_and_no_others() {
        let pairs = colliding_pairs(3);
        for (a, b) in &pairs {
            assert_ne!(a, b);
            assert_eq!(hash_key(a), hash_key(b), "test setup is not a collision");
        }

        // Below the threshold, where lookup is a plain scan.
        let mut small: OrderedMap<&str> = OrderedMap::new();
        let (a, b) = &pairs[0];
        small.insert(a.clone(), "a");
        assert_eq!(small.get(a), Some(&"a"));
        assert_eq!(
            small.get(b),
            None,
            "an absent key with a colliding hash was found"
        );
        small.insert(b.clone(), "b");
        assert_eq!(small.get(a), Some(&"a"));
        assert_eq!(small.get(b), Some(&"b"));

        // Above it, where the run of equal hashes has to be walked. The
        // colliding keys go in first so that they land in the covered prefix
        // rather than in the tail the lookup scans anyway.
        let mut big: OrderedMap<String> = OrderedMap::new();
        for (a, _) in &pairs {
            big.insert(a.clone(), format!("{a}!"));
        }
        for i in 0..200 {
            big.insert(nth_key(1_000_000 + i), i.to_string());
        }
        assert!(
            big.index.covered() > pairs.len(),
            "the indexed path was never exercised"
        );
        assert_index_consistent(&big);

        for (a, b) in &pairs {
            assert_eq!(big.get(a), Some(&format!("{a}!")));
            assert_eq!(big.get(b), None, "absent colliding key was found");
        }
        // Now with both halves of each pair present.
        for (_, b) in &pairs {
            big.insert(b.clone(), format!("{b}?"));
        }
        for _ in 0..20 {
            // Force a rebuild so the collisions are indexed rather than tailed.
            big.insert(nth_key(2_000_000 + big.len()), String::new());
        }
        assert_index_consistent(&big);
        for (a, b) in &pairs {
            assert_eq!(big.get(a), Some(&format!("{a}!")));
            assert_eq!(big.get(b), Some(&format!("{b}?")));
        }
    }

    #[test]
    fn colliding_keys_survive_removal() {
        let pairs = colliding_pairs(2);
        let mut map: OrderedMap<String> = OrderedMap::new();
        for (a, b) in &pairs {
            map.insert(a.clone(), format!("{a}!"));
            map.insert(b.clone(), format!("{b}?"));
        }
        for i in 0..100 {
            map.insert(nth_key(3_000_000 + i), i.to_string());
        }
        for (a, b) in &pairs {
            assert_eq!(map.remove(a), Some(format!("{a}!")));
            assert_eq!(map.get(a), None);
            assert_eq!(
                map.get(b),
                Some(&format!("{b}?")),
                "removing one of a colliding pair lost the other"
            );
            assert_index_consistent(&map);
        }
    }

    // --- removal ------------------------------------------------------------

    #[test]
    fn remove_preserves_order() {
        for n in [1usize, 8, 9, 40, 128] {
            for &target in &[0usize, n / 2, n - 1] {
                let mut map = map_of(n);
                let key = nth_key(target);
                assert_eq!(map.remove(&key), Some(target));
                assert_eq!(map.remove(&key), None, "removed twice");
                assert_eq!(map.len(), n - 1);
                assert_index_consistent(&map);

                let expected: Vec<String> = (0..n).filter(|&i| i != target).map(nth_key).collect();
                assert_eq!(map.keys().cloned().collect::<Vec<_>>(), expected);

                for i in 0..n {
                    let found = map.get(&nth_key(i));
                    if i == target {
                        assert_eq!(found, None, "n = {n}, removed = {target}");
                    } else {
                        assert_eq!(found, Some(&i), "n = {n}, removed = {target}");
                    }
                }
            }
        }
    }

    #[test]
    fn remove_everything_one_at_a_time() {
        let mut map = map_of(100);
        for i in (0..100).rev() {
            assert_eq!(map.remove_entry(&nth_key(i)), Some((nth_key(i), i)));
            assert_index_consistent(&map);
            assert_eq!(map.len(), i);
            for j in 0..i {
                assert_eq!(map.get(&nth_key(j)), Some(&j));
            }
        }
        assert!(map.is_empty());
    }

    #[test]
    fn remove_from_the_front_repeatedly() {
        let mut map = map_of(60);
        for i in 0..60 {
            assert_eq!(map.remove(&nth_key(i)), Some(i));
            assert_index_consistent(&map);
            assert_eq!(map.first().map(|(k, _)| k.to_string()), {
                if i + 1 < 60 {
                    Some(nth_key(i + 1))
                } else {
                    None
                }
            });
        }
    }

    #[test]
    fn clear_forgets_everything() {
        let mut map = map_of(50);
        map.clear();
        assert!(map.is_empty());
        assert_eq!(map.get(&nth_key(0)), None);
        assert_index_consistent(&map);
        map.insert("a".into(), 1);
        assert_eq!(map.get("a"), Some(&1));
        assert_eq!(map.len(), 1);
    }

    // --- differential against BTreeMap --------------------------------------

    /// SplitMix64, so the script is the same on every run and on every machine.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    #[test]
    fn matches_btreemap_over_a_long_script() {
        let mut rng = Rng(0x5EED_1234_ABCD_0001);
        let mut map: OrderedMap<u64> = OrderedMap::new();
        let mut reference: BTreeMap<String, u64> = BTreeMap::new();
        // The order the map claims to keep, maintained independently.
        let mut order: Vec<String> = Vec::new();

        // A small key space, so collisions of *keys* (not just hashes) are
        // frequent and duplicate inserts and misses are both common.
        const KEY_SPACE: u64 = 400;

        for step in 0..20_000u32 {
            let key = nth_key(rng.below(KEY_SPACE) as usize);
            match rng.below(100) {
                0..=54 => {
                    let value = rng.next();
                    let ours = map.insert(key.clone(), value);
                    let theirs = reference.insert(key.clone(), value);
                    assert_eq!(ours, theirs, "step {step}: insert returned differently");
                    if theirs.is_none() {
                        order.push(key);
                    }
                }
                55..=74 => {
                    assert_eq!(
                        map.get(&key),
                        reference.get(&key),
                        "step {step}: get disagreed"
                    );
                }
                75..=84 => {
                    assert_eq!(
                        map.contains_key(&key),
                        reference.contains_key(&key),
                        "step {step}: contains_key disagreed"
                    );
                }
                85..=97 => {
                    let ours = map.remove(&key);
                    let theirs = reference.remove(&key);
                    assert_eq!(ours, theirs, "step {step}: remove returned differently");
                    if theirs.is_some() {
                        let at = order.iter().position(|k| *k == key).unwrap();
                        order.remove(at);
                    }
                }
                _ => {
                    // Occasionally reach in through `entry`, which takes a
                    // different path to the same places.
                    let value = rng.next();
                    let ours = *map.entry(key.clone()).or_insert(value);
                    let theirs = *reference.entry(key.clone()).or_insert(value);
                    assert_eq!(ours, theirs, "step {step}: entry disagreed");
                    if ours == value && theirs == value && !order.contains(&key) {
                        order.push(key);
                    }
                }
            }

            assert_eq!(map.len(), reference.len(), "step {step}: length disagreed");

            if step % 97 == 0 {
                assert_index_consistent(&map);
                // Same keys, same values, whatever the order.
                let mut ours: Vec<(String, u64)> =
                    map.iter().map(|(k, v)| (k.clone(), *v)).collect();
                ours.sort();
                let theirs: Vec<(String, u64)> =
                    reference.iter().map(|(k, v)| (k.clone(), *v)).collect();
                assert_eq!(ours, theirs, "step {step}: contents disagreed");
                // And the order is the one the script produced.
                assert_eq!(
                    map.keys().cloned().collect::<Vec<_>>(),
                    order,
                    "step {step}: order disagreed"
                );
            }
        }

        assert!(!map.is_empty(), "the script never exercised a filled map");
    }

    // --- entry --------------------------------------------------------------

    #[test]
    fn entry_vacant_and_occupied() {
        let mut map: OrderedMap<Vec<u32>> = OrderedMap::new();

        map.entry("a".into()).or_default().push(1);
        map.entry("a".into()).or_default().push(2);
        assert_eq!(map.get("a"), Some(&vec![1, 2]));
        assert_eq!(map.len(), 1);

        *map.entry("b".into()).or_insert(vec![9]) = vec![8];
        assert_eq!(map.get("b"), Some(&vec![8]));

        let mut calls = 0;
        map.entry("b".into()).or_insert_with(|| {
            calls += 1;
            vec![7]
        });
        assert_eq!(calls, 0, "or_insert_with ran for an occupied entry");
        map.entry("c".into()).or_insert_with(|| {
            calls += 1;
            vec![7]
        });
        assert_eq!(calls, 1);

        map.entry("c".into()).and_modify(|v| v.push(6)).or_default();
        assert_eq!(map.get("c"), Some(&vec![7, 6]));
        map.entry("d".into()).and_modify(|v| v.push(5)).or_default();
        assert_eq!(map.get("d"), Some(&vec![]));

        assert_eq!(
            map.keys().map(String::as_str).collect::<Vec<_>>(),
            ["a", "b", "c", "d"]
        );

        match map.entry("a".into()) {
            Entry::Occupied(mut e) => {
                assert_eq!(e.key(), "a");
                assert_eq!(e.get(), &vec![1, 2]);
                e.get_mut().push(3);
                assert_eq!(e.insert(vec![0]), vec![1, 2, 3]);
            }
            Entry::Vacant(_) => panic!("a is present"),
        }
        assert_eq!(map.get("a"), Some(&vec![0]));
        assert_eq!(map.first().map(|(k, _)| k), Some("a"), "entry moved a key");

        match map.entry("zz".into()) {
            Entry::Vacant(e) => {
                assert_eq!(e.key(), "zz");
                assert_eq!(e.into_key(), "zz");
            }
            Entry::Occupied(_) => panic!("zz is absent"),
        }
        assert_eq!(map.len(), 4, "into_key inserted something");
    }

    #[test]
    fn entry_removal_keeps_order() {
        let mut map = map_of(30);
        match map.entry(nth_key(10)) {
            Entry::Occupied(e) => assert_eq!(e.remove(), 10),
            Entry::Vacant(_) => panic!("present"),
        }
        assert_index_consistent(&map);
        assert_eq!(map.len(), 29);
        assert_eq!(map.get(&nth_key(10)), None);
        assert_eq!(map.get_index(10), Some((nth_key(11).as_str(), &11)));
    }

    #[test]
    fn entry_stays_indexed_above_the_threshold() {
        let mut map = map_of(100);
        assert!(map.index.covered() > 0);
        for i in 0..100 {
            assert_eq!(*map.entry(nth_key(i)).or_insert(usize::MAX), i);
        }
        assert_eq!(map.len(), 100);
        assert_eq!(*map.entry(nth_key(500)).or_insert(7), 7);
        assert_eq!(map.len(), 101);
        assert_index_consistent(&map);
    }

    // --- equality -----------------------------------------------------------

    #[test]
    fn equality_ignores_order() {
        let mut forwards: OrderedMap<i32> = OrderedMap::new();
        let mut backwards: OrderedMap<i32> = OrderedMap::new();
        for i in 0..40 {
            forwards.insert(nth_key(i), i as i32);
        }
        for i in (0..40).rev() {
            backwards.insert(nth_key(i), i as i32);
        }
        assert_ne!(
            forwards.keys().collect::<Vec<_>>(),
            backwards.keys().collect::<Vec<_>>(),
            "the two maps are supposed to differ in order"
        );
        assert_eq!(forwards, backwards);

        let mut differing = backwards.clone();
        differing.insert(nth_key(7), -1);
        assert_ne!(forwards, differing);

        let mut shorter = forwards.clone();
        shorter.remove(&nth_key(39));
        assert_ne!(forwards, shorter);
        assert_ne!(shorter, forwards);

        let empty: OrderedMap<i32> = OrderedMap::new();
        assert_eq!(empty, OrderedMap::new());
        assert_ne!(empty, forwards);

        // Same length, disjoint keys.
        let mut other: OrderedMap<i32> = OrderedMap::new();
        for i in 100..140 {
            other.insert(nth_key(i), i as i32);
        }
        assert_ne!(forwards, other);
    }

    #[test]
    fn clone_keeps_order_and_lookups() {
        let map = map_of(70);
        let copy = map.clone();
        assert_eq!(map, copy);
        assert_eq!(
            map.keys().collect::<Vec<_>>(),
            copy.keys().collect::<Vec<_>>()
        );
        for i in 0..70 {
            assert_eq!(copy.get(&nth_key(i)), Some(&i));
        }
        assert_index_consistent(&copy);
    }

    // --- ordering utilities -------------------------------------------------

    #[test]
    fn sort_keys_reorders_and_lookups_still_work() {
        for n in [0usize, 1, 8, 9, 65, 300] {
            let mut map: OrderedMap<usize> = OrderedMap::new();
            for i in (0..n).rev() {
                map.insert(nth_key(i), i);
            }
            map.sort_keys();
            assert_index_consistent(&map);

            let mut expected: Vec<String> = (0..n).map(nth_key).collect();
            expected.sort();
            assert_eq!(map.keys().cloned().collect::<Vec<_>>(), expected);

            for i in 0..n {
                assert_eq!(map.get(&nth_key(i)), Some(&i), "lost a key at n = {n}");
            }
            assert_eq!(
                map.first().map(|(k, _)| k.to_string()),
                expected.first().cloned()
            );
            assert_eq!(
                map.last().map(|(k, _)| k.to_string()),
                expected.last().cloned()
            );
        }
    }

    #[test]
    fn positional_accessors() {
        let mut map = map_of(20);
        assert_eq!(map.first(), Some((nth_key(0).as_str(), &0)));
        assert_eq!(map.last(), Some((nth_key(19).as_str(), &19)));
        assert_eq!(map.get_index(20), None);
        assert_eq!(map.get_index_mut(20).map(|(_, v)| *v), None);

        if let Some((key, value)) = map.get_index_mut(5) {
            assert_eq!(key, nth_key(5));
            *value = 500;
        }
        assert_eq!(map.get(&nth_key(5)), Some(&500));
    }

    #[test]
    fn retain_keeps_order_and_rebuilds() {
        let mut map = map_of(100);
        map.retain(|_, value| *value % 3 == 0);
        assert_index_consistent(&map);

        let expected: Vec<String> = (0..100).filter(|i| i % 3 == 0).map(nth_key).collect();
        assert_eq!(map.keys().cloned().collect::<Vec<_>>(), expected);
        for i in 0..100 {
            let found = map.get(&nth_key(i));
            assert_eq!(found, if i % 3 == 0 { Some(&i) } else { None });
        }

        // Down to under the threshold, where the index goes away entirely.
        map.retain(|key, _| key == nth_key(0));
        assert_eq!(map.len(), 1);
        assert_eq!(map.index.covered(), 0);
        assert_eq!(map.get(&nth_key(0)), Some(&0));
    }

    // --- iteration and conversions ------------------------------------------

    #[test]
    fn iterators_agree_on_order() {
        let mut map = map_of(30);
        let keys: Vec<String> = (0..30).map(nth_key).collect();

        assert_eq!(map.iter().len(), 30);
        assert_eq!(map.keys().cloned().collect::<Vec<_>>(), keys);
        assert_eq!(
            map.values().copied().collect::<Vec<_>>(),
            (0..30).collect::<Vec<_>>()
        );
        assert_eq!(
            map.iter().rev().map(|(k, _)| k.clone()).collect::<Vec<_>>(),
            keys.iter().rev().cloned().collect::<Vec<_>>()
        );

        for (i, (key, value)) in (&mut map).into_iter().enumerate() {
            assert_eq!(key, &keys[i]);
            *value += 1000;
        }
        for value in map.values_mut() {
            *value += 1;
        }
        assert_eq!(map.get(&keys[0]), Some(&1001));

        let owned: Vec<(String, usize)> = map.clone().into_iter().collect();
        assert_eq!(owned.len(), 30);
        assert_eq!(owned[0].0, keys[0]);

        let rebuilt: OrderedMap<usize> = owned.into_iter().collect();
        assert_eq!(rebuilt, map);
        assert_eq!(rebuilt.keys().cloned().collect::<Vec<_>>(), keys);
        assert_index_consistent(&rebuilt);
    }

    #[test]
    fn from_array_and_extend() {
        let mut map = OrderedMap::from([
            ("b".to_string(), 2),
            ("a".to_string(), 1),
            ("b".to_string(), 3),
        ]);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("b"), Some(&3));
        assert_eq!(
            map.keys().map(String::as_str).collect::<Vec<_>>(),
            ["b", "a"]
        );

        map.extend([("c".to_string(), 4), ("a".to_string(), 5)]);
        assert_eq!(
            map.keys().map(String::as_str).collect::<Vec<_>>(),
            ["b", "a", "c"]
        );
        assert_eq!(map["a"], 5);
        assert_eq!(map["c"], 4);
    }

    #[test]
    #[should_panic(expected = "no entry found for key")]
    fn index_panics_on_a_missing_key() {
        let map = map_of(3);
        let _ = map["nope"];
    }

    #[test]
    fn debug_prints_as_a_map_in_order() {
        let map = OrderedMap::from([("z".to_string(), 1), ("a".to_string(), 2)]);
        assert_eq!(format!("{map:?}"), r#"{"z": 1, "a": 2}"#);
    }

    #[test]
    fn capacity_is_honoured() {
        let mut map: OrderedMap<u8> = OrderedMap::with_capacity(32);
        assert!(map.capacity() >= 32);
        map.reserve(100);
        assert!(map.capacity() >= 100);
        assert!(map.is_empty());
    }

    #[test]
    fn values_of_a_type_that_is_neither_clone_nor_default() {
        struct NotCloneable(u8);
        let mut map: OrderedMap<NotCloneable> = OrderedMap::default();
        map.insert("x".into(), NotCloneable(1));
        assert_eq!(map.get("x").map(|v| v.0), Some(1));
    }
}

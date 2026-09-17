//! An insertion-ordered map with `String` keys.
//!
//! A JSON object is a sequence of members, and the order a document lists them
//! in is part of what it looks like. A sorted map loses that: read a document
//! and write it back and the members come out alphabetised, which makes the
//! result impossible to diff against anything that kept the order, including
//! serde_json and Glaze's own `glz::generic`. [`OrderedMap`] is the map that
//! keeps it. It is the Rust counterpart of Glaze's `glz::ordered_map`, built
//! for the same shape of data: many small objects, a few large ones, and
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
//! `O(n)` and is the price of not disturbing everything else; the bucket table
//! records positions, so the same removal walks it once to decrement the
//! positions above the hole.
//!
//! # Linear below eight, hashed above
//!
//! Below `LINEAR_MAX` entries there is no index at all: a lookup compares keys
//! one after another, straight down a vector that is already in cache. The
//! threshold is eight because that is where the overwhelming majority of JSON
//! objects live, and because below it the linear scan is not a fallback but the
//! faster answer: hashing a key costs more than the handful of
//! length-then-bytes comparisons it would save, and an index would be a second
//! allocation for a map that fits in one cache line's worth of pointers.
//!
//! The ninth entry allocates a `HashTable`: a power-of-two array of buckets,
//! each holding the position of an entry and 32 bits of that entry's hash,
//! addressed by the low bits of the hash and resolved by robin hood probing
//! over open addressing. This is the structure Glaze's `glz::ordered_map` uses.
//! A bucket is eight bytes and holds no key, so a probe walks a dense array and
//! rejects a wrong bucket on a 32-bit compare without touching the entries at
//! all. The stored hash is a filter, never an answer: a bucket whose hash
//! matches is still confirmed against the stored key before its position is
//! returned, so a collision costs one comparison and can never produce the
//! wrong entry.
//!
//! Filling the table is linear. An insert places a single bucket, and the table
//! is rebuilt only when the entries outgrow three quarters of it, which doubles
//! the buckets and so costs `O(1)` per insert amortised.
//!
//! # Robin hood, and why a miss costs what a hit does
//!
//! A bucket's *distance* is how far it sits from the bucket its hash asked for.
//! Probing walks forward from that ideal bucket, and on the way an insert swaps
//! itself with any occupant closer to home than the probe has already
//! travelled, taking from the rich and carrying the displaced entry onward. The
//! effect is that distances along a probe chain stay in the order the probe
//! visits them, which gives a lookup a second way to stop: reaching an occupant
//! whose own distance is shorter than the distance already walked proves the
//! key is absent, because an insert would have stolen that bucket. So a miss
//! ends on the same short walk a hit does, instead of running to the next empty
//! bucket. Deletion has to preserve that ordering, which is why removing an
//! entry shifts the rest of its chain back one bucket rather than leaving a
//! tombstone behind: see `HashTable::erase`.
//!
//! # Why not a sorted index
//!
//! The obvious index for a map that is filled once and then read is an array of
//! `(hash, position)` sorted by hash and binary-searched, which is what
//! `glz::ordered_small_map` carries and what this module held until it was
//! measured. Glaze can afford it because it builds the array *lazily*, on the
//! first lookup after an insert, through a `mutable` member reached from a
//! `const` method. Rust has no such route: `get(&self)` cannot rebuild
//! anything, and the ways to make it able to, interior mutability on the hot
//! path or a `&mut self` lookup that would spread through every caller, cost
//! more than they are worth here. That leaves rebuilding from `insert`, where a
//! sort per insert is `O(n log n)` per insert. Sorting only every eighth insert
//! and scanning the unsorted tail divides that by eight without changing what
//! it is: filling `n` entries stayed quadratic, and at a thousand keys this map
//! cost eight times what a `BTreeMap` did. The same structure that is linear to
//! fill in C++ is quadratic here, and the whole of the difference is the lazy
//! rebuild.
//!
//! An open-addressed table has no such dependency. The insert that invalidates
//! it also repairs it, one bucket at a time, so a lookup never has anything to
//! fix up and never needs to mutate: [`get`](OrderedMap::get),
//! [`get_mut`](OrderedMap::get_mut) and
//! [`contains_key`](OrderedMap::contains_key) read and nothing more. There is
//! no interior mutability here, no `Cell`, and no `unsafe`.
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

/// Entry count at or below which no table is built and lookups scan.
///
/// See the module docs: eight is where a scan of a contiguous vector still
/// beats hashing plus a probe, and it covers the great majority of real JSON
/// objects.
const LINEAR_MAX: usize = 8;

/// The smallest bucket table, which is the one a map gets the moment it
/// outgrows [`LINEAR_MAX`].
///
/// Sixteen rather than eight because nine entries is where the table first
/// appears, and nine will not fit in eight buckets at all, let alone under the
/// load factor. Nine in sixteen is a load of 0.56, so the first table has room
/// to take a few more entries before it doubles.
const MIN_BUCKETS: usize = 16;

/// How full the bucket table is allowed to get, as a count rather than a
/// fraction.
///
/// Three quarters, the same as Glaze's `max_load_factor` and as `std`'s. Robin
/// hood probing degrades with load rather than falling off a cliff, so the
/// number trades memory for probe length smoothly; it is not tuned here because
/// nothing measured has asked it to move. The bucket count is a power of two,
/// so the division is exact and the limit is an integer.
#[inline]
const fn load_limit(buckets: usize) -> usize {
    buckets / 4 * 3
}

/// Buckets enough to hold `entries` under [`load_limit`], never fewer than
/// [`MIN_BUCKETS`].
///
/// Doubling from the minimum rather than computing the power of two directly:
/// the loop runs at most as many times as the table has doublings in it, it is
/// only ever reached from a rebuild that is already `O(n)`, and it cannot
/// overflow the way `n * 4 / 3` rounded up can.
fn buckets_for(entries: usize) -> usize {
    let mut buckets = MIN_BUCKETS;
    while entries > load_limit(buckets) {
        buckets *= 2;
    }
    buckets
}

/// Seed for [`full_hash`]. Any odd constant will do - it is the multiplier in
/// the final `bitmix`, and multiplying by an odd number is a bijection, so no
/// input bits are lost. This is a different constant from the one the
/// compile-time key search starts at, only so that the two are visibly
/// independent choices.
const HASH_SEED: u64 = 0xA076_1D64_78BD_642F;

/// Widest map the table can address, since a bucket stores its entry's
/// position as a `u32`.
///
/// A map this large is not a thing any document produces - it would need tens
/// of gigabytes of keys - but the bound has to be stated somewhere, and stating
/// it here means the degenerate case loses its table and keeps its correctness
/// rather than truncating a position. A map past this point has no table at
/// all: every lookup scans the entries, which is slow and right, and it gets
/// its table back the moment removals bring it under the bound. The last
/// position such a map holds is `u32::MAX - 1`, one below [`EMPTY`], so no
/// entry can be mistaken for an empty bucket.
const MAX_INDEXED: usize = u32::MAX as usize;

/// Hash of a key, narrowed to the 32 bits a bucket stores.
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

/// Marks a bucket that holds nothing.
///
/// `u32::MAX` rather than a flag byte beside the position: it keeps a bucket to
/// eight bytes and two aligned loads, and [`MAX_INDEXED`] is chosen so that no
/// live position ever reaches it.
const EMPTY: u32 = u32::MAX;

/// One bucket: which entry lives here, and enough of its hash to reject it
/// without reading the entry.
///
/// Eight bytes, so a cache line holds eight of them and a probe that has to
/// walk usually walks within one.
#[derive(Clone, Copy)]
struct Bucket {
    /// Position of the entry in the map's vector, or [`EMPTY`].
    index: u32,
    /// [`hash_key`] of that entry's key. Meaningless when the bucket is empty.
    hash: u32,
}

impl Bucket {
    /// A bucket holding nothing. The hash is zero only because it has to be
    /// something; nothing reads it without checking [`Bucket::is_empty`] first.
    const VACANT: Bucket = Bucket {
        index: EMPTY,
        hash: 0,
    };

    #[inline]
    fn is_empty(self) -> bool {
        self.index == EMPTY
    }
}

/// The open-addressed bucket table.
///
/// Either it is empty, meaning the map scans, or it describes *every* entry;
/// there is no state in between. That is the whole of the bookkeeping: whoever
/// changes the entries either keeps the table in step bucket by bucket or drops
/// it, and a lookup's only question is which of the two it is looking at.
///
/// The table is a separate type rather than a bare `Vec` in the map so that the
/// probe and its invariant sit together, away from the ordering work that is
/// the map's own business.
#[derive(Clone, Default)]
struct HashTable {
    /// A power-of-two number of buckets, or none at all. Indexed by the low
    /// bits of a hash, so the power of two is what makes the mask work.
    buckets: Vec<Bucket>,
}

impl HashTable {
    /// Whether there is no table, and so whether lookups have to scan.
    #[inline]
    fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }

    /// Give up the table. The buckets' capacity is kept for the next rebuild,
    /// which is usually the next insert when a map is hovering around
    /// [`LINEAR_MAX`].
    #[inline]
    fn clear(&mut self) {
        self.buckets.clear();
    }

    /// The mask that turns a hash into a bucket. Only valid for a table that
    /// has buckets, which every probe has already established.
    #[inline]
    fn mask(&self) -> u32 {
        debug_assert!(!self.buckets.is_empty());
        (self.buckets.len() - 1) as u32
    }

    /// The bucket a hash asks for, before any probing.
    #[inline]
    fn ideal(&self, hash: u32) -> u32 {
        hash & self.mask()
    }

    /// How far the bucket at `at` sits from the one its hash asked for.
    ///
    /// Wrapping subtraction under the mask, so a chain that runs off the end of
    /// the table and back to the start measures the same as one that does not.
    #[inline]
    fn distance(&self, at: u32, hash: u32) -> u32 {
        at.wrapping_sub(self.ideal(hash)) & self.mask()
    }

    /// The next bucket in a probe, wrapping at the end of the table.
    #[inline]
    fn next(&self, at: u32) -> u32 {
        (at + 1) & self.mask()
    }

    /// Position of `key`, whose hash is `hash`, or `None`.
    ///
    /// Two things end the walk. An empty bucket means the key was never
    /// inserted, since an insert would have taken that bucket rather than pass
    /// it. An occupant closer to its ideal bucket than the probe has already
    /// travelled means the same, because an insert would have displaced it - so
    /// a miss ends where a hit would have, rather than at the end of the chain.
    ///
    /// The stored hash only filters. Every bucket whose hash matches is
    /// confirmed against the stored key, which is what keeps a 32-bit collision
    /// to the cost of one comparison.
    fn find<V>(&self, entries: &[(String, V)], key: &str, hash: u32) -> Option<usize> {
        let mut at = self.ideal(hash);
        let mut travelled = 0u32;
        loop {
            let bucket = self.buckets[at as usize];
            if bucket.is_empty() || self.distance(at, bucket.hash) < travelled {
                return None;
            }
            if bucket.hash == hash {
                let pos = bucket.index as usize;
                if entries[pos].0.as_str() == key {
                    return Some(pos);
                }
            }
            at = self.next(at);
            travelled += 1;
        }
    }

    /// Put `entry` in the table, taking any bucket from an occupant that is
    /// closer to home than the probe has travelled and carrying that occupant
    /// on.
    ///
    /// No duplicate check: the caller has already established that this entry's
    /// key is not in the table, either by probing for it or by having just
    /// emptied the table to rebuild it.
    ///
    /// This terminates because the load factor keeps at least one bucket empty.
    fn place(&mut self, entry: Bucket) {
        debug_assert!(!entry.is_empty());
        let mut carried = entry;
        let mut at = self.ideal(carried.hash);
        let mut travelled = 0u32;
        loop {
            let occupant = self.buckets[at as usize];
            if occupant.is_empty() {
                self.buckets[at as usize] = carried;
                return;
            }
            let theirs = self.distance(at, occupant.hash);
            if theirs < travelled {
                self.buckets[at as usize] = carried;
                carried = occupant;
                travelled = theirs;
            }
            at = self.next(at);
            travelled += 1;
        }
    }

    /// The bucket holding the entry at `pos`.
    ///
    /// Used by removal, which knows the position and needs the bucket. The walk
    /// cannot use the robin hood early exit, since it is looking for a
    /// particular position rather than for a key, so it stops at the position
    /// itself; the table describes every entry, so it is always there. The loop
    /// is bounded anyway, because a bound that can only be crossed by a bug is
    /// cheaper to state than to debug.
    fn bucket_of<V>(&self, entries: &[(String, V)], pos: u32) -> u32 {
        let hash = hash_key(&entries[pos as usize].0);
        let mut at = self.ideal(hash);
        for _ in 0..self.buckets.len() {
            if self.buckets[at as usize].index == pos {
                return at;
            }
            at = self.next(at);
        }
        unreachable!("structio: OrderedMap entry {pos} has no bucket");
    }

    /// Empty the bucket at `at`, pulling the rest of its chain back one.
    ///
    /// A tombstone would be simpler and would wreck the probe: every lookup
    /// that walks past one pays for it forever, and the robin hood early exit
    /// stops being sound because a displaced entry can sit behind a bucket that
    /// no longer holds the key that displaced it. So each following bucket that
    /// is not already where its hash asked for it - that is, each one that was
    /// pushed along by something - moves back one place, and the hole travels
    /// to the end of the chain. Distances all drop by one, which preserves
    /// their order, and every entry stays reachable from its ideal bucket.
    fn erase(&mut self, at: u32) {
        let mut hole = at;
        let mut curr = self.next(at);
        loop {
            let bucket = self.buckets[curr as usize];
            if bucket.is_empty() || self.distance(curr, bucket.hash) == 0 {
                self.buckets[hole as usize] = Bucket::VACANT;
                return;
            }
            self.buckets[hole as usize] = bucket;
            hole = curr;
            curr = self.next(curr);
        }
    }

    /// Account for the entry at `pos` having been removed and everything above
    /// it having shifted down one.
    ///
    /// Every bucket has to be visited, since positions are scattered across the
    /// table by hash and there is nothing to search. It is a linear sweep of a
    /// dense array with no branches worth predicting, and it is on a removal
    /// that already shifted `O(n)` entries.
    fn shift_down_above(&mut self, pos: u32) {
        for bucket in &mut self.buckets {
            if !bucket.is_empty() && bucket.index > pos {
                bucket.index -= 1;
            }
        }
    }

    /// Size the table for `entries` and fill it from scratch, hashing every
    /// key.
    ///
    /// This is for the first table a map builds and for anything that has moved
    /// entries around wholesale, where there is no table left worth reading.
    /// Growing a table that is merely full is [`HashTable::grow`], which does
    /// not hash anything.
    fn rebuild<V>(&mut self, entries: &[(String, V)]) {
        debug_assert!(entries.len() <= MAX_INDEXED);
        self.buckets.clear();
        self.buckets
            .resize(buckets_for(entries.len()), Bucket::VACANT);
        for (pos, (key, _)) in entries.iter().enumerate() {
            self.place(Bucket {
                index: pos as u32,
                hash: hash_key(key),
            });
        }
    }

    /// Double the table, carrying every bucket over as it stands.
    ///
    /// Nothing is hashed and nothing is read from the entries. A bucket already
    /// holds its entry's hash and its position, and neither changes; all that
    /// changes is which bucket that hash asks for, since the mask is one bit
    /// wider. Re-hashing here would cost a map more hashing over its lifetime
    /// than all of its lookups put together, since a fill hashes every key once
    /// per doubling it lives through.
    fn grow(&mut self) {
        let doubled = self.buckets.len() * 2;
        let old = std::mem::replace(&mut self.buckets, vec![Bucket::VACANT; doubled]);
        for bucket in old {
            if !bucket.is_empty() {
                self.place(bucket);
            }
        }
    }
}

/// What a lookup found, and what an insert needs to know when it did not.
enum Lookup {
    /// The key is at this position.
    Occupied(usize),
    /// The key is absent, with the hash the probe computed on its way to
    /// finding that out, so that an insert does not hash the same key twice.
    /// `None` when the map was small enough to answer by scanning and no hash
    /// was taken at all.
    Vacant(Option<u32>),
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
    /// Lookup acceleration over all of `entries`, or empty for a map small
    /// enough to scan. Never consulted for an answer without a key comparison
    /// to confirm it.
    table: HashTable,
}

impl<V> OrderedMap<V> {
    /// An empty map. Allocates nothing.
    #[inline]
    pub fn new() -> Self {
        OrderedMap {
            entries: Vec::new(),
            table: HashTable::default(),
        }
    }

    /// An empty map with room for `capacity` entries.
    ///
    /// The table is not allocated here, however large the capacity. A table
    /// that exists describes every entry, so one built ahead of the entries
    /// would have to be maintained through the first eight inserts that the
    /// whole point of `LINEAR_MAX` is to leave alone; the map builds it at the
    /// entry that first needs it, sized from the entries it actually holds.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        OrderedMap {
            entries: Vec::with_capacity(capacity),
            table: HashTable::default(),
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
        self.table.clear();
    }

    /// Position of `key`, by whichever route this map's size calls for, and the
    /// key's hash when the route computed one.
    ///
    /// The two routes are the whole of the lookup story: a scan for a map at or
    /// below [`LINEAR_MAX`], a single probe of the table for anything larger.
    #[inline]
    fn locate(&self, key: &str) -> Lookup {
        if self.table.is_empty() {
            return match self.entries.iter().position(|(k, _)| k.as_str() == key) {
                Some(pos) => Lookup::Occupied(pos),
                None => Lookup::Vacant(None),
            };
        }
        let hash = hash_key(key);
        match self.table.find(&self.entries, key, hash) {
            Some(pos) => Lookup::Occupied(pos),
            None => Lookup::Vacant(Some(hash)),
        }
    }

    /// Position of `key`, for the callers that have nothing to insert.
    #[inline]
    fn find(&self, key: &str) -> Option<usize> {
        match self.locate(key) {
            Lookup::Occupied(pos) => Some(pos),
            Lookup::Vacant(_) => None,
        }
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
    ///
    /// There is no separate duplicate check to pay for. The probe that would
    /// find an existing key is the same probe that finds the bucket a new one
    /// goes in, so an insert costs one walk of the table either way.
    pub fn insert(&mut self, key: String, value: V) -> Option<V> {
        match self.locate(&key) {
            Lookup::Occupied(pos) => Some(std::mem::replace(&mut self.entries[pos].1, value)),
            Lookup::Vacant(hash) => {
                self.push_new(key, value, hash);
                None
            }
        }
    }

    /// Append an entry whose key is known to be absent, and give it a bucket.
    ///
    /// `hash` is what the probe that established the key's absence already
    /// computed, or `None` if it answered by scanning. Either way the entry is
    /// hashed at most once.
    ///
    /// Returns the new entry's position.
    fn push_new(&mut self, key: String, value: V, hash: Option<u32>) -> usize {
        let pos = self.entries.len();
        self.entries.push((key, value));
        let len = self.entries.len();
        if len <= LINEAR_MAX {
            // Still small enough to scan, and by the table's invariant there is
            // nothing allocated to keep in step.
            debug_assert!(self.table.is_empty());
        } else if len > MAX_INDEXED {
            // One position too many to address. The table goes, and lookups
            // scan until removals bring the map back under the bound.
            self.table.clear();
        } else if self.table.is_empty() {
            // The first table this map has needed, over the eight entries that
            // never had one and the ninth that has just asked for one.
            self.reindex();
        } else {
            if len > load_limit(self.table.buckets.len()) {
                // Out of room. Doubling is the only `O(n)` work an insert ever
                // does, and a map does it once per doubling it lives through,
                // which is what makes filling one linear rather than quadratic.
                self.table.grow();
            }
            let hash = hash.unwrap_or_else(|| hash_key(&self.entries[pos].0));
            self.table.place(Bucket {
                index: pos as u32,
                hash,
            });
        }
        pos
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

    /// Remove the entry at `pos`, shifting the rest down and repairing the
    /// table.
    ///
    /// Two repairs, and both are needed. The removed key's own bucket is
    /// emptied by backward shift, so that nothing probing past it is cut off
    /// from an entry that its own insert displaced. Then every bucket holding a
    /// position above the hole is decremented, because the entries above it
    /// have all moved down one.
    ///
    /// A removal that takes the map back down to [`LINEAR_MAX`] drops the table
    /// instead. Below that size the map scans, so keeping a table would be
    /// keeping a thing no lookup reads and every insert would have to maintain.
    fn remove_at(&mut self, pos: usize) -> (String, V) {
        if self.table.is_empty() || self.entries.len() - 1 <= LINEAR_MAX {
            // Nothing to repair, because there is no table or there is about to
            // be none. The one case where reindexing does more than drop it
            // here is a map coming back under MAX_INDEXED, which is how a map
            // too wide to address gets its table back.
            let removed = self.entries.remove(pos);
            self.reindex();
            return removed;
        }
        // The bucket has to be found while the entry is still there: it is
        // found by hashing the key it holds.
        let bucket = self.table.bucket_of(&self.entries, pos as u32);
        self.table.erase(bucket);
        let removed = self.entries.remove(pos);
        self.table.shift_down_above(pos as u32);
        removed
    }

    /// Rebuild or discard the table after the entries have been reordered or
    /// thinned wholesale.
    ///
    /// Used where positions change in ways no bucket-by-bucket fixup can
    /// follow, and where the table has outgrown its load factor. A map at or
    /// below [`LINEAR_MAX`] simply drops its table and goes back to scanning,
    /// as does one so wide that [`MAX_INDEXED`] cannot address it.
    fn reindex(&mut self) {
        if self.entries.len() > LINEAR_MAX && self.entries.len() <= MAX_INDEXED {
            self.table.rebuild(&self.entries);
        } else {
            self.table.clear();
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
        match self.locate(&key) {
            Lookup::Occupied(pos) => Entry::Occupied(OccupiedEntry { map: self, pos }),
            Lookup::Vacant(hash) => Entry::Vacant(VacantEntry {
                map: self,
                key,
                hash,
            }),
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
            // table describes it exactly as well as it describes the original,
            // and copying the buckets beats probing them all back in.
            table: self.table.clone(),
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
    /// The hash the probe computed when it found the key absent, or `None` if
    /// it scanned instead. Carrying it here is sound because the entry holds
    /// the map exclusively: nothing can change the key or the table between the
    /// probe and the insert.
    hash: Option<u32>,
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
        let pos = self.map.push_new(self.key, value, self.hash);
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
    ///
    /// A table either describes the whole map or is not there at all, so the
    /// first question is which of the two this is. For a table that is there:
    /// every occupied bucket points at a live entry whose key still hashes to
    /// the hash the bucket stored, every entry has exactly one bucket, every
    /// entry is reachable by probing forward from the bucket its hash asked
    /// for, the robin hood distance ordering holds along the way, and the load
    /// factor is not exceeded.
    fn assert_index_consistent<V>(map: &OrderedMap<V>) {
        let len = map.len();
        if map.table.is_empty() {
            assert!(
                len <= LINEAR_MAX || len > MAX_INDEXED,
                "a map of {len} entries is missing its table"
            );
            return;
        }
        assert!(len > LINEAR_MAX, "a map of {len} entries built a table");

        let buckets = map.table.buckets.len();
        assert!(
            buckets.is_power_of_two(),
            "{buckets} buckets is not a power of two"
        );
        assert!(
            buckets >= MIN_BUCKETS,
            "{buckets} buckets is below the minimum"
        );
        assert!(
            len <= load_limit(buckets),
            "{len} entries in {buckets} buckets is over the load factor"
        );

        let mut bucketed = vec![false; len];
        for (at, bucket) in map.table.buckets.iter().enumerate() {
            if bucket.is_empty() {
                continue;
            }
            let at = at as u32;
            let pos = bucket.index as usize;
            assert!(pos < len, "bucket {at} points past the entries");
            assert_eq!(
                bucket.hash,
                hash_key(&map.entries[pos].0),
                "bucket {at} holds a stale hash"
            );
            assert!(
                !std::mem::replace(&mut bucketed[pos], true),
                "two buckets point at entry {pos}"
            );

            // Reachability and the robin hood ordering are one statement: every
            // bucket this entry was pushed past is occupied by something at
            // least as far from home as the probe is when it goes by, so a
            // probe for this key neither stops at an empty bucket nor takes the
            // displacement exit before it arrives.
            let ideal = map.table.ideal(bucket.hash);
            for step in 0..map.table.distance(at, bucket.hash) {
                let passed_at = (ideal + step) & map.table.mask();
                let passed = map.table.buckets[passed_at as usize];
                assert!(
                    !passed.is_empty(),
                    "entry {pos} is unreachable: bucket {passed_at} is empty"
                );
                assert!(
                    map.table.distance(passed_at, passed.hash) >= step,
                    "robin hood ordering is broken at bucket {passed_at}"
                );
            }
        }
        assert!(
            bucketed.iter().all(|&seen| seen),
            "an entry has no bucket of its own"
        );

        // And the probe does find them all, which is what the rest is for.
        for (pos, (key, _)) in map.entries.iter().enumerate() {
            assert_eq!(
                map.table.find(&map.entries, key, hash_key(key)),
                Some(pos),
                "entry {pos} is not where a probe looks for it"
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
            assert_eq!(
                map.table.buckets.len(),
                map_of(n).table.buckets.len(),
                "a duplicate insert grew the table at n = {n}"
            );
        }
    }

    // --- the bucket table ---------------------------------------------------

    /// Keys whose hash asks for `bucket` in a table of `buckets` buckets.
    ///
    /// Searched rather than written down, for the reason
    /// [`colliding_pairs`] gives: a hard-coded key stops meaning anything the
    /// moment the seed or the hash changes, and the test would go on passing
    /// while testing nothing.
    fn keys_for_bucket(bucket: u32, buckets: usize, wanted: usize) -> Vec<String> {
        let mask = (buckets - 1) as u32;
        let mut found = Vec::new();
        for i in 0..1_000_000usize {
            let key = nth_key(i);
            if hash_key(&key) & mask == bucket {
                found.push(key);
                if found.len() == wanted {
                    return found;
                }
            }
        }
        panic!("fewer than {wanted} keys land in bucket {bucket} of {buckets}");
    }

    /// Which bucket a present key sits in.
    fn bucket_of<V>(map: &OrderedMap<V>, key: &str) -> u32 {
        let pos = map.find(key).expect("key is present") as u32;
        map.table.bucket_of(&map.entries, pos)
    }

    /// How far a present key sits from the bucket it asked for.
    fn distance_of<V>(map: &OrderedMap<V>, key: &str) -> u32 {
        map.table.distance(bucket_of(map, key), hash_key(key))
    }

    #[test]
    fn the_table_appears_and_doubles_at_the_right_sizes() {
        let mut map: OrderedMap<usize> = OrderedMap::new();
        for n in 1..=200usize {
            map.insert(nth_key(n - 1), n - 1);
            assert_eq!(map.len(), n);
            if n <= LINEAR_MAX {
                assert!(map.table.is_empty(), "a map of {n} entries built a table");
            } else {
                assert_eq!(
                    map.table.buckets.len(),
                    buckets_for(n),
                    "wrong table size at n = {n}"
                );
            }
            assert_index_consistent(&map);
            for i in 0..n {
                assert_eq!(map.get(&nth_key(i)), Some(&i), "lost a key at n = {n}");
            }
        }
        // The sizes above are not a restatement of the code: the first table
        // holds the ninth entry, and each one after it is twice the last.
        assert_eq!(buckets_for(LINEAR_MAX + 1), MIN_BUCKETS);
        for n in 1..=200usize {
            let buckets = buckets_for(n);
            assert!(n <= load_limit(buckets));
            assert!(buckets == MIN_BUCKETS || n > load_limit(buckets / 2));
        }
    }

    #[test]
    fn probes_wrap_around_the_end_of_the_table() {
        // Four keys asking for the last bucket of the smallest table, so that
        // their chain runs off the end and continues at the start.
        let last = (MIN_BUCKETS - 1) as u32;
        let chain = keys_for_bucket(last, MIN_BUCKETS, 4);
        let mut map: OrderedMap<String> = OrderedMap::new();
        for key in &chain {
            map.insert(key.clone(), format!("{key}!"));
        }
        let mut filler = Vec::new();
        while map.len() < load_limit(MIN_BUCKETS) {
            let key = nth_key(5_000_000 + filler.len());
            map.insert(key.clone(), String::new());
            filler.push(key);
        }
        assert_eq!(map.table.buckets.len(), MIN_BUCKETS);
        assert_index_consistent(&map);

        let wrapped = chain.iter().filter(|k| bucket_of(&map, k) < last).count();
        assert_eq!(wrapped, chain.len() - 1, "the chain did not wrap");

        // Every one of them is still found, and so is everything else, which
        // means the probe wrapped too.
        for key in &chain {
            assert_eq!(map.get(key), Some(&format!("{key}!")));
        }
        assert_eq!(map.get("absent"), None);

        // Removing the one bucket that is not wrapped shifts the wrapped tail
        // of the chain back across the end of the table.
        let head = chain[0].clone();
        assert_eq!(bucket_of(&map, &head), last);
        assert_eq!(map.remove(&head), Some(format!("{head}!")));
        assert_index_consistent(&map);
        assert_eq!(map.get(&head), None);
        for key in chain[1..].iter().chain(&filler) {
            assert!(map.contains_key(key), "lost {key} to a wrapped removal");
        }
    }

    #[test]
    fn a_later_insert_displaces_an_entry_that_is_closer_to_home() {
        // Eight fillers, parked away from buckets 0 to 3 so that what happens
        // in that corner of the table is only about the three keys below.
        let mut filler = keys_for_bucket(8, MIN_BUCKETS, 6);
        filler.extend(keys_for_bucket(4, MIN_BUCKETS, 2));
        let mut map: OrderedMap<u32> = OrderedMap::new();
        for (i, key) in filler.iter().enumerate() {
            map.insert(key.clone(), i as u32);
        }
        assert_eq!(map.len(), LINEAR_MAX);

        // `zero` wants bucket 0 twice over and `one` wants bucket 1, which it
        // gets first. The second `zero` key then finds bucket 0 taken by an
        // entry that is home, walks on to bucket 1 one place from its own home,
        // and finds `one` sitting there with nothing invested - so it takes the
        // bucket and carries `one` on to bucket 2.
        let zero = keys_for_bucket(0, MIN_BUCKETS, 2);
        let one = keys_for_bucket(1, MIN_BUCKETS, 1);
        map.insert(zero[0].clone(), 100);
        map.insert(one[0].clone(), 101);
        map.insert(zero[1].clone(), 102);

        assert_eq!(map.table.buckets.len(), MIN_BUCKETS);
        assert_eq!(bucket_of(&map, &zero[0]), 0);
        assert_eq!(
            bucket_of(&map, &zero[1]),
            1,
            "the later insert did not take the bucket"
        );
        assert_eq!(
            bucket_of(&map, &one[0]),
            2,
            "the entry that was home was not displaced"
        );
        assert_index_consistent(&map);

        assert_eq!(map.get(&zero[0]), Some(&100));
        assert_eq!(map.get(&one[0]), Some(&101));
        assert_eq!(map.get(&zero[1]), Some(&102));
        for (i, key) in filler.iter().enumerate() {
            assert_eq!(map.get(key), Some(&(i as u32)));
        }
    }

    #[test]
    fn removing_the_head_of_a_chain_keeps_the_rest_reachable() {
        // Five keys that all ask for the same bucket, so they sit in a run of
        // five with distances 0 to 4. Removing one of them has to pull the rest
        // of the run back, or the ones behind the hole become unreachable.
        let chain = keys_for_bucket(3, MIN_BUCKETS, 5);
        let mut map: OrderedMap<String> = OrderedMap::new();
        for key in &chain {
            map.insert(key.clone(), format!("{key}!"));
        }
        let mut filler = Vec::new();
        while map.len() < load_limit(MIN_BUCKETS) {
            let key = nth_key(6_000_000 + filler.len());
            map.insert(key.clone(), String::new());
            filler.push(key);
        }
        assert_index_consistent(&map);
        for (step, key) in chain.iter().enumerate() {
            assert_eq!(
                distance_of(&map, key),
                step as u32,
                "the keys did not form one chain"
            );
        }

        // The head first, which is the bucket everything behind it was probing
        // past, then the new head of what is left.
        for removed in [0usize, 1] {
            let key = &chain[removed];
            assert_eq!(map.remove(key), Some(format!("{key}!")));
            assert_index_consistent(&map);
            assert_eq!(map.get(key), None);
            for survivor in &chain[removed + 1..] {
                assert_eq!(
                    map.get(survivor),
                    Some(&format!("{survivor}!")),
                    "removing {key} cut {survivor} out of its chain"
                );
            }
            for key in &filler {
                assert_eq!(map.get(key), Some(&String::new()));
            }
        }
    }

    #[test]
    fn filling_rebuilds_a_bounded_number_of_times() {
        // The point of the table, stated without a clock: filling a map
        // rebuilds it once per doubling, and the entries those rebuilds walk
        // add up to a small multiple of the map rather than of its square.
        const N: usize = 4_000;
        let mut map: OrderedMap<usize> = OrderedMap::new();
        let mut buckets = 0;
        let mut rebuilds = 0usize;
        let mut rebuilt_entries = 0usize;
        for i in 0..N {
            map.insert(nth_key(i), i);
            if map.table.buckets.len() != buckets {
                buckets = map.table.buckets.len();
                rebuilds += 1;
                rebuilt_entries += map.len();
            }
        }
        assert_eq!(map.len(), N);
        assert_index_consistent(&map);
        assert!(
            rebuilds <= (N.ilog2() as usize) + 1,
            "{rebuilds} rebuilds filling {N} entries"
        );
        assert!(
            rebuilt_entries <= 2 * N,
            "rebuilding walked {rebuilt_entries} entries filling {N}"
        );
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

        // Above it, where the probe has to walk past a bucket whose stored hash
        // matches a key that is not the one being looked for.
        let mut big: OrderedMap<String> = OrderedMap::new();
        for (a, _) in &pairs {
            big.insert(a.clone(), format!("{a}!"));
        }
        for i in 0..200 {
            big.insert(nth_key(1_000_000 + i), i.to_string());
        }
        assert!(!big.table.is_empty(), "the hashed path was never exercised");
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
            // Enough further inserts to carry the table through a rebuild with
            // both halves of every pair in it.
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
    fn crossing_the_linear_threshold_in_both_directions() {
        let mut map = map_of(12);
        assert!(!map.table.is_empty());

        // Down past the threshold, from the front, so that every remaining
        // entry moves and every bucket has to be corrected.
        for removed in 0..5 {
            assert_eq!(map.remove(&nth_key(removed)), Some(removed));
            assert_index_consistent(&map);
            assert_eq!(map.len(), 11 - removed);
            for survivor in removed + 1..12 {
                assert_eq!(map.get(&nth_key(survivor)), Some(&survivor));
            }
        }
        assert!(
            map.table.is_empty(),
            "the table outlived the map's need for it"
        );
        assert_eq!(
            map.keys().cloned().collect::<Vec<_>>(),
            (5..12).map(nth_key).collect::<Vec<_>>()
        );

        // And back up, through the rebuild that the ninth entry asks for.
        for added in 12..24 {
            map.insert(nth_key(added), added);
            assert_index_consistent(&map);
        }
        assert_eq!(map.len(), 19);
        assert!(!map.table.is_empty());
        for i in 0..24 {
            let found = map.get(&nth_key(i));
            assert_eq!(found, if i < 5 { None } else { Some(&i) });
        }
        assert_eq!(
            map.keys().cloned().collect::<Vec<_>>(),
            (5..24).map(nth_key).collect::<Vec<_>>()
        );
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

        // Refilling past the threshold builds a table over the entries that are
        // there now, not over the ones that were.
        map.clear();
        for i in 0..40 {
            map.insert(nth_key(i), i);
            assert_index_consistent(&map);
        }
        assert_eq!(map.get("a"), None);
        assert_eq!(
            map.keys().cloned().collect::<Vec<_>>(),
            (0..40).map(nth_key).collect::<Vec<_>>()
        );
        for i in 0..40 {
            assert_eq!(map.get(&nth_key(i)), Some(&i));
        }
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
        assert!(!map.table.is_empty());
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

        // Down to under the threshold, where the table goes away entirely.
        map.retain(|key, _| key == nth_key(0));
        assert_eq!(map.len(), 1);
        assert!(map.table.is_empty());
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

//! Leptos stores: a state struct whose fields notify separately.
//!
//! A [`RwSignal`](crate::reactive::RwSignal) holds one value. Put a whole
//! state struct in one and every screen that read any of it rebuilds when any
//! field changes, because the signal cannot tell fields apart. Splitting the
//! struct into one signal per field fixes that, at the cost of wrapping every
//! field by hand.
//!
//! A store keeps the struct a plain Rust value and does the wrapping for you:
//! each field path has its own reactive trigger. Reading a field subscribes
//! the caller to that field and its children; writing it notifies the field,
//! its children, and its parents, but never its siblings. A screen that reads
//! `store.title()` does not rebuild because `store.open()` changed.
//!
//! Derive [`Store`](reactive_stores::Store) on the struct, which adds a
//! getter for each field, and hold it in an [`ArcStore`], which lives as long
//! as a clone of it does, like `ArcRwSignal`:
//!
//! ```
//! use leptoskit_native::reactive::Get;
//! use leptoskit_native::reactive::Set;
//! use leptoskit_native::store::ArcStore;
//!
//! #[derive(Default, leptoskit_native::store::Store)]
//! struct Library {
//! 	title: String,
//! 	open: bool,
//! }
//!
//! let library = ArcStore::new(Library::default());
//! let title = library.clone().title();
//! let open = library.clone().open();
//!
//! assert_eq!(title.get(), "");
//! open.set(true);
//! assert!(open.get());
//! ```
//!
//! The generated getters take the store by value, so an `ArcStore` is cloned
//! for each field — a reference bump — or kept as the field handle `title`
//! and `open` are above. [`Store::new`](reactive_stores::Store::new) instead
//! of `ArcStore::new` follows the same ownership rule as `RwSignal`: the
//! store belongs to the reactive owner current where it was created, is
//! disposed with it, and is `Copy`, so its getters need no clone.
//!
//! The reads and writes are the ordinary signal traits: `get`, `set`,
//! `with`, and `update` from [`Get`](crate::reactive::Get) and its sibling
//! traits.
//!
//! To apply a whole new value — a reducer's output, or fresh data from the
//! server — derive [`Patch`](reactive_stores::Patch) on the same structs and
//! call `.patch(next)`: fields that changed notify, fields that did not stay
//! quiet. Every field type must implement `PatchField`: the primitives,
//! `String`, `Option`, `Vec`, and tuples do, and so does any other struct or
//! enum that derives `Patch`. When the field types come from another crate
//! that should not depend on the reactive crates, write the fields directly
//! instead of patching.
//!
//! Two neighbours still apply. State whose keys are decided at run time — a
//! document's variables, say — is one signal per value in a map, not a store,
//! because the derive needs the fields at compile time. And a handle or value
//! that must never notify, such as a subscription key, stays in a
//! `StoredValue`; a store field always notifies.

pub use reactive_stores::*;

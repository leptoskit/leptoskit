//! `#[derive(Event)]`, behind `leptoskit_analytics`'s typed events. Apps use
//! it through `leptoskit_analytics` with its `derive` feature; the
//! documentation lives there.

use convert_case::Case;
use convert_case::Casing as _;
use proc_macro::TokenStream;
use proc_macro_crate::FoundCrate;
use proc_macro2::TokenStream as Tokens;
use quote::ToTokens as _;
use quote::quote;
use quote::quote_spanned;
use syn::DeriveInput;
use syn::LitStr;
use syn::ext::IdentExt as _;
use syn::spanned::Spanned as _;

/// Declares an analytics event. See `leptoskit_analytics::Event`.
#[proc_macro_derive(Event, attributes(event))]
pub fn derive_event(input: TokenStream) -> TokenStream {
	match expand(&syn::parse_macro_input!(input as DeriveInput)) {
		Ok(tokens) => tokens.into(),
		Err(error) => error.to_compile_error().into(),
	}
}

/// The most bytes an event's name may have, as the service accepts it.
const MAX_NAME_BYTES: usize = 200;

fn expand(input: &DeriveInput) -> syn::Result<Tokens> {
	let mut name = None;
	for attribute in input
		.attrs
		.iter()
		.filter(|attribute| attribute.path().is_ident("event"))
	{
		attribute.parse_nested_meta(|meta| {
			if meta.path.is_ident("name") {
				name = Some(meta.value()?.parse::<LitStr>()?);
				Ok(())
			} else {
				Err(meta.error("expected `name = \"...\"`"))
			}
		})?;
	}
	let ident = &input.ident;
	let name = name.map_or_else(
		|| ident.to_string().to_case(Case::Snake),
		|name| name.value(),
	);
	// The service's own rule (`accept::valid_name`), so a name that compiles
	// is a name the service stores, plus `$`, which leptoskit's events use.
	if name.trim().is_empty()
		|| name.len() > MAX_NAME_BYTES
		|| name.chars().any(char::is_control)
		|| name.starts_with('$')
	{
		return Err(syn::Error::new_spanned(
			ident,
			"an event's name needs 1 to 200 bytes, not only spaces, with no control characters \
			 such as newlines, and can't start with `$`, which leptoskit's own events use: set it \
			 with `#[event(name = \"...\")]`",
		));
	}
	if !input.generics.params.is_empty() {
		return Err(syn::Error::new_spanned(
			&input.generics,
			"an event can't be generic: its schema must be the same every time it's sent",
		));
	}
	let syn::Data::Struct(data) = &input.data else {
		return Err(syn::Error::new_spanned(
			ident,
			"an event is a struct with named fields, one per property",
		));
	};
	let fields: Vec<_> = match &data.fields {
		syn::Fields::Named(fields) => fields.named.iter().collect(),
		syn::Fields::Unit => Vec::new(),
		syn::Fields::Unnamed(fields) => {
			return Err(syn::Error::new_spanned(
				fields,
				"an event's properties need names: use a struct with named fields",
			));
		}
	};

	// The logical name: a field written `r#type` is the property `type`.
	let names: Vec<String> = fields
		.iter()
		.map(|field| field.ident.as_ref().expect("named").unraw().to_string())
		.collect();
	let types: Vec<String> = fields
		.iter()
		.map(|field| field.ty.to_token_stream().to_string().replace(' ', ""))
		.collect();
	let schema_id = schema_id(&name, &names, &types);
	let krate = crate_path();
	// Each conversion carries its field's span, so a type that can't be a
	// property is reported on that field.
	let convert = |field: &&syn::Field| {
		let (ident, ty) = (field.ident.as_ref().expect("named"), &field.ty);
		quote_spanned! {ty.span()=> <#krate::wire::Value as ::core::convert::From<#ty>>::from(self.#ident)}
	};
	let values: Vec<_> = fields.iter().map(convert).collect();
	let positional = values.clone();
	let (_, _, where_clause) = input.generics.split_for_impl();

	Ok(quote! {
		impl #krate::Event for #ident #where_clause {
			const NAME: &'static str = #name;
			const FIELDS: &'static [(&'static str, &'static str)] = &[#((#names, #types)),*];
			const SCHEMA_ID: u64 = #schema_id;

			fn into_properties(self) -> #krate::wire::Properties {
				::std::vec![#((::std::string::String::from(#names), #values)),*]
			}

			fn into_values(self) -> ::std::vec::Vec<#krate::wire::Value> {
				::std::vec![#(#positional),*]
			}
		}

		// Collected for the event catalogue in server builds only.
		#[cfg(not(target_arch = "wasm32"))]
		#krate::__inventory::submit! {
			#krate::EventSchema {
				name: #name,
				schema_id: #schema_id,
				fields: <#ident as #krate::Event>::FIELDS,
			}
		}
	})
}

/// FNV-1a over the name and each field's name and type: the same declaration
/// always has the same id, and changing a field's name or type changes it.
///
/// A type is hashed as it's spelled, without whitespace, not as what it
/// resolves to: `String` and `std::string::String`, or a type alias and its
/// target, give different ids though they're the same type.
fn schema_id(name: &str, names: &[String], types: &[String]) -> u64 {
	let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
	let mut eat = |text: &str| {
		for byte in text.bytes().chain([0]) {
			hash ^= u64::from(byte);
			hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
		}
	};
	eat(name);
	for (name, ty) in names.iter().zip(types) {
		eat(name);
		eat(ty);
	}
	hash
}

fn crate_path() -> Tokens {
	match proc_macro_crate::crate_name("leptoskit_analytics") {
		Ok(FoundCrate::Name(name)) => {
			let name = syn::Ident::new(&name, proc_macro2::Span::call_site());
			quote!(::#name)
		}
		// The crate's own tests.
		Ok(FoundCrate::Itself) | Err(_) => quote!(::leptoskit_analytics),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_schema_id_follows_the_declaration() {
		let base = schema_id("checkout", &["plan".into()], &["String".into()]);
		assert_eq!(
			base,
			schema_id("checkout", &["plan".into()], &["String".into()])
		);
		assert_ne!(
			base,
			schema_id("checkout", &["plans".into()], &["String".into()])
		);
		assert_ne!(
			base,
			schema_id("checkout", &["plan".into()], &["u32".into()])
		);
		assert_ne!(
			base,
			schema_id("checkouts", &["plan".into()], &["String".into()])
		);
		// The separator keeps "ab" + "c" apart from "a" + "bc".
		assert_ne!(
			schema_id("e", &["ab".into()], &["c".into()]),
			schema_id("e", &["a".into()], &["bc".into()])
		);
	}
}

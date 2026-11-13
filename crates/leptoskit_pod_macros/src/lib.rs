//! `#[pod::data]` — derive Pod encode/decode for structs and enums.

use proc_macro::TokenStream;
use quote::quote;
use syn::Data;
use syn::DataEnum;
use syn::DataStruct;
use syn::DeriveInput;
use syn::Fields;
use syn::Index;
use syn::LitInt;
use syn::Path;
use syn::Token;
use syn::parse::Parse;
use syn::parse::ParseStream;
use syn::parse_macro_input;

/// Derives [`leptoskit_pod::PodEncode`] and [`leptoskit_pod::PodDecode`].
///
/// Optional arguments:
/// - path to the `leptoskit_pod` crate (positional or `crate = …`); defaults to
///   `::leptoskit_pod`
/// - `version = N` — layout version (`u16`, default `0`)
///
/// Example: `#[pod::data(version = 1)]` or
/// `#[pod::data(::leptoskit_rpc::__private::leptoskit_pod, version = 1)]`.
#[proc_macro_attribute]
pub fn data(attr: TokenStream, item: TokenStream) -> TokenStream {
	let attrs = match syn::parse::<PodAttrs>(attr) {
		Ok(attrs) => attrs,
		Err(error) => return error.to_compile_error().into(),
	};
	let input = parse_macro_input!(item as DeriveInput);
	let name = &input.ident;
	let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
	let pod = &attrs.pod_path;
	let version = attrs.version;

	let encode_body = match &input.data {
		Data::Struct(data) => encode_struct(data, pod, version),
		Data::Enum(data) => encode_enum(data, pod),
		Data::Union(_) => {
			return syn::Error::new_spanned(&input.ident, "#[pod::data] does not support unions")
				.to_compile_error()
				.into();
		}
	};

	let decode_body = match &input.data {
		Data::Struct(data) => decode_struct(data, pod, version),
		Data::Enum(data) => decode_enum(data, pod),
		Data::Union(_) => unreachable!(),
	};

	let expanded = quote! {
		#input

		impl #impl_generics #pod::PodEncode for #name #ty_generics #where_clause {
			fn encode(&self, w: &mut #pod::Writer) -> #pod::Result<()> {
				#encode_body
			}
		}

		impl #impl_generics #pod::PodDecode<'_> for #name #ty_generics #where_clause {
			fn decode(r: &mut #pod::Reader<'_>) -> #pod::Result<Self> {
				#decode_body
			}
		}
	};

	expanded.into()
}

struct PodAttrs {
	pod_path: Path,
	version: u16,
}

impl Parse for PodAttrs {
	fn parse(input: ParseStream) -> syn::Result<Self> {
		let mut pod_path: Option<Path> = None;
		let mut version = 0u16;

		if input.is_empty() {
			return Ok(Self {
				pod_path: syn::parse_quote!(::leptoskit_pod),
				version,
			});
		}

		if !input.peek(syn::Ident) || input.peek2(Token![::]) || input.peek(Token![::]) {
			if input.peek(Token![crate]) && input.peek2(Token![=]) {
				// fall through to key parsing
			} else if looks_like_path(input) {
				pod_path = Some(input.parse()?);
				if input.peek(Token![,]) {
					let _: Token![,] = input.parse()?;
				}
			}
		}

		while !input.is_empty() {
			let key: syn::Ident = input.parse()?;
			let _: Token![=] = input.parse()?;
			match key.to_string().as_str() {
				"crate" => {
					pod_path = Some(input.parse()?);
				}
				"version" => {
					let lit: LitInt = input.parse()?;
					version = lit.base10_parse()?;
				}
				other => {
					return Err(syn::Error::new(
						key.span(),
						format!("unknown #[pod::data] key `{other}`; expected crate, version"),
					));
				}
			}
			if input.peek(Token![,]) {
				let _: Token![,] = input.parse()?;
			}
		}

		Ok(Self {
			pod_path: pod_path.unwrap_or_else(|| syn::parse_quote!(::leptoskit_pod)),
			version,
		})
	}
}

fn looks_like_path(input: ParseStream) -> bool {
	input.peek(Token![::])
		|| input.peek(Token![crate])
		|| (input.peek(syn::Ident) && input.peek2(Token![::]))
}

fn encode_struct(data: &DataStruct, pod: &Path, version: u16) -> proc_macro2::TokenStream {
	match &data.fields {
		Fields::Named(fields) => {
			let writes = fields.named.iter().map(|f| {
				let ident = f.ident.as_ref().expect("named");
				quote! { #pod::PodEncode::encode(&self.#ident, w)?; }
			});
			quote! {
				#pod::encode_struct(w, #version, |w| {
					#(#writes)*
					::core::result::Result::Ok(())
				})
			}
		}
		Fields::Unnamed(fields) => {
			let writes = fields.unnamed.iter().enumerate().map(|(i, _)| {
				let index = Index::from(i);
				quote! { #pod::PodEncode::encode(&self.#index, w)?; }
			});
			quote! {
				#pod::encode_struct(w, #version, |w| {
					#(#writes)*
					::core::result::Result::Ok(())
				})
			}
		}
		Fields::Unit => {
			quote! {
				#pod::encode_struct(w, #version, |_w| ::core::result::Result::Ok(()))
			}
		}
	}
}

fn decode_struct(data: &DataStruct, pod: &Path, version: u16) -> proc_macro2::TokenStream {
	match &data.fields {
		Fields::Named(fields) => {
			let idents: Vec<_> = fields
				.named
				.iter()
				.map(|f| f.ident.as_ref().expect("named"))
				.collect();
			let reads = idents.iter().map(|ident| {
				quote! { #ident: #pod::PodDecode::decode(r)?, }
			});
			quote! {
				#pod::decode_struct_version(r, #version, |r| {
					::core::result::Result::Ok(Self {
						#(#reads)*
					})
				})
			}
		}
		Fields::Unnamed(fields) => {
			let reads = fields.unnamed.iter().map(|_| {
				quote! { #pod::PodDecode::decode(r)?, }
			});
			quote! {
				#pod::decode_struct_version(r, #version, |r| {
					::core::result::Result::Ok(Self(#(#reads)*))
				})
			}
		}
		Fields::Unit => {
			quote! {
				#pod::decode_struct_version(r, #version, |_r| {
					::core::result::Result::Ok(Self)
				})
			}
		}
	}
}

fn encode_enum(data: &DataEnum, pod: &Path) -> proc_macro2::TokenStream {
	let arms = data.variants.iter().enumerate().map(|(disc, v)| {
		let ident = &v.ident;
		let disc = disc as u16;
		match &v.fields {
			Fields::Unit => {
				quote! {
					Self::#ident => {
						w.variant(#disc);
						::core::result::Result::Ok(())
					}
				}
			}
			Fields::Unnamed(fields) => {
				let bindings: Vec<_> = (0..fields.unnamed.len())
					.map(|i| syn::Ident::new(&format!("f{i}"), proc_macro2::Span::call_site()))
					.collect();
				let writes = bindings.iter().map(|b| {
					quote! { #pod::PodEncode::encode(#b, w)?; }
				});
				quote! {
					Self::#ident(#(#bindings),*) => {
						w.variant(#disc);
						#(#writes)*
						::core::result::Result::Ok(())
					}
				}
			}
			Fields::Named(fields) => {
				let idents: Vec<_> = fields
					.named
					.iter()
					.map(|f| f.ident.as_ref().expect("named"))
					.collect();
				let writes = idents.iter().map(|ident| {
					quote! { #pod::PodEncode::encode(#ident, w)?; }
				});
				quote! {
					Self::#ident { #(#idents),* } => {
						w.variant(#disc);
						#pod::encode_struct_v0(w, |w| {
							#(#writes)*
							::core::result::Result::Ok(())
						})
					}
				}
			}
		}
	});
	quote! {
		match self {
			#(#arms),*
		}
	}
}

fn decode_enum(data: &DataEnum, pod: &Path) -> proc_macro2::TokenStream {
	let arms = data.variants.iter().enumerate().map(|(disc, v)| {
		let ident = &v.ident;
		let disc = disc as u16;
		match &v.fields {
			Fields::Unit => {
				quote! {
					#disc => ::core::result::Result::Ok(Self::#ident)
				}
			}
			Fields::Unnamed(fields) => {
				let reads = fields.unnamed.iter().map(|_| {
					quote! { #pod::PodDecode::decode(r)?, }
				});
				quote! {
					#disc => ::core::result::Result::Ok(Self::#ident(#(#reads)*))
				}
			}
			Fields::Named(fields) => {
				let idents: Vec<_> = fields
					.named
					.iter()
					.map(|f| f.ident.as_ref().expect("named"))
					.collect();
				let reads = idents.iter().map(|ident| {
					quote! { #ident: #pod::PodDecode::decode(r)?, }
				});
				quote! {
					#disc => #pod::decode_struct_version(r, 0, |r| {
						::core::result::Result::Ok(Self::#ident {
							#(#reads)*
						})
					})
				}
			}
		}
	});
	quote! {
		match r.variant()? {
			#(#arms,)*
			other => ::core::result::Result::Err(#pod::Error::InvalidTag(other)),
		}
	}
}

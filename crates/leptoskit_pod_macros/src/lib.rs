//! `#[pod::data]` — derive Pod encode/decode for structs and enums.

use proc_macro::TokenStream;
use quote::quote;
use syn::Data;
use syn::DataEnum;
use syn::DataStruct;
use syn::DeriveInput;
use syn::Fields;
use syn::Index;
use syn::Path;
use syn::Token;
use syn::parse::Parser;
use syn::parse_macro_input;

/// Derives [`leptoskit_pod::PodEncode`] and [`leptoskit_pod::PodDecode`].
///
/// Optional attribute argument: a path to the `leptoskit_pod` crate, such as
/// `#[pod::data(::leptoskit_rpc::__private::leptoskit_pod)]` when the consumer
/// only depends on `leptoskit_rpc`. Defaults to `::leptoskit_pod`.
#[proc_macro_attribute]
pub fn data(attr: TokenStream, item: TokenStream) -> TokenStream {
	let pod_path = match parse_pod_path(attr) {
		Ok(path) => path,
		Err(error) => return error.to_compile_error().into(),
	};
	let input = parse_macro_input!(item as DeriveInput);
	let name = &input.ident;
	let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

	let encode_body = match &input.data {
		Data::Struct(data) => encode_struct(data, &pod_path),
		Data::Enum(data) => encode_enum(data, &pod_path),
		Data::Union(_) => {
			return syn::Error::new_spanned(&input.ident, "#[pod::data] does not support unions")
				.to_compile_error()
				.into();
		}
	};

	let decode_body = match &input.data {
		Data::Struct(data) => decode_struct(data, &pod_path),
		Data::Enum(data) => decode_enum(data, &pod_path),
		Data::Union(_) => unreachable!(),
	};

	let expanded = quote! {
		#input

		impl #impl_generics #pod_path::PodEncode for #name #ty_generics #where_clause {
			fn encode(&self, w: &mut #pod_path::Writer) -> #pod_path::Result<()> {
				#encode_body
			}
		}

		impl #impl_generics #pod_path::PodDecode<'_> for #name #ty_generics #where_clause {
			fn decode(r: &mut #pod_path::Reader<'_>) -> #pod_path::Result<Self> {
				#decode_body
			}
		}
	};

	expanded.into()
}

fn parse_pod_path(attr: TokenStream) -> syn::Result<Path> {
	if attr.is_empty() {
		return Ok(syn::parse_quote!(::leptoskit_pod));
	}
	let parser = |input: syn::parse::ParseStream| -> syn::Result<Path> {
		if input.peek(Token![crate]) {
			let _: Token![crate] = input.parse()?;
			let _: Token![=] = input.parse()?;
		}
		input.parse()
	};
	parser.parse(attr)
}

fn encode_struct(data: &DataStruct, pod: &Path) -> proc_macro2::TokenStream {
	match &data.fields {
		Fields::Named(fields) => {
			let writes = fields.named.iter().map(|f| {
				let ident = f.ident.as_ref().expect("named");
				quote! { #pod::PodEncode::encode(&self.#ident, w)?; }
			});
			quote! {
				#pod::encode_struct_v0(w, |w| {
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
				#pod::encode_struct_v0(w, |w| {
					#(#writes)*
					::core::result::Result::Ok(())
				})
			}
		}
		Fields::Unit => {
			quote! {
				#pod::encode_struct_v0(w, |_w| ::core::result::Result::Ok(()))
			}
		}
	}
}

fn decode_struct(data: &DataStruct, pod: &Path) -> proc_macro2::TokenStream {
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
				#pod::decode_struct_version(r, 0, |r| {
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
				#pod::decode_struct_version(r, 0, |r| {
					::core::result::Result::Ok(Self(#(#reads)*))
				})
			}
		}
		Fields::Unit => {
			quote! {
				#pod::decode_struct_version(r, 0, |_r| {
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

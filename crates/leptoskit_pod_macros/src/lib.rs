//! `#[pod::data]` — derive Pod encode/decode for structs and enums.

use proc_macro::TokenStream;
use quote::quote;
use syn::parse_macro_input;
use syn::Data;
use syn::DataEnum;
use syn::DataStruct;
use syn::DeriveInput;
use syn::Fields;
use syn::Index;

/// Derives [`leptoskit_pod::PodEncode`] and [`leptoskit_pod::PodDecode`].
///
/// Named structs get a version-0 dense header. Enums get a `u16` discriminator
/// in variant declaration order. Struct-variant payloads are themselves
/// versioned structs.
#[proc_macro_attribute]
pub fn data(_attr: TokenStream, item: TokenStream) -> TokenStream {
	let input = parse_macro_input!(item as DeriveInput);
	let name = &input.ident;
	let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

	let encode_body = match &input.data {
		Data::Struct(data) => encode_struct(data),
		Data::Enum(data) => encode_enum(data),
		Data::Union(_) => {
			return syn::Error::new_spanned(&input.ident, "#[pod::data] does not support unions")
				.to_compile_error()
				.into();
		}
	};

	let decode_body = match &input.data {
		Data::Struct(data) => decode_struct(data),
		Data::Enum(data) => decode_enum(data),
		Data::Union(_) => unreachable!(),
	};

	let expanded = quote! {
		#input

		impl #impl_generics ::leptoskit_pod::PodEncode for #name #ty_generics #where_clause {
			fn encode(&self, w: &mut ::leptoskit_pod::Writer) -> ::leptoskit_pod::Result<()> {
				#encode_body
			}
		}

		impl #impl_generics ::leptoskit_pod::PodDecode<'_> for #name #ty_generics #where_clause {
			fn decode(r: &mut ::leptoskit_pod::Reader<'_>) -> ::leptoskit_pod::Result<Self> {
				#decode_body
			}
		}
	};

	expanded.into()
}

fn encode_struct(data: &DataStruct) -> proc_macro2::TokenStream {
	match &data.fields {
		Fields::Named(fields) => {
			let writes = fields.named.iter().map(|f| {
				let ident = f.ident.as_ref().expect("named");
				quote! { ::leptoskit_pod::PodEncode::encode(&self.#ident, w)?; }
			});
			quote! {
				::leptoskit_pod::encode_struct_v0(w, |w| {
					#(#writes)*
					::core::result::Result::Ok(())
				})
			}
		}
		Fields::Unnamed(fields) => {
			let writes = fields.unnamed.iter().enumerate().map(|(i, _)| {
				let index = Index::from(i);
				quote! { ::leptoskit_pod::PodEncode::encode(&self.#index, w)?; }
			});
			quote! {
				::leptoskit_pod::encode_struct_v0(w, |w| {
					#(#writes)*
					::core::result::Result::Ok(())
				})
			}
		}
		Fields::Unit => quote! {
			::leptoskit_pod::encode_struct_v0(w, |_w| ::core::result::Result::Ok(()))
		},
	}
}

fn decode_struct(data: &DataStruct) -> proc_macro2::TokenStream {
	match &data.fields {
		Fields::Named(fields) => {
			let idents: Vec<_> = fields
				.named
				.iter()
				.map(|f| f.ident.as_ref().expect("named"))
				.collect();
			let reads = idents.iter().map(|ident| {
				quote! { #ident: ::leptoskit_pod::PodDecode::decode(r)?, }
			});
			quote! {
				::leptoskit_pod::decode_struct_version(r, 0, |r| {
					::core::result::Result::Ok(Self {
						#(#reads)*
					})
				})
			}
		}
		Fields::Unnamed(fields) => {
			let reads = fields.unnamed.iter().map(|_| {
				quote! { ::leptoskit_pod::PodDecode::decode(r)?, }
			});
			quote! {
				::leptoskit_pod::decode_struct_version(r, 0, |r| {
					::core::result::Result::Ok(Self(#(#reads)*))
				})
			}
		}
		Fields::Unit => quote! {
			::leptoskit_pod::decode_struct_version(r, 0, |_r| {
				::core::result::Result::Ok(Self)
			})
		},
	}
}

fn encode_enum(data: &DataEnum) -> proc_macro2::TokenStream {
	let arms = data.variants.iter().enumerate().map(|(disc, v)| {
		let ident = &v.ident;
		let disc = disc as u16;
		match &v.fields {
			Fields::Unit => quote! {
				Self::#ident => {
					w.variant(#disc);
					::core::result::Result::Ok(())
				}
			},
			Fields::Unnamed(fields) => {
				let bindings: Vec<_> = (0..fields.unnamed.len())
					.map(|i| syn::Ident::new(&format!("f{i}"), proc_macro2::Span::call_site()))
					.collect();
				let writes = bindings.iter().map(|b| {
					quote! { ::leptoskit_pod::PodEncode::encode(#b, w)?; }
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
					quote! { ::leptoskit_pod::PodEncode::encode(#ident, w)?; }
				});
				quote! {
					Self::#ident { #(#idents),* } => {
						w.variant(#disc);
						::leptoskit_pod::encode_struct_v0(w, |w| {
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

fn decode_enum(data: &DataEnum) -> proc_macro2::TokenStream {
	let arms = data.variants.iter().enumerate().map(|(disc, v)| {
		let ident = &v.ident;
		let disc = disc as u16;
		match &v.fields {
			Fields::Unit => quote! {
				#disc => ::core::result::Result::Ok(Self::#ident)
			},
			Fields::Unnamed(fields) => {
				let reads = fields.unnamed.iter().map(|_| {
					quote! { ::leptoskit_pod::PodDecode::decode(r)?, }
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
					quote! { #ident: ::leptoskit_pod::PodDecode::decode(r)?, }
				});
				quote! {
					#disc => ::leptoskit_pod::decode_struct_version(r, 0, |r| {
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
			other => ::core::result::Result::Err(::leptoskit_pod::Error::InvalidTag(other)),
		}
	}
}

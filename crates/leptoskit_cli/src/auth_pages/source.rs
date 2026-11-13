//! Inspect crate roots without expanding macros or claiming runtime wiring.

use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use syn::ext::IdentExt as _;
use syn::visit::Visit;

/// Resolve both roots before writing screens, configuration, or features.
pub(super) fn preflight(layout: &super::layout::Layout, database: bool) -> Result<Source> {
	let client = Source::read(&layout.client.entry)?;
	client.module("auth")?;

	if database && let Some(server) = &layout.server {
		Source::read(&server.entry)?.module("sign_in")?;
		let directory = server.entry.parent().context("server target directory")?;
		ensure!(
			!directory.join("sign_in/mod.rs").try_exists()?,
			"{} already exists; rename or integrate that module before generating sign-in",
			directory.join("sign_in/mod.rs").display()
		);
	}

	Ok(client)
}

pub(super) struct Source {
	file: syn::File,
	path: String,
	pub calls: Calls,
}

impl Source {
	pub fn read(path: &Path) -> Result<Self> {
		let text = std::fs::read_to_string(path)?;
		let file = syn::parse_file(&text)
			.with_context(|| format!("parse {} before generating auth", path.display()))?;
		let mut calls = Calls::default();
		calls.visit_file(&file);

		Ok(Self {
			file,
			path: path.display().to_string(),
			calls,
		})
	}

	/// Existing out-of-line declarations retain their visibility and
	/// attributes. Inline or redirected modules cannot consume files at the
	/// generated path.
	pub fn module(&self, name: &str) -> Result<bool> {
		let mut found = false;

		for item in &self.file.items {
			let syn::Item::Mod(module) = item else {
				continue;
			};

			if module.ident.unraw() != name {
				continue;
			}

			ensure!(
				!found,
				"{} declares `{name}` more than once; integrate the generated module manually",
				self.path
			);
			ensure!(
				module.content.is_none(),
				"{} has an inline `{name}` module; rename or integrate it before generating auth",
				self.path
			);

			for attribute in &module.attrs {
				ensure!(
					!redirects(&attribute.meta)?,
					"{} redirects `{name}` with #[path]; integrate that module before generating \
					 auth",
					self.path
				);
			}

			found = true;
		}

		Ok(found)
	}
}

/// Check every conditional branch: generation cannot assume which features
/// the app will enable later, and nested `cfg_attr` can change a module's file.
fn redirects(meta: &syn::Meta) -> Result<bool> {
	if meta.path().is_ident("path") {
		return Ok(true);
	}

	if let syn::Meta::List(list) = meta
		&& list.path.is_ident("cfg_attr")
	{
		let arguments = list.parse_args_with(
			syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
		)?;

		for argument in arguments.iter().skip(1) {
			if redirects(argument)? {
				return Ok(true);
			}
		}
	}

	Ok(false)
}

#[derive(Default)]
#[allow(
	clippy::struct_excessive_bools,
	reason = "independent syntax observations, not a runtime state machine"
)]
pub(super) struct Calls {
	pub provide: bool,
	pub gate: bool,
	pub navigate: bool,
	pub service: bool,
	pub provider: bool,
}

impl<'ast> Visit<'ast> for Calls {
	fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
		if !test_only(&node.attrs) {
			syn::visit::visit_item_fn(self, node);
		}
	}

	fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
		if !test_only(&node.attrs) {
			syn::visit::visit_item_mod(self, node);
		}
	}

	fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
		self.provide |= function(&node.func, "auth", "provide");
		self.service |= function(&node.func, "sign_in", "service");
		self.provider |= function(&node.func, "sign_in", "provider");
		syn::visit::visit_expr_call(self, node);
	}

	fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
		if let syn::Expr::Call(call) = node.receiver.as_ref()
			&& function(&call.func, "auth", "screens")
		{
			self.gate |= node.method == "gate";
			self.navigate |= node.method == "navigate_with";
		}

		syn::visit::visit_expr_method_call(self, node);
	}
}

fn test_only(attributes: &[syn::Attribute]) -> bool {
	attributes.iter().any(|attribute| {
		attribute.path().is_ident("test")
			|| (attribute.path().is_ident("cfg")
				&& attribute
					.parse_args::<syn::Meta>()
					.is_ok_and(|meta| meta.path().is_ident("test")))
	})
}

fn function(expression: &syn::Expr, module: &str, name: &str) -> bool {
	let syn::Expr::Path(expression) = expression else {
		return false;
	};

	let segments = &expression.path.segments;
	let offset = usize::from(
		segments
			.first()
			.is_some_and(|segment| segment.ident == "crate" || segment.ident == "self"),
	);

	expression.qself.is_none()
		&& segments.len() == offset + 2
		&& segments[offset].ident == module
		&& segments[offset + 1].ident == name
}

#[cfg(test)]
mod tests;

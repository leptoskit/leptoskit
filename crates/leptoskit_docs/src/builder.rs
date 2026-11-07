//! Builder calls for code written with `view!` markup.
//!
//! The book writes component trees as leptoskit `view!` markup. Readers who
//! write interfaces with builder calls can switch every sample to the same
//! tree written as `View().class(…).child(Text(…))`, and this module derives
//! that variant when the site is built, so the markup stays the one source.
//!
//! Only the `view!` invocations change. Everything around them, such as
//! imports, functions, and comments, and every expression inside the markup is
//! kept as written; a `view!` nested in an expression is converted where it
//! stands. leptos's `view!` is left alone: one written as `leptos::view!`,
//! one whose markup has lowercase HTML tags, or a bare `view!` in a block that
//! imports leptos's but not leptoskit's.
//!
//! Built-in components become their builder functions, with the values those
//! take as arguments: `<Text>"Hi"</Text>` is `Text("Hi")`, and
//! `<Image source=photo />` is `Image(photo)`. A `#[component]` defined on
//! the same page is called positionally, as builder code calls it; one
//! defined elsewhere goes through its props builder,
//! `Card::builder().title(…).build()`. The result is laid out the way
//! rustfmt lays out the book's own code.

use std::cell::Cell;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ops::Range;

use proc_macro2::Delimiter;
use proc_macro2::Group;
use proc_macro2::Span;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use quote::ToTokens;
use rstml::Infallible;
use rstml::Parser;
use rstml::ParserConfig;
use rstml::node::KVAttributeValue;
use rstml::node::KeyedAttribute;
use rstml::node::KeyedAttributeValue;
use rstml::node::Node;
use rstml::node::NodeAttribute;
use rstml::node::NodeBlock;
use rstml::node::NodeElement;
use rstml::node::NodeName;
use rstml::node::NodeNameFragment;
use syn::Expr;
use syn::FnArg;
use syn::Lit;
use syn::Meta;
use syn::Pat;
use syn::Stmt;
use syn::Token;
use syn::Type;
use syn::parse::Parser as _;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;

/// The width rustfmt wraps the book's code at.
const MAX_WIDTH: usize = 100;
/// The widest chain or argument list rustfmt keeps on one line.
const SHORT_WIDTH: usize = 60;
/// How wide a tab is when measuring a line.
const TAB_WIDTH: usize = 4;
/// How many lines a chain's overflowing last call takes before it wins
/// outright; a shorter one wins only when the call takes as many lines on a
/// line of its own.
const TALL_OVERFLOW: usize = 5;
/// Where a comment outside every tag can go instead.
const OUT_OF_THE_VIEW: &str = "move it out of the `view!`";

/// Iterator and `Option` adapters: a child built with one of these yields
/// elements, so it goes to `.children(…)`.
const ADAPTERS: [&str; 11] = [
	"chain",
	"collect",
	"filter",
	"filter_map",
	"flat_map",
	"flatten",
	"map",
	"rev",
	"skip",
	"take",
	"then",
];

/// Why a code block's markup has no builder form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unconvertible {
	/// The 1-indexed line in the block.
	pub line: usize,
	/// The 1-indexed column in the block.
	pub column: usize,
	/// What cannot be converted, written for the author of the page.
	pub message: String,
}

/// The `#[component]` functions the code on one page defines, so markup
/// that uses them converts to calls with their arguments in order.
#[derive(Debug, Default)]
pub(crate) struct PageComponents {
	/// Each component's signature, or `None` when the page defines two
	/// different components with the same name.
	signatures: HashMap<String, Option<Signature>>,
}

impl PageComponents {
	/// The components defined in a page's Rust code blocks.
	pub(crate) fn collect<'a>(blocks: impl IntoIterator<Item = &'a str>) -> Self {
		let mut signatures: HashMap<String, Option<Signature>> = HashMap::new();

		for block in blocks {
			if !block.contains("component") {
				continue;
			}

			let Ok(tokens) = block.parse::<TokenStream>() else {
				continue;
			};
			let leptos = bare_view_is_leptos(&tokens);
			let mut found = Vec::new();
			find_components(block, tokens, leptos, &mut found);

			for (name, signature) in found {
				record_signature(&mut signatures, name, signature);
			}
		}

		Self { signatures }
	}

	fn get(&self, name: &str) -> Option<&Signature> {
		self.signatures.get(name).and_then(Option::as_ref)
	}
}

/// Records a component's signature. A component defined twice with
/// different signatures has none: the page does not say which one a tag
/// means.
fn record_signature(
	signatures: &mut HashMap<String, Option<Signature>>,
	name: String,
	signature: Signature,
) {
	match signatures.entry(name) {
		Entry::Vacant(entry) => {
			entry.insert(Some(signature));
		}
		Entry::Occupied(mut entry) if entry.get().as_ref() != Some(&signature) => {
			entry.insert(None);
		}
		Entry::Occupied(_) => {}
	}
}

/// What a `#[component]` takes and returns.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Signature {
	params: Vec<Param>,
	/// Whether it returns an `Element`, which `.child(…)` takes as it is.
	returns_element: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Param {
	name: String,
	kind: ParamKind,
	/// `#[prop(into)]`: the value converts into the parameter's type.
	into: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParamKind {
	Required,
	/// `#[prop(optional)]` and `#[prop(default = …)]`.
	Optional {
		/// The `default = …` expression, as written.
		default: Option<String>,
		/// Whether a set value is wrapped in `Some`: an `Option` type
		/// without `optional_no_strip`.
		wrap: bool,
		/// The value when left out and no default is given.
		absent: &'static str,
	},
	/// `children: Children`.
	Children,
}

/// The block with every leptoskit `view!` written as builder calls, or
/// `None` when it has no leptoskit markup.
pub(crate) fn builder_variant(
	source: &str,
	components: &PageComponents,
) -> Result<Option<String>, Unconvertible> {
	if !source.contains("view!") {
		return Ok(None);
	}

	let tokens = source.parse::<TokenStream>().map_err(|error| {
		unconvertible(
			error.span(),
			format!("this block is not Rust that leptoskit can read: {error}"),
		)
	})?;
	let converter = Converter {
		source,
		unit: indent_unit(source),
		leptos: bare_view_is_leptos(&tokens),
		components,
	};
	let mut replacements = Vec::new();
	converter.collect(tokens, &mut replacements)?;

	if replacements.is_empty() {
		return Ok(None);
	}

	Ok(Some(splice(source, 0..source.len(), &replacements)))
}

struct Converter<'a> {
	source: &'a str,
	/// One level of indentation, as the block writes it.
	unit: String,
	/// Whether a bare `view!` in this block is leptos's.
	leptos: bool,
	components: &'a PageComponents,
}

/// How the value a child expression builds is passed to its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
	/// An `Element`: `.child(…)`.
	Element,
	/// Another view, such as a component returning `impl IntoView`:
	/// `.child(….into_view())`.
	View,
	/// Text: `.child(Text(…))`.
	Text,
	/// Several elements, such as an `Option` or an iterator:
	/// `.children(…)`.
	Elements,
	/// Anything `view!` accepts as a child: `Fragment::with(…)`.
	Any,
}

/// A converted tag and how its parent takes it.
struct Tree {
	piece: Piece,
	shape: Shape,
}

/// An attribute read from markup.
struct Attribute {
	/// The builder method it calls: `class`, or `on_press` for `on:press`.
	method: String,
	/// Its arguments: the value, a tuple's elements, or `true`.
	args: Vec<Piece>,
	/// The value as written, for literals and precedence.
	value: Option<Expr>,
	comments: Vec<String>,
	span: Span,
}

/// A child read from markup.
enum Child {
	/// Literal text: its literal as written, and its value.
	Text { literal: String, value: String },
	/// An expression in braces.
	Expr {
		expr: Box<Expr>,
		code: Piece,
		comments: Vec<String>,
		span: Span,
	},
	/// A nested tag.
	Tag(Tree),
	/// A comment between children, and the byte it starts at.
	Comment { text: String, at: usize },
}

impl Converter<'_> {
	/// Finds the leptoskit `view!` invocations in `tokens`, and records each
	/// one's byte range and builder code.
	fn collect(
		&self,
		tokens: TokenStream,
		out: &mut Vec<(Range<usize>, String)>,
	) -> Result<(), Unconvertible> {
		let trees = tokens.into_iter().collect::<Vec<_>>();
		let mut index = 0;

		while index < trees.len() {
			if let Some(invocation) = invocation_at(&trees, index)
				&& let Some(nodes) = self.markup(
					&invocation.path,
					invocation.group.stream(),
					invocation.group.span_open(),
				)? {
				let piece = self.root(&nodes, inner_range(&invocation.group))?;
				let range = invocation.range();
				out.push((range.clone(), self.render_at(&piece, range.start)));
				index += 3;
				continue;
			}

			if let TokenTree::Group(group) = &trees[index] {
				self.collect_group(group, callee_before(&trees, index).as_deref(), out)?;
			}

			index += 1;
		}

		Ok(())
	}

	/// The replacements for a group: the whole group when it is a call's
	/// only argument and a `view!`, or else the `view!`s inside it.
	fn collect_group(
		&self,
		group: &Group,
		callee: Option<&str>,
		out: &mut Vec<(Range<usize>, String)>,
	) -> Result<(), Unconvertible> {
		match self.sole_argument(group, callee)? {
			Some(replacement) => out.push((group.span().byte_range(), replacement)),
			None => self.collect(group.stream(), out)?,
		}

		Ok(())
	}

	/// The markup of a `view!` invocation when it is leptoskit's, or `None`
	/// for leptos's and any other macro named `view`.
	fn markup(
		&self,
		path: &[String],
		tokens: TokenStream,
		open: Span,
	) -> Result<Option<Vec<Node>>, Unconvertible> {
		let explicit = match path.split_last() {
			Some((_, [])) if self.leptos => return Ok(None),
			Some((_, [])) => false,

			Some((_, [first, ..])) if first.starts_with("leptoskit") => true,
			_ => return Ok(None),
		};

		let parser = Parser::new(ParserConfig::new().recover_block(true));
		let (nodes, diagnostics) = parser.parse_recoverable(tokens).split();
		let nodes = nodes.unwrap_or_default();

		if !explicit && nodes.iter().any(has_html_tag) {
			return Ok(None);
		}

		if !diagnostics.is_empty() {
			return Err(unconvertible(open, "this `view!` markup does not parse"));
		}

		Ok(Some(nodes))
	}

	/// One root tag stays a tree; anything else is a `Fragment`.
	fn root(&self, nodes: &[Node], inside: Range<usize>) -> Result<Piece, Unconvertible> {
		let mut at = inside.start;

		for node in nodes {
			let range = node.span().byte_range();

			if let Node::Comment(comment) = node {
				return Err(unconvertible(
					comment.span(),
					"a comment outside every tag has no place in builder calls; move it out of \
					 the `view!`",
				));
			}

			self.no_comments(at..range.start, OUT_OF_THE_VIEW)?;
			at = range.end;
		}

		self.no_comments(at..inside.end, OUT_OF_THE_VIEW)?;

		match nodes {
			[Node::Element(element)] => Ok(self.element(element)?.piece),
			[Node::Fragment(fragment)] => {
				let children = self.children(&fragment.children, fragment_range(fragment))?;
				self.fragment(children)
			}
			_ => {
				let children = self.children(nodes, inside)?;
				self.fragment(children)
			}
		}
	}

	/// A tag as builder calls. Most of the work is refusing tags that have
	/// no builder form, each with its own message: HTML tags, generic
	/// arguments, and lowercase names all read as components otherwise.
	fn element(&self, element: &NodeElement<Infallible>) -> Result<Tree, Unconvertible> {
		let name = element.name();
		let NodeName::Path(path) = name else {
			return Err(unconvertible(
				name.span(),
				format!(
					"`{}` is not a component; tags name a component, like `<View>`",
					name.to_token_stream()
				),
			));
		};

		if !element.open_tag.generics.args.is_empty() {
			return Err(unconvertible(
				element.open_tag.generics.span(),
				"tags take no generic arguments",
			));
		}

		let segments = path
			.path
			.segments
			.iter()
			.map(|segment| segment.ident.to_string())
			.collect::<Vec<_>>();
		let written = self.text(path.span());
		let Some(last) = segments.last() else {
			return Err(unconvertible(path.span(), "a tag needs a name"));
		};

		if segments.len() == 1 && last.starts_with(char::is_lowercase) {
			return Err(unconvertible(
				path.span(),
				format!(
					"`<{last}>` is not a component; component names start with a capital letter"
				),
			));
		}

		let attributes = self.attributes(element)?;
		let inside = children_range(element);
		let children = self.children(&element.children, inside)?;

		let builtin = match segments.as_slice() {
			[name] => builtin(name),

			[.., module, name] if module == "helpers" || module == "components" => builtin(name),
			_ => None,
		};

		match builtin {
			Some(start) => self.builtin(&written, last, start, attributes, children, element),
			None => self.component(&written, last, attributes, children, element),
		}
	}

	/// A built-in component: its builder function, then its attributes and
	/// children as builder calls.
	fn builtin(
		&self,
		written: &str,
		name: &str,
		start: Start,
		mut attributes: Vec<Attribute>,
		children: Vec<Child>,
		element: &NodeElement<Infallible>,
	) -> Result<Tree, Unconvertible> {
		let tag = element.name().span();
		let (args, children) = match start {
			Start::Bare => (Vec::new(), children),
			Start::Content => {
				let (content, rest) = content(name, children, true, element)?;
				(vec![content], rest)
			}
			Start::Heading => {
				let level = heading_level(&mut attributes);
				let (content, rest) = content(name, children, true, element)?;
				(vec![level, content], rest)
			}
			Start::Region => {
				let (content, rest) = content(name, children, false, element)?;
				(vec![content], rest)
			}
			Start::Attribute { name: key, default } => {
				let value =
					attribute_argument(&mut attributes, key, default)?.ok_or_else(|| {
						unconvertible(tag, format!("`{name}()` needs `{key}=…` to call it with"))
					})?;
				(vec![value], children)
			}
			Start::Radio => (radio_arguments(&mut attributes, tag)?, children),
		};

		let mut links = attribute_links(attributes);
		self.child_links(children, &mut links)?;

		Ok(Tree {
			piece: chain(
				Piece::Call {
					callee: written.to_owned(),
					args,
				},
				links,
			),
			shape: Shape::Element,
		})
	}

	/// A `#[component]`: a positional call when the page defines it, or its
	/// props builder when it does not.
	fn component(
		&self,
		written: &str,
		name: &str,
		attributes: Vec<Attribute>,
		children: Vec<Child>,
		element: &NodeElement<Infallible>,
	) -> Result<Tree, Unconvertible> {
		let Some(signature) = self.components.get(name) else {
			return self.props_builder(written, attributes, children);
		};

		let tag = element.name().span();
		let mut attributes = attributes.into_iter().map(Some).collect::<Vec<_>>();
		let mut args = signature
			.params
			.iter()
			.map(|param| positional_argument(param, &mut attributes, name, tag))
			.collect::<Result<Vec<_>, _>>()?;

		if let Some(attribute) = attributes.into_iter().flatten().next() {
			return Err(unconvertible(
				attribute.span,
				format!("`<{name}>` has no prop `{}`", attribute.method),
			));
		}

		if !children.is_empty() {
			let slot = signature
				.params
				.iter()
				.position(|param| param.kind == ParamKind::Children)
				.ok_or_else(|| unconvertible(tag, format!("`<{name}>` takes no children")))?;
			args[slot] = self.fragment(children)?;
		}

		let shape = if signature.returns_element {
			Shape::Element
		} else {
			Shape::View
		};

		Ok(Tree {
			piece: Piece::Call {
				callee: written.to_owned(),
				args,
			},
			shape,
		})
	}

	/// A component the page does not define, or defines twice: its props
	/// builder, with each attribute as a call and the children as one.
	fn props_builder(
		&self,
		written: &str,
		attributes: Vec<Attribute>,
		children: Vec<Child>,
	) -> Result<Tree, Unconvertible> {
		let mut links = attribute_links(attributes);

		if !children.is_empty() {
			links.push(link("children", vec![self.fragment(children)?]));
		}

		links.push(link("build", Vec::new()));

		Ok(Tree {
			piece: chain(code(format!("{written}::builder()")), links),
			shape: Shape::View,
		})
	}

	/// The builder calls that add `children` to a container.
	fn child_links(
		&self,
		children: Vec<Child>,
		links: &mut Vec<Link>,
	) -> Result<(), Unconvertible> {
		let mut comments = Vec::new();
		// Children of unknown shape, which only a fragment can hold.
		let mut run: Vec<(Vec<String>, Piece)> = Vec::new();
		let mut trailing = None;

		for child in children {
			let (method, arg) = match child {
				Child::Comment { text, at } => {
					trailing.get_or_insert(at);
					comments.push(text);
					continue;
				}
				Child::Text { literal, .. } => ("child", text_call(code(literal))),
				Child::Tag(tree) => ("child", element_of(tree)),
				Child::Expr {
					expr,
					code: piece,
					comments: inner,
					..
				} => {
					trailing = None;
					comments.extend(inner);

					match adding_call(self.shape(&expr), piece) {
						Ok(call) => call,
						Err(piece) => {
							run.push((std::mem::take(&mut comments), piece));
							continue;
						}
					}
				}
			};

			trailing = None;
			flush(&mut run, links);
			links.push(Link {
				comments: std::mem::take(&mut comments),
				method: method.to_owned(),
				args: vec![arg],
			});
		}

		flush(&mut run, links);
		self.no_trailing(trailing, &comments)
	}

	/// An error for comments after the last child, which builder calls have
	/// no place for.
	fn no_trailing(&self, at: Option<usize>, comments: &[String]) -> Result<(), Unconvertible> {
		match (at, comments.first()) {
			(Some(at), Some(comment)) => {
				let (line, column) = self.position(at);
				Err(Unconvertible {
					line,
					column,
					message: format!(
						"the comment `{comment}` follows the last child; move it above a child"
					),
				})
			}
			_ => Ok(()),
		}
	}

	/// `Fragment::new().with(…)` for a list of children.
	fn fragment(&self, children: Vec<Child>) -> Result<Piece, Unconvertible> {
		let mut comments = Vec::new();
		let mut trailing = None;
		let mut links = Vec::new();

		for child in children {
			let piece = match child {
				Child::Comment { text, at } => {
					trailing.get_or_insert(at);
					comments.push(text);
					continue;
				}
				Child::Text { literal, .. } => code(literal),
				Child::Tag(tree) => tree.piece,
				Child::Expr {
					code: piece,
					comments: inner,
					..
				} => {
					comments.extend(inner);
					piece
				}
			};

			trailing = None;
			links.push(Link {
				comments: std::mem::take(&mut comments),
				method: "with".to_owned(),
				args: vec![piece],
			});
		}

		self.no_trailing(trailing, &comments)?;
		Ok(chain(code("Fragment::new()"), links))
	}

	fn attributes(
		&self,
		element: &NodeElement<Infallible>,
	) -> Result<Vec<Attribute>, Unconvertible> {
		let mut at = element.open_tag.name.span().byte_range().end;
		let mut attributes = Vec::new();

		for attribute in element.attributes() {
			let range = attribute.span().byte_range();
			let comments = self.comments(at..range.start);
			at = range.end;

			let NodeAttribute::Attribute(attribute) = attribute else {
				return Err(unconvertible(
					attribute.span(),
					"attributes need a name, like `class=…` or `on:press=…`",
				));
			};

			attributes.push(self.attribute(attribute, comments)?);
		}

		let end = element.open_tag.end_tag.span().byte_range().start;
		self.no_comments(at..end, "move it above the tag")?;
		Ok(attributes)
	}

	fn attribute(
		&self,
		attribute: &KeyedAttribute,
		comments: Vec<String>,
	) -> Result<Attribute, Unconvertible> {
		let method = method(&attribute.key)?;
		let span = attribute.key.span();

		let (args, value) = match &attribute.possible_value {
			KeyedAttributeValue::None => (vec![code("true")], None),
			KeyedAttributeValue::Value(value) => self.attribute_value(&value.value)?,
			KeyedAttributeValue::Binding(binding) => {
				return Err(unconvertible(
					binding.span(),
					format!("write the handler as a value: `{}=|…| …`", attribute.key),
				));
			}
		};

		Ok(Attribute {
			method,
			args,
			value,
			comments,
			span,
		})
	}

	/// An attribute's value as call arguments: a tuple's values each, or the
	/// one expression with its braces removed, which is kept too.
	fn attribute_value(
		&self,
		value: &KVAttributeValue,
	) -> Result<(Vec<Piece>, Option<Expr>), Unconvertible> {
		match value {
			KVAttributeValue::Expr(Expr::Tuple(tuple)) => {
				let args = tuple
					.elems
					.iter()
					.map(|element| self.code(element))
					.collect::<Result<Vec<_>, _>>()?;
				Ok((args, None))
			}
			KVAttributeValue::Expr(expr) => {
				let expr = self.unbrace(expr);
				Ok((vec![self.code(&expr)?], Some(expr)))
			}
			KVAttributeValue::InvalidBraced(block) => {
				Err(unconvertible(block.span(), "this value does not parse"))
			}
		}
	}

	/// The children of a tag or fragment, with the comments between them.
	fn children(&self, nodes: &[Node], inside: Range<usize>) -> Result<Vec<Child>, Unconvertible> {
		let mut children = Vec::new();
		let mut at = inside.start;

		for node in nodes {
			let range = node.span().byte_range();
			children.extend(self.comment_children(at..range.start));
			at = range.end;
			children.extend(self.node_children(node, range.start)?);
		}

		children.extend(self.comment_children(at..inside.end));
		Ok(children)
	}

	/// The children one node adds: itself, or a fragment's children, or the
	/// comments in an empty block. `start` is the byte the node starts at.
	fn node_children(&self, node: &Node, start: usize) -> Result<Vec<Child>, Unconvertible> {
		let child = match node {
			Node::Element(element) => Child::Tag(self.element(element)?),
			Node::Fragment(fragment) => {
				return self.children(&fragment.children, fragment_range(fragment));
			}
			Node::Text(text) => {
				Child::Text {
					literal: self.text(text.value.span()),
					value: text.value.value(),
				}
			}
			Node::RawText(text) => {
				let value = text.to_string_best();
				Child::Text {
					literal: format!("{value:?}"),
					value,
				}
			}
			Node::Comment(comment) => {
				Child::Comment {
					text: format!("// {}", comment.value.to_string_best().trim()),
					at: start,
				}
			}
			Node::Block(NodeBlock::ValidBlock(block)) => return self.block_children(block),
			Node::Block(invalid @ NodeBlock::Invalid(_)) => {
				return Err(unconvertible(invalid.span(), "this block does not parse"));
			}
			Node::Doctype(_) | Node::Custom(_) => {
				return Err(unconvertible(node.span(), "a view has no doctype"));
			}
		};

		Ok(vec![child])
	}

	/// A braced child: its one expression with the comments around it, a
	/// block of statements kept as written, or the comments of an empty
	/// block.
	fn block_children(&self, block: &syn::Block) -> Result<Vec<Child>, Unconvertible> {
		let braces = block.brace_token.span.join().byte_range();
		let inner = braces.start + 1..braces.end - 1;

		if block.stmts.is_empty() {
			return Ok(self.comment_children(inner));
		}

		let Some(expr) = only_expr(block) else {
			let expr = Expr::Block(syn::ExprBlock {
				attrs: Vec::new(),
				label: None,
				block: block.clone(),
			});
			return Ok(vec![Child::Expr {
				code: self.code_range(braces, expr.to_token_stream())?,
				expr: Box::new(expr),
				comments: Vec::new(),
				span: block.span(),
			}]);
		};

		let range = expr.span().byte_range();
		let mut comments = self.comments(inner.start..range.start);
		comments.extend(self.comments(range.end..inner.end));

		Ok(vec![Child::Expr {
			code: self.code(&expr)?,
			span: expr.span(),
			expr: Box::new(expr),
			comments,
		}])
	}

	/// The comments in `range`, a line each, with the byte each starts at.
	fn comment_children(&self, range: Range<usize>) -> Vec<Child> {
		if range.start >= range.end {
			return Vec::new();
		}

		let mut at = range.start;
		let mut comments = Vec::new();

		for line in self.source[range].split_inclusive('\n') {
			let text = line.trim();

			if !text.is_empty() {
				comments.push(Child::Comment {
					text: text.to_owned(),
					at: at + line.len() - line.trim_start().len(),
				});
			}

			at += line.len();
		}

		comments
	}

	/// How a parent takes the value `expr` builds, read from how it is
	/// written: the converter has no types to go on.
	fn shape(&self, expr: &Expr) -> Shape {
		match expr {
			Expr::Lit(literal) if matches!(literal.lit, Lit::Str(_)) => Shape::Text,
			Expr::Macro(mac) => self.macro_shape(&mac.mac),
			Expr::Path(path) if path.path.is_ident("children") => Shape::Elements,
			Expr::MethodCall(call) if ADAPTERS.contains(&call.method.to_string().as_str()) => {
				Shape::Elements
			}
			Expr::MethodCall(_) => Shape::Element,
			Expr::Call(call) => self.call_shape(call),
			Expr::Paren(inner) => self.shape(&inner.expr),
			_ => Shape::Any,
		}
	}

	/// `format!` builds text, and a `view!` one element or several.
	fn macro_shape(&self, mac: &syn::Macro) -> Shape {
		let path = mac
			.path
			.segments
			.iter()
			.map(|segment| segment.ident.to_string())
			.collect::<Vec<_>>();

		match path.last().map(String::as_str) {
			Some("format") => Shape::Text,
			Some("view") => {
				match self.markup(&path, mac.tokens.clone(), mac.delimiter.span().open()) {
					Ok(Some(nodes)) if is_one_tag(&nodes) => Shape::Element,
					Ok(Some(_)) => Shape::Elements,
					_ => Shape::Any,
				}
			}
			_ => Shape::Any,
		}
	}

	/// A call builds an element, unless it calls a component the page
	/// defines that returns a view.
	fn call_shape(&self, call: &syn::ExprCall) -> Shape {
		let Expr::Path(path) = &*call.func else {
			return Shape::Element;
		};
		let returns_view = path.path.segments.last().is_some_and(|segment| {
			self.components
				.get(&segment.ident.to_string())
				.is_some_and(|signature| !signature.returns_element)
		});

		if returns_view {
			Shape::View
		} else {
			Shape::Element
		}
	}

	/// `{ value }` is `value`, unless the braces also hold a comment.
	fn unbrace(&self, expr: &Expr) -> Expr {
		if let Expr::Block(block) = expr
			&& block.attrs.is_empty()
			&& block.label.is_none()
			&& let Some(inner) = only_expr(&block.block)
		{
			let braces = block.span().byte_range();
			let range = inner.span().byte_range();

			if self.comments(braces.start + 1..range.start).is_empty()
				&& self.comments(range.end..braces.end - 1).is_empty()
			{
				return inner;
			}
		}

		expr.clone()
	}

	/// The builder code for a `view!` that is a call's only argument, as in
	/// `render(view! { … })`, laid out with the call's parentheses the way
	/// rustfmt lays out a single argument.
	fn sole_argument(
		&self,
		group: &Group,
		callee: Option<&str>,
	) -> Result<Option<String>, Unconvertible> {
		let Some(callee) = callee else {
			return Ok(None);
		};

		if group.delimiter() != Delimiter::Parenthesis {
			return Ok(None);
		}

		let trees = group.stream().into_iter().collect::<Vec<_>>();
		let reference = trees.first().is_some_and(|tree| is_punct(tree, '&'));
		let rest = &trees[usize::from(reference)..];

		let Some(view_at) = rest.len().checked_sub(3) else {
			return Ok(None);
		};
		let Some(invocation) = invocation_at(rest, view_at) else {
			return Ok(None);
		};

		if invocation.start.byte_range().start != rest[0].span().byte_range().start {
			return Ok(None);
		}

		let Some(nodes) = self.markup(
			&invocation.path,
			invocation.group.stream(),
			invocation.group.span_open(),
		)?
		else {
			return Ok(None);
		};

		let mut piece = self.root(&nodes, inner_range(&invocation.group))?;

		if reference {
			piece = prefixed(piece, "&");
		}

		let open = group.span_open().byte_range().start;
		let line_start = self.source[..open]
			.rfind('\n')
			.map_or(0, |newline| newline + 1);
		let layout = Layout::new(&self.unit);
		Ok(Some(layout.args(
			callee,
			&[piece],
			self.line_indent(open),
			width(&self.source[line_start..open]),
			true,
		)))
	}

	/// An expression as builder code. A `view!` becomes builder calls, and
	/// calls and method chains written without comments become pieces, so
	/// both are laid out where they go; anything else is kept as written,
	/// with any `view!` inside it converted.
	fn code(&self, expr: &Expr) -> Result<Piece, Unconvertible> {
		if let Expr::Macro(mac) = expr
			&& let Some(piece) = self.view_macro(&mac.mac)?
		{
			return Ok(piece);
		}

		let range = expr.span().byte_range();
		let written = &self.source[range.clone()];
		let commented = written.contains("//") || written.contains("/*");

		match expr {
			Expr::Call(call)
				if !commented && call.attrs.is_empty() && matches!(&*call.func, Expr::Path(_)) =>
			{
				Ok(Piece::Call {
					callee: self.text(call.func.span()),
					args: self.code_args(&call.args)?,
				})
			}

			Expr::MethodCall(call) if !commented && call.attrs.is_empty() => self.method_call(call),
			_ => self.code_range(range, expr.to_token_stream()),
		}
	}

	/// A method call as one more link on its receiver's chain.
	fn method_call(&self, call: &syn::ExprMethodCall) -> Result<Piece, Unconvertible> {
		let method = match &call.turbofish {
			Some(turbofish) => format!("{}{}", call.method, self.text(turbofish.span())),
			None => call.method.to_string(),
		};
		let link = Link {
			comments: Vec::new(),
			method,
			args: self.code_args(&call.args)?,
		};

		Ok(match self.code(&call.receiver)? {
			Piece::Chain { root, mut links } => {
				links.push(link);
				Piece::Chain { root, links }
			}
			receiver => chain(receiver, vec![link]),
		})
	}

	fn code_args(&self, args: &Punctuated<Expr, Token![,]>) -> Result<Vec<Piece>, Unconvertible> {
		args.iter().map(|arg| self.code(arg)).collect()
	}

	/// The builder calls for a leptoskit `view!`, or `None` for any other
	/// macro.
	fn view_macro(&self, mac: &syn::Macro) -> Result<Option<Piece>, Unconvertible> {
		let path = mac
			.path
			.segments
			.iter()
			.map(|segment| segment.ident.to_string())
			.collect::<Vec<_>>();

		if path.last().map(String::as_str) != Some("view") {
			return Ok(None);
		}

		let delimiters = mac.delimiter.span();
		let Some(nodes) = self.markup(&path, mac.tokens.clone(), delimiters.open())? else {
			return Ok(None);
		};

		let range = delimiters.join().byte_range();
		self.root(&nodes, range.start + 1..range.end - 1).map(Some)
	}

	fn code_range(&self, range: Range<usize>, tokens: TokenStream) -> Result<Piece, Unconvertible> {
		let mut replacements = Vec::new();
		self.collect(tokens, &mut replacements)?;
		Ok(Piece::Code {
			text: splice(self.source, range.clone(), &replacements),
			base: self.line_indent(range.start).to_owned(),
		})
	}

	/// `piece` laid out where the source's `view!` started.
	fn render_at(&self, piece: &Piece, at: usize) -> String {
		let line_start = self.source[..at]
			.rfind('\n')
			.map_or(0, |newline| newline + 1);
		let layout = Layout::new(&self.unit);
		layout.render(
			piece,
			self.line_indent(at),
			width(&self.source[line_start..at]),
		)
	}

	/// The whitespace that indents the line holding byte `at`.
	fn line_indent(&self, at: usize) -> &str {
		let line_start = self.source[..at]
			.rfind('\n')
			.map_or(0, |newline| newline + 1);
		let line = &self.source[line_start..];
		&line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
	}

	fn text(&self, span: Span) -> String {
		self.source[span.byte_range()].to_owned()
	}

	/// The comments between two tokens: everything there but whitespace.
	fn comments(&self, range: Range<usize>) -> Vec<String> {
		if range.start >= range.end {
			return Vec::new();
		}

		self.source[range]
			.lines()
			.map(str::trim)
			.filter(|line| !line.is_empty())
			.map(str::to_owned)
			.collect()
	}

	/// An error for a comment in `range`, where builder calls have no place
	/// for one; `hint` says where it can go instead.
	fn no_comments(&self, range: Range<usize>, hint: &str) -> Result<(), Unconvertible> {
		match self.comment_children(range).into_iter().next() {
			Some(Child::Comment { text, at }) => {
				let (line, column) = self.position(at);
				Err(Unconvertible {
					line,
					column,
					message: format!("the comment `{text}` has no place in builder calls; {hint}"),
				})
			}
			_ => Ok(()),
		}
	}

	fn position(&self, at: usize) -> (usize, usize) {
		let before = &self.source[..at];
		let line = before.matches('\n').count() + 1;
		let column = before
			.rsplit('\n')
			.next()
			.map_or(0, |line| line.chars().count())
			+ 1;
		(line, column)
	}
}

/// The bytes between a tag's `>` and its closing tag.
fn children_range(element: &NodeElement<Infallible>) -> Range<usize> {
	let start = element.open_tag.end_tag.span().byte_range().end;
	let end = element
		.close_tag
		.as_ref()
		.map_or(start, |close| close.start_tag.span().byte_range().start);
	start..end
}

/// The bytes between a fragment's `<>` and `</>`.
fn fragment_range(fragment: &rstml::node::NodeFragment<Infallible>) -> Range<usize> {
	let start = fragment.tag_open.span().byte_range().end;
	let end = fragment
		.tag_close
		.as_ref()
		.map_or(start, |close| close.span().byte_range().start);
	start..end
}

/// The bytes inside a group's delimiters.
fn inner_range(group: &Group) -> Range<usize> {
	let range = group.span().byte_range();
	range.start + 1..range.end.saturating_sub(1).max(range.start + 1)
}

/// How a built-in component's builder function gets its arguments.
#[derive(Debug, Clone, Copy)]
enum Start {
	/// No arguments: `View()`.
	Bare,
	/// Text children are the one argument: `Text("Hi")`, `Button("Save")`.
	Content,
	/// `Heading(level, content)`, the level from a literal `aria_level`.
	Heading,
	/// A live region whose text children are its one argument:
	/// `Status("Saved")`.
	Region,
	/// One argument from an attribute, or a default when it is left out.
	Attribute {
		name: &'static str,
		default: Option<&'static str>,
	},
	/// `Radio(group, checked)`.
	Radio,
}

/// The builder function behind a built-in tag, and how it takes its
/// arguments: the `tag!` declarations in `leptoskit_native`.
fn builtin(name: &str) -> Option<Start> {
	let attribute = |name, default| Start::Attribute { name, default };
	Some(match name {
		"View" | "ScrollView" | "TextInput" | "PasswordInput" | "Row" | "Column" | "Wrap"
		| "Center" | "Spacer" | "ClipRect" | "Divider" | "VerticalDivider" | "Card"
		| "ListView" => Start::Bare,
		"Text" | "Button" => Start::Content,
		"Heading" => Start::Heading,
		"Status" | "Alert" => Start::Region,
		"Image" => attribute("source", Some("\"\"")),
		"Switch" | "Checkbox" => attribute("checked", Some("false")),
		"Slider" => attribute("slider", Some("SliderState::default()")),
		"LinearProgressIndicator" => attribute("progress", Some("Progress::default()")),
		"ActivityIndicator" => attribute("running", Some("true")),
		"Pressable" => attribute("accessibility_label", None),
		"Vector" => attribute("view_box", Some("ViewBox::UNIT")),
		"Rive" => attribute("rive_playback", None),
		"Radio" => Start::Radio,

		_ => return None,
	})
}

/// A text component's content and the children left over: its text
/// children joined, one expression, or a `format!` of both.
fn content(
	name: &str,
	children: Vec<Child>,
	takes_tags: bool,
	element: &NodeElement<Infallible>,
) -> Result<(Piece, Vec<Child>), Unconvertible> {
	let tag = element.name().span();

	if let Some(Child::Comment { text, .. }) = children
		.iter()
		.find(|child| matches!(child, Child::Comment { .. }))
	{
		return Err(unconvertible(
			tag,
			format!("the comment `{text}` in this `{name}`'s text has no place in a builder call"),
		));
	}

	if children.iter().any(|child| matches!(child, Child::Tag(_))) {
		return text_beside_tags(name, children, takes_tags, tag);
	}

	match children.as_slice() {
		[] if !takes_tags => {
			Err(unconvertible(
				tag,
				format!("`{name}()` takes its text as an argument; this one has none"),
			))
		}
		[] => Ok((code("\"\""), Vec::new())),
		[Child::Expr { code, .. }] => Ok((code.clone(), Vec::new())),
		_ if children
			.iter()
			.all(|child| matches!(child, Child::Text { .. })) =>
		{
			Ok((joined(&children), Vec::new()))
		}
		_ => Ok((formatted(&children)?, Vec::new())),
	}
}

/// The content of a container whose children include tags: its literal
/// text joined, with the tags left over as children.
fn text_beside_tags(
	name: &str,
	children: Vec<Child>,
	takes_tags: bool,
	tag: Span,
) -> Result<(Piece, Vec<Child>), Unconvertible> {
	if !takes_tags {
		return Err(unconvertible(
			tag,
			format!("`{name}()` takes text; its tag children have no builder form"),
		));
	}

	if let Some(Child::Expr { span, .. }) = children
		.iter()
		.find(|child| matches!(child, Child::Expr { .. }))
	{
		return Err(unconvertible(
			*span,
			format!("a `{name}` with tag children takes only literal text beside them"),
		));
	}

	let (texts, tags): (Vec<_>, Vec<_>) = children
		.into_iter()
		.partition(|child| matches!(child, Child::Text { .. }));

	Ok((joined(&texts), tags))
}

/// Literal text children as one literal.
fn joined(texts: &[Child]) -> Piece {
	match texts {
		[] => code("\"\""),
		[Child::Text { literal, .. }] => code(literal.clone()),
		_ => {
			let value = texts
				.iter()
				.filter_map(|child| {
					match child {
						Child::Text { value, .. } => Some(value.as_str()),
						_ => None,
					}
				})
				.collect::<String>();
			code(format!("{value:?}"))
		}
	}
}

/// Text and expressions joined with `format!`, as a text component joins
/// its children.
fn formatted(children: &[Child]) -> Result<Piece, Unconvertible> {
	let mut template = String::new();
	let mut args = Vec::new();

	for child in children {
		match child {
			Child::Text { value, .. } => {
				template.push_str(&value.replace('{', "{{").replace('}', "}}"));
			}
			Child::Expr {
				expr, code, span, ..
			} => {
				if matches!(**expr, Expr::Closure(_)) {
					return Err(unconvertible(
						*span,
						"a closure cannot join literal text; write the whole label as one `{move \
						 || format!(…)}`",
					));
				}

				// A variable goes inside the template, as in `{count}`.
				template.push('{');

				match inline_name(expr) {
					Some(name) => template.push_str(&name),
					None => args.push(code.clone()),
				}

				template.push('}');
			}
			Child::Tag(_) | Child::Comment { .. } => {
				unreachable!("tags and comments are handled before formatting")
			}
		}
	}

	let mut all = vec![code(format!("{template:?}"))];
	all.extend(args);
	Ok(Piece::Call {
		callee: "format!".to_owned(),
		args: all,
	})
}

/// The variable an expression names, which `format!` takes inline.
fn inline_name(expr: &Expr) -> Option<String> {
	let Expr::Path(path) = expr else {
		return None;
	};

	if path.qself.is_some() {
		return None;
	}

	path.path.get_ident().map(ToString::to_string)
}

/// Adds a run of children of unknown shape as one fragment, and empties
/// the run.
fn flush(run: &mut Vec<(Vec<String>, Piece)>, links: &mut Vec<Link>) {
	let mut drained = std::mem::take(run).into_iter();
	let Some((first_comments, first)) = drained.next() else {
		return;
	};
	let mut with = vec![link("with", vec![first])];
	with.extend(drained.map(|(comments, piece)| {
		Link {
			comments,
			method: "with".to_owned(),
			args: vec![piece],
		}
	}));
	links.push(Link {
		comments: first_comments,
		method: "children".to_owned(),
		args: vec![chain(code("Fragment::new()"), with)],
	});
}

/// The call that adds an expression child of this shape to a container,
/// or the piece back when its shape is unknown and only a fragment can
/// hold it.
fn adding_call(shape: Shape, piece: Piece) -> Result<(&'static str, Piece), Piece> {
	match shape {
		Shape::Element => Ok(("child", piece)),
		Shape::View => Ok(("child", into_view(piece))),
		Shape::Text => Ok(("child", text_call(piece))),
		Shape::Elements => Ok(("children", piece)),
		Shape::Any => Err(piece),
	}
}

/// `Text(…)` around a piece.
fn text_call(piece: Piece) -> Piece {
	Piece::Call {
		callee: "Text".to_owned(),
		args: vec![piece],
	}
}

/// Removes the attribute named `name` when it is set exactly once; one set
/// twice stays as two calls.
fn take_attribute(attributes: &mut Vec<Attribute>, name: &str) -> Option<Attribute> {
	single_index(attributes, name).map(|index| attributes.remove(index))
}

/// `Heading`'s level, its first argument. A literal `aria_level` becomes
/// it; any other value stays an `aria_level` call over the default of 2.
fn heading_level(attributes: &mut Vec<Attribute>) -> Piece {
	let literal = single_index(attributes, "aria_level")
		.and_then(|index| Some((index, literal_level(attributes[index].value.as_ref()?)?)));
	let Some((index, level)) = literal else {
		return code("2");
	};

	attributes.remove(index);
	code(level)
}

/// The argument a builder function takes from the attribute `key`, or its
/// `default` when the tag leaves the attribute out; `None` when it has no
/// default.
fn attribute_argument(
	attributes: &mut Vec<Attribute>,
	key: &str,
	default: Option<&str>,
) -> Result<Option<Piece>, Unconvertible> {
	match take_attribute(attributes, key) {
		Some(attribute) => single_argument(attribute).map(Some),
		None => Ok(default.map(code)),
	}
}

/// `Radio()`'s arguments: its group, then whether it starts checked.
fn radio_arguments(
	attributes: &mut Vec<Attribute>,
	tag: Span,
) -> Result<Vec<Piece>, Unconvertible> {
	let group = take_attribute(attributes, "radio_group")
		.ok_or_else(|| unconvertible(tag, "`Radio()` needs `radio_group=…` to call it with"))?;
	let checked = match take_attribute(attributes, "checked") {
		Some(attribute) => single_argument(attribute)?,
		None => code("false"),
	};

	Ok(vec![single_argument(group)?, checked])
}

/// The argument a page-defined component's parameter takes: its prop's
/// value, or what the parameter takes without one. The tag's children
/// replace the empty fragment in their slot afterwards.
fn positional_argument(
	param: &Param,
	attributes: &mut [Option<Attribute>],
	name: &str,
	tag: Span,
) -> Result<Piece, Unconvertible> {
	if param.kind == ParamKind::Children {
		return Ok(code("Fragment::new()"));
	}

	let found = attributes
		.iter_mut()
		.find(|attribute| {
			attribute
				.as_ref()
				.is_some_and(|attribute| attribute.method == param.name)
		})
		.and_then(Option::take);
	let Some(attribute) = found else {
		return absent_argument(param, name, tag);
	};

	if !attribute.comments.is_empty() {
		return Err(unconvertible(
			attribute.span,
			"a comment beside a prop has no place in a positional call; move it out of the `view!`",
		));
	}

	let span = attribute.span;
	let precedence = attribute.value.as_ref().is_none_or(binds_tightly);
	let mut value = single_argument(attribute)?;

	if param.into {
		let text = value
			.flat()
			.ok_or_else(|| unconvertible(span, "write this value on one line"))?;
		value = code(if precedence {
			format!("{text}.into()")
		} else {
			format!("({text}).into()")
		});
	}

	if matches!(param.kind, ParamKind::Optional { wrap: true, .. }) {
		value = Piece::Call {
			callee: "Some".to_owned(),
			args: vec![value],
		};
	}

	Ok(value)
}

/// What a parameter takes when the tag leaves its prop out: its default,
/// or an error for a required prop.
fn absent_argument(param: &Param, name: &str, tag: Span) -> Result<Piece, Unconvertible> {
	match &param.kind {
		ParamKind::Optional {
			default: Some(default),
			..
		} => Ok(code(default.clone())),
		ParamKind::Optional { absent, .. } => Ok(code(*absent)),
		ParamKind::Required | ParamKind::Children => {
			Err(unconvertible(
				tag,
				format!("`<{name}>` is missing the required prop `{}`", param.name),
			))
		}
	}
}

fn single_index(attributes: &[Attribute], name: &str) -> Option<usize> {
	let mut matching = attributes
		.iter()
		.enumerate()
		.filter(|(_, attribute)| attribute.method == name);
	let (index, _) = matching.next()?;
	matching.next().is_none().then_some(index)
}

/// A heading level written as a literal from 1 to 9.
fn literal_level(value: &Expr) -> Option<String> {
	let Expr::Lit(literal) = value else {
		return None;
	};
	let Lit::Int(int) = &literal.lit else {
		return None;
	};
	matches!(int.base10_parse::<u8>(), Ok(1..=9)).then(|| int.base10_digits().to_owned())
}

fn single_argument(attribute: Attribute) -> Result<Piece, Unconvertible> {
	let span = attribute.span;
	let mut args = attribute.args.into_iter();

	match (args.next(), args.next()) {
		(Some(arg), None) => Ok(arg),
		_ => {
			Err(unconvertible(
				span,
				format!("`{}` takes one value here", attribute.method),
			))
		}
	}
}

fn attribute_links(attributes: Vec<Attribute>) -> Vec<Link> {
	attributes
		.into_iter()
		.map(|attribute| {
			Link {
				comments: attribute.comments,
				method: attribute.method,
				args: attribute.args,
			}
		})
		.collect()
}

/// A tag as an `Element` for `.child(…)`.
fn element_of(tree: Tree) -> Piece {
	match tree.shape {
		Shape::Element => tree.piece,
		_ => into_view(tree.piece),
	}
}

/// `piece.into_view()`. Only calls reach here, which bind tighter than the
/// method call.
fn into_view(piece: Piece) -> Piece {
	match piece {
		Piece::Chain { root, mut links } => {
			links.push(link("into_view", Vec::new()));
			Piece::Chain { root, links }
		}
		other => chain(other, vec![link("into_view", Vec::new())]),
	}
}

/// The builder method an attribute calls: `class` calls `.class`, and
/// `on:press` calls `.on_press`.
fn method(key: &NodeName) -> Result<String, Unconvertible> {
	match key {
		NodeName::Path(path) if path.path.get_ident().is_some() => {
			Ok(path.path.segments[0].ident.to_string())
		}

		NodeName::Punctuated(parts) => {
			let pairs = parts.pairs().collect::<Vec<_>>();
			if let [first, second] = pairs.as_slice()
				&& let (NodeNameFragment::Ident(prefix), Some(punct)) =
					(first.value(), first.punct())
				&& prefix == "on"
				&& punct.as_char() == ':'
				&& let NodeNameFragment::Ident(event) = second.value()
			{
				return Ok(format!("on_{event}"));
			}

			Err(unconvertible(
				key.span(),
				format!(
					"`{key}` is not an attribute: use a prop name, or `on:event` for a handler"
				),
			))
		}
		_ => {
			Err(unconvertible(
				key.span(),
				format!("`{key}` is not an attribute: use a prop name like `class`"),
			))
		}
	}
}

/// Whether `.into()` on `expr` applies to all of it, without parentheses.
fn binds_tightly(expr: &Expr) -> bool {
	matches!(
		expr,
		Expr::Array(_)
			| Expr::Block(_)
			| Expr::Call(_)
			| Expr::Field(_)
			| Expr::Index(_)
			| Expr::Lit(_)
			| Expr::Macro(_)
			| Expr::MethodCall(_)
			| Expr::Paren(_)
			| Expr::Path(_)
			| Expr::Struct(_)
			| Expr::Tuple(_)
	)
}

/// Whether markup has a lowercase HTML tag, which only leptos's `view!`
/// accepts.
fn has_html_tag(node: &Node) -> bool {
	match node {
		Node::Element(element) => {
			let html = match element.name() {
				NodeName::Path(path) => {
					path.path
						.get_ident()
						.is_some_and(|ident| ident.to_string().starts_with(char::is_lowercase))
				}
				NodeName::Punctuated(_) => true,
				NodeName::Block(_) => false,
			};

			html || element.children.iter().any(has_html_tag)
		}
		Node::Fragment(fragment) => fragment.children.iter().any(has_html_tag),
		_ => false,
	}
}

/// Whether markup is one tag, which `view!` returns as an `Element`.
fn is_one_tag(nodes: &[Node]) -> bool {
	let mut nodes = nodes
		.iter()
		.filter(|node| !matches!(node, Node::Comment(_)));
	matches!((nodes.next(), nodes.next()), (Some(Node::Element(_)), None))
}

/// A `view!` invocation in a token list.
struct Invocation {
	/// The path's segments, ending in `view`.
	path: Vec<String>,
	/// Where the path starts.
	start: Span,
	group: Group,
}

impl Invocation {
	fn range(&self) -> Range<usize> {
		self.start.byte_range().start..self.group.span().byte_range().end
	}
}

/// The `view!` invocation whose `view` is at `index`, if one is.
fn invocation_at(trees: &[TokenTree], index: usize) -> Option<Invocation> {
	let (TokenTree::Ident(ident), Some(TokenTree::Punct(bang)), Some(TokenTree::Group(group))) =
		(&trees[index], trees.get(index + 1), trees.get(index + 2))
	else {
		return None;
	};

	if ident != "view" || bang.as_char() != '!' {
		return None;
	}

	let mut path = vec![ident.to_string()];
	let mut start = index;

	while start >= 3
		&& is_punct(&trees[start - 1], ':')
		&& is_punct(&trees[start - 2], ':')
		&& let TokenTree::Ident(segment) = &trees[start - 3]
	{
		path.insert(0, segment.to_string());
		start -= 3;
	}

	if start >= 2 && is_punct(&trees[start - 1], ':') && is_punct(&trees[start - 2], ':') {
		start -= 2;
	}

	Some(Invocation {
		path,
		start: trees[start].span(),
		group: group.clone(),
	})
}

fn is_punct(tree: &TokenTree, character: char) -> bool {
	matches!(tree, TokenTree::Punct(punct) if punct.as_char() == character)
}

/// Whether a bare `view!` in a block is leptos's: the block imports leptos's
/// `view` and not leptoskit's.
fn bare_view_is_leptos(tokens: &TokenStream) -> bool {
	let imports = imports(tokens);
	let imports_view = |krate: &str| {
		imports.iter().any(|path| {
			let Some(rest) = path
				.strip_prefix(krate)
				.and_then(|rest| rest.strip_prefix("::"))
			else {
				return false;
			};
			rest == "*"
				|| rest.ends_with("::*")
				|| rest == "view"
				|| rest.ends_with("::view")
				|| rest.rsplit_once('{').is_some_and(|(_, names)| {
					names
						.trim_end_matches('}')
						.split(',')
						.any(|name| matches!(name, "view" | "*"))
				})
		})
	};

	imports_view("leptos") && !imports_view("leptoskit")
}

/// The paths of a block's top-level `use` declarations, without whitespace.
fn imports(tokens: &TokenStream) -> Vec<String> {
	let mut imports = Vec::new();
	let mut current: Option<String> = None;

	for tree in tokens.clone() {
		match (&mut current, &tree) {
			(None, TokenTree::Ident(ident)) if ident == "use" => current = Some(String::new()),

			(Some(path), TokenTree::Punct(punct)) if punct.as_char() == ';' => {
				imports.push(std::mem::take(path));
				current = None;
			}

			(Some(path), tree) => {
				path.extend(
					tree.to_string()
						.chars()
						.filter(|character| !character.is_whitespace()),
				);
			}
			_ => {}
		}
	}

	imports
}

/// Collects the `#[component]` functions in `tokens`.
fn find_components(
	source: &str,
	tokens: TokenStream,
	leptos: bool,
	out: &mut Vec<(String, Signature)>,
) {
	let trees = tokens.into_iter().collect::<Vec<_>>();

	for (index, tree) in trees.iter().enumerate() {
		if let TokenTree::Group(group) = tree {
			find_components(source, group.stream(), leptos, out);
		}

		if !is_punct(tree, '#') {
			continue;
		}

		let Some(TokenTree::Group(attribute)) = trees.get(index + 1) else {
			continue;
		};
		let path = attribute
			.stream()
			.to_string()
			.chars()
			.filter(|character| !character.is_whitespace())
			.collect::<String>();

		let is_component = match path.as_str() {
			"component" => !leptos,
			"leptoskit::component" | "leptoskit::prelude::component" => true,
			_ => false,
		};

		if attribute.delimiter() == Delimiter::Bracket
			&& is_component
			&& let Some(found) = signature_after(source, &trees[index + 2..])
		{
			out.push(found);
		}
	}
}

/// The name and signature of the function `trees` starts with, after any
/// further attributes and its visibility.
fn signature_after(source: &str, trees: &[TokenTree]) -> Option<(String, Signature)> {
	let fn_at = trees.iter().position(|tree| is_ident(tree, "fn"))?;
	let TokenTree::Ident(name) = trees.get(fn_at + 1)? else {
		return None;
	};
	let params_at = fn_at
		+ 2
		+ trees[fn_at + 2..]
			.iter()
			.position(|tree| is_group(tree, Delimiter::Parenthesis))?;
	let TokenTree::Group(params) = &trees[params_at] else {
		return None;
	};
	let inputs = Punctuated::<FnArg, Token![,]>::parse_terminated
		.parse2(params.stream())
		.ok()?;
	let params = inputs
		.iter()
		.map(|input| param(source, input))
		.collect::<Option<Vec<_>>>()?;
	let returns = return_type(&trees[params_at + 1..]);

	Some((
		name.to_string(),
		Signature {
			params,
			returns_element: returns.as_deref() == Some("Element"),
		},
	))
}

/// The last name in a function's return type, as in `Element` for
/// `-> Element`: `trees` follow the parameters, up to the body or a
/// `where` clause.
fn return_type(trees: &[TokenTree]) -> Option<String> {
	trees
		.iter()
		.take_while(|tree| !is_group(tree, Delimiter::Brace) && !is_ident(tree, "where"))
		.filter_map(|tree| {
			match tree {
				TokenTree::Ident(ident) => Some(ident.to_string()),
				_ => None,
			}
		})
		.last()
}

fn is_ident(tree: &TokenTree, name: &str) -> bool {
	matches!(tree, TokenTree::Ident(ident) if ident == name)
}

fn is_group(tree: &TokenTree, delimiter: Delimiter) -> bool {
	matches!(tree, TokenTree::Group(group) if group.delimiter() == delimiter)
}

fn param(source: &str, input: &FnArg) -> Option<Param> {
	let FnArg::Typed(typed) = input else {
		return None;
	};
	let Pat::Ident(ident) = &*typed.pat else {
		return None;
	};
	let name = ident.ident.to_string();

	if name == "children" {
		return Some(Param {
			name,
			kind: ParamKind::Children,
			into: false,
		});
	}

	let options = PropOptions::read(source, &typed.attrs)?;
	let last_type = last_segment(&typed.ty);
	let kind = if options.optional || options.default.is_some() {
		ParamKind::Optional {
			wrap: last_type.as_deref() == Some("Option") && !options.keep_option,
			absent: absent_value(last_type.as_deref()),
			default: options.default,
		}
	} else {
		ParamKind::Required
	};

	Some(Param {
		name,
		kind,
		into: options.into,
	})
}

/// What a parameter's `#[prop(…)]` attributes say about its prop.
#[derive(Default)]
struct PropOptions {
	/// `into`: the value is converted with `.into()`.
	into: bool,
	/// `optional` or `optional_no_strip`: a tag may leave the prop out.
	optional: bool,
	/// `optional_no_strip`: an `Option` prop takes the `Option` itself
	/// rather than the value inside it.
	keep_option: bool,
	/// `default = …`, as written.
	default: Option<String>,
}

impl PropOptions {
	/// Every `#[prop(…)]` on a parameter, or `None` when one does not parse.
	fn read(source: &str, attributes: &[syn::Attribute]) -> Option<Self> {
		let mut options = Self::default();
		let props = attributes
			.iter()
			.filter(|attribute| attribute.path().is_ident("prop"));

		for attribute in props {
			let metas = attribute
				.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
				.ok()?;

			for meta in &metas {
				options.apply(source, meta);
			}
		}

		Some(options)
	}

	fn apply(&mut self, source: &str, meta: &Meta) {
		match meta {
			Meta::Path(path) if path.is_ident("into") => self.into = true,
			Meta::Path(path) if path.is_ident("optional") => self.optional = true,
			Meta::Path(path) if path.is_ident("optional_no_strip") => {
				self.optional = true;
				self.keep_option = true;
			}
			Meta::NameValue(pair) if pair.path.is_ident("default") => {
				self.default = Some(source[pair.value.span().byte_range()].to_owned());
			}
			_ => {}
		}
	}
}

/// The last segment of a type's path, as in `Option` for `Option<String>`.
fn last_segment(ty: &Type) -> Option<String> {
	let Type::Path(path) = ty else {
		return None;
	};

	path.path
		.segments
		.last()
		.map(|segment| segment.ident.to_string())
}

/// What a component takes for an optional prop its tag leaves out.
fn absent_value(last_type: Option<&str>) -> &'static str {
	match last_type {
		Some("Option") => "None",
		Some("bool") => "false",
		_ => "Default::default()",
	}
}

/// `source[range]`, with each replacement written over its bytes.
fn splice(source: &str, range: Range<usize>, replacements: &[(Range<usize>, String)]) -> String {
	let mut text = String::new();
	let mut at = range.start;

	for (replaced, replacement) in replacements {
		text.push_str(&source[at..replaced.start]);
		text.push_str(replacement);
		at = replaced.end;
	}

	text.push_str(&source[at..range.end]);
	text
}

/// The expression a block holds when it holds only one, such as the
/// `view! { … }` in `{ // Note \n view! { … } }`.
fn only_expr(block: &syn::Block) -> Option<Expr> {
	match block.stmts.as_slice() {
		[Stmt::Expr(expr, None)] => Some(expr.clone()),

		[Stmt::Macro(stmt)] if stmt.semi_token.is_none() => {
			Some(Expr::Macro(syn::ExprMacro {
				attrs: stmt.attrs.clone(),
				mac: stmt.mac.clone(),
			}))
		}

		_ => None,
	}
}

/// One level of indentation as `source` writes it: the fewest spaces that
/// start a line, or a tab, which the book's code indents with.
fn indent_unit(source: &str) -> String {
	if source.lines().any(|line| line.starts_with('\t')) {
		return "\t".to_owned();
	}

	source
		.lines()
		.filter(|line| !line.trim().is_empty())
		.map(|line| line.len() - line.trim_start_matches(' ').len())
		.filter(|&spaces| spaces > 0)
		.min()
		.map_or_else(|| "\t".to_owned(), |spaces| " ".repeat(spaces))
}

fn unconvertible(span: Span, message: impl Into<String>) -> Unconvertible {
	let start = span.start();
	Unconvertible {
		line: start.line,
		column: start.column + 1,
		message: message.into(),
	}
}

/// Builder code before it is laid out.
#[derive(Debug, Clone)]
enum Piece {
	/// Code kept as written. Lines after the first are indented relative to
	/// `base`, the indentation of the line the code started on.
	Code { text: String, base: String },
	/// `callee(args)`.
	Call { callee: String, args: Vec<Piece> },
	/// `root.method(args)…`.
	Chain { root: Box<Piece>, links: Vec<Link> },
}

/// One method call in a chain, with the comments written above it.
#[derive(Debug, Clone)]
struct Link {
	comments: Vec<String>,
	method: String,
	args: Vec<Piece>,
}

fn code(text: impl Into<String>) -> Piece {
	Piece::Code {
		text: text.into(),
		base: String::new(),
	}
}

fn link(method: &str, args: Vec<Piece>) -> Link {
	Link {
		comments: Vec::new(),
		method: method.to_owned(),
		args,
	}
}

fn chain(root: Piece, links: Vec<Link>) -> Piece {
	if links.is_empty() {
		return root;
	}

	Piece::Chain {
		root: Box::new(root),
		links,
	}
}

impl Piece {
	/// The piece on one line, unless part of it spans lines or has a comment.
	fn flat(&self) -> Option<String> {
		match self {
			Self::Code { text, .. } => (!text.contains('\n')).then(|| text.clone()),
			Self::Call { callee, args } => Some(format!("{callee}({})", flat_args(args)?)),
			Self::Chain { root, links } => {
				Some(root.flat()? + &calls_on_one_line(links, flat_args)?)
			}
		}
	}

	/// The piece on one line when rustfmt keeps it on one: no part spans
	/// lines, and every chain of two or more calls and every argument list
	/// of two or more values is at most [`SHORT_WIDTH`] wide.
	fn compact(&self) -> Option<String> {
		match self {
			Self::Code { text, .. } => (!text.contains('\n')).then(|| text.clone()),
			Self::Call { callee, args } => Some(format!("{callee}({})", compact_args(args)?)),
			Self::Chain { root, links } => {
				let text = root.compact()? + &calls_on_one_line(links, compact_args)?;
				(links.len() < 2 || width(&text) <= SHORT_WIDTH).then_some(text)
			}
		}
	}
}

/// A chain's calls after its root on one line, with each call's arguments
/// as `args` writes them; `None` when a call has a comment or `args` cannot
/// write its arguments on one line.
fn calls_on_one_line(links: &[Link], args: fn(&[Piece]) -> Option<String>) -> Option<String> {
	let mut text = String::new();

	for link in links {
		if !link.comments.is_empty() {
			return None;
		}

		text.push('.');
		text.push_str(&link.method);
		text.push('(');
		text.push_str(&args(&link.args)?);
		text.push(')');
	}

	Some(text)
}

fn flat_args(args: &[Piece]) -> Option<String> {
	Some(
		args.iter()
			.map(Piece::flat)
			.collect::<Option<Vec<_>>>()?
			.join(", "),
	)
}

fn compact_args(args: &[Piece]) -> Option<String> {
	let text = args
		.iter()
		.map(Piece::compact)
		.collect::<Option<Vec<_>>>()?
		.join(", ");
	// rustfmt limits two or more arguments, and a lone call, to
	// `SHORT_WIDTH`; a lone chain or value may take the whole line.
	let limited = args.len() > 1 || matches!(args, [Piece::Call { .. }]);
	(!limited || width(&text) <= SHORT_WIDTH).then_some(text)
}

/// `piece` with `prefix`, such as `&`, written before it.
fn prefixed(piece: Piece, prefix: &str) -> Piece {
	match piece {
		Piece::Code { text, base } => {
			Piece::Code {
				text: format!("{prefix}{text}"),
				base,
			}
		}
		Piece::Call { callee, args } => {
			Piece::Call {
				callee: format!("{prefix}{callee}"),
				args,
			}
		}
		Piece::Chain { root, links } => {
			Piece::Chain {
				root: Box::new(prefixed(*root, prefix)),
				links,
			}
		}
	}
}

/// The function or method a parenthesised group calls, as rustfmt measures
/// it: `.method` for a method, and `None` when the group is not a call.
fn callee_before(trees: &[TokenTree], index: usize) -> Option<String> {
	const KEYWORDS: [&str; 12] = [
		"as", "await", "break", "else", "for", "if", "in", "let", "match", "move", "return",
		"while",
	];

	let TokenTree::Ident(name) = trees.get(index.checked_sub(1)?)? else {
		return None;
	};
	let name = name.to_string();

	if KEYWORDS.contains(&name.as_str()) {
		return None;
	}

	match index.checked_sub(2).and_then(|before| trees.get(before)) {
		Some(tree) if is_punct(tree, '.') => Some(format!(".{name}")),

		Some(tree) if is_punct(tree, ':') => Some(format!("::{name}")),
		_ => Some(name),
	}
}

/// Whether code that spans lines ends on a closing bracket, like a closure's
/// block, so rustfmt keeps it on the line of the call it is passed to.
fn is_block_like(text: &str) -> bool {
	text.lines()
		.last()
		.is_some_and(|line| line.trim_start().starts_with([')', '}', ']']))
}

/// Lays builder code out the way rustfmt does, closely enough that the
/// samples read like the book's own code.
struct Layout<'a> {
	unit: &'a str,
	/// Whether a chain was broken call by call since this was cleared, which
	/// rules out overflowing a lone chain argument.
	broke: Cell<bool>,
}

impl<'a> Layout<'a> {
	fn new(unit: &'a str) -> Self {
		Self {
			unit,
			broke: Cell::new(false),
		}
	}

	/// Runs a layout that may be thrown away, and returns it with whether it
	/// broke a chain call by call; [`Self::keep`] records that for one that
	/// is used.
	fn trial<T>(&self, attempt: impl FnOnce() -> T) -> (T, bool) {
		let outer = self.broke.replace(false);
		let value = attempt();
		(value, self.broke.replace(outer))
	}

	fn keep(&self, broke: bool) {
		if broke {
			self.broke.set(true);
		}
	}

	/// `piece` starting at `column` of a line indented by `indent`.
	fn render(&self, piece: &Piece, indent: &str, column: usize) -> String {
		match piece {
			Piece::Code { text, base } => reindent(text, base, indent),
			Piece::Call { callee, args } => {
				let args = self.args(callee, args, indent, column + width(callee), true);
				format!("{callee}{args}")
			}
			Piece::Chain { root, links } => {
				piece
					.compact()
					.filter(|flat| column + width(flat) <= MAX_WIDTH)
					.or_else(|| self.overflow_last(root, links, indent, column))
					.unwrap_or_else(|| self.vertical(root, links, indent, column))
			}
		}
	}

	/// A chain on one line but for its last call, whose arguments break
	/// over several lines; `None` when rustfmt breaks every call instead.
	fn overflow_last(
		&self,
		root: &Piece,
		links: &[Link],
		indent: &str,
		column: usize,
	) -> Option<String> {
		let [init @ .., last] = links else {
			return None;
		};

		if !last.comments.is_empty() {
			return None;
		}

		let prefix = chain(root.clone(), init.to_vec()).compact()?;
		let callee = format!(".{}", last.method);
		let head = format!("{prefix}{callee}");
		let (args, broke) =
			self.trial(|| self.args(&callee, &last.args, indent, column + width(&head), true));
		let overflowed = format!("{head}{args}");
		let height = overflowed.lines().count();

		// The first line has the room of a whole line when the chain is one
		// call long, and `SHORT_WIDTH` when it is longer.
		let space = MAX_WIDTH.saturating_sub(column);
		let budget = if links.len() > 1 {
			space.min(SHORT_WIDTH)
		} else {
			space
		};

		if height < 2 || overflowed.lines().next().map_or(0, width) > budget {
			return None;
		}

		if height >= TALL_OVERFLOW {
			self.keep(broke);

			return Some(overflowed);
		}

		// A short overflow wins only when the call on a line of its own
		// takes as many lines.
		let inner = format!("{indent}{}", self.unit);
		let (alone, _) = self.trial(|| {
			self.args(
				&callee,
				&last.args,
				&inner,
				width(&inner) + width(&callee),
				true,
			)
		});

		if alone.lines().count() < height {
			return None;
		}

		self.keep(broke);
		Some(overflowed)
	}

	/// A chain with each call on a line of its own.
	fn vertical(&self, root: &Piece, links: &[Link], indent: &str, column: usize) -> String {
		self.broke.set(true);
		let mut out = self.render(root, indent, column);
		let mut links = links;

		// A root narrower than a tab at the start of a line takes the calls
		// after it until it is wider, as in `items.iter()`.
		let reach = TAB_WIDTH.saturating_sub(column.saturating_sub(width(indent)));

		while width(&out) <= reach
			&& !out.contains('\n')
			&& let [first, rest @ ..] = links
			&& first.comments.is_empty()
		{
			let callee = format!(".{}", first.method);
			let at = column + width(&out) + width(&callee);
			out.push_str(&callee);
			out.push_str(&self.args(&callee, &first.args, indent, at, true));
			links = rest;
		}

		let inner = format!("{indent}{}", self.unit);
		// Calls after a root that ends on its own line, like `Row(\n…\n)`,
		// line up with that closing bracket.
		let link_indent = if out.contains('\n') && is_block_like(&out) {
			indent
		} else {
			&inner
		};

		for link in links {
			out.push('\n');

			for comment in &link.comments {
				out.push_str(link_indent);
				out.push_str(comment);
				out.push('\n');
			}

			let callee = format!(".{}", link.method);
			out.push_str(link_indent);
			out.push_str(&callee);
			let column = width(link_indent) + width(&callee);
			out.push_str(&self.args(&callee, &link.args, link_indent, column, true));
		}

		out
	}

	/// The parenthesised arguments of a call to `callee` that ends at
	/// `column`. `flat` is false when the call itself is too wide for one
	/// line, so its arguments cannot be on one either.
	fn args(
		&self,
		callee: &str,
		args: &[Piece],
		indent: &str,
		column: usize,
		flat: bool,
	) -> String {
		if flat
			&& let Some(text) = compact_args(args)
			&& column + width(&text) + 2 <= MAX_WIDTH
		{
			return format!("({text})");
		}

		if let [only] = args
			&& let Some(text) = self.lone_argument(callee, only, indent, column + 1)
		{
			return format!("({text})");
		}

		self.vertical_args(callee, args, indent)
	}

	/// A lone argument starting at `start`, when it keeps its opening
	/// bracket on the call's line: when the callee is narrower than a tab,
	/// as in `Ok(…)`, when it is a call or a block, and when it is too wide
	/// for any line.
	fn lone_argument(
		&self,
		callee: &str,
		only: &Piece,
		indent: &str,
		start: usize,
	) -> Option<String> {
		if width(callee) < TAB_WIDTH {
			return Some(self.render(only, indent, start));
		}

		match only {
			Piece::Call { callee, args } => {
				let args = self.args(callee, args, indent, start + width(callee), false);
				Some(format!("{callee}{args}"))
			}
			Piece::Code { text, .. } if text.contains('\n') => {
				is_block_like(text).then(|| self.render(only, indent, start))
			}
			Piece::Code { text, .. } => {
				let inner = width(indent) + width(self.unit);
				(inner + width(text) + 1 > MAX_WIDTH).then(|| text.clone())
			}
			// rustfmt overflows a lone chain only when no chain in it
			// breaks call by call; code kept as written does not count.
			Piece::Chain { root, links } => {
				let (text, broke) = self.trial(|| self.overflow_last(root, links, indent, start));
				text.filter(|_| !broke)
			}
		}
	}

	/// Arguments a line each, one level further in than the call.
	fn vertical_args(&self, callee: &str, args: &[Piece], indent: &str) -> String {
		let inner = format!("{indent}{}", self.unit);
		// Macros, like `format!`, take no trailing comma.
		let comma = if callee.ends_with('!') { "" } else { "," };
		let mut out = String::from("(\n");

		for (index, arg) in args.iter().enumerate() {
			out.push_str(&inner);
			out.push_str(&self.render(arg, &inner, width(&inner)));

			out.push_str(if index + 1 == args.len() { comma } else { "," });
			out.push('\n');
		}

		out.push_str(indent);
		out.push(')');
		out
	}
}

/// Code written at indentation `base`, moved to `indent`.
fn reindent(text: &str, base: &str, indent: &str) -> String {
	let mut lines = text.split('\n');
	let mut out = lines.next().unwrap_or_default().to_owned();

	for line in lines {
		out.push('\n');

		if line.trim().is_empty() {
			continue;
		}

		out.push_str(indent);
		out.push_str(line.strip_prefix(base).unwrap_or_else(|| line.trim_start()));
	}

	out
}

/// How many columns one line takes, with tabs four wide.
fn width(line: &str) -> usize {
	line.chars()
		.map(|character| if character == '\t' { TAB_WIDTH } else { 1 })
		.sum()
}

#[cfg(test)]
mod tests {
	use super::*;

	/// `guide/02-creating.md`: a screen with comments, `format!` text, and
	/// a nested `view!` in a commented block.
	const WATER_SCREEN: &str = r#"use leptoskit::prelude::*;

use crate::Counter;

/// Glasses of water in a day's goal.
const GOAL: i32 = 8;

/// Builds the same screen for the browser and every native app.
pub fn screen(state: &Counter) -> Element {
	let glasses = state.value();
	let filled = glasses.clamp(0, GOAL) as f32 / GOAL as f32;
	let counter = state.clone();

	view! {
		<View class="min-h-screen items-center justify-center gap-6 p-6 bg-sky-950 text-white">
			<Text class="text-3xl font-bold">"Water today"</Text>
			<Text class="text-xl">{format!("{glasses} of {GOAL} glasses")}</Text>
			{
				// A progress bar made of two boxes: the inner one is a
				// fraction of the outer one's width.
				view! {
					<View class="h-3 w-64 rounded-[6px] bg-sky-800">
						<View
							class="h-full rounded-[6px] bg-sky-400"
							with_style=move |style| style.width = Dimension::Percent(filled)
						/>
					</View>
				}
			}
			<Button
				class="rounded-lg bg-sky-400 px-6 py-3 font-semibold text-sky-950"
				on:press=move || counter.increment()
			>
				"Drink a glass"
			</Button>
			<Text class="text-sm text-sky-200">{format!("Drawn with {}", toolkit())}</Text>
		</View>
	}
}
"#;

	/// `reference/view.md`: a component with `into`, optional, and children
	/// props, used by another component on the same page.
	const SETTINGS: &str = r#"use leptoskit::prelude::*;

/// A settings row with a trailing control.
#[component]
pub fn SettingRow(
	/// The row's label.
	#[prop(into)]
	title: String,
	/// Shown under the title when set.
	#[prop(optional, into)]
	detail: Option<String>,
	children: Children,
) -> impl IntoView {
	view! {
		<View class="flex-row gap-3 items-center px-4 h-12">
			<View class="flex-1">
				<Text class="text-base">{title}</Text>
				{detail.map(
					|detail| view! { <Text class="text-sm text-slate-500">{detail}</Text> },
				)}
			</View>
			{children}
		</View>
	}
}

#[component]
pub fn Settings(wifi: bool, toggle: impl Fn(bool) + Send + Sync + 'static) -> Element {
	view! {
		<ScrollView class="flex-1">
			<SettingRow title="Wi-Fi" detail="Home 5G">
				<Switch checked=wifi accessibility_label="Wi-Fi" on:toggle=toggle />
			</SettingRow>
		</ScrollView>
	}
}
"#;

	/// `guide/07-shared-components.md`: `view!` as values inside a call.
	const PREFERENCES: &str = r#"use leptoskit::components::helpers::Card;
use leptoskit::components::helpers::ListTile;
use leptoskit::prelude::*;

#[component]
pub fn Preferences(checked: bool, update: impl Fn(bool) + Send + Sync + 'static) -> Element {
	view! {
		<Card class="gap-4">
			<Text class="text-xl font-semibold">"Preferences"</Text>
			{ListTile(
				None,
				view! { <Text>"Notifications"</Text> },
				Some(view! {
					<Switch checked=checked aria_label="Notifications" on:toggle=update />
				}),
			)}
		</Card>
	}
}
"#;

	/// `guide/19-localization.md`: an iterator of views as children.
	const LANGUAGE_MENU: &str = r#"use leptoskit::prelude::*;

use crate::l10n::use_l10n;

#[component]
pub fn LanguageMenu() -> impl IntoView {
	let l10n = use_l10n();
	view! {
		<View class="flex-row gap-2">
			{l10n
				.supported_locales()
				.iter()
				.map(|locale| {
					let tag = locale.to_string();
					let choice = locale.clone();
					view! {
						<Button on:press=move || l10n.set_locale(&choice)>{tag}</Button>
					}
				})
				.collect::<Vec<_>>()}
		</View>
	}
}
"#;

	/// `guide/20-rive.md`: a `view!` as the only argument of `Ok`.
	const RIVE_STATUS: &str = r#"use leptoskit::components::Rive;
use leptoskit::native::Element;
use leptoskit::rive::Asset;
use leptoskit::rive::Error;
use leptoskit::rive::Input;
use leptoskit::rive::Name;
use leptoskit::rive::Playback;
use leptoskit::rive::Scene;
use leptoskit::view;

fn upload_status(asset: &Asset, progress: f64) -> Result<Element, Error> {
	let mut playback =
		Playback::new(asset.clone()).scene(Scene::StateMachine(Name::new("Status")?));
	playback.set_input(Name::new("progress")?, Input::Number(progress), 0)?;
	Ok(view! {
		<Rive
			rive_playback=playback
			class="w-40 h-40"
			aria_label="Upload progress"
			running=true
		/>
	})
}
"#;

	/// `guide/24-real-time.md`: a reactive closure as a child.
	const CHANNEL: &str = r#"use leptoskit::prelude::*;
use leptoskit::rpc::use_live;

#[component]
pub fn Channel(channel: ChannelId) -> impl IntoView {
	let messages = use_live::<messages>(move || {
		messages {
			channel,
			limit: 100,
		}
	});

	view! {
		<View class="flex-col gap-2 p-4">
			{move || {
				messages
					.data()
					.unwrap_or_default()
					.into_iter()
					.map(MessageRow)
			}}
		</View>
	}
}
"#;

	fn convert_with(source: &str, components: &PageComponents) -> Option<String> {
		builder_variant(source, components).unwrap_or_else(|error| panic!("{error:?}"))
	}

	fn convert(source: &str) -> Option<String> {
		convert_with(source, &PageComponents::default())
	}

	/// A whole sample's builder form, which must still be a Rust file.
	fn convert_file(source: &str) -> String {
		let components = PageComponents::collect([source]);
		let builder = convert_with(source, &components).expect("the sample has leptoskit markup");
		syn::parse_file(&builder).unwrap_or_else(|error| panic!("{error}\n{builder}"));
		builder
	}

	fn failure(source: &str, components: &PageComponents) -> Unconvertible {
		builder_variant(source, components).expect_err("the markup has no builder form")
	}

	fn badge() -> PageComponents {
		PageComponents::collect([r"
#[component]
fn Badge(
	#[prop(into)] label: String,
	#[prop(optional)] count: Option<u32>,
	#[prop(default = Tone::Info)] tone: Tone,
	children: Children,
) -> impl IntoView {
	view! { <View /> }
}
"])
	}

	#[test]
	fn converts_the_water_screen() {
		insta::assert_snapshot!(convert_file(WATER_SCREEN));
	}

	#[test]
	fn converts_components_with_their_props_in_order() {
		insta::assert_snapshot!(convert_file(SETTINGS));
	}

	#[test]
	fn converts_views_passed_as_values() {
		insta::assert_snapshot!(convert_file(PREFERENCES));
	}

	#[test]
	fn converts_iterators_of_views() {
		insta::assert_snapshot!(convert_file(LANGUAGE_MENU));
	}

	#[test]
	fn converts_a_view_returned_in_ok() {
		insta::assert_snapshot!(convert_file(RIVE_STATUS));
	}

	#[test]
	fn converts_reactive_children() {
		insta::assert_snapshot!(convert_file(CHANNEL));
	}

	#[test]
	fn leaves_code_without_leptoskit_markup_alone() {
		for source in [
			"fn main() {}",
			"leptos::view! { <div>\"Hi\"</div> }",
			"view! { <div class=\"card\">\"Hi\"</div> }",
			"use leptos::prelude::*;\n\nview! { <Card /> }",
			"macro_rules! view { () => {} }",
		] {
			assert_eq!(convert(source), None, "{source}");
		}
	}

	#[test]
	fn converts_a_bare_view_when_leptoskit_provides_it() {
		assert_eq!(
			convert("use leptos::prelude::*;\nuse leptoskit::prelude::*;\n\nview! { <View /> }"),
			Some("use leptos::prelude::*;\nuse leptoskit::prelude::*;\n\nView()".to_owned()),
		);
		assert_eq!(
			convert("leptoskit::view! { <View /> }"),
			Some("View()".to_owned())
		);
	}

	#[test]
	fn converts_leptoskit_markup_inside_leptos_markup() {
		assert_eq!(
			convert(
				"leptos::view! { <main>{leptoskit::web::render(view! { <Text>\"Shared\"</Text> \
				 })}</main> }"
			),
			Some(
				"leptos::view! { <main>{leptoskit::web::render(Text(\"Shared\"))}</main> }"
					.to_owned()
			),
		);
	}

	#[test]
	fn calls_built_in_components_with_their_arguments() {
		for (markup, builder) in [
			(
				r#"<Text class="title">"Hello"</Text>"#,
				r#"Text("Hello").class("title")"#,
			),
			(
				r#"<Text>"Count: " {count}</Text>"#,
				r#"Text(format!("Count: {count}"))"#,
			),
			(
				r#"<Text>"Total: " {cart.total()} "."</Text>"#,
				r#"Text(format!("Total: {}.", cart.total()))"#,
			),
			(
				r#"<Text>"{braces}" {count}</Text>"#,
				r#"Text(format!("{{braces}}{count}"))"#,
			),
			(
				r#"<Heading aria_level=3>"Settings"</Heading>"#,
				r#"Heading(3, "Settings")"#,
			),
			(
				r#"<Heading aria_level=level>"Settings"</Heading>"#,
				r#"Heading(2, "Settings").aria_level(level)"#,
			),
			(
				r#"<Button on:press=save>"Save"</Button>"#,
				r#"Button("Save").on_press(save)"#,
			),
			(r#"<Status>"Saved"</Status>"#, r#"Status("Saved")"#),
			("<Image />", r#"Image("")"#),
			(
				r#"<Image source=photo accessibility_label="Sunset" />"#,
				r#"Image(photo).accessibility_label("Sunset")"#,
			),
			(
				"<Switch checked=on on:toggle=set />",
				"Switch(on).on_toggle(set)",
			),
			("<Checkbox />", "Checkbox(false)"),
			(
				"<Radio radio_group=group checked=true />",
				"Radio(group, true)",
			),
			("<ActivityIndicator />", "ActivityIndicator(true)"),
			(
				r#"<Pressable accessibility_label="Open" />"#,
				r#"Pressable("Open")"#,
			),
			("<TextInput disabled />", "TextInput().disabled(true)"),
			(
				"<TextInput value=(name, set_name) />",
				"TextInput().value(name, set_name)",
			),
			("<helpers::Card />", "helpers::Card()"),
			("<View>\"Loose\"</View>", "View().child(Text(\"Loose\"))"),
			(
				r#"<View><Text>"a"</Text><Text>"b"</Text></View>"#,
				r#"View().child(Text("a")).child(Text("b"))"#,
			),
			(
				r#"<><Text>"a"</Text><Text>"b"</Text></>"#,
				r#"Fragment::new().with(Text("a")).with(Text("b"))"#,
			),
			(
				r#"<Text>"a"</Text><Text>"b"</Text>"#,
				r#"Fragment::new().with(Text("a")).with(Text("b"))"#,
			),
			("<View>{children}</View>", "View().children(children)"),
			(
				"<View>{rows.iter().map(row)}</View>",
				"View().children(rows.iter().map(row))",
			),
			("<View>{header()}</View>", "View().child(header())"),
			(
				"<View>{move || label.get()}</View>",
				"View().children(Fragment::new().with(move || label.get()))",
			),
		] {
			assert_eq!(
				convert(&format!("view! {{ {markup} }}")).as_deref(),
				Some(builder),
				"{markup}"
			);
		}
	}

	#[test]
	fn calls_components_defined_on_the_page_in_parameter_order() {
		let components = badge();
		for (markup, builder) in [
			(
				r#"<Badge label="New" count=3 tone=Tone::Warning />"#,
				r#"Badge("New".into(), Some(3), Tone::Warning, Fragment::new())"#,
			),
			(
				r#"<Badge label="New" />"#,
				r#"Badge("New".into(), None, Tone::Info, Fragment::new())"#,
			),
			(
				r#"<Badge label=a + b><Text>"!"</Text></Badge>"#,
				"Badge(\n\t(a + \
				 b).into(),\n\tNone,\n\tTone::Info,\n\tFragment::new().with(Text(\"!\")),\n)",
			),
			(
				r#"<View><Badge label="New" /></View>"#,
				r#"View().child(Badge("New".into(), None, Tone::Info, Fragment::new()).into_view())"#,
			),
		] {
			assert_eq!(
				convert_with(&format!("view! {{ {markup} }}"), &components).as_deref(),
				Some(builder),
				"{markup}"
			);
		}
	}

	#[test]
	fn builds_components_from_elsewhere_with_their_props_builder() {
		let panel = [
			"Panel::builder()",
			r#".title("Hi")"#,
			r#".children(Fragment::new().with(Text("Body")))"#,
			".build()",
		]
		.join("\n\t");
		assert_eq!(
			convert(r#"view! { <Panel title="Hi"><Text>"Body"</Text></Panel> }"#).as_deref(),
			Some(panel.as_str()),
		);
		assert_eq!(
			convert(r#"view! { <View><Panel title="Hi" /></View> }"#).as_deref(),
			Some(r#"View().child(Panel::builder().title("Hi").build().into_view())"#),
		);
	}

	#[test]
	fn builds_a_component_the_page_defines_twice_with_its_props_builder() {
		let components = PageComponents::collect([
			"#[component]\nfn Counter(count: u32) -> Element { todo!() }",
			"#[component]\nfn Counter(count: u32, step: u32) -> Element { todo!() }",
		]);
		assert_eq!(
			convert_with("view! { <Counter count=1 /> }", &components).as_deref(),
			Some("Counter::builder().count(1).build()"),
		);
	}

	#[test]
	fn keeps_comments_above_the_calls_they_describe() {
		let source = "view! {\n\t<View\n\t\t// Fills the \
		              screen.\n\t\tclass=\"flex-1\"\n\t>\n\t\t// The \
		              title.\n\t\t<Text>\"Hi\"</Text>\n\t</View>\n}";
		assert_eq!(
			convert(source).as_deref(),
			Some(
				"View()\n\t// Fills the screen.\n\t.class(\"flex-1\")\n\t// The \
				 title.\n\t.child(Text(\"Hi\"))"
			),
		);
	}

	#[test]
	fn keeps_the_indentation_a_block_uses() {
		let source = "fn card() -> Element {\n    view! {\n        <View class=\"p-4 bg-white \
		              rounded-lg shadow\">\n            <Text>\"Hello there\"</Text>\n        \
		              </View>\n    }\n}";
		assert_eq!(
			convert(source).as_deref(),
			Some(
				"fn card() -> Element {\n    View()\n        .class(\"p-4 bg-white rounded-lg \
				 shadow\")\n        .child(Text(\"Hello there\"))\n}"
			),
		);
	}

	#[test]
	fn lays_out_long_chains_like_rustfmt() {
		assert_eq!(
			convert(
				"let panel = view! { <View class=\"p-4 bg-white rounded-lg\" \
				 aria_label=\"Profile\"><Text>\"Ada\"</Text></View> };"
			)
			.as_deref(),
			Some(
				"let panel = View()\n\t.class(\"p-4 bg-white \
				 rounded-lg\")\n\t.aria_label(\"Profile\")\n\t.child(Text(\"Ada\"));"
			),
		);
	}

	#[test]
	fn reports_markup_without_a_builder_form_at_its_position() {
		let none = PageComponents::default();
		let components = badge();
		for (source, components, line, column, message) in [
			(
				"view! {\n\t<View>\n\t\t<Text>\"Hi\"</Text>\n\t\t// Nothing follows.\n\t</View>\n}",
				&none,
				4,
				3,
				"the comment `// Nothing follows.` follows the last child; move it above a child",
			),
			(
				"view! {\n\t// The screen.\n\t<View />\n}",
				&none,
				2,
				2,
				"the comment `// The screen.` has no place in builder calls; move it out of the \
				 `view!`",
			),
			(
				"view! { <Text>\"Count: \" {move || count.get()}</Text> }",
				&none,
				1,
				26,
				"a closure cannot join literal text; write the whole label as one `{move || \
				 format!(…)}`",
			),
			(
				"view! { <Pressable /> }",
				&none,
				1,
				10,
				"`Pressable()` needs `accessibility_label=…` to call it with",
			),
			(
				"leptoskit::view! { <div /> }",
				&none,
				1,
				21,
				"`<div>` is not a component; component names start with a capital letter",
			),
			(
				"view! { <Badge /> }",
				&components,
				1,
				10,
				"`<Badge>` is missing the required prop `label`",
			),
			(
				"view! { <Badge label=\"x\" shade=1 /> }",
				&components,
				1,
				26,
				"`<Badge>` has no prop `shade`",
			),
			(
				"view! { <View on:press:capture=go /> }",
				&none,
				1,
				15,
				"`on:press:capture` is not an attribute: use a prop name, or `on:event` for a \
				 handler",
			),
			(
				"view! { <View class= /> }",
				&none,
				1,
				7,
				"this `view!` markup does not parse",
			),
		] {
			assert_eq!(
				failure(source, components),
				Unconvertible {
					line,
					column,
					message: message.to_owned(),
				},
				"{source}"
			);
		}
	}

	#[test]
	fn reads_one_level_of_indentation() {
		assert_eq!(indent_unit("fn a() {\n\tb();\n}"), "\t");
		assert_eq!(indent_unit("fn a() {\n  b(\n    c,\n  );\n}"), "  ");
		assert_eq!(indent_unit("view! { <View /> }"), "\t");
	}
}

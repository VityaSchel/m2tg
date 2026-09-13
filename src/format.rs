use html5ever::tendril::TendrilSink;
use html5ever::{Attribute, ParseOpts, QualName, local_name, ns, parse_fragment};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use crate::matrix::Content;

const MAX_DEPTH: usize = 64;
const LIST_INDENT: &str = "  ";

pub fn render_text(content: &Content, plain: &str) -> String {
	match (content.format.as_deref(), content.formatted_body.as_deref()) {
		(Some("org.matrix.custom.html"), Some(html)) => {
			to_telegram(html).unwrap_or_else(|| escape_html(plain))
		}
		_ => escape_html(plain),
	}
}

fn to_telegram(html: &str) -> Option<String> {
	let context = QualName::new(None, ns!(html), local_name!("div"));
	let dom = parse_fragment(
		RcDom::default(),
		ParseOpts::default(),
		context,
		vec![],
		false,
	)
	.one(html);
	let mut renderer = Renderer::default();
	renderer.children(&dom.document).ok()?;
	Some(renderer.out.trim().to_string())
}

struct TooDeep;

type Rendered = Result<(), TooDeep>;

enum List {
	Bulleted,
	Numbered(i64),
}

#[derive(Default)]
struct Renderer {
	out: String,
	line_start: usize,
	depth: usize,
	lists: Vec<List>,
	in_blockquote: bool,
	in_link: bool,
	in_spoiler: bool,
	in_code: bool,
	in_pre: bool,
}

impl Renderer {
	fn children(&mut self, node: &Handle) -> Rendered {
		for child in node.children.borrow().iter() {
			self.node(child)?;
		}
		Ok(())
	}

	fn node(&mut self, node: &Handle) -> Rendered {
		match &node.data {
			NodeData::Text { contents } => {
				self.text(&contents.borrow());
				Ok(())
			}
			NodeData::Element { name, attrs, .. } => {
				if self.depth == MAX_DEPTH {
					return Err(TooDeep);
				}
				self.depth += 1;
				self.element(node, name.local.as_ref(), &attrs.borrow())?;
				self.depth -= 1;
				Ok(())
			}
			_ => Ok(()),
		}
	}

	fn text(&mut self, text: &str) {
		if self.in_pre {
			escape_into(text, &mut self.out);
			return;
		}
		if text.contains('\n') && text.trim().is_empty() {
			if !self.at_line_start() {
				self.out.push('\n');
			}
			return;
		}
		let text = if self.at_line_start() {
			text.trim_start_matches('\n')
		} else {
			text
		};
		escape_into(text, &mut self.out);
	}

	fn element(&mut self, node: &Handle, tag: &str, attrs: &[Attribute]) -> Rendered {
		if self.in_code {
			if tag == "br" {
				self.out.push('\n');
				return Ok(());
			}
			return self.children(node);
		}
		match tag {
			"b" | "strong" => self.wrap(node, "b"),
			"i" | "em" => self.wrap(node, "i"),
			"u" | "ins" => self.wrap(node, "u"),
			"s" | "strike" | "del" => self.wrap(node, "s"),
			"tg-spoiler" => self.spoiler(node),
			"span" if attr(attrs, "data-mx-spoiler").is_some() => self.spoiler(node),
			"a" => self.link(node, attrs),
			"code" => self.code(node),
			"pre" => self.pre(node),
			"blockquote" => self.blockquote(node),
			"h1" | "h2" | "h3" | "h4" | "h5" | "h6" => self.block(|r| r.wrap(node, "b")),
			"p" | "div" | "table" => self.block(|r| r.children(node)),
			"ul" => self.list(node, List::Bulleted),
			"ol" => self.list(node, List::Numbered(list_start(attrs))),
			"li" => self.list_item(node),
			"tr" => self.table_row(node),
			"br" => {
				self.out.push('\n');
				Ok(())
			}
			"hr" => {
				self.ensure_newline();
				Ok(())
			}
			"img" => {
				if let Some(text) = image_text(attrs) {
					escape_into(text, &mut self.out);
				}
				Ok(())
			}
			"mx-reply" => Ok(()),
			_ => self.children(node),
		}
	}

	fn wrap(&mut self, node: &Handle, tag: &str) -> Rendered {
		self.enclose(&format!("<{tag}>"), &format!("</{tag}>"), |r| {
			r.children(node)
		})
	}

	fn enclose(
		&mut self,
		open: &str,
		close: &str,
		render: impl FnOnce(&mut Self) -> Rendered,
	) -> Rendered {
		let at_line_start = self.at_line_start();
		self.out.push_str(open);
		if at_line_start {
			self.line_start = self.out.len();
		}
		render(self)?;
		self.out.push_str(close);
		Ok(())
	}

	fn block(&mut self, render: impl FnOnce(&mut Self) -> Rendered) -> Rendered {
		self.ensure_blank_line();
		render(self)?;
		self.ensure_blank_line();
		Ok(())
	}

	fn link(&mut self, node: &Handle, attrs: &[Attribute]) -> Rendered {
		let href = attr(attrs, "href")
			.map(str::trim)
			.filter(|href| scheme_ok(href));
		let Some(href) = href.filter(|_| !self.in_link) else {
			return self.children(node);
		};
		let mut open = String::from("<a href=\"");
		escape_into(href, &mut open);
		open.push_str("\">");
		self.in_link = true;
		self.enclose(&open, "</a>", |r| r.children(node))?;
		self.in_link = false;
		Ok(())
	}

	fn spoiler(&mut self, node: &Handle) -> Rendered {
		let in_spoiler = std::mem::replace(&mut self.in_spoiler, true);
		self.wrap(node, "tg-spoiler")?;
		self.in_spoiler = in_spoiler;
		Ok(())
	}

	fn code_tags_allowed(&self) -> bool {
		!self.in_link && !self.in_spoiler
	}

	fn code(&mut self, node: &Handle) -> Rendered {
		let (open, close) = if self.code_tags_allowed() {
			("<code>", "</code>")
		} else {
			("", "")
		};
		self.in_code = true;
		self.enclose(open, close, |r| r.children(node))?;
		self.in_code = false;
		Ok(())
	}

	fn pre(&mut self, node: &Handle) -> Rendered {
		let (open, close) = match code_language(node) {
			_ if !self.code_tags_allowed() => (String::new(), ""),
			Some(language) => (
				format!("<pre><code class=\"language-{language}\">"),
				"</code></pre>",
			),
			None => ("<pre>".to_string(), "</pre>"),
		};
		self.block(|r| {
			r.in_code = true;
			r.in_pre = true;
			r.enclose(&open, close, |r| {
				r.children(node)?;
				r.trim_trailing_newlines();
				Ok(())
			})?;
			r.in_code = false;
			r.in_pre = false;
			Ok(())
		})
	}

	fn blockquote(&mut self, node: &Handle) -> Rendered {
		if self.in_blockquote {
			return self.block(|r| r.children(node));
		}
		self.block(|r| {
			r.in_blockquote = true;
			r.enclose("<blockquote>", "</blockquote>", |r| {
				r.children(node)?;
				r.trim_trailing_newlines();
				Ok(())
			})?;
			r.in_blockquote = false;
			Ok(())
		})
	}

	fn list(&mut self, node: &Handle, list: List) -> Rendered {
		self.ensure_newline();
		self.lists.push(list);
		self.children(node)?;
		self.lists.pop();
		self.ensure_newline();
		Ok(())
	}

	fn list_item(&mut self, node: &Handle) -> Rendered {
		self.ensure_newline();
		for _ in 1..self.lists.len() {
			self.out.push_str(LIST_INDENT);
		}
		match self.lists.last_mut() {
			Some(List::Numbered(number)) => {
				self.out.push_str(&number.to_string());
				self.out.push_str(". ");
				*number = number.saturating_add(1);
			}
			_ => self.out.push_str("• "),
		}
		self.line_start = self.out.len();
		self.children(node)
	}

	fn table_row(&mut self, node: &Handle) -> Rendered {
		self.ensure_newline();
		let mut first_cell = true;
		for child in node.children.borrow().iter() {
			if is_blank_text(child) {
				continue;
			}
			if is_table_cell(child) {
				if !first_cell {
					self.out.push_str(" | ");
				}
				first_cell = false;
			}
			self.node(child)?;
		}
		self.ensure_newline();
		Ok(())
	}

	fn at_line_start(&self) -> bool {
		self.out.len() == self.line_start || self.out.ends_with('\n')
	}

	fn ensure_newline(&mut self) {
		if !self.at_line_start() {
			self.out.push('\n');
		}
	}

	fn ensure_blank_line(&mut self) {
		if self.out.len() == self.line_start {
			return;
		}
		let trailing = self
			.out
			.bytes()
			.rev()
			.take_while(|&b| b == b'\n')
			.take(2)
			.count();
		for _ in trailing..2 {
			self.out.push('\n');
		}
	}

	fn trim_trailing_newlines(&mut self) {
		let trimmed = self.out.trim_end_matches('\n').len();
		self.out.truncate(trimmed);
	}
}

fn attr<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a str> {
	attrs
		.iter()
		.find(|a| a.name.local.as_ref() == name)
		.map(|a| a.value.as_ref())
}

fn list_start(attrs: &[Attribute]) -> i64 {
	attr(attrs, "start")
		.and_then(|start| start.trim().parse().ok())
		.unwrap_or(1)
}

fn image_text(attrs: &[Attribute]) -> Option<&str> {
	attr(attrs, "alt")
		.filter(|alt| !alt.is_empty())
		.or_else(|| attr(attrs, "title"))
}

fn is_blank_text(node: &Handle) -> bool {
	match &node.data {
		NodeData::Text { contents } => contents.borrow().trim().is_empty(),
		_ => false,
	}
}

fn is_table_cell(node: &Handle) -> bool {
	match &node.data {
		NodeData::Element { name, .. } => matches!(name.local.as_ref(), "td" | "th"),
		_ => false,
	}
}

fn code_language(pre: &Handle) -> Option<String> {
	let children = pre.children.borrow();
	let [child] = children.as_slice() else {
		return None;
	};
	let NodeData::Element { name, attrs, .. } = &child.data else {
		return None;
	};
	if name.local.as_ref() != "code" {
		return None;
	}
	let attrs = attrs.borrow();
	let language: String = attr(&attrs, "class")?
		.split_ascii_whitespace()
		.find_map(|class| class.strip_prefix("language-"))?
		.chars()
		.filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-'))
		.collect();
	(!language.is_empty()).then_some(language)
}

fn scheme_ok(url: &str) -> bool {
	let url = url.to_ascii_lowercase();
	url.starts_with("http://") || url.starts_with("https://")
}

pub fn escape_html(text: &str) -> String {
	let mut out = String::with_capacity(text.len());
	escape_into(text, &mut out);
	out
}

fn escape_into(text: &str, out: &mut String) {
	for c in text.chars() {
		match c {
			'<' => out.push_str("&lt;"),
			'>' => out.push_str("&gt;"),
			'&' => out.push_str("&amp;"),
			'"' => out.push_str("&quot;"),
			_ => out.push(c),
		}
	}
}

#[cfg(test)]
mod tests;

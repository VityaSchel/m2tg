use super::{render_text, to_telegram};
use crate::matrix::Content;

fn telegram(html: &str) -> String {
	to_telegram(html).expect("html within depth cap")
}

fn html_content(html: String) -> Content {
	Content {
		format: Some("org.matrix.custom.html".into()),
		formatted_body: Some(html),
		..Default::default()
	}
}

#[test]
fn render_text_uses_formatted_body_when_custom_html() {
	let content = html_content("<strong>x</strong>".into());
	assert_eq!(render_text(&content, "ignored plaintext"), "<b>x</b>");
}

#[test]
fn render_text_falls_back_to_escaped_plaintext() {
	let content = Content::default();
	assert_eq!(render_text(&content, "a < b & c"), "a &lt; b &amp; c");
}

#[test]
fn render_text_ignores_formatted_body_without_custom_html_format() {
	let content = Content {
		formatted_body: Some("<b>x</b>".into()),
		..Default::default()
	};
	assert_eq!(render_text(&content, "plain"), "plain");
}

#[test]
fn render_text_falls_back_to_plain_when_nesting_is_too_deep() {
	let rendered = std::thread::Builder::new()
		.stack_size(2 * 1024 * 1024)
		.spawn(|| {
			let html = format!("{}x{}", "<b>".repeat(20000), "</b>".repeat(20000));
			render_text(&html_content(html), "x < y")
		})
		.unwrap()
		.join()
		.unwrap();
	assert_eq!(rendered, "x &lt; y");
}

#[test]
fn moderate_nesting_is_rendered() {
	let html = format!("{}x{}", "<i>".repeat(30), "</i>".repeat(30));
	assert_eq!(telegram(&html), html);
}

#[test]
fn renames_and_escapes() {
	assert_eq!(
		telegram("<strong>a</strong> &amp; <em>b</em>"),
		"<b>a</b> &amp; <i>b</i>"
	);
}

#[test]
fn drops_reply_fallback() {
	let html = "<mx-reply><blockquote><a href=\"https://matrix.to/#/!room:example.org/$event\">In reply to</a> <a href=\"https://matrix.to/#/@alice:example.org\">@alice:example.org</a><br>quoted</blockquote></mx-reply>my reply";
	assert_eq!(telegram(html), "my reply");
}

#[test]
fn spoiler_span() {
	assert_eq!(
		telegram("<span data-mx-spoiler>secret</span>"),
		"<tg-spoiler>secret</tg-spoiler>"
	);
}

#[test]
fn escapes_quote_in_href() {
	assert_eq!(
		telegram("<a href=\"https://e.com/a&quot;b\">t</a>"),
		"<a href=\"https://e.com/a&quot;b\">t</a>"
	);
}

#[test]
fn keeps_good_link_and_escapes_href() {
	assert_eq!(
		telegram("<a href=\"https://e.com/a&amp;b\">t</a>"),
		"<a href=\"https://e.com/a&amp;b\">t</a>"
	);
}

#[test]
fn drops_mailto_link() {
	assert_eq!(
		telegram("<a href=\"mailto:a@example.org\">mail</a>"),
		"mail"
	);
}

#[test]
fn code_inside_link_or_spoiler_loses_code_tags() {
	assert_eq!(
		telegram("<a href=\"https://x.org\"><code>foo()</code></a>"),
		"<a href=\"https://x.org\">foo()</a>"
	);
	assert_eq!(
		telegram("<span data-mx-spoiler><code>s</code></span>"),
		"<tg-spoiler>s</tg-spoiler>"
	);
	assert_eq!(
		telegram("<tg-spoiler><b>a</b></tg-spoiler> <code>b</code>"),
		"<tg-spoiler><b>a</b></tg-spoiler> <code>b</code>"
	);
}

#[test]
fn rejects_bad_link_scheme() {
	assert_eq!(telegram("<a href=\"javascript:alert(1)\">x</a>"), "x");
}

#[test]
fn rejects_tg_link_scheme() {
	assert_eq!(telegram("<a href=\"tg://resolve?domain=x\">x</a>"), "x");
}

#[test]
fn nested_link_keeps_inner_text() {
	let html = "<a href=\"https://a.example\">outer <object><a href=\"https://b.example\">inner</a></object></a>";
	assert_eq!(
		telegram(html),
		"<a href=\"https://a.example\">outer inner</a>"
	);
}

#[test]
fn custom_emoji_uses_alt_text() {
	let html = "hi <img data-mx-emoticon src=\"mxc://example.org/abc\" alt=\":blob&lt;3:\" title=\":blob:\" height=\"32\" />";
	assert_eq!(telegram(html), "hi :blob&lt;3:");
}

#[test]
fn image_falls_back_to_title() {
	assert_eq!(
		telegram("<img src=\"mxc://example.org/abc\" title=\":cat:\">"),
		":cat:"
	);
}

#[test]
fn image_without_text_is_dropped() {
	assert_eq!(telegram("a<img src=\"mxc://example.org/abc\">b"), "ab");
}

#[test]
fn horizontal_rule_becomes_line_break() {
	assert_eq!(telegram("above<hr>below"), "above\nbelow");
	assert_eq!(
		telegram("<p>above</p>\n<hr />\n<p>below</p>\n"),
		"above\n\nbelow"
	);
}

#[test]
fn paragraphs_are_separated_by_one_blank_line() {
	assert_eq!(telegram("<p>one</p>\n<p>two</p>\n"), "one\n\ntwo");
}

#[test]
fn blocks_never_produce_three_newlines() {
	let html = "<p>a</p>\n\n<div><p>b</p></div>\n<h2>c</h2>\n<p>d<br></p>\n<div>e</div>";
	let rendered = telegram(html);
	assert_eq!(rendered, "a\n\nb\n\n<b>c</b>\n\nd\n\ne");
	assert!(!rendered.contains("\n\n\n"));
}

#[test]
fn heading_is_bold_block() {
	assert_eq!(
		telegram("<h1>Title</h1>\n<p>body</p>\n"),
		"<b>Title</b>\n\nbody"
	);
}

#[test]
fn line_breaks() {
	assert_eq!(
		telegram("first<br />second<br />\nthird"),
		"first\nsecond\nthird"
	);
}

#[test]
fn inline_whitespace_is_kept() {
	assert_eq!(telegram("<b>a</b> <i>b</i>"), "<b>a</b> <i>b</i>");
	assert_eq!(
		telegram("<p><em>a</em>\n<strong>b</strong></p>"),
		"<i>a</i>\n<b>b</b>"
	);
}

#[test]
fn bulleted_list() {
	assert_eq!(
		telegram("<ul>\n<li>one</li>\n<li>two</li>\n</ul>\n"),
		"• one\n• two"
	);
}

#[test]
fn numbered_list_honors_start() {
	assert_eq!(
		telegram("<ol start=\"3\">\n<li>three</li>\n<li>four</li>\n</ol>\n"),
		"3. three\n4. four"
	);
}

#[test]
fn numbered_list_start_does_not_overflow() {
	assert_eq!(
		telegram("<ol start=\"9223372036854775807\"><li>a</li><li>b</li></ol>"),
		"9223372036854775807. a\n9223372036854775807. b"
	);
}

#[test]
fn numbered_list_defaults_to_one() {
	assert_eq!(
		telegram("<ol>\n<li>a</li>\n<li>b</li>\n</ol>\n"),
		"1. a\n2. b"
	);
}

#[test]
fn nested_list_starts_on_new_line() {
	let html = "<ul>\n<li>one\n<ol>\n<li>nested</li>\n</ol>\n</li>\n<li>two<ul><li>tight</li></ul></li>\n</ul>\n";
	assert_eq!(telegram(html), "• one\n  1. nested\n• two\n  • tight");
}

#[test]
fn loose_list_items_are_spaced() {
	let html = "<ul>\n<li>\n<p>a</p>\n</li>\n<li>\n<p>b</p>\n</li>\n</ul>\n";
	assert_eq!(telegram(html), "• a\n\n• b");
}

#[test]
fn list_between_paragraphs() {
	let html = "<p>intro</p>\n<ul>\n<li>a</li>\n</ul>\n<p>outro</p>\n";
	assert_eq!(telegram(html), "intro\n\n• a\n\noutro");
}

#[test]
fn text_before_list_breaks_line() {
	assert_eq!(
		telegram("intro<ul><li>a</li></ul>outro"),
		"intro\n• a\noutro"
	);
}

#[test]
fn blockquote_trims_inner_newlines() {
	let html =
		"<blockquote>\n<p><strong>quoted</strong> line</p>\n</blockquote>\n<p>my reply</p>\n";
	assert_eq!(
		telegram(html),
		"<blockquote><b>quoted</b> line</blockquote>\n\nmy reply"
	);
}

#[test]
fn blockquote_with_multiple_paragraphs() {
	let html = "<blockquote>\n<p>one</p>\n<p>two</p>\n</blockquote>\n";
	assert_eq!(telegram(html), "<blockquote>one\n\ntwo</blockquote>");
}

#[test]
fn nested_blockquote_is_flattened() {
	let html =
		"<blockquote>\n<p>outer</p>\n<blockquote>\n<p>inner</p>\n</blockquote>\n</blockquote>\n";
	assert_eq!(telegram(html), "<blockquote>outer\n\ninner</blockquote>");
}

#[test]
fn code_block_with_language() {
	let html = "<pre><code class=\"language-rust\">fn main() {\n\n\n    println!(\"&lt;hi&gt;\");\n}\n</code></pre>\n<p>after</p>\n";
	assert_eq!(
		telegram(html),
		"<pre><code class=\"language-rust\">fn main() {\n\n\n    println!(&quot;&lt;hi&gt;&quot;);\n}</code></pre>\n\nafter"
	);
}

#[test]
fn code_block_language_is_sanitized() {
	let html = "<pre><code class=\"hljs language-c++&quot;&gt;x\">a</code></pre>";
	assert_eq!(
		telegram(html),
		"<pre><code class=\"language-c++x\">a</code></pre>"
	);
}

#[test]
fn code_block_without_language() {
	assert_eq!(
		telegram("<pre><code>plain\n</code></pre>\n"),
		"<pre>plain</pre>"
	);
}

#[test]
fn pre_keeps_whitespace_and_strips_entities() {
	let html = "<pre>\n  <b>a</b>\n\n\n  <i>b</i><br>c</pre>";
	assert_eq!(telegram(html), "<pre>  a\n\n\n  b\nc</pre>");
}

#[test]
fn inline_code_strips_entities() {
	assert_eq!(
		telegram("run <code><b>cargo</b> &lt;test&gt; <a href=\"https://e.com\">x</a></code>"),
		"run <code>cargo &lt;test&gt; x</code>"
	);
}

#[test]
fn table_rows_and_cells() {
	let html = "<table>\n<thead>\n<tr>\n<th>Name</th>\n<th>Value</th>\n</tr>\n</thead>\n<tbody>\n<tr>\n<td>a</td>\n<td>1</td>\n</tr>\n</tbody>\n</table>\n<p>after</p>\n";
	assert_eq!(telegram(html), "Name | Value\na | 1\n\nafter");
}

#[test]
fn unknown_tag_keeps_text() {
	assert_eq!(
		telegram("<font color=\"red\">red</font> x<sup>2</sup>"),
		"red x2"
	);
}

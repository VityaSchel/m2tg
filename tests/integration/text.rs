use super::*;

#[tokio::test]
async fn text_message_is_sent_as_escaped_html() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;

	put_txn(
		&app,
		"t1",
		vec![text("$1", "if a < b && c > d then **ship**")],
	)
	.await;

	let calls = mock.wait_for_calls(1).await;
	let call = &calls[0];
	assert_eq!(call.method, "sendMessage");
	assert_eq!(call.param("chat_id"), Some(CHAT_ID));
	assert_eq!(call.param("parse_mode"), Some("HTML"));
	assert_eq!(call.text(), "if a &lt; b &amp;&amp; c &gt; d then **ship**");
	assert_eq!(call.params["link_preview_options"]["is_disabled"], true);
}

#[tokio::test]
async fn formatted_body_is_converted_to_telegram_html() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	let formatted_body = concat!(
		"<p>Release <strong>2.0</strong> is <a href=\"https://example.com/notes\">out</a>!</p>\n",
		"<ul>\n<li>faster <code>sync</code></li>\n<li><del>bugs</del></li>\n</ul>\n",
		"<pre><code class=\"language-rust\">fn main() {}\n</code></pre>\n",
	);

	put_txn(
		&app,
		"t1",
		vec![html(
			"$1",
			"Release **2.0** is [out](https://example.com/notes)!",
			formatted_body,
		)],
	)
	.await;

	let calls = mock.wait_for_calls(1).await;
	assert_eq!(
		calls[0].text(),
		concat!(
			"Release <b>2.0</b> is <a href=\"https://example.com/notes\">out</a>!\n\n",
			"• faster <code>sync</code>\n• <s>bugs</s>\n\n",
			"<pre><code class=\"language-rust\">fn main() {}</code></pre>",
		)
	);
	assert_eq!(calls[0].param("parse_mode"), Some("HTML"));
}

#[tokio::test]
async fn emoji_only_message_sends_emote_alt_text() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	let formatted_body = concat!(
		"<img data-mx-emoticon=\"\" src=\"mxc://media.test/blob\" alt=\":blob:\" ",
		"title=\":blob:\" height=\"32\" vertical-align=\"middle\" />",
	);

	put_txn(&app, "t1", vec![html("$1", ":blob:", formatted_body)]).await;

	let calls = mock.wait_for_calls(1).await;
	assert_eq!(texts(&calls), [":blob:"]);
}

#[tokio::test]
async fn quote_only_message_without_reply_is_delivered() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	let formatted = html(
		"$formatted",
		"> to be or not to be",
		"<blockquote>\n<p>to be or not to be</p>\n</blockquote>\n",
	);
	let plain = text("$plain", "> just a quote");

	put_txn(&app, "t1", vec![formatted, plain]).await;

	let calls = mock.wait_for_calls(2).await;
	assert_eq!(
		texts(&calls),
		[
			"<blockquote>to be or not to be</blockquote>",
			"&gt; just a quote"
		]
	);
}

#[tokio::test]
async fn reply_fallback_is_stripped_when_in_reply_to_is_present() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	let in_reply_to = json!({ "m.in_reply_to": { "event_id": "$original" } });
	let formatted_reply = message(
		"$formatted",
		json!({
			"msgtype": "m.text",
			"body": "> <@bob:test> original message\n\nI agree",
			"format": "org.matrix.custom.html",
			"formatted_body": concat!(
				"<mx-reply><blockquote>",
				"<a href=\"https://matrix.to/#/!room:test/$original\">In reply to</a> ",
				"<a href=\"https://matrix.to/#/@bob:test\">@bob:test</a><br>original message",
				"</blockquote></mx-reply>I agree",
			),
			"m.relates_to": in_reply_to,
		}),
	);
	let plain_reply = message(
		"$plain",
		json!({
			"msgtype": "m.text",
			"body": "> <@bob:test> original message\n> second line\n\nplain reply",
			"m.relates_to": in_reply_to,
		}),
	);

	put_txn(&app, "t1", vec![formatted_reply, plain_reply]).await;

	let calls = mock.wait_for_calls(2).await;
	assert_eq!(texts(&calls), ["I agree", "plain reply"]);
}

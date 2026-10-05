use super::*;

#[tokio::test]
async fn text_after_image_becomes_its_caption() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("greedy", "image/png", PNG);
	let caption = html(
		"$txt",
		"Meet [Greedy](https://example.org)!\nJoin us",
		"Meet <a href=\"https://example.org\">Greedy</a>!<br>Join us",
	);

	put_txn(&app, "t1", vec![image("$img", "greedy"), caption]).await;

	let calls = mock.wait_for_calls(1).await;
	assert_eq!(methods(&calls), ["sendPhoto"]);
	assert_eq!(
		calls[0].param("caption"),
		Some("Meet <a href=\"https://example.org\">Greedy</a>!\nJoin us")
	);
	assert_eq!(calls[0].param("parse_mode"), Some("HTML"));
	assert_eq!(calls[0].files[0].filename, "greedy.png");

	put_txn(&app, "t2", vec![last_message()]).await;

	let calls = mock.wait_for_calls(2).await;
	assert_eq!(texts(&calls), ["", LAST_MESSAGE]);
}

#[tokio::test]
async fn text_after_album_captions_its_first_item() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("first", "image/png", PNG);
	mock.serve_media("second", "image/png", PNG);

	put_txn(
		&app,
		"t1",
		vec![
			image("$1", "first"),
			image("$2", "second"),
			text("$3", "two pictures"),
		],
	)
	.await;

	let calls = mock.wait_for_calls(1).await;
	assert_eq!(methods(&calls), ["sendMediaGroup"]);
	let entries = calls[0].media_group();
	assert_eq!(entries.len(), 2);
	assert_eq!(entries[0]["caption"], "two pictures");
	assert_eq!(entries[0]["parse_mode"], "HTML");
	assert!(entries[1].get("caption").is_none());
}

#[tokio::test]
async fn text_after_file_becomes_its_caption() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("notes", "text/plain", b"notes");
	let file = message(
		"$file",
		json!({ "msgtype": "m.file", "body": "notes.txt", "url": mxc("notes") }),
	);

	put_txn(&app, "t1", vec![file, text("$txt", "meeting notes")]).await;

	let calls = mock.wait_for_calls(1).await;
	assert_eq!(methods(&calls), ["sendDocument"]);
	assert_eq!(calls[0].param("caption"), Some("meeting notes"));
	assert_eq!(calls[0].files[0].filename, "notes.txt");
}

#[tokio::test]
async fn only_the_first_text_becomes_the_caption() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("cat", "image/png", PNG);

	put_txn(
		&app,
		"t1",
		vec![
			image("$img", "cat"),
			text("$1", "caption"),
			text("$2", "comment"),
		],
	)
	.await;

	let calls = mock.wait_for_calls(2).await;
	assert_eq!(methods(&calls), ["sendPhoto", "sendMessage"]);
	assert_eq!(calls[0].param("caption"), Some("caption"));
	assert_eq!(calls[1].text(), "comment");
}

#[tokio::test]
async fn text_from_another_sender_is_not_a_caption() {
	assert_text_stays_a_message(from(BOB, text("$txt", "not mine")), "not mine").await;
}

#[tokio::test]
async fn reply_is_not_a_caption() {
	let reply = message(
		"$txt",
		json!({
			"msgtype": "m.text",
			"body": "> <@bob:test> question\n\nanswer",
			"m.relates_to": { "m.in_reply_to": { "event_id": "$question" } },
		}),
	);
	assert_text_stays_a_message(reply, "answer").await;
}

#[tokio::test]
async fn text_sent_exactly_at_the_end_of_the_hold_is_a_caption() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("cat", "image/png", PNG);
	let caption = sent_after(Duration::from_secs(10), text("$txt", "just in time"));

	put_txn(&app, "t1", vec![image("$img", "cat"), caption]).await;

	let calls = mock.wait_for_calls(1).await;
	assert_eq!(calls[0].param("caption"), Some("just in time"));
}

#[tokio::test]
async fn text_sent_long_after_the_media_is_not_a_caption() {
	assert_text_stays_a_message(
		sent_after(Duration::from_secs(11), text("$txt", "later")),
		"later",
	)
	.await;
}

#[tokio::test]
async fn text_longer_than_a_caption_is_not_a_caption() {
	let long = "a".repeat(1025);
	assert_text_stays_a_message(text("$txt", &long), &long).await;
}

#[tokio::test]
async fn text_after_captioned_media_is_not_a_caption() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("cat", "image/png", PNG);
	let captioned = message(
		"$img",
		json!({
			"msgtype": "m.image",
			"body": "own caption",
			"filename": "cat.png",
			"url": mxc("cat"),
		}),
	);

	put_txn(&app, "t1", vec![captioned, text("$txt", "comment")]).await;

	let calls = mock.wait_for_calls(2).await;
	assert_eq!(methods(&calls), ["sendPhoto", "sendMessage"]);
	assert_eq!(calls[0].param("caption"), Some("own caption"));
	assert_eq!(calls[1].text(), "comment");
}

#[tokio::test]
async fn media_from_different_senders_is_not_grouped() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("first", "image/png", PNG);
	mock.serve_media("second", "image/png", PNG);

	put_txn(
		&app,
		"t1",
		vec![
			image("$1", "first"),
			from(BOB, image("$2", "second")),
			text("$3", "alice again"),
		],
	)
	.await;

	let calls = mock.wait_for_calls(3).await;
	assert_eq!(methods(&calls), ["sendPhoto", "sendPhoto", "sendMessage"]);
	assert_eq!(calls[0].files[0].filename, "first.png");
	assert_eq!(calls[1].files[0].filename, "second.png");
	assert_eq!(calls[1].param("caption"), None);
	assert_eq!(calls[2].text(), "alice again");
}

#[tokio::test]
async fn caption_sent_five_seconds_later_in_another_transaction_is_merged() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("greedy", "image/png", PNG);
	let gap = Duration::from_millis(5152);

	put_txn(&app, "t1", vec![image("$img", "greedy")]).await;
	tokio::time::sleep(gap).await;
	put_txn(
		&app,
		"t2",
		vec![sent_after(gap, text("$txt", "Meet Greedy"))],
	)
	.await;

	let calls = mock.wait_for_calls(1).await;
	assert_eq!(methods(&calls), ["sendPhoto"]);
	assert_eq!(calls[0].param("caption"), Some("Meet Greedy"));
}

#[tokio::test]
async fn held_media_is_sent_after_the_hold_without_a_caption() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("cat", "image/png", PNG);
	let started = std::time::Instant::now();

	put_txn(&app, "t1", vec![image("$img", "cat")]).await;

	let calls = mock.wait_for_calls_within(1, Duration::from_secs(15)).await;
	let waited = calls[0].received_at - started;
	assert!(
		(Duration::from_secs(9)..Duration::from_secs(12)).contains(&waited),
		"media should be held for about 10 s, waited {waited:?}"
	);
	assert_eq!(methods(&calls), ["sendPhoto"]);
	assert_eq!(calls[0].param("caption"), None);
}

#[tokio::test]
async fn caption_gap_is_measured_from_the_latest_media_and_includes_the_hold() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("first", "image/png", PNG);
	mock.serve_media("second", "image/png", PNG);

	put_txn(
		&app,
		"t1",
		vec![
			image("$1", "first"),
			sent_after(Duration::from_secs(8), image("$2", "second")),
			sent_after(Duration::from_secs(18), text("$3", "caption")),
		],
	)
	.await;

	let calls = mock.wait_for_calls(1).await;
	assert_eq!(methods(&calls), ["sendMediaGroup"]);
	assert_eq!(calls[0].media_group()[0]["caption"], "caption");
}

#[tokio::test]
async fn eleventh_photo_starts_a_new_album_that_takes_the_caption() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	let ids: Vec<String> = (1..=11).map(|i| format!("photo{i}")).collect();
	for id in &ids {
		mock.serve_media(id, "image/png", PNG);
	}
	let mut events: Vec<Value> = ids.iter().map(|id| image(&format!("${id}"), id)).collect();
	events.push(text("$txt", "eleven"));

	put_txn(&app, "t1", events).await;

	let calls = mock.wait_for_calls(2).await;
	assert_eq!(methods(&calls), ["sendMediaGroup", "sendPhoto"]);
	let album = calls[0].media_group();
	assert_eq!(album.len(), 10);
	assert!(album.iter().all(|entry| entry.get("caption").is_none()));
	assert_eq!(calls[1].files[0].filename, "photo11.png");
	assert_eq!(calls[1].param("caption"), Some("eleven"));
}

#[tokio::test]
async fn file_is_not_grouped_with_a_following_photo() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("notes", "text/plain", b"notes");
	mock.serve_media("cat", "image/png", PNG);
	let file = message(
		"$file",
		json!({ "msgtype": "m.file", "body": "notes.txt", "url": mxc("notes") }),
	);

	put_txn(
		&app,
		"t1",
		vec![file, image("$img", "cat"), text("$txt", "cat")],
	)
	.await;

	let calls = mock.wait_for_calls(2).await;
	assert_eq!(methods(&calls), ["sendDocument", "sendPhoto"]);
	assert_eq!(calls[0].param("caption"), None);
	assert_eq!(calls[1].param("caption"), Some("cat"));
}

#[tokio::test]
async fn captioned_file_is_sent_without_waiting() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("notes", "text/plain", b"notes");
	let file = message(
		"$file",
		json!({
			"msgtype": "m.file",
			"body": "meeting notes",
			"filename": "notes.txt",
			"url": mxc("notes"),
		}),
	);

	put_txn(&app, "t1", vec![file]).await;

	let calls = mock.wait_for_calls(1).await;
	assert_eq!(methods(&calls), ["sendDocument"]);
	assert_eq!(calls[0].param("caption"), Some("meeting notes"));
}

#[tokio::test]
async fn text_after_album_with_a_captioned_item_is_not_a_caption() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("first", "image/png", PNG);
	mock.serve_media("second", "image/png", PNG);
	let captioned = message(
		"$2",
		json!({
			"msgtype": "m.image",
			"body": "own caption",
			"filename": "second.png",
			"url": mxc("second"),
		}),
	);

	put_txn(
		&app,
		"t1",
		vec![image("$1", "first"), captioned, text("$3", "comment")],
	)
	.await;

	let calls = mock.wait_for_calls(2).await;
	assert_eq!(methods(&calls), ["sendMediaGroup", "sendMessage"]);
	let album = calls[0].media_group();
	assert!(album[0].get("caption").is_none());
	assert_eq!(album[1]["caption"], "own caption");
	assert_eq!(calls[1].text(), "comment");
}

#[tokio::test]
async fn merged_caption_keeps_its_plain_text_in_the_document_fallback() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("cat", "image/png", PNG);
	mock.respond_once(
		"sendPhoto",
		400,
		bad_request("Bad Request: IMAGE_PROCESS_FAILED"),
	);

	put_txn(
		&app,
		"t1",
		vec![
			image("$img", "cat"),
			html("$txt", "a **fluffy** cat", "a <strong>fluffy</strong> cat"),
		],
	)
	.await;

	let calls = mock.wait_for_calls(2).await;
	assert_eq!(methods(&calls), ["sendPhoto", "sendDocument"]);
	assert_eq!(calls[0].param("caption"), Some("a <b>fluffy</b> cat"));
	assert_eq!(calls[1].param("caption"), Some("a **fluffy** cat"));
}

#[tokio::test]
async fn merged_caption_is_sent_as_message_when_its_media_is_rejected() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("notes", "text/plain", b"notes");
	mock.respond_once(
		"sendDocument",
		400,
		bad_request("Bad Request: file too big"),
	);
	let file = message(
		"$file",
		json!({ "msgtype": "m.file", "body": "notes.txt", "url": mxc("notes") }),
	);

	put_txn(&app, "t1", vec![file, text("$txt", "meeting notes")]).await;

	let calls = mock.wait_for_calls(2).await;
	assert_eq!(methods(&calls), ["sendDocument", "sendMessage"]);
	assert_eq!(calls[0].param("caption"), Some("meeting notes"));
	assert_eq!(calls[1].text(), "meeting notes");
}

#[tokio::test]
async fn merged_album_caption_is_sent_as_message_when_its_item_is_rejected() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("first", "image/png", PNG);
	mock.serve_media("second", "image/png", PNG);
	let rejected = bad_request("Bad Request: wrong file identifier");
	for method in ["sendMediaGroup", "sendPhoto", "sendDocument"] {
		mock.respond_once(method, 400, rejected.clone());
	}

	put_txn(
		&app,
		"t1",
		vec![
			image("$1", "first"),
			image("$2", "second"),
			text("$3", "two pictures"),
		],
	)
	.await;

	let calls = mock.wait_for_calls(5).await;
	assert_eq!(
		methods(&calls),
		[
			"sendMediaGroup",
			"sendPhoto",
			"sendDocument",
			"sendPhoto",
			"sendMessage"
		]
	);
	assert_eq!(calls[3].files[0].filename, "second.png");
	assert_eq!(calls[3].param("caption"), None);
	assert_eq!(calls[4].text(), "two pictures");
}

async fn assert_text_stays_a_message(text: Value, expected: &str) {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("cat", "image/png", PNG);

	put_txn(&app, "t1", vec![image("$img", "cat"), text]).await;

	let calls = mock.wait_for_calls(2).await;
	assert_eq!(methods(&calls), ["sendPhoto", "sendMessage"]);
	assert_eq!(calls[0].param("caption"), None);
	assert_eq!(calls[1].text(), expected);
}

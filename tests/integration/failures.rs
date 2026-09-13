use std::time::Duration;

use super::*;

#[tokio::test]
async fn rate_limited_message_is_retried_after_retry_after() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.respond_once(
		"sendMessage",
		429,
		json!({
			"ok": false,
			"error_code": 429,
			"description": "Too Many Requests: retry after 1",
			"parameters": { "retry_after": 1 },
		}),
	);

	put_txn(&app, "t1", vec![text("$1", "patience"), last_message()]).await;

	let calls = mock.wait_for_calls(3).await;
	assert_eq!(texts(&calls), ["patience", "patience", LAST_MESSAGE]);
	let waited = calls[1].received_at - calls[0].received_at;
	assert!(
		(Duration::from_millis(950)..Duration::from_millis(1900)).contains(&waited),
		"retry should wait retry_after (1 s), waited {waited:?}"
	);
}

#[tokio::test]
async fn rate_limited_download_is_retried_after_retry_after_ms() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("cat", "image/png", PNG);
	mock.respond_once(
		"download",
		429,
		json!({ "errcode": "M_LIMIT_EXCEEDED", "error": "Too many requests", "retry_after_ms": 300 }),
	);

	put_txn(&app, "t1", vec![image("$img", "cat"), last_message()]).await;

	let calls = mock.wait_for_calls(2).await;
	assert_eq!(methods(&calls), ["sendPhoto", "sendMessage"]);
	let downloads = mock.downloads();
	assert_eq!(downloads.len(), 2);
	let waited = downloads[1].received_at - downloads[0].received_at;
	assert!(
		(Duration::from_millis(250)..Duration::from_millis(1500)).contains(&waited),
		"retry should wait retry_after_ms (300 ms), waited {waited:?}"
	);
}

#[tokio::test]
async fn unparsable_html_is_resent_as_plain_text() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.respond_once(
		"sendMessage",
		400,
		bad_request(
			"Bad Request: can't parse entities: Unsupported start tag \"b\" at byte offset 0",
		),
	);

	put_txn(
		&app,
		"t1",
		vec![
			html("$1", "**bold** move", "<b>bold</b> move"),
			last_message(),
		],
	)
	.await;

	let calls = mock.wait_for_calls(3).await;
	assert_eq!(
		texts(&calls),
		["<b>bold</b> move", "**bold** move", LAST_MESSAGE]
	);
	assert_eq!(calls[0].param("parse_mode"), Some("HTML"));
	assert_eq!(calls[1].param("parse_mode"), None);
}

#[tokio::test]
async fn rejected_photo_is_resent_as_document() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("fluffy", "image/png", PNG);
	mock.respond_once(
		"sendPhoto",
		400,
		bad_request("Bad Request: IMAGE_PROCESS_FAILED"),
	);
	let captioned = message(
		"$img",
		json!({
			"msgtype": "m.image",
			"body": "a **fluffy** cat",
			"format": "org.matrix.custom.html",
			"formatted_body": "a <strong>fluffy</strong> cat",
			"filename": "fluffy.png",
			"url": mxc("fluffy"),
			"info": { "mimetype": "image/png" },
		}),
	);

	put_txn(&app, "t1", vec![captioned, last_message()]).await;

	let calls = mock.wait_for_calls(3).await;
	assert_eq!(
		methods(&calls),
		["sendPhoto", "sendDocument", "sendMessage"]
	);
	let (photo, document) = (&calls[0], &calls[1]);
	assert_eq!(photo.param("caption"), Some("a <b>fluffy</b> cat"));
	assert_eq!(photo.param("parse_mode"), Some("HTML"));
	assert_eq!(document.param("caption"), Some("a **fluffy** cat"));
	assert_eq!(document.param("parse_mode"), None);
	assert_eq!(document.files[0].field, "document");
	assert_eq!(document.files[0].filename, "fluffy.png");
	assert_eq!(document.files[0].bytes, PNG);
}

#[tokio::test]
async fn rejected_album_is_sent_item_by_item() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("first", "image/png", PNG);
	mock.serve_media("second", "image/png", PNG);
	mock.respond_once(
		"sendMediaGroup",
		400,
		bad_request("Bad Request: wrong file identifier"),
	);

	put_txn(
		&app,
		"t1",
		vec![image("$1", "first"), image("$2", "second"), last_message()],
	)
	.await;

	let calls = mock.wait_for_calls(4).await;
	assert_eq!(
		methods(&calls),
		["sendMediaGroup", "sendPhoto", "sendPhoto", "sendMessage"]
	);
	assert_eq!(calls[1].files[0].filename, "first.png");
	assert_eq!(calls[2].files[0].filename, "second.png");
}

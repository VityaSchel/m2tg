use super::*;
use crate::mock::WAIT_TIMEOUT;

#[tokio::test]
async fn single_image_then_text_sends_photo_then_message() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("cat", "image/png", PNG);

	put_txn(
		&app,
		"t1",
		vec![image("$img", "cat"), text("$txt", "nice cat")],
	)
	.await;

	let calls = mock.wait_for_calls(2).await;
	assert_eq!(methods(&calls), ["sendPhoto", "sendMessage"]);
	let photo = &calls[0];
	assert_eq!(photo.param("chat_id"), Some(CHAT_ID));
	assert_eq!(photo.param("caption"), None);
	assert_eq!(photo.param("parse_mode"), None);
	let [file] = photo.files.as_slice() else {
		panic!("expected one file part, got {:?}", photo.files);
	};
	assert_eq!(file.field, "photo");
	assert_eq!(file.filename, "cat.png");
	assert_eq!(file.content_type.as_deref(), Some("image/png"));
	assert_eq!(file.bytes, PNG);
	assert_eq!(calls[1].text(), "nice cat");

	let downloads = mock.downloads();
	assert_eq!(downloads.len(), 1);
	assert_eq!(downloads[0].media_id, "cat");
	assert_eq!(
		downloads[0].authorization,
		Some(format!("Bearer {AS_TOKEN}"))
	);
}

#[tokio::test]
async fn two_images_then_text_send_one_album_then_message() {
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

	let calls = mock.wait_for_calls(2).await;
	assert_eq!(methods(&calls), ["sendMediaGroup", "sendMessage"]);
	assert_album(&calls[0], &["first.png", "second.png"]);
	assert_eq!(calls[1].text(), "two pictures");
}

#[tokio::test]
async fn image_sent_as_file_uses_send_document() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("diagram", "image/png", PNG);
	let file = message(
		"$file",
		json!({
			"msgtype": "m.file",
			"body": "diagram.png",
			"url": mxc("diagram"),
			"info": { "mimetype": "image/png", "size": PNG.len() },
		}),
	);

	put_txn(&app, "t1", vec![file]).await;

	let calls = mock.wait_for_calls(1).await;
	assert_eq!(methods(&calls), ["sendDocument"]);
	assert_eq!(calls[0].files[0].field, "document");
	assert_eq!(calls[0].files[0].filename, "diagram.png");
}

#[tokio::test]
async fn gif_uses_send_animation_with_downloaded_content_type() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	mock.serve_media("party", "image/gif", GIF);
	let gif = message(
		"$gif",
		json!({ "msgtype": "m.image", "body": "party.gif", "url": mxc("party") }),
	);

	put_txn(&app, "t1", vec![gif]).await;

	let calls = mock.wait_for_calls(1).await;
	assert_eq!(methods(&calls), ["sendAnimation"]);
	let file = &calls[0].files[0];
	assert_eq!(file.field, "animation");
	assert_eq!(file.filename, "party.gif");
	assert_eq!(file.content_type.as_deref(), Some("image/gif"));
	assert_eq!(file.bytes, GIF);
}

#[tokio::test]
async fn unavailable_media_is_announced_as_text_without_retrying() {
	let TestBridge { mock, app, .. } = TestBridge::start().await;
	let captioned = message(
		"$captioned",
		json!({
			"msgtype": "m.image",
			"body": "look at our cat",
			"filename": "cat.png",
			"url": mxc("deleted"),
		}),
	);

	put_txn(&app, "t1", vec![captioned, image("$bare", "gone")]).await;

	let calls = mock.wait_for_calls(2).await;
	assert_eq!(
		texts(&calls),
		[
			"[media unavailable] look at our cat",
			"[media unavailable] gone.png"
		]
	);
	assert_eq!(mock.downloads().len(), 2);
}

#[tokio::test]
async fn buffered_album_is_flushed_on_shutdown() {
	let TestBridge { mock, app, outbox } = TestBridge::start().await;
	mock.serve_media("first", "image/png", PNG);
	mock.serve_media("second", "image/png", PNG);

	put_txn(
		&app,
		"t1",
		vec![image("$1", "first"), image("$2", "second")],
	)
	.await;
	drop(app);

	tokio::time::timeout(WAIT_TIMEOUT, outbox)
		.await
		.expect("outbox should drain well before the album delay")
		.expect("outbox task should not panic");
	let calls = mock.calls();
	assert_eq!(methods(&calls), ["sendMediaGroup"]);
	assert_album(&calls[0], &["first.png", "second.png"]);
}

fn assert_album(call: &TelegramCall, filenames: &[&str]) {
	assert_eq!(call.method, "sendMediaGroup");
	assert_eq!(call.param("chat_id"), Some(CHAT_ID));
	let entries = call.media_group();
	assert_eq!(entries.len(), filenames.len());
	assert_eq!(call.files.len(), filenames.len());
	for ((entry, file), filename) in entries.iter().zip(&call.files).zip(filenames) {
		assert_eq!(entry["type"], "photo");
		assert_eq!(entry["media"], format!("attach://{}", file.field));
		assert_eq!(file.filename, *filename);
		assert_eq!(file.content_type.as_deref(), Some("image/png"));
	}
}

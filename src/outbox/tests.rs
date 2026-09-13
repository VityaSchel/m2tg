use std::time::Duration;

use super::{RequestError, extension_for, retry};

#[test]
fn extension_for_media_types() {
	assert_eq!(extension_for("image/png"), ".png");
	assert_eq!(extension_for("IMAGE/PNG"), ".png");
	assert_eq!(extension_for("video/mp4"), ".mp4");
	assert_eq!(extension_for("audio/ogg; codecs=opus"), ".ogg");
	assert_eq!(extension_for("application/pdf"), "");
	assert_eq!(extension_for("image"), "");
}

#[tokio::test(start_paused = true)]
async fn retry_backs_off_on_transient_errors_and_gives_up() {
	let started = tokio::time::Instant::now();
	let mut calls = 0;
	let result: Result<(), _> = retry("test", || {
		calls += 1;
		async {
			Err(RequestError::Transient {
				retry_after: None,
				message: "boom".into(),
			})
		}
	})
	.await;
	assert!(matches!(result, Err(RequestError::Transient { .. })));
	assert_eq!(calls, 5);
	assert_eq!(started.elapsed(), Duration::from_secs(2 + 4 + 8 + 16));
}

#[tokio::test(start_paused = true)]
async fn retry_honors_retry_after_and_stops_on_permanent_errors() {
	let started = tokio::time::Instant::now();
	let mut calls = 0;
	let result: Result<(), _> = retry("test", || {
		calls += 1;
		let call = calls;
		async move {
			match call {
				1 => Err(RequestError::Transient {
					retry_after: Some(Duration::from_secs(7)),
					message: "slow down".into(),
				}),
				_ => Err(RequestError::Permanent {
					message: "bad request".into(),
				}),
			}
		}
	})
	.await;
	assert!(matches!(result, Err(RequestError::Permanent { .. })));
	assert_eq!(calls, 2);
	assert_eq!(started.elapsed(), Duration::from_secs(7));
}

use reqwest::StatusCode;

use super::{RequestError, excerpt};

#[test]
fn status_classification() {
	let classify = |code: u16| {
		RequestError::from_status(StatusCode::from_u16(code).unwrap(), None, String::new())
	};
	assert!(matches!(classify(429), RequestError::Transient { .. }));
	assert!(matches!(classify(502), RequestError::Transient { .. }));
	assert!(matches!(classify(400), RequestError::Permanent { .. }));
	assert!(matches!(classify(403), RequestError::Permanent { .. }));
}

#[test]
fn excerpt_flattens_and_caps_long_bodies() {
	assert_eq!(
		excerpt("<html>\n<body>oops</body>"),
		"<html> <body>oops</body>"
	);
	let long = excerpt(&"é".repeat(500));
	assert_eq!(long.chars().count(), 201);
	assert!(long.ends_with("é…"));
	assert_eq!(excerpt(&"a".repeat(200)), "a".repeat(200));
}

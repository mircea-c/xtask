use pretty_assertions::assert_eq;

pub fn assert_json_eq(actual: &str, expected: &str) {
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(actual).unwrap(),
        serde_json::from_str::<serde_json::Value>(expected).unwrap()
    );
}

pub fn assert_serialized_json<T: serde::Serialize>(value: &T, expected: &str) {
    let actual = serde_json::to_string(value).unwrap();
    assert_json_eq(&actual, expected);
}

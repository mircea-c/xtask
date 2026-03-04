use {serde::Serialize, serde::Serializer};

#[derive(Debug, PartialEq)]
pub struct WaitStep {}

impl Serialize for WaitStep {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("wait", "~")?;
        map.end()
    }
}

#[cfg(test)]
mod tests {
    use {super::*, crate::buildkite::test_utils::assert_serialized_json};

    #[test]
    fn test_wait_step_serialize_json() {
        assert_serialized_json(
            &WaitStep {},
            r#"{
                "wait": "~"
            }"#,
        );
    }
}

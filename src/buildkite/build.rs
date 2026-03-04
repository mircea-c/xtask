use {serde::Serialize, std::collections::HashMap};

#[derive(Debug, Serialize, PartialEq)]
pub struct Build {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buildkite::test_utils::assert_serialized_json;

    #[test]
    fn test_build_serialize_json() {
        assert_serialized_json(
            &Build {
                branch: Some(String::from("${BUILDKITE_BRANCH}")),
                commit: Some(String::from("${BUILDKITE_COMMIT}")),
                message: Some(String::from("${BUILDKITE_MESSAGE}")),
                env: Some(HashMap::from([(
                    String::from("TRIGGERED_BUILDKITE_TAG"),
                    String::from("${BUILDKITE_TAG}"),
                )])),
            },
            r#"{
                "branch": "${BUILDKITE_BRANCH}",
                "commit": "${BUILDKITE_COMMIT}",
                "message": "${BUILDKITE_MESSAGE}",
                "env": {"TRIGGERED_BUILDKITE_TAG": "${BUILDKITE_TAG}"}
            }"#,
        );
    }
}

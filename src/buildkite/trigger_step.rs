use {super::Build, serde::Serialize};

#[derive(Debug, Serialize, PartialEq)]
pub struct TriggerStep {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,

    #[serde(skip_serializing_if = "String::is_empty")]
    pub trigger: String,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<String>,

    #[serde(rename = "async", skip_serializing_if = "Option::is_none")]
    pub is_async: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub soft_fail: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<Build>,
}

#[cfg(test)]
mod tests {
    use {
        super::*, crate::buildkite::test_utils::assert_serialized_json, std::collections::HashMap,
    };

    #[test]
    fn test_trigger_step_serialize_json() {
        assert_serialized_json(
            &TriggerStep {
                name: String::from("Trigger Build on agave-secondary"),
                trigger: String::from("agave-secondary"),
                branches: vec![String::from("!pull/*")],
                is_async: Some(true),
                soft_fail: Some(true),
                build: Some(Build {
                    message: Some(String::from("${BUILDKITE_MESSAGE}")),
                    commit: Some(String::from("${BUILDKITE_COMMIT}")),
                    branch: Some(String::from("${BUILDKITE_BRANCH}")),
                    env: Some(HashMap::from([(
                        String::from("TRIGGERED_BUILDKITE_TAG"),
                        String::from("${BUILDKITE_TAG}"),
                    )])),
                }),
            },
            r#"{
                "name": "Trigger Build on agave-secondary",
                "trigger": "agave-secondary",
                "branches": ["!pull/*"],
                "async": true,
                "soft_fail": true,
                "build": {
                    "branch": "${BUILDKITE_BRANCH}",
                    "commit": "${BUILDKITE_COMMIT}",
                    "message": "${BUILDKITE_MESSAGE}",
                    "env": {"TRIGGERED_BUILDKITE_TAG": "${BUILDKITE_TAG}"}
                }
            }"#,
        );

        assert_serialized_json(
            &TriggerStep {
                name: String::from("Trigger Build on agave-secondary"),
                trigger: String::from("agave-secondary"),
                branches: vec![],
                is_async: None,
                soft_fail: None,
                build: None,
            },
            r#"{
                "name": "Trigger Build on agave-secondary",
                "trigger": "agave-secondary"
            }"#,
        );
    }
}

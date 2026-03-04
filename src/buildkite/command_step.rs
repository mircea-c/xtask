use {serde::Serialize, std::collections::HashMap};

#[derive(Debug, Serialize, Default, PartialEq)]
pub struct CommandStep {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,

    #[serde(skip_serializing_if = "String::is_empty")]
    pub command: String,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_on_build_failing: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub soft_fail: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<HashMap<String, String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_in_minutes: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<HashMap<String, String>>,
}

#[cfg(test)]
mod tests {
    use {super::*, crate::buildkite::test_utils::assert_serialized_json};

    #[test]
    fn test_command_step_basic_name_only() {
        assert_serialized_json(
            &CommandStep {
                name: String::from("basic test"),
                ..Default::default()
            },
            r#"{
                "name": "basic test"
            }"#,
        );
    }

    #[test]
    fn test_command_step_full() {
        assert_serialized_json(
            &CommandStep {
                name: String::from("full command step"),
                command: String::from("npm test"),
                cancel_on_build_failing: Some(true),
                soft_fail: Some(false),
                agents: Some(HashMap::from([(
                    String::from("queue"),
                    String::from("test"),
                )])),
                timeout_in_minutes: Some(15),
                retry: Some(HashMap::from([(
                    String::from("automatic"),
                    String::from("true"),
                )])),
                ..Default::default()
            },
            r#"{
                "name": "full command step",
                "command": "npm test",
                "cancel_on_build_failing": true,
                "soft_fail": false,
                "agents": {"queue": "test"},
                "timeout_in_minutes": 15,
                "retry": {"automatic": "true"}
            }"#,
        );
    }
}

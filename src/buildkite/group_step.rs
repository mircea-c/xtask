use {super::Step, serde::Serialize};

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct GroupStep {
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(rename = "group")]
    pub name: String,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::buildkite::{test_utils::assert_serialized_json, CommandStep, WaitStep},
    };

    #[test]
    fn test_group_step_serialize_json() {
        assert_serialized_json(
            &GroupStep {
                name: String::from("empty group"),
                steps: vec![
                    Step::Command(CommandStep {
                        name: String::from("step 1"),
                        command: String::from("echo 1"),
                        ..Default::default()
                    }),
                    Step::Wait(WaitStep {}),
                    Step::Command(CommandStep {
                        name: String::from("step 2"),
                        command: String::from("echo 2"),
                        ..Default::default()
                    }),
                ],
            },
            r#"{
                "group": "empty group",
                "steps": [
                    {"name": "step 1", "command": "echo 1"},
                    {"wait": "~"},
                    {"name": "step 2", "command": "echo 2"}
                ]
            }"#,
        );
    }
}

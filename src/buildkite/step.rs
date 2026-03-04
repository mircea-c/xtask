use {
    super::{CommandStep, GroupStep, TriggerStep, WaitStep},
    serde::Serialize,
};

#[derive(Debug, Serialize, PartialEq)]
#[serde(untagged)]
pub enum Step {
    Command(CommandStep),
    Wait(WaitStep),
    Group(GroupStep),
    Trigger(TriggerStep),
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::buildkite::{test_utils::assert_serialized_json, CommandStep},
    };

    #[test]
    fn test_step_command_variant_serialize_json() {
        assert_serialized_json(
            &Step::Command(CommandStep {
                name: String::from("test with command"),
                command: String::from("echo hello"),
                ..Default::default()
            }),
            r#"{
                "name": "test with command",
                "command": "echo hello"
            }"#,
        );
    }

    #[test]
    fn test_step_wait_variant_serialize_json() {
        assert_serialized_json(
            &Step::Wait(WaitStep {}),
            r#"{
                "wait": "~"
            }"#,
        );
    }
}

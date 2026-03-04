use {super::Step, serde::Serialize};

#[derive(Debug, Serialize, PartialEq)]
pub struct Pipeline {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,

    pub steps: Vec<Step>,
}

impl Pipeline {
    pub fn new() -> Self {
        Pipeline {
            priority: None,
            steps: Vec::new(),
        }
    }

    pub fn set_priority(&mut self, priority: i64) {
        self.priority = Some(priority);
    }

    pub fn add_step(&mut self, step: Step) {
        self.steps.push(step);
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*, crate::buildkite::test_utils::assert_serialized_json,
        crate::buildkite::CommandStep,
    };

    #[test]
    fn test_pipeline_creation() {
        let mut pipeline = Pipeline::new();
        assert!(pipeline.steps.is_empty());

        pipeline.add_step(Step::Command(CommandStep {
            name: String::from("test"),
            ..Default::default()
        }));

        assert_eq!(pipeline.steps.len(), 1);
    }

    #[test]
    fn test_priority_pipeline_serialize_json() {
        let mut pipeline = Pipeline::new();
        pipeline.set_priority(10);
        assert_eq!(pipeline.priority, Some(10));

        pipeline.add_step(Step::Command(CommandStep {
            name: String::from("step 1"),
            command: String::from("echo 1"),
            ..Default::default()
        }));

        pipeline.add_step(Step::Command(CommandStep {
            name: String::from("step 2"),
            command: String::from("echo 2"),
            ..Default::default()
        }));

        assert_serialized_json(
            &pipeline,
            r#"{
                "priority": 10,
                "steps": [
                    {"name": "step 1", "command": "echo 1"},
                    {"name": "step 2", "command": "echo 2"}
                ]
            }"#,
        );
    }

    #[test]
    fn test_pipeline_default() {
        let default_pipeline = Pipeline::default();
        let empty_pipeline = Pipeline::new();
        assert_eq!(default_pipeline, empty_pipeline);
    }
}

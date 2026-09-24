use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentScene {
    Ask,
    Organize,
    Partition,
    Extract,
    Move,
}

impl AgentScene {
    pub fn skill_id(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Organize => "organize",
            Self::Partition => "topic-partition",
            Self::Extract => "topic-extract",
            Self::Move => "move",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSkillSelector {
    pub id: String,
    pub version: u32,
}

impl AgentSkillSelector {
    pub fn for_scene(scene: AgentScene) -> Self {
        Self {
            id: scene.skill_id().into(),
            version: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSkillDefinition {
    pub id: String,
    pub version: u32,
    pub scene: AgentScene,
    pub allowed_tools: Vec<String>,
}

use contracts::{
    AgentErrorCode, AgentScene, AgentSkillDefinition, AgentSkillSelector, AgentToolError,
};

const BUNDLED_SKILLS: &[(AgentScene, &str)] = &[
    (
        AgentScene::Ask,
        include_str!("../../../resources/assistant/skills/ask/SKILL.md"),
    ),
    (
        AgentScene::Organize,
        include_str!("../../../resources/assistant/skills/organize/SKILL.md"),
    ),
    (
        AgentScene::Partition,
        include_str!("../../../resources/assistant/skills/topic-partition/SKILL.md"),
    ),
    (
        AgentScene::Extract,
        include_str!("../../../resources/assistant/skills/topic-extract/SKILL.md"),
    ),
    (
        AgentScene::Move,
        include_str!("../../../resources/assistant/skills/move/SKILL.md"),
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundledAgentSkill {
    pub definition: AgentSkillDefinition,
    pub instructions: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SkillRegistry {
    skills: Vec<BundledAgentSkill>,
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self {
            skills: BUNDLED_SKILLS
                .iter()
                .map(|(scene, source)| {
                    parse_bundled_skill(*scene, source)
                        .expect("bundled Assistant Skill frontmatter must be valid")
                })
                .collect(),
        }
    }
}

impl SkillRegistry {
    #[allow(clippy::result_large_err)]
    pub(crate) fn resolve(
        &self,
        scene: AgentScene,
        selector: &AgentSkillSelector,
    ) -> Result<BundledAgentSkill, AgentToolError> {
        self.skills
            .iter()
            .find(|skill| {
                skill.definition.scene == scene
                    && skill.definition.id == selector.id
                    && skill.definition.version == selector.version
            })
            .cloned()
            .ok_or_else(|| {
                AgentToolError::new(
                    AgentErrorCode::SkillNotFound,
                    format!(
                        "skill {} version {} is not available for scene {}",
                        selector.id,
                        selector.version,
                        scene.skill_id()
                    ),
                    false,
                )
            })
    }
}

fn parse_bundled_skill(scene: AgentScene, source: &str) -> Result<BundledAgentSkill, String> {
    let mut lines = source.lines();
    if lines.next() != Some("---") {
        return Err("Skill must start with YAML frontmatter".into());
    }
    let mut id = None;
    let mut version = None;
    let mut allowed_tools = Vec::new();
    let mut in_allowed_tools = false;
    let mut closed = false;
    for line in lines.by_ref() {
        if line == "---" {
            closed = true;
            break;
        }
        if let Some(value) = line.strip_prefix("id: ") {
            id = Some(value.trim().to_string());
            in_allowed_tools = false;
        } else if let Some(value) = line.strip_prefix("version: ") {
            version = Some(
                value
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| "Skill version must be an unsigned integer")?,
            );
            in_allowed_tools = false;
        } else if line == "allowed_tools:" {
            in_allowed_tools = true;
        } else if in_allowed_tools {
            let value = line.strip_prefix("  - ").ok_or_else(|| {
                "allowed_tools entries must use two-space list indentation".to_string()
            })?;
            if value.trim().is_empty() {
                return Err("allowed_tools entries cannot be empty".into());
            }
            allowed_tools.push(value.trim().to_string());
        } else if !line.trim().is_empty() {
            return Err(format!("unsupported Skill frontmatter field: {line}"));
        }
    }
    if !closed {
        return Err("Skill frontmatter is not closed".into());
    }
    let id = id.ok_or_else(|| "Skill id is required".to_string())?;
    if id != scene.skill_id() {
        return Err(format!(
            "Skill id {id} does not match scene id {}",
            scene.skill_id()
        ));
    }
    if allowed_tools.is_empty() {
        return Err("Skill allowed_tools cannot be empty".into());
    }
    let instructions = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    if instructions.is_empty() {
        return Err("Skill instructions cannot be empty".into());
    }
    Ok(BundledAgentSkill {
        definition: AgentSkillDefinition {
            id,
            version: version.ok_or_else(|| "Skill version is required".to_string())?,
            scene,
            allowed_tools,
        },
        instructions,
    })
}

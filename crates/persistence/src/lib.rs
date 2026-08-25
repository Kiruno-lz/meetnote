use application::ProjectRepository;
use domain::{MeetingProject, ProjectId};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

#[derive(Clone, Default)]
pub struct InMemoryProjectRepository {
    projects: Arc<RwLock<BTreeMap<String, MeetingProject>>>,
}

impl ProjectRepository for InMemoryProjectRepository {
    fn save(&self, project: MeetingProject) -> Result<(), String> {
        self.projects
            .write()
            .map_err(|_| "project repository lock poisoned".to_string())?
            .insert(project.id.as_str().to_string(), project);
        Ok(())
    }

    fn get(&self, project_id: &ProjectId) -> Result<Option<MeetingProject>, String> {
        Ok(self
            .projects
            .read()
            .map_err(|_| "project repository lock poisoned".to_string())?
            .get(project_id.as_str())
            .cloned())
    }

    fn list(&self) -> Result<Vec<MeetingProject>, String> {
        Ok(self
            .projects
            .read()
            .map_err(|_| "project repository lock poisoned".to_string())?
            .values()
            .cloned()
            .collect())
    }
}

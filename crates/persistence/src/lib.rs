use application::ProjectRepository;
use domain::{MeetingProject, ProjectId, ProjectStatus};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
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

#[derive(Clone)]
pub struct JsonProjectRepository {
    path: PathBuf,
    projects: Arc<RwLock<BTreeMap<String, MeetingProject>>>,
}

impl JsonProjectRepository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        let projects = if path.is_file() {
            let content = fs::read_to_string(&path).map_err(|error| error.to_string())?;
            serde_json::from_str::<Vec<MeetingProject>>(&content)
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(|mut project| {
                    if project.status == ProjectStatus::Processing {
                        project.status = ProjectStatus::Failed;
                    }
                    (project.id.as_str().to_string(), project)
                })
                .collect()
        } else {
            BTreeMap::new()
        };
        Ok(Self {
            path,
            projects: Arc::new(RwLock::new(projects)),
        })
    }

    fn flush(&self, projects: &BTreeMap<String, MeetingProject>) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let values: Vec<_> = projects.values().cloned().collect();
        let content = serde_json::to_vec_pretty(&values).map_err(|error| error.to_string())?;
        let temporary = self.path.with_extension("json.tmp");
        fs::write(&temporary, content).map_err(|error| error.to_string())?;
        fs::rename(temporary, &self.path).map_err(|error| error.to_string())
    }
}

impl ProjectRepository for JsonProjectRepository {
    fn save(&self, project: MeetingProject) -> Result<(), String> {
        let mut projects = self
            .projects
            .write()
            .map_err(|_| "project repository lock poisoned".to_string())?;
        projects.insert(project.id.as_str().to_string(), project);
        self.flush(&projects)
    }

    fn get(&self, project_id: &ProjectId) -> Result<Option<MeetingProject>, String> {
        self.projects
            .read()
            .map_err(|_| "project repository lock poisoned".to_string())
            .map(|projects| projects.get(project_id.as_str()).cloned())
    }

    fn list(&self) -> Result<Vec<MeetingProject>, String> {
        self.projects
            .read()
            .map_err(|_| "project repository lock poisoned".to_string())
            .map(|projects| projects.values().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_lists_projects_without_a_database() {
        let repository = InMemoryProjectRepository::default();
        let project = MeetingProject::new(
            ProjectId::new("project-1"),
            "Fixture meeting",
            "fixture.wav",
        );

        repository.save(project.clone()).unwrap();

        assert_eq!(repository.get(&project.id).unwrap(), Some(project.clone()));
        assert_eq!(repository.list().unwrap(), vec![project]);
    }

    #[test]
    fn persists_projects_as_json() {
        let path = std::env::temp_dir().join(format!(
            "meetnote-projects-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let repository = JsonProjectRepository::open(&path).unwrap();
        let project = MeetingProject::new(ProjectId::new("project-2"), "Saved meeting", "a.wav");

        repository.save(project.clone()).unwrap();

        let reopened = JsonProjectRepository::open(&path).unwrap();
        assert_eq!(reopened.get(&project.id).unwrap(), Some(project));
        std::fs::remove_file(path).unwrap();
    }
}

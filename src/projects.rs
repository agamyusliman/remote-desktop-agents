use crate::config::Config;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub session: String,
}

#[derive(Serialize, Deserialize, Default)]
struct Library {
    #[serde(default)]
    projects: Vec<Project>,
}

pub struct Projects {
    inner: Mutex<Library>,
    file: PathBuf,
    roots: Vec<PathBuf>,
    default_id: String,
}

impl Projects {
    pub fn load(cfg: &Config) -> Result<Self> {
        let roots: Vec<PathBuf> = if cfg.allowed_roots.is_empty() {
            vec![cfg.cwd.clone()]
        } else {
            cfg.allowed_roots.clone()
        };
        let mut library = match fs::read_to_string(&cfg.projects_file) {
            Ok(raw) => serde_json::from_str::<Library>(&raw).unwrap_or_default(),
            Err(_) => Library::default(),
        };
        // App-managed sessions: never attach to a stale user shell named "omp".
        for project in &mut library.projects {
            project.session = format!("omp-{}", project.id);
        }

        if library.projects.is_empty() {
            discover(&mut library, &roots);
        }
        if !library
            .projects
            .iter()
            .any(|p| Path::new(&p.path) == cfg.cwd)
        {
            let id = unique_id(&library, "workspace");
            library.projects.insert(
                0,
                Project {
                    session: format!("omp-{id}"),
                    id,
                    name: path_name(&cfg.cwd),
                    path: cfg.cwd.display().to_string(),
                },
            );
        }

        let default_id = library
            .projects
            .iter()
            .find(|p| Path::new(&p.path) == cfg.cwd)
            .map(|p| p.id.clone())
            .or_else(|| library.projects.first().map(|p| p.id.clone()))
            .unwrap_or_else(|| "workspace".into());

        let projects = Self {
            inner: Mutex::new(library),
            file: cfg.projects_file.clone(),
            roots,
            default_id,
        };
        projects.save();
        Ok(projects)
    }

    pub fn list(&self) -> Vec<Project> {
        self.inner.lock().unwrap().projects.clone()
    }

    pub fn get(&self, id: &str) -> Option<Project> {
        self.inner
            .lock()
            .unwrap()
            .projects
            .iter()
            .find(|p| p.id == id)
            .cloned()
    }

    pub fn default_id(&self) -> String {
        self.default_id.clone()
    }

    pub fn default_project(&self) -> Option<Project> {
        self.get(&self.default_id)
            .or_else(|| self.list().into_iter().next())
    }

    pub fn roots(&self) -> Vec<String> {
        self.roots
            .iter()
            .map(|p| p.display().to_string())
            .collect()
    }

    pub fn add(&self, name: &str, path: &str) -> Result<Project> {
        let canonical = fs::canonicalize(expand_tilde(path))
            .with_context(|| format!("path not found: {path}"))?;
        if !canonical.is_dir() {
            bail!("not a directory: {}", canonical.display());
        }
        if !self.roots.iter().any(|root| canonical.starts_with(root)) {
            bail!(
                "path is outside allowed roots: {}",
                self.roots
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }

        let mut library = self.inner.lock().unwrap();
        if let Some(existing) = library
            .projects
            .iter()
            .find(|p| Path::new(&p.path) == canonical)
        {
            return Ok(existing.clone());
        }
        if library.projects.len() >= 120 {
            bail!("project library is full");
        }

        let label = if name.trim().is_empty() {
            path_name(&canonical)
        } else {
            name.trim().to_string()
        };
        let id = unique_id(&library, &slug(&label));
        let session = format!("omp-{id}");
        let project = Project {
            id,
            name: label,
            path: canonical.display().to_string(),
            session,
        };
        library.projects.push(project.clone());
        drop(library);
        self.save();
        Ok(project)
    }

    /// Drop a project from the library. Files on disk are never touched.
    pub fn remove(&self, id: &str) -> bool {
        let mut library = self.inner.lock().unwrap();
        let removed = {
            let before = library.projects.len();
            library.projects.retain(|p| p.id != id);
            library.projects.len() != before
        };
        drop(library);
        if removed {
            self.save();
        }
        removed
    }

    fn save(&self) {
        let library = self.inner.lock().unwrap();
        if let Ok(raw) = serde_json::to_string_pretty(&*library) {
            let _ = fs::write(&self.file, raw);
        }
    }
}

fn discover(library: &mut Library, roots: &[PathBuf]) {
    let add = |path: &Path, library: &mut Library| {
        if library.projects.len() >= 60 {
            return;
        }
        if library
            .projects
            .iter()
            .any(|p| Path::new(&p.path) == path)
        {
            return;
        }
        let name = path_name(path);
        let id = unique_id(library, &slug(&name));
        let sess = format!("omp-{id}");
        library.projects.push(Project {
            id,
            name,
            path: path.display().to_string(),
            session: sess,
        });
    };

    for root in roots {
        add(root, library);
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        let mut children: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && !p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with('.'))
                        .unwrap_or(true)
            })
            .filter(|p| looks_like_project(p))
            .collect();
        children.sort();
        for child in children {
            add(&child, library);
        }
    }
}

pub(crate) fn looks_like_project(path: &Path) -> bool {
    const MARKERS: &[&str] = &[
        ".git",
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "requirements.txt",
        "go.mod",
        "composer.json",
        "pubspec.yaml",
        "Gemfile",
        "AGENTS.md",
        "CLAUDE.md",
        ".omp",
    ];
    MARKERS.iter().any(|marker| path.join(marker).exists())
}

fn path_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.display().to_string())
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "project".into()
    } else {
        out.chars().take(40).collect()
    }
}

fn unique_id(library: &Library, base: &str) -> String {
    if !library.projects.iter().any(|p| p.id == base) {
        return base.to_string();
    }
    for n in 2..500 {
        let candidate = format!("{base}-{n}");
        if !library.projects.iter().any(|p| p.id == candidate) {
            return candidate;
        }
    }
    format!("{base}-{}", library.projects.len() + 1)
}

fn expand_tilde(input: &str) -> PathBuf {
    if let Some(rest) = input.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(input)
}

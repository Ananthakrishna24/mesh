use std::path::PathBuf;

use directories::{ProjectDirs, UserDirs};

use crate::{StoreError, StoreResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorePaths {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub cache_dir: PathBuf,
    pub model_cache_dir: PathBuf,
}

pub fn default_store_paths() -> StoreResult<StorePaths> {
    let project = ProjectDirs::from("dev", "Mesh", "Mesh").ok_or_else(|| {
        StoreError::Path("could not resolve application data directory".to_owned())
    })?;
    let data_dir = project.data_dir().to_path_buf();
    let cache_dir = project.cache_dir().to_path_buf();
    let _ = UserDirs::new();

    Ok(StorePaths {
        db_path: data_dir.join("mesh.db"),
        model_cache_dir: data_dir.join("model-cache"),
        data_dir,
        cache_dir,
    })
}

impl StorePaths {
    pub fn isolated(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            db_path: root.join("mesh.db"),
            cache_dir: root.join("cache"),
            model_cache_dir: root.join("model-cache"),
            data_dir: root,
        }
    }
}

use std::path::{Path, PathBuf};

use rand::seq::IndexedRandom;

use crate::error::Result;

pub struct BackgroundManager {
    pub directory: Option<PathBuf>,
    pub cache_file: PathBuf,
    pub current: Option<PathBuf>,
    cached_images: Vec<PathBuf>,
}

impl BackgroundManager {
    pub fn new(directory: Option<&str>) -> Self {
        let cache_dir = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let cache_file = cache_dir.join("custerm").join("wallpapers.txt");

        Self {
            directory: directory.map(PathBuf::from),
            cache_file,
            current: None,
            cached_images: Vec::new(),
        }
    }

    /// Load cached image list or rebuild from directory.
    pub fn load_cache(&mut self) -> Result<()> {
        if self.cache_file.exists() {
            let contents = std::fs::read_to_string(&self.cache_file)?;
            self.cached_images = contents
                .lines()
                .filter(|l| !l.is_empty())
                .map(PathBuf::from)
                .filter(|p| p.exists())
                .collect();
        }

        if self.cached_images.is_empty() {
            self.rebuild_cache()?;
        }

        Ok(())
    }

    /// Scan directory for valid images and rebuild cache.
    pub fn rebuild_cache(&mut self) -> Result<()> {
        let dir = match &self.directory {
            Some(d) => d.clone(),
            None => return Ok(()),
        };

        let mut images = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if is_image_file(&path) {
                    images.push(path);
                }
            }
        }

        // Ensure cache directory exists
        if let Some(parent) = self.cache_file.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let contents: String = images
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&self.cache_file, contents)?;

        self.cached_images = images;
        Ok(())
    }

    /// Pick a random image, avoiding the current one.
    pub fn next(&mut self) -> Option<&Path> {
        if self.cached_images.is_empty() {
            return None;
        }

        let mut rng = rand::rng();
        let candidates: Vec<_> = self
            .cached_images
            .iter()
            .filter(|p| self.current.as_ref() != Some(p))
            .collect();

        let chosen = if candidates.is_empty() {
            self.cached_images.choose(&mut rng)?
        } else {
            candidates.choose(&mut rng)?
        };

        self.current = Some(chosen.to_path_buf());
        self.current.as_deref()
    }

    /// Remove current image from cache.
    pub fn delete_current(&mut self) -> Result<Option<PathBuf>> {
        let current = match self.current.take() {
            Some(c) => c,
            None => return Ok(None),
        };

        self.cached_images.retain(|p| p != &current);

        // Update cache file
        let contents: String = self
            .cached_images
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&self.cache_file, &contents)?;

        Ok(Some(current))
    }
}

fn is_image_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    matches!(
        ext.as_deref(),
        Some("jpg" | "jpeg" | "png" | "webp" | "bmp")
    )
}

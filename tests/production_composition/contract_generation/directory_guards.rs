impl Drop for OwnedOutsideDirectory {
    fn drop(&mut self) {
        let temporary_root = std::env::temp_dir();
        let has_owned_name = self
            .path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with("jamye-task-12-contract-generation-outside-"));
        let is_direct_child = self.path.parent() == Some(temporary_root.as_path());
        let is_real_directory = fs::symlink_metadata(&self.path)
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink());
        if has_owned_name && is_direct_child && is_real_directory {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

struct OwnedSymlink {
    path: PathBuf,
}

impl OwnedSymlink {
    fn new(path: PathBuf, target: PathBuf) -> TestResult<Self> {
        if !is_owned_child(&path, &generation_root()) {
            return Err(io::Error::other("Task-12 symlink escaped its owned test root").into());
        }
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("Task-12 symlink has no parent"))?;
        ensure_real_directory(&generation_root())?;
        ensure_real_directory(parent)?;
        std::os::unix::fs::symlink(target, &path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for OwnedSymlink {
    fn drop(&mut self) {
        let is_owned_symlink = is_owned_child(&self.path, &generation_root())
            && fs::symlink_metadata(&self.path)
                .is_ok_and(|metadata| metadata.file_type().is_symlink());
        if is_owned_symlink {
            let _ = fs::remove_file(&self.path);
        }
    }
}

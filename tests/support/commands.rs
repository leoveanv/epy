use ers::integrations::ExecutableSearch;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub struct CommandFixture {
    pub root: tempfile::TempDir,
    pub search: ExecutableSearch,
}

impl CommandFixture {
    pub fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("commands with spaces");
        std::fs::create_dir(&directory).unwrap();
        let search = ExecutableSearch {
            path: directory.clone().into_os_string(),
            directory,
        };
        Self { root, search }
    }

    pub fn install(&self, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = self.search.directory.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    pub fn wait_file(&self, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.root.path().join(name).exists() {
            assert!(Instant::now() < deadline, "The helper never created {name}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

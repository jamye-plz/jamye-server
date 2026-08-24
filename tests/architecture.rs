use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[test]
fn domain_is_framework_free() -> io::Result<()> {
    assert_no_tokens(
        &source_root().join("domain"),
        &["axum::", "sqlx::", "redis::", "reqwest::", "aws_sdk_"],
    )
}

#[test]
fn application_does_not_depend_on_outer_layers() -> io::Result<()> {
    assert_no_tokens(
        &source_root().join("application"),
        &[
            "crate::adapters",
            "crate::transport",
            "jamye_server::adapters",
            "jamye_server::transport",
        ],
    )
}

#[test]
fn sqlx_is_confined_to_postgres_adapters() -> io::Result<()> {
    let source = source_root();
    let allowed = source.join("adapters/postgres");
    for file in rust_files(&source)? {
        if file.starts_with(&allowed) {
            continue;
        }
        let content = fs::read_to_string(&file)?;
        assert!(
            !content.contains("sqlx::"),
            "SQLx escaped PostgreSQL adapters: {}",
            file.display()
        );
    }
    Ok(())
}

#[test]
fn runtime_plugin_registries_are_not_used() -> io::Result<()> {
    assert_no_tokens(
        &source_root(),
        &[
            "inventory::",
            "linkme::",
            "register_feature",
            "register_handler",
        ],
    )
}

fn assert_no_tokens(root: &Path, forbidden: &[&str]) -> io::Result<()> {
    for file in rust_files(root)? {
        let content = fs::read_to_string(&file)?;
        for token in forbidden {
            assert!(
                !content.contains(token),
                "forbidden dependency {token} in {}",
                file.display()
            );
        }
    }
    Ok(())
}

fn rust_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_rust_files(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_rust_files(root: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_rust_files(&path, files)?;
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            files.push(path);
        }
    }
    Ok(())
}

fn source_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

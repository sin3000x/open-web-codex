use std::path::Path;
use std::process::Command;

use open_web_codex_git_runtime::{CommitAuthor, GitRuntime, GitRuntimeConfig, GitRuntimeError};
use tempfile::TempDir;
use uuid::Uuid;

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn fixture() -> (TempDir, GitRuntime, String) {
    let root = TempDir::new().expect("runner tempdir");
    let source = root.path().join("source");
    std::fs::create_dir(&source).expect("source directory");
    git(&source, &["init", "-b", "main"]);
    std::fs::write(source.join("README.md"), "initial\n").expect("fixture readme");
    git(&source, &["add", "README.md"]);
    git(
        &source,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "initial",
        ],
    );
    let runtime =
        GitRuntime::new(GitRuntimeConfig::new(root.path().join("runner")).with_local_sources())
            .expect("Git Runtime");
    (root, runtime, source.to_string_lossy().to_string())
}

#[test]
fn validates_remote_sources_and_branch_names() {
    let root = TempDir::new().unwrap();
    let runtime = GitRuntime::new(GitRuntimeConfig::new(root.path().join("runner"))).unwrap();
    assert!(runtime
        .validate_source("https://github.com/openai/codex.git")
        .is_ok());
    assert!(runtime
        .validate_source("git@github.com:openai/codex.git")
        .is_ok());
    assert!(runtime
        .validate_source("https://token@github.com/openai/codex.git")
        .is_err());
    assert!(runtime.validate_source("file:///private/repo").is_err());
    let managed_id = Uuid::now_v7();
    assert!(runtime
        .validate_source(&format!("managed://{managed_id}"))
        .is_ok());
    assert!(runtime
        .validate_external_source(&format!("managed://{managed_id}"))
        .is_err());
    assert!(runtime.validate_ref("feature/safe-name").is_ok());
    for value in [
        "--upload-pack=x",
        "../main",
        "refs//main",
        "main.lock",
        "main~1",
    ] {
        assert!(runtime.validate_ref(value).is_err(), "accepted {value}");
    }
}

#[tokio::test]
async fn provisions_a_server_managed_empty_project() {
    let root = TempDir::new().unwrap();
    let runtime = GitRuntime::new(GitRuntimeConfig::new(root.path().join("runner"))).unwrap();
    let project_id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let source = runtime
        .validate_source(&format!("managed://{project_id}"))
        .unwrap();
    let branch = runtime.validate_ref("main").unwrap();

    let checkout = runtime
        .provision(project_id, workspace_id, &source, &branch)
        .await
        .unwrap();

    assert_eq!(checkout.branch, format!("codex-runs/{workspace_id}"));
    assert!(runtime
        .status(workspace_id)
        .await
        .unwrap()
        .changes
        .is_empty());
}

#[tokio::test]
async fn provisions_isolated_workspaces_serializes_mirror_and_commits_selected_files() {
    let (_root, runtime, source) = fixture();
    let source = runtime.validate_source(&source).unwrap();
    let branch = runtime.validate_ref("main").unwrap();
    let project_id = Uuid::now_v7();
    let first_id = Uuid::now_v7();
    let second_id = Uuid::now_v7();
    let (first, second) = tokio::join!(
        runtime.provision(project_id, first_id, &source, &branch),
        runtime.provision(project_id, second_id, &source, &branch),
    );
    let first = first.expect("first workspace");
    let second = second.expect("second workspace");
    assert_ne!(first.root, second.root);
    assert_eq!(first.head_commit, second.head_commit);

    std::fs::write(first.root.join("README.md"), "changed\n").unwrap();
    std::fs::write(first.root.join("unselected.txt"), "leave me\n").unwrap();
    let status = runtime.status(first_id).await.unwrap();
    assert_eq!(status.changes.len(), 2);
    assert!(status
        .changes
        .iter()
        .all(|change| !change.path.starts_with('/')));

    let commit = runtime
        .commit_selected(
            first_id,
            &["README.md".to_string()],
            "selected change",
            &CommitAuthor {
                name: "Codex User".to_string(),
                email: "user@example.invalid".to_string(),
            },
        )
        .await
        .expect("commit selected file");
    assert_ne!(commit, first.head_commit);
    let status = runtime.status(first_id).await.unwrap();
    assert_eq!(status.changes.len(), 1);
    assert_eq!(status.changes[0].path, "unselected.txt");

    runtime.remove_workspace(first_id).await.unwrap();
    assert!(!first.root.exists());
    assert!(second.root.exists());
}

#[tokio::test]
async fn reports_binary_and_large_file_metadata_without_exposing_absolute_paths() {
    let (_root, runtime, source) = fixture();
    let source = runtime.validate_source(&source).unwrap();
    let branch = runtime.validate_ref("main").unwrap();
    let workspace_id = Uuid::now_v7();
    let checkout = runtime
        .provision(Uuid::now_v7(), workspace_id, &source, &branch)
        .await
        .unwrap();
    std::fs::write(checkout.root.join("binary.bin"), [0, 159, 146, 150]).unwrap();
    std::fs::write(checkout.root.join("large.txt"), vec![b'x'; 1024 * 1024 + 1]).unwrap();
    git(&checkout.root, &["add", "binary.bin", "large.txt"]);

    let status = runtime.status(workspace_id).await.unwrap();
    let binary = status
        .changes
        .iter()
        .find(|change| change.path == "binary.bin")
        .unwrap();
    assert!(binary.binary);
    let large = status
        .changes
        .iter()
        .find(|change| change.path == "large.txt")
        .unwrap();
    assert!(large.large);
    assert_eq!(large.size_bytes, Some(1024 * 1024 + 1));
    assert!(status
        .changes
        .iter()
        .all(|change| !change.path.starts_with('/')));
}

#[tokio::test]
async fn lists_reads_diffs_stages_unstages_and_reverts_workspace_files() {
    let (_root, runtime, source) = fixture();
    let source = runtime.validate_source(&source).unwrap();
    let branch = runtime.validate_ref("main").unwrap();
    let workspace_id = Uuid::now_v7();
    let checkout = runtime
        .provision(Uuid::now_v7(), workspace_id, &source, &branch)
        .await
        .unwrap();
    std::fs::write(checkout.root.join("README.md"), "changed\n").unwrap();
    std::fs::create_dir(checkout.root.join("src")).unwrap();
    std::fs::write(checkout.root.join("src/new.rs"), "fn added() {}\n").unwrap();

    let files = runtime.list_files(workspace_id).await.unwrap();
    assert_eq!(files, vec!["README.md", "src/new.rs"]);
    let read = runtime.read_file(workspace_id, "src/new.rs").await.unwrap();
    assert_eq!(read.content, "fn added() {}\n");
    assert!(!read.truncated);
    let diffs = runtime.diffs(workspace_id).await.unwrap();
    assert_eq!(diffs.len(), 2);
    assert!(diffs
        .iter()
        .find(|diff| diff.path == "README.md")
        .unwrap()
        .diff
        .contains("+changed"));
    assert!(diffs
        .iter()
        .find(|diff| diff.path == "src/new.rs")
        .unwrap()
        .diff
        .contains("+fn added() {}"));

    runtime
        .stage_paths(workspace_id, &["README.md".to_string()])
        .await
        .unwrap();
    git(
        &checkout.root,
        &["diff", "--cached", "--quiet", "--", "src/new.rs"],
    );
    runtime
        .unstage_paths(workspace_id, &["README.md".to_string()])
        .await
        .unwrap();
    git(&checkout.root, &["diff", "--cached", "--quiet"]);

    runtime
        .revert_paths(workspace_id, &["src/new.rs".to_string()])
        .await
        .unwrap();
    assert!(!checkout.root.join("src/new.rs").exists());
    runtime.revert_all(workspace_id).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(checkout.root.join("README.md")).unwrap(),
        "initial\n"
    );
    assert!(runtime
        .status(workspace_id)
        .await
        .unwrap()
        .changes
        .is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_workspace_file_traversal_and_symlinks() {
    use std::os::unix::fs::symlink;

    let (root, runtime, source) = fixture();
    let source = runtime.validate_source(&source).unwrap();
    let branch = runtime.validate_ref("main").unwrap();
    let workspace_id = Uuid::now_v7();
    let checkout = runtime
        .provision(Uuid::now_v7(), workspace_id, &source, &branch)
        .await
        .unwrap();
    let outside = root.path().join("outside.txt");
    std::fs::write(&outside, "secret\n").unwrap();
    symlink(&outside, checkout.root.join("linked.txt")).unwrap();

    assert!(matches!(
        runtime.read_file(workspace_id, "../outside.txt").await,
        Err(GitRuntimeError::UnsafePath(_))
    ));
    assert!(matches!(
        runtime.read_file(workspace_id, "linked.txt").await,
        Err(GitRuntimeError::UnsafePath(_))
    ));

    symlink(&outside, checkout.root.join("linked.png")).unwrap();
    assert!(matches!(
        runtime
            .read_image_asset(workspace_id, "../outside.png")
            .await,
        Err(GitRuntimeError::UnsafePath(_))
    ));
    assert!(matches!(
        runtime.read_image_asset(workspace_id, "linked.png").await,
        Err(GitRuntimeError::UnsafePath(_))
    ));
}

#[tokio::test]
async fn reads_only_bounded_images_from_workspace() {
    let (_root, runtime, source) = fixture();
    let source = runtime.validate_source(&source).unwrap();
    let branch = runtime.validate_ref("main").unwrap();
    let workspace_id = Uuid::now_v7();
    let checkout = runtime
        .provision(Uuid::now_v7(), workspace_id, &source, &branch)
        .await
        .unwrap();

    let png = b"\x89PNG\r\n\x1a\nworkspace-image";
    std::fs::write(checkout.root.join("preview.png"), png).unwrap();
    let asset = runtime
        .read_image_asset(workspace_id, "preview.png")
        .await
        .unwrap();
    assert_eq!(asset.media_type, "image/png");
    assert_eq!(asset.bytes, png);

    std::fs::write(checkout.root.join("not-image.png"), b"<html>unsafe</html>").unwrap();
    assert!(matches!(
        runtime
            .read_image_asset(workspace_id, "not-image.png")
            .await,
        Err(GitRuntimeError::UnsupportedImage(_))
    ));
    assert!(matches!(
        runtime.read_image_asset(workspace_id, "README.md").await,
        Err(GitRuntimeError::UnsupportedImage(_))
    ));

    let oversized = std::fs::File::create(checkout.root.join("oversized.png")).unwrap();
    oversized.set_len(50 * 1024 * 1024 + 1).unwrap();
    assert!(matches!(
        runtime
            .read_image_asset(workspace_id, "oversized.png")
            .await,
        Err(GitRuntimeError::ImageTooLarge)
    ));
}

#[tokio::test]
async fn lists_switches_and_reports_workspace_branch_history() {
    let (_root, runtime, source) = fixture();
    let source = runtime.validate_source(&source).unwrap();
    let branch = runtime.validate_ref("main").unwrap();
    let workspace_id = Uuid::now_v7();
    let checkout = runtime
        .provision(Uuid::now_v7(), workspace_id, &source, &branch)
        .await
        .unwrap();

    runtime
        .create_branch(workspace_id, "feature/browser")
        .await
        .unwrap();
    let branches = runtime.list_branches(workspace_id).await.unwrap();
    assert!(branches
        .iter()
        .any(|branch| branch.name == "feature/browser"));
    let log = runtime.log(workspace_id, 40).await.unwrap();
    assert_eq!(log.total, 1);
    assert_eq!(log.entries[0].summary, "initial");
    assert_eq!(log.entries[0].author, "Fixture");
    assert_eq!(log.upstream, None);
    assert_eq!(
        runtime.remote(workspace_id).await.unwrap().as_deref(),
        Some(source.as_str())
    );

    runtime
        .checkout_branch(workspace_id, &checkout.branch)
        .await
        .unwrap();
    std::fs::write(checkout.root.join("README.md"), "dirty\n").unwrap();
    assert!(matches!(
        runtime
            .checkout_branch(workspace_id, "feature/browser")
            .await,
        Err(GitRuntimeError::Conflict(_))
    ));
}

#[tokio::test]
async fn refuses_to_commit_when_an_unselected_path_is_already_staged() {
    let (_root, runtime, source) = fixture();
    let source = runtime.validate_source(&source).unwrap();
    let branch = runtime.validate_ref("main").unwrap();
    let workspace_id = Uuid::now_v7();
    let checkout = runtime
        .provision(Uuid::now_v7(), workspace_id, &source, &branch)
        .await
        .unwrap();
    std::fs::write(checkout.root.join("README.md"), "selected\n").unwrap();
    std::fs::write(checkout.root.join("other.txt"), "staged\n").unwrap();
    git(&checkout.root, &["add", "other.txt"]);

    let error = runtime
        .commit_selected(
            workspace_id,
            &["README.md".to_string()],
            "must not commit other.txt",
            &CommitAuthor {
                name: "Codex User".to_string(),
                email: "user@example.invalid".to_string(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, GitRuntimeError::Conflict(_)));
    let status = runtime.status(workspace_id).await.unwrap();
    assert_eq!(status.head_commit, checkout.head_commit);
}

#[tokio::test]
async fn rejects_directory_pathspecs_before_they_can_expand_the_selection() {
    let (_root, runtime, source) = fixture();
    let source = runtime.validate_source(&source).unwrap();
    let branch = runtime.validate_ref("main").unwrap();
    let workspace_id = Uuid::now_v7();
    let checkout = runtime
        .provision(Uuid::now_v7(), workspace_id, &source, &branch)
        .await
        .unwrap();
    std::fs::create_dir(checkout.root.join("src")).unwrap();
    std::fs::write(checkout.root.join("src/first.rs"), "fn first() {}\n").unwrap();
    std::fs::write(checkout.root.join("src/second.rs"), "fn second() {}\n").unwrap();

    let error = runtime
        .commit_selected(
            workspace_id,
            &["src".to_string()],
            "must not expand directory pathspec",
            &CommitAuthor {
                name: "Codex User".to_string(),
                email: "user@example.invalid".to_string(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, GitRuntimeError::Conflict(_)));
    git(&checkout.root, &["diff", "--cached", "--quiet"]);
}

#[cfg(unix)]
#[tokio::test]
async fn refuses_a_symlink_at_a_server_generated_workspace_path() {
    use std::os::unix::fs::symlink;

    let (root, runtime, source) = fixture();
    let source = runtime.validate_source(&source).unwrap();
    let branch = runtime.validate_ref("main").unwrap();
    let workspace_id = Uuid::now_v7();
    let outside = root.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    symlink(&outside, runtime.workspace_path(workspace_id)).unwrap();

    let error = runtime
        .provision(Uuid::now_v7(), workspace_id, &source, &branch)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        GitRuntimeError::Conflict(_) | GitRuntimeError::UnsafePath(_)
    ));
    assert!(outside.exists());
}

#[tokio::test]
async fn copies_agents_md_without_overwriting_the_target() {
    let (_root, runtime, source) = fixture();
    let source = runtime.validate_source(&source).unwrap();
    let branch = runtime.validate_ref("main").unwrap();
    let project_id = Uuid::now_v7();
    let parent_id = Uuid::now_v7();
    let child_id = Uuid::now_v7();
    let parent = runtime
        .provision(project_id, parent_id, &source, &branch)
        .await
        .unwrap();
    let child = runtime
        .provision(project_id, child_id, &source, &branch)
        .await
        .unwrap();
    std::fs::write(parent.root.join("AGENTS.md"), "parent instructions\n").unwrap();

    assert!(runtime.copy_agents_md(parent_id, child_id).await.unwrap());
    assert_eq!(
        std::fs::read_to_string(child.root.join("AGENTS.md")).unwrap(),
        "parent instructions\n"
    );
    std::fs::write(child.root.join("AGENTS.md"), "child instructions\n").unwrap();
    assert!(!runtime.copy_agents_md(parent_id, child_id).await.unwrap());
    assert_eq!(
        std::fs::read_to_string(child.root.join("AGENTS.md")).unwrap(),
        "child instructions\n"
    );
}

#[tokio::test]
async fn writes_only_the_workspace_agents_file_through_the_typed_operation() {
    let (_root, runtime, source) = fixture();
    let source = runtime.validate_source(&source).unwrap();
    let branch = runtime.validate_ref("main").unwrap();
    let workspace_id = Uuid::now_v7();
    let checkout = runtime
        .provision(Uuid::now_v7(), workspace_id, &source, &branch)
        .await
        .unwrap();

    runtime
        .write_agents_md(workspace_id, "browser-managed instructions\n")
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(checkout.root.join("AGENTS.md")).unwrap(),
        "browser-managed instructions\n"
    );
}

#[tokio::test]
async fn scans_and_scopes_git_operations_to_a_nested_repository() {
    let (_root, runtime, source) = fixture();
    let source = runtime.validate_source(&source).unwrap();
    let branch = runtime.validate_ref("main").unwrap();
    let workspace_id = Uuid::now_v7();
    let checkout = runtime
        .provision(Uuid::now_v7(), workspace_id, &source, &branch)
        .await
        .unwrap();
    let nested = checkout.root.join("packages/inner");
    std::fs::create_dir_all(&nested).unwrap();
    git(&nested, &["init", "-b", "main"]);
    std::fs::write(nested.join("nested.txt"), "initial\n").unwrap();
    git(&nested, &["add", "nested.txt"]);
    git(
        &nested,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "nested initial",
        ],
    );
    let skipped = checkout.root.join("target/skipped");
    std::fs::create_dir_all(&skipped).unwrap();
    git(&skipped, &["init", "-b", "main"]);

    let roots = runtime.list_git_roots(workspace_id, 4).await.unwrap();
    assert_eq!(roots, vec!["packages/inner"]);

    std::fs::write(checkout.root.join("README.md"), "outer dirty\n").unwrap();
    std::fs::write(nested.join("nested.txt"), "nested dirty\n").unwrap();
    runtime
        .set_workspace_git_root(workspace_id, Some("packages/inner"))
        .await
        .unwrap();
    let nested_status = runtime.status(workspace_id).await.unwrap();
    assert_eq!(nested_status.changes.len(), 1);
    assert_eq!(nested_status.changes[0].path, "nested.txt");

    runtime
        .set_workspace_git_root(workspace_id, None)
        .await
        .unwrap();
    let outer_status = runtime.status(workspace_id).await.unwrap();
    assert!(outer_status
        .changes
        .iter()
        .any(|change| change.path == "README.md"));
    assert!(!outer_status
        .changes
        .iter()
        .any(|change| change.path == "nested.txt"));
}

#[tokio::test]
async fn returns_bounded_per_file_commit_diffs() {
    let (_root, runtime, source) = fixture();
    let source = runtime.validate_source(&source).unwrap();
    let branch = runtime.validate_ref("main").unwrap();
    let workspace_id = Uuid::now_v7();
    let checkout = runtime
        .provision(Uuid::now_v7(), workspace_id, &source, &branch)
        .await
        .unwrap();
    std::fs::write(checkout.root.join("README.md"), "initial\nupdated\n").unwrap();
    git(&checkout.root, &["add", "README.md"]);
    git(
        &checkout.root,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "update readme",
        ],
    );
    let sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&checkout.root)
        .output()
        .unwrap();
    let sha = String::from_utf8(sha.stdout).unwrap();

    let diffs = runtime
        .commit_diffs(workspace_id, sha.trim())
        .await
        .unwrap();
    assert_eq!(diffs.len(), 1);
    assert_eq!(diffs[0].path, "README.md");
    assert!(diffs[0].diff.contains("+updated"));
}

#[tokio::test]
async fn applies_staged_unstaged_and_untracked_changes_to_a_clean_parent() {
    let (_root, runtime, source) = fixture();
    let source = runtime.validate_source(&source).unwrap();
    let branch = runtime.validate_ref("main").unwrap();
    let project_id = Uuid::now_v7();
    let parent_id = Uuid::now_v7();
    let child_id = Uuid::now_v7();
    let parent = runtime
        .provision(project_id, parent_id, &source, &branch)
        .await
        .unwrap();
    let child = runtime
        .provision(project_id, child_id, &source, &branch)
        .await
        .unwrap();

    std::fs::write(child.root.join("README.md"), "staged\n").unwrap();
    git(&child.root, &["add", "README.md"]);
    std::fs::write(child.root.join("README.md"), "staged\nunstaged\n").unwrap();
    std::fs::write(child.root.join("new.txt"), "untracked\n").unwrap();

    runtime
        .apply_workspace_changes(child_id, parent_id)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(parent.root.join("README.md")).unwrap(),
        "staged\nunstaged\n"
    );
    assert_eq!(
        std::fs::read_to_string(parent.root.join("new.txt")).unwrap(),
        "untracked\n"
    );

    let error = runtime
        .apply_workspace_changes(child_id, parent_id)
        .await
        .unwrap_err();
    assert!(matches!(error, GitRuntimeError::Conflict(_)));
}

#[tokio::test]
async fn allocates_a_unique_name_when_renaming_a_workspace_branch() {
    let (_root, runtime, source) = fixture();
    let source = runtime.validate_source(&source).unwrap();
    let branch = runtime.validate_ref("main").unwrap();
    let workspace_id = Uuid::now_v7();
    runtime
        .provision(Uuid::now_v7(), workspace_id, &source, &branch)
        .await
        .unwrap();
    runtime
        .create_branch(workspace_id, "feature/browser")
        .await
        .unwrap();
    runtime.checkout_branch(workspace_id, "main").await.unwrap();

    let renamed = runtime
        .rename_branch(workspace_id, "feature/browser")
        .await
        .unwrap();
    assert_eq!(renamed, "feature/browser-2");
    let branches = runtime.list_branches(workspace_id).await.unwrap();
    assert!(branches.iter().any(|branch| branch.name == renamed));
    assert!(branches
        .iter()
        .any(|branch| branch.name == "feature/browser"));
}

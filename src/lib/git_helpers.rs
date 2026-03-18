use crate::command_helpers::did_command_exit_successfully;
use std::process::Command;

pub fn get_pwd_git_toplevel(passed_dir: &Option<String>) -> (String, bool) {
    let path_to_operate_on = match passed_dir {
        Some(_) => passed_dir.clone(),
        None => match std::env::current_dir() {
            Ok(current_dir) => Some(current_dir.display().to_string()),
            Err(_) => None,
        },
    };

    if let Some(repo_path) = path_to_operate_on {
        let mut git_cmd = Command::new("git");
        git_cmd
            .arg("-C")
            .arg(repo_path)
            .arg("rev-parse")
            .arg("--show-toplevel");
        let git_cmd_output = git_cmd.output();
        match did_command_exit_successfully(&git_cmd_output) {
            false => ("".to_string(), false),
            true => {
                let git_cmd_output_unwrapped = git_cmd_output.unwrap();
                let git_cmd_stdout = String::from_utf8_lossy(&git_cmd_output_unwrapped.stdout)
                    .trim()
                    .to_string();
                (git_cmd_stdout, true)
            }
        }
    } else {
        ("".to_string(), false)
    }
}

pub fn restore_file(local_git_repo: &str, file_to_restore: &str) -> bool {
    let mut git_restore_cmd = Command::new("git");
    git_restore_cmd
        .arg("-C")
        .arg(local_git_repo)
        .arg("restore")
        .arg("--")
        .arg(file_to_restore);
    let git_restore_cmd_output = git_restore_cmd.output();
    did_command_exit_successfully(&git_restore_cmd_output)
}

pub fn perform_git_pull(local_git_repo: &str) {
    let mut git_pull_cmd = Command::new("git");
    git_pull_cmd
        .arg("-C")
        .arg(local_git_repo)
        .arg("pull")
        .arg("--ff-only");
    let git_pull_cmd_output = git_pull_cmd.output();
    if !did_command_exit_successfully(&git_pull_cmd_output) {
        let git_pull_cmd_output_unwrapped = git_pull_cmd_output.unwrap();
        let git_pull_cmd_stderr = String::from_utf8_lossy(&git_pull_cmd_output_unwrapped.stderr)
            .trim()
            .to_string();
        eprintln!(
            "Warning: Failed to pull changes from remote for the specified flake (STDERR:\n'''{}''')",
            git_pull_cmd_stderr
        );
    }
}

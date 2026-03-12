use std::process::Command;

pub fn display_full_command(cmd: &Command) -> String {
    format!(
        "{} {}",
        cmd.get_program().display(),
        cmd.get_args()
            .map(|s| s.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    )
}

pub fn did_command_exit_successfully(
    command_output: &Result<std::process::Output, std::io::Error>,
) -> bool {
    match command_output {
        Ok(command_result) => match command_result.status.code() {
            Some(exit_code) => exit_code == 0,
            None => false,
        },
        Err(_) => false,
    }
}

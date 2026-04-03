use crate::{command_helpers::*, git_helpers::get_pwd_git_toplevel};

use std::{
    collections::HashMap,
    env,
    error::Error,
    fs,
    path::Path,
    process,
    process::Command,
    time::{Duration, SystemTime},
};

static NIX_SYSTEMS_SUPPORTED_BY_NAVYA_CI: [&str; 5] = [
    "aarch64-linux",
    "riscv64-linux",
    "x86_64-linux",
    "aarch64-darwin",
    "x86_64-darwin",
];

#[derive(PartialEq)]
pub enum MachineRole {
    Node,
    Server,
    QuickCI,
}
impl std::fmt::Debug for MachineRole {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}
impl std::fmt::Display for MachineRole {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Node => write!(f, "node"),
            Self::Server => write!(f, "server"),
            Self::QuickCI => write!(f, "quick_ci"),
        }
    }
}

#[derive(Debug)]
pub struct NixConfig {
    pub flake_outputs_to_build: Vec<String>,
    pub ignore_missing_flake_outputs: bool,
    pub nix_systems: Vec<String>,
    pub flake_local_reference: String,
    pub sleep_break: u64,
    pub update_lockfile: bool,
    pub machine_role: MachineRole,
    pub signing_key_path: String,
    pub ignore_signing_error: bool,
    pub nix_copy_machines: Vec<String>,
    pub copy_unsigned_paths: bool,
    pub max_parallelism: usize,
    pub ignore_derivations_eval_errors: bool,
}

#[derive(Debug, PartialEq, PartialOrd, Eq, Ord)]
pub struct NixDerivation {
    pub derivation_attribute: String,
    pub flake_store_path: String,
    pub fully_qualified_derivation_path: String,
    pub derivation_host_platform: String,
    pub drvpath: String,
    pub outpath: String,
}

fn create_nix_command() -> Command {
    let mut nix_command = Command::new("nix");
    nix_command
        .arg("--extra-experimental-features")
        .arg("nix-command")
        .arg("--extra-experimental-features")
        .arg("flakes");
    nix_command
}

fn print_help() {
    let help_message = format!("Usage: navya-ci [OPTIONS...] [ARGS...]
    --help                                       Print this help

    --flake-path [FLAKE_PATH]                    Specify the local flake path
    --nix-system [NIX_SYSTEM]...                 The Nix system(s) to build derivations for (see bottom for supported
                                                 Nix systems)
    --machine-role [node|server|quick_ci]        The role of this system, only one of the specified values
    --flake-output-to-build [FLAKE_OUTPUT]...    The flake toplevel output(s) to build

    --ignore-missing-flake-outputs               Specifying this argument will ensure if any specified flake toplevel
                                                 output(s) are missing, will be treated as a warning, not an error
    --sleep-break [SECONDS]                      Time in seconds to sleep for between two continuous runs, if there is
                                                 no change to the flake (default is 600)
    --update-lockfile                            Specifying this argument will ensure that the lockfile is updated
                                                 \"manually\" by `navya-ci`, instead of relying on `git-pull`s
    --ignore-derivations-eval-errors             Treat derivation evaluation errors as warnings, instead of hard
                                                 stopping errors.
    --signing-key-path [SIGNING_KEY_PATH]        File path used to sign Nix store paths
    --ignore-signing-error                       Specifying this argument will treat signing errors by the use of the
                                                 specified signing key file, as warnings
    --nix-copy-machine [REMOTE_STORE_URI]...     The remote store URI(s) where the built/cached (node|server) paths will
                                                 be copied to
    --copy-unsigned-paths                        Specifying this argument will copy even unsigned paths to the remote
                                                 store URI(s)


    Supported Nix systems: '{}'
    ", NIX_SYSTEMS_SUPPORTED_BY_NAVYA_CI.join("', '"));
    println!("{}", help_message);
}

pub fn get_nix_config() -> Result<NixConfig, lexopt::Error> {
    let meminfo = fs::read_to_string("/proc/meminfo").expect("Could not read /proc/meminfo");
    let memtotal = meminfo
        .lines()
        .find(|line| line.starts_with("MemTotal:"))
        .and_then(|memtotal_line| memtotal_line.split_whitespace().nth(1))
        .and_then(|memtotal_kb| memtotal_kb.parse().ok())
        .unwrap_or(0);
    if memtotal < 1 {
        return Err("Could not determine total memory of this host".into());
    }

    if env::args().count() == 1 {
        print_help();
        process::exit(1);
    }

    use lexopt::prelude::*;
    let mut parser = lexopt::Parser::from_env();

    let mut flake_outputs_to_build: Vec<String> = Vec::new();
    let mut ignore_missing_flake_outputs: bool = false;
    let mut nix_systems: Vec<String> = Vec::new();
    let mut flake_local_reference: Option<String> = None;
    let mut sleep_break: u64 = 600;
    let mut update_lockfile: bool = false;
    let mut machine_role: Option<MachineRole> = None;
    let mut signing_key_path: Option<String> = None;
    let mut ignore_signing_error: Option<bool> = None;
    let mut nix_copy_machines: Vec<String> = Vec::new();
    let mut copy_unsigned_paths: bool = false;
    let total_mem = memtotal / 1024 / 1024;
    let mut max_parallelism: usize = total_mem / 2;
    let mut ignore_derivations_eval_errors = false;

    while let Some(arg) = parser.next()? {
        match arg {
            Long("flake-output-to-build") => {
                let indv_flake_output_to_build = parser.value()?.string()?;
                if flake_outputs_to_build.contains(&indv_flake_output_to_build) {
                    eprintln!(
                        "Warning: `--flake-output-to-build` has a duplicate specification of output '{}'",
                        indv_flake_output_to_build
                    );
                } else {
                    flake_outputs_to_build.push(indv_flake_output_to_build);
                }
            }

            Long("ignore-missing-flake-outputs") => {
                ignore_missing_flake_outputs = true;
            }

            Long("nix-system") => {
                let nix_system = parser.value()?.string()?;
                if !NIX_SYSTEMS_SUPPORTED_BY_NAVYA_CI.contains(&nix_system.as_str()) {
                    return Err(format!(
                        "The specified Nix system '{}' is unsupported by navya-ci",
                        nix_system
                    )
                    .into());
                }
                nix_systems.push(nix_system);
            }

            Long("flake-path") => {
                let specified_flake_path = parser.value()?.string()?;
                flake_local_reference = Some(specified_flake_path);
            }

            Long("machine-role") => {
                let specified_machine_role = parser.value()?;
                let specified_machine_role = specified_machine_role.to_str().unwrap();
                let valid_machine_role = match specified_machine_role {
                    "node" => MachineRole::Node,
                    "server" => MachineRole::Server,
                    "quick_ci" => MachineRole::QuickCI,
                    _ => return Err(format!(
                        "`{}` is an invalid machine role; Valid roles are: 'node server quick_ci'",
                        specified_machine_role
                    )
                    .into()),
                };
                machine_role = Some(valid_machine_role);
            }

            Long("sleep-break") => {
                let specified_sleep_break = parser.value()?.string()?;
                let sleep_break_u64 = specified_sleep_break.trim().parse().expect(
                    "Failed to parse the value of `--sleep-break` from a `String` into a `u64`",
                );
                if sleep_break_u64 == 0 {
                    return Err("--sleep-break cannot be 0".into());
                }
                sleep_break = sleep_break_u64;
            }

            Long("update-lockfile") => {
                update_lockfile = true;
            }

            Long("ignore-derivations-eval-errors") => {
                ignore_derivations_eval_errors = true;
            }

            Long("signing-key-path") => {
                let specified_signing_key_path = parser.value()?.string()?;
                if Path::new(&specified_signing_key_path).exists() {
                    signing_key_path = Some(specified_signing_key_path);
                } else {
                    return Err("Signing key path was specified but does not exist".into());
                }
            }

            Long("ignore-signing-error") => {
                ignore_signing_error = Some(true);
            }

            Long("nix-copy-machine") => {
                nix_copy_machines.push(parser.value()?.string()?);
            }

            Long("copy-unsigned-paths") => {
                copy_unsigned_paths = true;
            }

            Long("help") => {
                print_help();
                process::exit(1);
            }

            _ => return Err(arg.unexpected()),
        }
    }

    if max_parallelism < 1 {
        max_parallelism = 1;
        eprintln!(
            "Warning: Either the host does not have enough memory or could not calculate maximum parallelism for whatever reason"
        );
    }

    let (flake_path_git_toplevel, flake_path_is_git_repo) =
        get_pwd_git_toplevel(&flake_local_reference);
    let flake_local_reference_unwrapped = match flake_local_reference {
        Some(val) => val,
        None => {
            print_help();
            return Err("`--flake-path` must be specified".into());
        }
    };
    match flake_path_is_git_repo {
        false => {
            return Err(format!(
                "`--flake-path` was specified as '{}' but it is not a git repository",
                flake_local_reference_unwrapped
            )
            .into());
        }
        true => {
            if flake_path_git_toplevel != flake_local_reference_unwrapped {
                return Err(format!(
                    "`--flake-path` was specified as '{}' but the git toplevel is '{}'",
                    flake_local_reference_unwrapped, flake_path_git_toplevel
                )
                .into());
            }
        }
    }
    let can_write_to_flake_local_reference = match fs::OpenOptions::new()
        .write(true)
        .open(format!("{}/flake.nix", &flake_local_reference_unwrapped))
    {
        Err(_) => false,
        Ok(flakefile_fd) => flakefile_fd
            .set_times(fs::FileTimes::new().set_modified(SystemTime::now()))
            .is_ok(),
    };
    if !can_write_to_flake_local_reference {
        return Err("Cannot modify the specified flake path".into());
    }

    if nix_systems.is_empty() {
        print_help();
        return Err(
            "At least one supported Nix system must be specified with `--nix-system`".into(),
        );
    } else {
        let mut nix_config_show_cmd = create_nix_command();
        nix_config_show_cmd.arg("config").arg("show");
        let nix_config_show_cmd_output = nix_config_show_cmd.output();
        match did_command_exit_successfully(&nix_config_show_cmd_output) {
            false => {
                let full_command = display_full_command(&nix_config_show_cmd);
                return Err(format!(
                    "The command `{}` exited with a non-zero return code",
                    full_command
                )
                .into());
            }
            true => {
                let nix_config_show_cmd_output_unwrapped = nix_config_show_cmd_output.unwrap();
                let nix_config_show_cmd_stdout =
                    String::from_utf8_lossy(&nix_config_show_cmd_output_unwrapped.stdout)
                        .trim()
                        .to_string();
                let nix_config_show_cmd_stdout = nix_config_show_cmd_stdout
                    .lines()
                    .filter(|line| {
                        line.starts_with("system = ") || line.starts_with("extra-platforms = ")
                    })
                    .map(String::from)
                    .collect::<String>();
                match nix_config_show_cmd_stdout.is_empty() {
                    true => {
                        return Err(
                            "Could not determine the Nix systems supported by this host".into()
                        );
                    }
                    false => {
                        let mut unsupported_nix_systems: Vec<&str> = Vec::new();
                        for ze_nix_system in nix_systems.iter() {
                            if !nix_config_show_cmd_stdout
                                .contains(format!(" {}", ze_nix_system).as_str())
                            {
                                unsupported_nix_systems.push(ze_nix_system);
                            }
                        }
                        if !unsupported_nix_systems.is_empty() {
                            let display_string = unsupported_nix_systems.join("' '");
                            return Err(format!("The specified Nix system(s) \"'{}'\" is/are unsupported by this host", display_string).into());
                        }
                    }
                }
            }
        }
    }

    if flake_outputs_to_build.is_empty() {
        print_help();
        return Err(
            "At least one flake output to build must be specified with `--flake-output-to-build`"
                .into(),
        );
    }

    let machine_role = match machine_role {
        Some(val) => val,
        None => {
            print_help();
            return Err(
                "`--machine-role` is a required argument; Valid roles are: 'node server quick_ci'"
                    .into(),
            );
        }
    };

    let (signing_key_path, ignore_signing_error) = match (signing_key_path, ignore_signing_error) {
        // No signing key specified, ignore signing errors
        (None, None) => ("".to_string(), true),
        (Some(key), Some(val)) => (key, val),
        (Some(val), None) => (val, false),
        (None, Some(_)) => {
            return Err(
                "--ignore-signing-error was specified but no signing key was specified".into(),
            );
        }
    };

    Ok(NixConfig {
        flake_outputs_to_build,
        ignore_missing_flake_outputs,
        nix_systems,
        flake_local_reference: flake_local_reference_unwrapped,
        sleep_break,
        update_lockfile,
        machine_role,
        signing_key_path,
        ignore_signing_error,
        nix_copy_machines,
        copy_unsigned_paths,
        max_parallelism,
        ignore_derivations_eval_errors,
    })
}

fn was_lockfile_updated_in_last_sleep_break(flake_local_reference: &str, threshold: u64) -> bool {
    let lockfile_path = format!("{}/flake.lock", flake_local_reference);
    let metadata = fs::metadata(lockfile_path);
    match metadata {
        Err(_) => {
            eprintln!(
                "Warning: Could not determine the metadata for the flake lockfile, assuming modified under last {} seconds",
                threshold
            );
            true
        }
        Ok(metadata) => {
            let modified = metadata.modified();
            match modified {
                Err(_) => {
                    eprintln!(
                        "Warning: Could not determine the mtime for the flake lockfile, assuming modified under last {} seconds",
                        threshold
                    );
                    true
                }
                Ok(modified) => {
                    let threshold = SystemTime::now() - Duration::from_secs(threshold);
                    modified > threshold
                }
            }
        }
    }
}

fn perform_nix_flake_update_unwrapped(flake_local_reference: &str) -> bool {
    let mut nix_flake_update_cmd = create_nix_command();
    nix_flake_update_cmd
        .arg("flake")
        .arg("update")
        .current_dir(flake_local_reference);

    if let Ok(github_token_val) = env::var("GITHUB_TOKEN") {
        eprintln!("Notice: Using the `GITHUB_TOKEN` to update the lockfile");
        nix_flake_update_cmd.env(
            "NIX_CONFIG",
            format!("access-tokens = github.com={}", github_token_val),
        );
    }

    let nix_flake_update_cmd_output = nix_flake_update_cmd.output();
    did_command_exit_successfully(&nix_flake_update_cmd_output)
}

pub fn perform_nix_flake_update(nix_config: &NixConfig) {
    if nix_config.update_lockfile {
        match was_lockfile_updated_in_last_sleep_break(
            &nix_config.flake_local_reference,
            nix_config.sleep_break,
        ) {
            true => {
                eprintln!(
                    "Notice: It appears that the lockfile was updated in the last {} seconds, skipping a lockfile update to be under any GitHub API rate limit(s)",
                    nix_config.sleep_break
                );
            }
            false => match perform_nix_flake_update_unwrapped(&nix_config.flake_local_reference) {
                true => (),
                false => {
                    eprintln!(
                        "Warning: Encountered an error updating the lockfile of the specified flake"
                    );
                }
            },
        }
    }
}

pub fn perform_nix_flake_archive(flake_local_reference: &str) -> Result<String, Box<dyn Error>> {
    let mut nix_flake_archive_cmd = create_nix_command();
    nix_flake_archive_cmd
        .arg("flake")
        .arg("archive")
        .arg("--json")
        .current_dir(flake_local_reference);
    let nix_flake_archive_cmd_output = nix_flake_archive_cmd.output();
    match did_command_exit_successfully(&nix_flake_archive_cmd_output) {
        true => {
            let nix_flake_archive_cmd_output_unwrapped = nix_flake_archive_cmd_output.unwrap();
            let nix_flake_archive_cmd_stdout =
                String::from_utf8_lossy(&nix_flake_archive_cmd_output_unwrapped.stdout)
                    .trim()
                    .to_string();
            let nix_flake_archive_cmd_jsonobj =
                nojson::RawJson::parse(&nix_flake_archive_cmd_stdout).unwrap_or_else(|_| {
                    panic!(
                        "Could not parse the JSON string ('{}')",
                        nix_flake_archive_cmd_stdout
                    )
                });
            let flake_store_path: String = nix_flake_archive_cmd_jsonobj
                .value()
                .to_path_member(&["path"])?
                .required()
                .expect("Could not determine the store path of the flake")
                .try_into()?;

            match flake_store_path.is_empty() {
                false => Ok(flake_store_path),
                true => Err("Could not determine the store path of the flake".into()),
            }
        }
        false => Err("Could not archive the flake inputs for the specified flake".into()),
    }
}

fn get_flake_toplevel_outputs(
    flake_store_path: &str,
    flake_outputs_to_find: &[String],
    ignore_missing_flake_outputs: bool,
) -> Result<Vec<String>, Box<dyn Error>> {
    eprintln!(
        "Notice: Checking if specified flake outputs to build are present in the `<flake>.outputs`"
    );

    let nix_expr_string = format!(
        "builtins.attrNames (builtins.getFlake \"{}\").outputs",
        flake_store_path
    );
    let mut nix_eval_cmd = Command::new("nix-instantiate");
    nix_eval_cmd
        .arg("--option")
        .arg("eval-cache")
        .arg("true")
        .arg("--eval")
        .arg("--json")
        .arg("--expr")
        .arg(nix_expr_string);
    let nix_eval_cmd_output = nix_eval_cmd.output();
    match did_command_exit_successfully(&nix_eval_cmd_output) {
        false => Err("Could not find the toplevel outputs of the specified flake".into()),
        true => {
            let mut flake_outputs_found: Vec<String> =
                Vec::with_capacity(flake_outputs_to_find.len());
            let mut flake_outputs_missing: Vec<String> =
                Vec::with_capacity(flake_outputs_to_find.len());

            let nix_eval_cmd_output_unwrapped = nix_eval_cmd_output.unwrap();
            let nix_eval_cmd_stdout =
                String::from_utf8_lossy(&nix_eval_cmd_output_unwrapped.stdout)
                    .trim()
                    .to_string();

            for flake_output in flake_outputs_to_find.iter() {
                let pattern = format!("\"{}\"", flake_output);
                if nix_eval_cmd_stdout.contains(&pattern) {
                    flake_outputs_found.push(flake_output.to_string());
                } else {
                    flake_outputs_missing.push(flake_output.to_string());
                }
            }

            match flake_outputs_missing.is_empty() {
                true => Ok(flake_outputs_found),
                false => {
                    let missing_outputs_message = format!(
                        "Of the specified flake outputs to build, these are missing: '{}'",
                        flake_outputs_missing.join("' '")
                    );
                    if ignore_missing_flake_outputs {
                        eprintln!("Warning: {}", missing_outputs_message);
                        Ok(flake_outputs_found)
                    } else {
                        Err(missing_outputs_message.into())
                    }
                }
            }
        }
    }
}

fn is_flake_output_arch_dependant(
    flake_store_path: &str,
    flake_output: &str,
) -> Result<bool, Box<dyn Error>> {
    let nix_expr_string = format!(
        "builtins.attrNames (builtins.getFlake \"{}\").outputs.{}",
        flake_store_path, flake_output
    );
    let mut nix_eval_cmd = Command::new("nix-instantiate");
    nix_eval_cmd
        .arg("--option")
        .arg("eval-cache")
        .arg("true")
        .arg("--eval")
        .arg("--json")
        .arg("--expr")
        .arg(nix_expr_string);
    let nix_eval_cmd_output = nix_eval_cmd.output();
    match did_command_exit_successfully(&nix_eval_cmd_output) {
        false => Err(format!("Could not determine if the flake output '{}' is dependant on architecture (ISA/Nix system) or not", flake_output).into()),
        true => {
            let nix_eval_cmd_output_unwrapped = nix_eval_cmd_output.unwrap();
            let nix_eval_cmd_stdout = String::from_utf8_lossy(&nix_eval_cmd_output_unwrapped.stdout).trim().to_string();

            let is_flake_output_arch_dependant = NIX_SYSTEMS_SUPPORTED_BY_NAVYA_CI.iter().any(|nix_system| {
                let pattern = format!("\"{}\"", nix_system);
                nix_eval_cmd_stdout.contains(&pattern)
            });
            Ok(is_flake_output_arch_dependant)
        }
    }
}

fn build_nix_derivation_struct_object(
    derivation_attribute: String,
    evaluated_drvpath: String,
    flake_store_path: String,
    derivation_host_platform: String,
) -> Option<NixDerivation> {
    let fully_qualified_derivation_path = format!("{}#{}", flake_store_path, derivation_attribute);
    let mut nix_store_query_cmd = Command::new("nix-store");
    nix_store_query_cmd
        .arg("--query")
        .arg("--outputs")
        .arg(&evaluated_drvpath);
    let nix_store_query_cmd_output = nix_store_query_cmd.output();

    match did_command_exit_successfully(&nix_store_query_cmd_output) {
        false => {
            eprintln!(
                "Warning: Could not evaluate outPath for attribute '{}'",
                fully_qualified_derivation_path
            );
            None
        }
        true => {
            let nix_store_query_cmd_output_unwrapped = nix_store_query_cmd_output.unwrap();
            let outpath = String::from_utf8_lossy(&nix_store_query_cmd_output_unwrapped.stdout)
                .trim()
                .to_string();
            Some(NixDerivation {
                derivation_attribute,
                flake_store_path,
                fully_qualified_derivation_path,
                derivation_host_platform,
                outpath,
                drvpath: evaluated_drvpath,
            })
        }
    }
}

fn find_flake_drvs_to_build(
    flake_store_path: &str,
    flake_outputs_to_build: &[String],
    supported_nix_systems: &[String],
) -> Result<HashMap<String, String>, Box<dyn Error>> {
    eprintln!("Notice: Determining the flake derivations to build");
    let mut nix_derivations_to_build: HashMap<String, String> =
        HashMap::with_capacity(flake_outputs_to_build.len());
    for flake_output in flake_outputs_to_build {
        let is_flake_output_arch_dependant =
            is_flake_output_arch_dependant(flake_store_path, flake_output)?;
        let mut nix_eval_cmd = Command::new("nix-instantiate");
        nix_eval_cmd
            .arg("--option")
            .arg("eval-cache")
            .arg("true")
            .arg("--eval")
            .arg("--json")
            .arg("--strict")
            .arg("--expr");
        if is_flake_output_arch_dependant {
            nix_eval_cmd.arg(format!("
              let
                systems = [ \"{}\" ];
                output = (builtins.getFlake \"{}\").outputs.{};
              in
              builtins.foldl'
                (acc: system: acc // {{ ${{system}} = builtins.attrNames output.${{system}}; }}) {{}} systems
            ", supported_nix_systems.join("\" \""), flake_store_path, flake_output));
        } else {
            nix_eval_cmd.arg(format!("
              let
                wantedSystems = [ \"{}\" ];
                configs = (builtins.getFlake \"{}\").outputs.{};
                filtered = builtins.filter (n: builtins.elem configs.${{n}}.config.nixpkgs.hostPlatform.system wantedSystems) (builtins.attrNames configs);
              in
              builtins.foldl'
                (acc: n: let arch = configs.${{n}}.config.nixpkgs.hostPlatform.system; in acc // {{ ${{arch}} = (acc.${{arch}} or []) ++ [n]; }}) {{}} filtered
            ", supported_nix_systems.join("\" \""), flake_store_path, flake_output));
        }
        let nix_eval_cmd_output = nix_eval_cmd.output();
        match did_command_exit_successfully(&nix_eval_cmd_output) {
            false => {
                return Err(format!(
                    "Could not evaluate sub-attributes of output '{}'",
                    flake_output
                )
                .into());
            }
            true => {
                let nix_eval_cmd_output_unwrapped = nix_eval_cmd_output.unwrap();
                let nix_eval_cmd_stdout =
                    String::from_utf8_lossy(&nix_eval_cmd_output_unwrapped.stdout)
                        .trim()
                        .to_string();
                let nix_eval_cmd_jsonobj = nojson::RawJson::parse(&nix_eval_cmd_stdout)
                    .unwrap_or_else(|_| {
                        panic!(
                            "Could not parse the JSON string ('{}')",
                            nix_eval_cmd_stdout
                        )
                    });
                let nix_eval_cmd_jsonobj = nix_eval_cmd_jsonobj.value().to_object()?;
                for (derivation_host_platform, drv_attrs) in nix_eval_cmd_jsonobj {
                    for derivation in drv_attrs.to_array()? {
                        let mut suffix = "";
                        match flake_output.as_str() {
                            "apps" => suffix = ".program",
                            "homeConfigurations" => suffix = ".activationPackage",
                            "isoImages" => suffix = ".config.system.build.isoImage",
                            "isoImagesUncompressed" => suffix = ".config.system.build.toplevel",
                            "kexecTree" => suffix = ".config.system.build.toplevel",
                            "nixosConfigurations" => suffix = ".config.system.build.toplevel",
                            _ => (),
                        }
                        let derivation_host_platform =
                            derivation_host_platform.as_string_str()?.to_string();
                        let nix_system_str = if is_flake_output_arch_dependant {
                            format!(".{}", derivation_host_platform)
                        } else {
                            "".to_string()
                        };
                        let derivation_attribute = format!(
                            "{}{}.{}{}",
                            flake_output,
                            nix_system_str,
                            derivation.as_string_str()?,
                            suffix
                        );
                        nix_derivations_to_build
                            .insert(derivation_attribute, derivation_host_platform);
                    }
                }
            }
        }
    }
    Ok(nix_derivations_to_build)
}

fn get_indv_derivation_outpath(
    nix_eval_cmd_args: &[&str],
    indv_drv: String,
    flake_store_path: String,
    nix_system: String,
) -> Option<NixDerivation> {
    let nix_expr_string = format!(
        "(builtins.getFlake \"{}\").outputs.{}.drvPath",
        flake_store_path, indv_drv
    );
    let mut nix_eval_cmd = Command::new("nix-instantiate");
    nix_eval_cmd
        .args(nix_eval_cmd_args)
        .arg("--raw")
        .arg("--expr")
        .arg(nix_expr_string);
    let nix_eval_cmd_output = nix_eval_cmd.output();
    match did_command_exit_successfully(&nix_eval_cmd_output) {
        false => {
            eprintln!(
                "Warning: Could not evaluate outPath of this Nix attribute: {}",
                indv_drv
            );
            None
        }
        true => {
            let nix_eval_cmd_output_unwrapped = nix_eval_cmd_output.unwrap();
            let nix_eval_cmd_stdout =
                String::from_utf8_lossy(&nix_eval_cmd_output_unwrapped.stdout)
                    .trim()
                    .to_string();
            build_nix_derivation_struct_object(
                indv_drv,
                nix_eval_cmd_stdout,
                flake_store_path,
                nix_system,
            )
        }
    }
}

fn get_derivations_outpaths(
    nix_derivations_and_systems: &HashMap<String, String>,
    supported_nix_systems: &[String],
    flake_store_path: &str,
    max_parallelism: usize,
    ignore_derivations_eval_errors: bool,
) -> Vec<NixDerivation> {
    let mut nix_derivations_struct_object = Vec::with_capacity(nix_derivations_and_systems.len());
    let mut encountered_derivations_without_outpaths = false;

    for nix_system in supported_nix_systems {
        let mut grouped_drvs: HashMap<String, Vec<String>> = HashMap::new();
        let nix_derivations_list = nix_derivations_and_systems
            .iter()
            .filter(|(_, nix_sys)| *nix_sys == nix_system)
            .map(|(ze_drv, _)| ze_drv.clone())
            .collect::<Vec<String>>();
        for drv in &nix_derivations_list {
            let toplevel = drv.split(".").next().unwrap();
            grouped_drvs
                .entry(toplevel.to_string())
                .or_default()
                .push(drv.to_string());
        }

        for (flake_toplevel_output, derivation_group) in grouped_drvs.iter() {
            let derivation_chunks = derivation_group
                .chunks(max_parallelism)
                .map(|chunk| chunk.to_vec())
                .collect::<Vec<Vec<String>>>();
            for derivations in derivation_chunks {
                if !derivations.is_empty() {
                    let derivation_expr = derivations
                        .iter()
                        .map(|indv_drv| {
                            format!("\"{}\" = flake.outputs.{}.drvPath;", indv_drv, indv_drv)
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    let nix_eval_string = format!(
                        "let flake = builtins.getFlake \"{}\"; in {{ {} }}",
                        flake_store_path, derivation_expr
                    );

                    eprintln!(
                        "Notice: Evaluating outputs under '{}' for system '{}'",
                        flake_toplevel_output, nix_system
                    );
                    let nix_eval_cmd_args = vec![
                        "--option",
                        "eval-cache",
                        "true",
                        "--eval",
                        "--option",
                        "eval-system",
                        nix_system,
                    ];
                    let mut nix_eval_cmd = Command::new("nix-instantiate");
                    nix_eval_cmd
                        .args(&nix_eval_cmd_args)
                        .arg("--json")
                        .arg("--strict")
                        .arg("--expr")
                        .arg(&nix_eval_string);
                    let nix_eval_cmd_output = nix_eval_cmd.output();
                    match did_command_exit_successfully(&nix_eval_cmd_output) {
                        false => {
                            for indv_drv in derivations {
                                match get_indv_derivation_outpath(
                                    &nix_eval_cmd_args,
                                    indv_drv,
                                    flake_store_path.to_string(),
                                    nix_system.to_string(),
                                ) {
                                    None => encountered_derivations_without_outpaths = true,
                                    Some(nix_derivation_struct_obj) => {
                                        nix_derivations_struct_object
                                            .push(nix_derivation_struct_obj)
                                    }
                                }
                            }
                        }
                        true => {
                            let nix_eval_cmd_output_unwrapped = nix_eval_cmd_output.unwrap();
                            let nix_eval_cmd_stdout =
                                String::from_utf8_lossy(&nix_eval_cmd_output_unwrapped.stdout)
                                    .trim()
                                    .to_string();
                            let nix_eval_cmd_stdout_jsonobj: nojson::Json<HashMap<String, String>> =
                                nix_eval_cmd_stdout.parse().unwrap_or_else(|_| {
                                    panic!(
                                        "Could not parse the JSON string ('{}')",
                                        nix_eval_cmd_stdout
                                    )
                                });
                            for (evaluated_nix_derivation, evaluated_drvpath) in
                                nix_eval_cmd_stdout_jsonobj.0.iter()
                            {
                                let nix_derivation_struct_obj = build_nix_derivation_struct_object(
                                    evaluated_nix_derivation.to_string(),
                                    evaluated_drvpath.to_string(),
                                    flake_store_path.to_string(),
                                    nix_system.to_string(),
                                );
                                if let Some(nix_derivation_struct_obj) = nix_derivation_struct_obj {
                                    nix_derivations_struct_object.push(nix_derivation_struct_obj);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if encountered_derivations_without_outpaths && !ignore_derivations_eval_errors {
        process::exit(1);
    }

    nix_derivations_struct_object.sort();
    nix_derivations_struct_object
}

pub fn get_nix_derivations_to_build(
    nix_config: &NixConfig,
    flake_store_path: &str,
) -> Result<Vec<NixDerivation>, Box<dyn Error>> {
    let flake_toplevel_outputs_discovered = get_flake_toplevel_outputs(
        flake_store_path,
        &nix_config.flake_outputs_to_build,
        nix_config.ignore_missing_flake_outputs,
    )?;
    let nix_derivations_and_systems = find_flake_drvs_to_build(
        flake_store_path,
        &flake_toplevel_outputs_discovered,
        &nix_config.nix_systems,
    )?;
    let nix_derivations_to_build = get_derivations_outpaths(
        &nix_derivations_and_systems,
        &nix_config.nix_systems,
        flake_store_path,
        nix_config.max_parallelism,
        nix_config.ignore_derivations_eval_errors,
    );

    Ok(nix_derivations_to_build)
}

fn do_single_drv_nix_build(nix_drv: &str, nix_build_common_args: &[&str]) -> bool {
    let mut nix_build_cmd = create_nix_command();
    nix_build_cmd.args(nix_build_common_args).arg(nix_drv);
    did_command_exit_successfully(&nix_build_cmd.output())
}

fn do_nix_build_unwrapped(nix_derivations_to_build: &[NixDerivation]) {
    eprintln!("Notice: Starting Nix build");
    let mut notices: Vec<String> = Vec::with_capacity(nix_derivations_to_build.len());
    let nix_build_common_args: Vec<&str> = vec!["build", "--keep-going", "--no-link", "--quiet"];

    let mut nix_build_cmd = create_nix_command();
    nix_build_cmd.args(&nix_build_common_args);
    nix_build_cmd.args(
        nix_derivations_to_build
            .iter()
            .map(|nix_drv| nix_drv.fully_qualified_derivation_path.clone())
            .collect::<Vec<String>>(),
    );
    let nix_build_cmd_output = nix_build_cmd.output();

    match did_command_exit_successfully(&nix_build_cmd_output) {
        true => {
            notices.extend(
                nix_derivations_to_build
                    .iter()
                    .map(|drv_struct| {
                        format!(
                            "Notice: Successful build: {} ---> {}",
                            drv_struct.fully_qualified_derivation_path, drv_struct.outpath
                        )
                    })
                    .collect::<Vec<String>>(),
            );
        }
        false => {
            for nix_drv_struct in nix_derivations_to_build {
                match do_single_drv_nix_build(
                    &nix_drv_struct.fully_qualified_derivation_path,
                    &nix_build_common_args,
                ) {
                    true => notices.push(format!(
                        "Notice: Successful build: {} ---> {}",
                        nix_drv_struct.fully_qualified_derivation_path, nix_drv_struct.outpath
                    )),
                    false => notices.push(format!(
                        "Notice: Unsuccessful build: {} -x-> {}",
                        nix_drv_struct.fully_qualified_derivation_path, nix_drv_struct.outpath
                    )),
                }
            }
        }
    }
    notices.sort();
    eprintln!("{}", notices.join("\n"));
}

fn is_drv_store_path_cached(drv_store_path: &str, nix_caches: &[String]) -> bool {
    let mut encountered_path_in_cache: bool = false;
    for remote_store in nix_caches {
        let mut nix_path_info_cmd = create_nix_command();
        nix_path_info_cmd
            .arg("path-info")
            .arg("--refresh")
            .arg("--store")
            .arg(remote_store)
            .arg(drv_store_path);
        if did_command_exit_successfully(&nix_path_info_cmd.output()) {
            encountered_path_in_cache = true;
            break;
        }
    }
    encountered_path_in_cache
}

fn do_nix_build_quick_ci(nix_derivations_to_build: &[NixDerivation]) {
    eprintln!("Notice: Started the Nix build for the QuickCI machine role");
    let mut nix_binary_caches: Vec<String> = Vec::new();
    let mut encountered_uncached_paths = false;
    let mut notices: Vec<String> = Vec::with_capacity(nix_derivations_to_build.len());
    let mut missing_notices: Vec<String> = Vec::new();

    let mut nix_config_show_cmd = create_nix_command();
    nix_config_show_cmd.arg("config").arg("show");
    let nix_config_show_cmd_output = nix_config_show_cmd.output();
    match did_command_exit_successfully(&nix_config_show_cmd_output) {
        false => {
            eprintln!("nix config show did not exit successfully");
        }
        true => {
            let nix_config_show_cmd_output_unwrapped = nix_config_show_cmd_output.unwrap();
            let nix_config_show_cmd_stdout =
                String::from_utf8_lossy(&nix_config_show_cmd_output_unwrapped.stdout)
                    .trim()
                    .to_string();
            let nix_config_show_cmd_stdout: String = nix_config_show_cmd_stdout
                .lines()
                .filter(|line| line.starts_with("substituters = "))
                .map(String::from)
                .collect();
            if !nix_config_show_cmd_stdout.is_empty() {
                let nix_binary_cache_config = nix_config_show_cmd_stdout
                    .split(" = ")
                    .collect::<Vec<&str>>();
                let nix_binary_cache_string = nix_binary_cache_config.get(1).unwrap().to_string();
                nix_binary_caches = nix_binary_cache_string
                    .split(" ")
                    .map(|cache| cache.trim().to_string())
                    .collect();
            }
        }
    }

    if nix_binary_caches.is_empty() {
        println!("Error: Could not determine configured Nix binary caches");
        process::exit(1);
    } else {
        for nix_drv_struct in nix_derivations_to_build {
            match is_drv_store_path_cached(&nix_drv_struct.outpath, &nix_binary_caches) {
                true => {
                    notices.push(format!(
                        "Notice: The Nix attribute '{}' is cached ('{}')",
                        nix_drv_struct.derivation_attribute, nix_drv_struct.outpath
                    ));
                }
                false => {
                    encountered_uncached_paths = true;
                    missing_notices.push(format!(
                        "Notice: The Nix attribute '{}' is NOT cached ('{}')",
                        nix_drv_struct.derivation_attribute, nix_drv_struct.outpath
                    ));
                }
            }
        }
    }

    notices.sort();
    missing_notices.sort();
    notices.extend(missing_notices);
    eprintln!("{}", notices.join("\n"));

    if encountered_uncached_paths {
        println!("Encountered uncached path(s)");
        process::exit(1);
    }
}

pub fn do_nix_build(nix_derivations_to_build: &[NixDerivation], machine_role: &MachineRole) {
    match machine_role {
        MachineRole::Node => do_nix_build_unwrapped(nix_derivations_to_build),
        MachineRole::Server => (),
        MachineRole::QuickCI => do_nix_build_quick_ci(nix_derivations_to_build),
    }
}

pub fn create_nix_gc_roots(
    nix_derivations_to_build: &[NixDerivation],
    flake_local_reference: &str,
) -> bool {
    let mut created_all_nix_gc_roots = true;
    assert!(nix_derivations_to_build.len() < 1000);
    for file_entry in fs::read_dir(flake_local_reference).unwrap() {
        let file_path = file_entry.unwrap().path();
        if file_path.is_symlink()
            && let Some(file_name) = file_path.file_name().and_then(|n| n.to_str())
            && file_name.starts_with("result")
        {
            fs::remove_file(&file_path).unwrap();
        }
    }

    for (counter, nix_drv_struct) in (0_u16..).zip(nix_derivations_to_build) {
        let out_link = format!("{}/result-{:03}", flake_local_reference, counter);
        if Path::new(&out_link).exists() {
            let _ = fs::remove_file(&out_link);
        }
        if Path::new(&nix_drv_struct.outpath).exists() {
            let mut nix_build_cmd = create_nix_command();
            nix_build_cmd
                .arg("build")
                .arg(&nix_drv_struct.outpath)
                .arg("--out-link")
                .arg(out_link);
            let _ = nix_build_cmd.output();
        } else {
            created_all_nix_gc_roots = false;
            let out_link = format!("{}-missing", out_link);
            let _ = std::os::unix::fs::symlink(
                format!("/missing{}", &nix_drv_struct.outpath),
                out_link,
            );
        }
    }
    created_all_nix_gc_roots
}

pub fn do_nix_sign(
    nix_derivations_to_build: &[NixDerivation],
    signing_key_path: &str,
    ignore_signing_error: bool,
) -> Result<(), Box<dyn Error>> {
    if !signing_key_path.is_empty() {
        eprintln!("Notice: Signing built paths using specified key");

        let mut nix_store_sign_cmd = create_nix_command();
        nix_store_sign_cmd
            .arg("store")
            .arg("sign")
            .arg("--recursive")
            .arg("--key-file")
            .arg(signing_key_path);
        for nix_drv_struct in nix_derivations_to_build {
            if Path::new(&nix_drv_struct.outpath).exists() {
                nix_store_sign_cmd.arg(&nix_drv_struct.outpath);
            }
        }
        let nix_store_sign_cmd_output = nix_store_sign_cmd.output();

        match did_command_exit_successfully(&nix_store_sign_cmd_output) {
            true => Ok(()),
            false => match ignore_signing_error {
                true => {
                    eprintln!("Warning: Signing store paths failed, ignoring the error");
                    Ok(())
                }
                false => Err("Signing store paths failed".into()),
            },
        }
    } else {
        Ok(())
    }
}

pub fn do_nix_copy(
    nix_derivations_to_build: &[NixDerivation],
    machine_role: &MachineRole,
    nix_copy_machines: &[String],
    copy_unsigned_paths: bool,
) -> bool {
    let mut nix_copy_successful = true;
    if machine_role == &MachineRole::QuickCI || nix_copy_machines.is_empty() {
        return nix_copy_successful;
    }

    eprintln!("Notice: Copying built paths to specified remote(s)");
    for machine_uri in nix_copy_machines {
        let mut nix_copy_cmd = create_nix_command();
        nix_copy_cmd
            .arg("copy")
            .arg("--refresh")
            .arg("--to")
            .arg(machine_uri);
        if copy_unsigned_paths {
            nix_copy_cmd.arg("--no-check-sigs");
        }
        for nix_drv_struct in nix_derivations_to_build {
            if Path::new(&nix_drv_struct.outpath).exists() {
                nix_copy_cmd.arg(&nix_drv_struct.outpath);
            }
        }
        let nix_copy_cmd_output = nix_copy_cmd.output();
        if !did_command_exit_successfully(&nix_copy_cmd_output) {
            nix_copy_successful = false;
            let nix_copy_cmd_output_unwrapped = nix_copy_cmd_output.unwrap();
            let nix_copy_cmd_stderr =
                String::from_utf8_lossy(&nix_copy_cmd_output_unwrapped.stderr)
                    .trim()
                    .to_string();
            let nix_copy_errors = nix_copy_cmd_stderr
                .lines()
                .filter(|line| line.starts_with("error:"))
                .map(|line| format!("Warning: nix copy: {}", line))
                .collect::<Vec<String>>();
            eprintln!(
                "Warning: Some paths could not be copied to the remote machine at '{}'\n{}",
                machine_uri,
                nix_copy_errors.join("\n")
            );
        }
    }
    nix_copy_successful
}

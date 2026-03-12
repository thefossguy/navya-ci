use navya_lib::git_helpers::*;
use navya_lib::nix_helpers::*;

use std::collections::HashMap;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let nix_config: NixConfig = get_nix_config()?;
    eprintln!("{:#?}\n", nix_config);
    let mut flake_path: String = "".to_string();

    loop {
        perform_git_pull(&nix_config.flake_local_reference);
        match perform_nix_flake_update(&nix_config) {
            true => (),
            false => {
                eprintln!(
                    "Warning: Encountered an error updating the lockfile of the specified flake, restoring the lockfile and retrying it once again"
                );
                match restore_lockfile(&nix_config) {
                    false => {
                        return Err("Could not restore the lockfile of the specified flake".into());
                    }
                    true => match perform_nix_flake_update(&nix_config) {
                        true => (),
                        false => {
                            return Err(
                                "Could not update the lockfile of the specified flake".into()
                            );
                        }
                    },
                };
            }
        };

        let current_flake_path: String = perform_nix_flake_archive(&nix_config)?;
        if flake_path == current_flake_path {
            std::thread::sleep(std::time::Duration::from_secs(nix_config.sleep_break));
        } else {
            flake_path = current_flake_path;
            eprintln!("\nNix store path for current flake state: {}", flake_path);

            let flake_outputs_to_build: Vec<String> = find_flake_toplevel_outputs(
                &flake_path,
                &nix_config.flake_outputs_to_build,
                nix_config.ignore_missing_flake_outputs,
            )?;
            let flake_drvs_to_build: Vec<String> = find_flake_drvs_to_build(
                &flake_path,
                &flake_outputs_to_build,
                &nix_config.nix_systems,
            )?;
            let flake_drvs_to_build: Vec<String> = flake_drvs_to_build
                .iter()
                .map(|nix_drv| format!("{}#{}", flake_path, nix_drv))
                .collect();
            let flake_drvs_and_outpaths: HashMap<String, String> =
                get_flake_drvs_outpaths(&flake_drvs_to_build);
            let flake_drvs_without_outpaths: HashMap<String, String> =
                flake_drvs_and_outpaths.clone();
            let flake_drvs_without_outpaths: Vec<String> = flake_drvs_without_outpaths
                .iter()
                .filter(|(_, v)| v.is_empty())
                .map(|(k, _)| k.chars().skip(51).take(k.len() - 51).collect::<String>())
                .collect();
            if !flake_drvs_without_outpaths.is_empty() {
                eprintln!(
                    "Warning: Expressions whose outPath could not be evaluated will not be built (`{}`)",
                    flake_drvs_without_outpaths.join(" ")
                );
            }

            do_nix_build(&flake_drvs_and_outpaths, &nix_config.machine_role);
            create_nix_gc_roots(&flake_drvs_and_outpaths, &nix_config.flake_local_reference);
            do_nix_sign(
                &flake_drvs_and_outpaths,
                &nix_config.signing_key_path,
                nix_config.ignore_signing_error,
            )?;
            do_nix_copy(
                &flake_drvs_and_outpaths,
                &nix_config.machine_role,
                &nix_config.nix_copy_machines,
                nix_config.copy_unsigned_paths,
            );
        }

        if nix_config.machine_role == MachineRole::QuickCI {
            break Ok(());
        }
    }
}

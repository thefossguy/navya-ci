use navya_lib::git_helpers::*;
use navya_lib::nix_helpers::*;

use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let nix_config = get_nix_config()?;
    eprintln!("{:#?}\n", nix_config);
    let mut flake_path = "".to_string();

    loop {
        perform_git_pull(&nix_config.flake_local_reference);
        perform_nix_flake_update(&nix_config)?;

        let current_flake_path = perform_nix_flake_archive(&nix_config.flake_local_reference)?;
        let has_missing_paths = has_missing_paths(&nix_config.flake_local_reference);

        if flake_path == current_flake_path || !has_missing_paths {
            std::thread::sleep(std::time::Duration::from_secs(nix_config.sleep_break));
        } else {
            flake_path = current_flake_path;
            eprintln!("\nNix store path for current flake state: {}", flake_path);

            let nix_derivations_to_build = get_nix_derivations_to_build(&nix_config, &flake_path)?;
            do_nix_build(&nix_derivations_to_build, &nix_config.machine_role);
            create_nix_gc_roots(&nix_derivations_to_build, &nix_config.flake_local_reference);
            do_nix_sign(
                &nix_derivations_to_build,
                &nix_config.signing_key_path,
                nix_config.ignore_signing_error,
            )?;
            do_nix_copy(
                &nix_derivations_to_build,
                &nix_config.machine_role,
                &nix_config.nix_copy_machines,
                nix_config.copy_unsigned_paths,
            );
            eprintln!(
                "Notice: All jobs for the current flake state ('{}') are complete",
                flake_path
            );
        }

        if nix_config.machine_role == MachineRole::QuickCI {
            break Ok(());
        }
    }
}

use crate::{dialogs, shared, shared::bundle, windows};
use anyhow::{anyhow, bail, Result};
use memmap2::Mmap;
use pretty_bytes_rust::pretty_bytes;
use std::{
    env,
    fs::{self, File},
    path::{Path, PathBuf},
    time::Duration,
};
use winsafe::{self as w, co};

pub fn install(debug_pkg: Option<&PathBuf>, install_to: Option<&PathBuf>) -> Result<()> {
    let osinfo = windows::os_info::get();
    info!("OS: {}, Arch={}", osinfo, osinfo.architecture().unwrap_or("unknown"));

    if !w::IsWindows7OrGreater()? {
        bail!("This installer requires Windows 7 or later and cannot run.");
    }

    let file = File::open(env::current_exe()?)?;
    let mmap = unsafe { Mmap::map(&file)? };
    let pkg = bundle::load_bundle_from_mmap(&mmap, debug_pkg)?;
    info!("Bundle loaded successfully.");

    // find and parse nuspec
    info!("Reading package manifest...");
    let app = pkg.read_manifest()?;

    info!("Package manifest loaded successfully.");
    info!("    Package ID: {}", &app.id);
    info!("    Package Version: {}", &app.version);
    info!("    Package Title: {}", &app.title);
    info!("    Package Authors: {}", &app.authors);
    info!("    Package Description: {}", &app.description);
    info!("    Package Machine Architecture: {}", &app.machine_architecture);
    info!("    Package Runtime Dependencies: {}", &app.runtime_dependencies);

    let _mutex = shared::retry_io(|| windows::create_global_mutex(&app))?;

    if !windows::prerequisite::prompt_and_install_all_missing(&app, None)? {
        info!("Cancelling setup. Pre-requisites not installed.");
        return Ok(());
    }

    info!("Determining install directory...");
    let (root_path, root_is_default) = if install_to.is_some() {
        (install_to.unwrap().clone(), false)
    } else {
        let appdata = w::SHGetKnownFolderPath(&co::KNOWNFOLDERID::LocalAppData, co::KF::DONT_UNEXPAND, None)?;
        (Path::new(&appdata).join(&app.id), true)
    };

    // path needs to exist for future operations (disk space etc)
    if !root_path.exists() {
        shared::retry_io(|| fs::create_dir_all(&root_path))?;
    }

    let root_path_str = root_path.to_str().unwrap();
    info!("Installation Directory: {:?}", root_path_str);

    // do we have enough disk space?
    let (compressed_size, extracted_size) = pkg.calculate_size();
    let required_space = compressed_size + extracted_size + (50 * 1000 * 1000); // archive + velopack overhead
    let mut free_space: u64 = 0;
    w::GetDiskFreeSpaceEx(Some(&root_path_str), None, None, Some(&mut free_space))?;
    if free_space < required_space {
        bail!(
            "{} requires at least {} disk space to be installed. There is only {} available.",
            &app.title,
            pretty_bytes(required_space, None),
            pretty_bytes(free_space, None)
        );
    }

    info!("There is {} free space available at destination, this package requires {}.", pretty_bytes(free_space, None), pretty_bytes(required_space, None));

    // does this app support this OS / architecture?
    if !app.os_min_version.is_empty() && !windows::is_os_version_or_greater(&app.os_min_version)? {
        bail!("This application requires Windows {} or later.", &app.os_min_version);
    }

    if !app.machine_architecture.is_empty() && !windows::is_cpu_architecture_supported(&app.machine_architecture)? {
        bail!("This application ({}) does not support your CPU architecture.", &app.machine_architecture);
    }

    let mut root_path_renamed = String::new();
    // does the target directory exist and have files? (eg. already installed)
    if !shared::is_dir_empty(&root_path) {
        // the target directory is not empty, and not dead
        if !dialogs::show_overwrite_repair_dialog(&app, &root_path, root_is_default) {
            // user cancelled overwrite prompt
            error!("Directory exists, and user cancelled overwrite.");
            return Ok(());
        }

        shared::force_stop_package(&root_path)
            .map_err(|z| anyhow!("Failed to stop application ({}), please close the application and try running the installer again.", z))?;

        root_path_renamed = format!("{}_{}", root_path_str, shared::random_string(8));
        info!("Renaming existing directory to '{}' to allow rollback...", root_path_renamed);

        shared::retry_io(|| fs::rename(&root_path, &root_path_renamed)).map_err(|_| {
            anyhow!(
                "Failed to remove existing application directory, please close the application and try running the installer again. \
                If the issue persists, try uninstalling first via Programs & Features, or restarting your computer."
            )
        })?;
    }

    info!("Preparing and cleaning installation directory...");
    remove_dir_all::ensure_empty_dir(&root_path)?;

    let tx = if dialogs::get_silent() {
        info!("Will not show splash because silent mode is on.");
        let (tx, _) = std::sync::mpsc::channel::<i16>();
        tx
    } else {
        info!("Reading splash image...");
        let splash_bytes = pkg.get_splash_bytes();
        windows::splash::show_splash_dialog(app.title.to_owned(), splash_bytes)
    };
    
    let install_result = install_impl(&pkg, &root_path, &tx);
    let _ = tx.send(windows::splash::MSG_CLOSE);

    if install_result.is_ok() {
        info!("Installation completed successfully!");
        if !root_path_renamed.is_empty() {
            info!("Removing rollback directory...");
            let _ = shared::retry_io(|| fs::remove_dir_all(&root_path_renamed));
        }
    } else {
        error!("Installation failed!");
        if !root_path_renamed.is_empty() {
            info!("Rolling back installation...");
            let _ = shared::force_stop_package(&root_path);
            let _ = shared::retry_io(|| fs::remove_dir_all(&root_path));
            let _ = shared::retry_io(|| fs::rename(&root_path_renamed, &root_path));
        }
        install_result?;
    }

    Ok(())
}

fn install_impl(pkg: &bundle::BundleInfo, root_path: &PathBuf, tx: &std::sync::mpsc::Sender<i16>) -> Result<()> {
    info!("Starting installation!");

    let app = pkg.read_manifest()?;

    // all application paths
    let updater_path = app.get_update_path(root_path);
    let packages_path = app.get_packages_path(root_path);
    let current_path = app.get_current_path(root_path);
    let nupkg_path = app.get_target_nupkg_path(root_path);
    let main_exe_path = app.get_main_exe_path(root_path);

    info!("Extracting Update.exe...");
    let _ = pkg
        .extract_zip_predicate_to_path(|name| name.ends_with("Squirrel.exe"), updater_path)
        .map_err(|_| anyhow!("This installer is missing a critical binary (Update.exe). Please contact the application author."))?;
    let _ = tx.send(5);

    info!("Copying nupkg to packages directory...");
    shared::retry_io(|| fs::create_dir_all(&packages_path))?;
    pkg.copy_bundle_to_file(&nupkg_path)?;
    let _ = tx.send(10);

    pkg.extract_lib_contents_to_path(&current_path, |p| {
        let _ = tx.send(((p as f32) / 100.0 * 80.0 + 10.0) as i16);
    })?;

    if !Path::new(&main_exe_path).exists() {
        bail!("The main executable could not be found in the package. Please contact the application author.");
    }

    info!("Creating new default shortcuts...");
    let _ = windows::create_default_lnks(&root_path, &app);

    let ver_string = app.version.to_string();
    info!("Starting process install hook: \"{}\" --veloapp-install {}", &main_exe_path, &ver_string);
    let args = vec!["--veloapp-install", &ver_string];
    if let Err(e) = windows::run_process_no_console_and_wait(&main_exe_path, args, &current_path, Some(Duration::from_secs(30))) {
        let setup_name = format!("{} Setup {}", app.title, app.version);
        error!("Process install hook failed: {}", e);
        let _ = tx.send(windows::splash::MSG_CLOSE);
        dialogs::show_warn(
            &setup_name,
            None,
            format!("Installation has completed, but the application install hook failed ({}). It may not have installed correctly.", e).as_str(),
        );
    }

    let _ = tx.send(100);

    app.write_uninstall_entry(root_path)?;

    if !dialogs::get_silent() {
        info!("Starting app...");
        shared::start_package(&app, &root_path, None, Some("VELOPACK_FIRSTRUN"))?;
    }

    Ok(())
}

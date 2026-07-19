use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    if cfg!(windows) && env::var_os("CARGO_FEATURE_INSTALLER_UI").is_some() {
        let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
        let manifest = manifest_dir.join("installer.manifest");
        let icon = manifest_dir.join("assets/installer.ico");
        let resource_script = manifest_dir.join("installer.rc");
        let resource = compile_windows_resource(&manifest_dir, &resource_script);
        // Explicit asInvoker metadata disables Windows' filename-based installer
        // elevation heuristic; protected writes request elevation only on demand.
        println!("cargo:rustc-link-arg-bin=BeatblockOnlineInstaller=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=BeatblockOnlineInstaller=/MANIFESTINPUT:{}",
            manifest.display()
        );
        println!(
            "cargo:rustc-link-arg-bin=BeatblockOnlineInstaller={}",
            resource.display()
        );
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rerun-if-changed={}", resource_script.display());
        println!("cargo:rerun-if-changed={}", icon.display());
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("lovely-version.dll");
    let explicit = env::var_os("BBT_LOVELY_DLL").map(PathBuf::from);
    // Release and developer builds consume the generated, pinned Lovely
    // artifact. The release orchestrator creates it from the exact upstream
    // commit plus our reviewed patch; no binary input lives in Git.
    let repository_fixture = PathBuf::from("../artifacts/lovely/version.dll");
    // Track the expected path even when it does not exist yet. Release jobs run
    // source checks before building payloads, and Cargo must re-run this script
    // once the verified DLL appears.
    println!("cargo:rerun-if-changed={}", repository_fixture.display());
    let source = explicit
        .filter(|path| path.is_file())
        .or_else(|| repository_fixture.is_file().then_some(repository_fixture));
    if let Some(source) = source {
        fs::copy(&source, &output).expect("copy Lovely payload");
    } else {
        fs::write(&output, []).expect("write empty Lovely payload");
        println!("cargo:warning=Lovely DLL payload is unavailable; set BBT_LOVELY_DLL for release builds");
    }
    println!("cargo:rerun-if-env-changed=BBT_LOVELY_DLL");
    let obs_output =
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("beatblock-online-obs.dll");
    let obs = env::var_os("BBT_OBS_PLUGIN_DLL")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| {
            // Release builds use the generated OBS 32 artifact. Developer
            // builds can still override it with BBT_OBS_PLUGIN_DLL.
            let path = PathBuf::from("../artifacts/obs/beatblock-online-obs.dll");
            println!("cargo:rerun-if-changed={}", path.display());
            path.is_file().then_some(path)
        });
    if let Some(source) = obs.as_ref() {
        fs::copy(source, obs_output).expect("copy OBS plugin");
        println!("cargo:rerun-if-changed={}", source.display());
    } else {
        fs::write(obs_output, []).expect("write empty OBS payload");
        println!(
            "cargo:warning=OBS plugin payload is unavailable; set BBT_OBS_PLUGIN_DLL for release builds"
        );
    }
    println!("cargo:rerun-if-env-changed=BBT_OBS_PLUGIN_DLL");

    // The public release is one installer download. The release packager builds
    // the lean runtime first and passes its exact artifact into this payload.
    let runtime_output =
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("BeatblockOnlineRuntime.exe");
    let runtime = env::var_os("BBT_RUNTIME_EXE")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| {
            let path = PathBuf::from("target/release/BeatblockOnlineRuntime.exe");
            path.is_file().then_some(path)
        });
    if let Some(source) = runtime {
        fs::copy(&source, &runtime_output).expect("copy runtime payload");
        println!("cargo:rerun-if-changed={}", source.display());
    } else if env::var_os("CARGO_FEATURE_INSTALLER_UI").is_none() {
        // Library tests exercise transactional installer behavior without
        // building the Windows GUI bundle first. Keep the placeholder scoped
        // to non-installer builds; real installer binaries still fail closed
        // when the separately built runtime artifact is missing.
        fs::write(
            runtime_output,
            b"MZ\0Beatblock Online test-only runtime payload",
        )
        .expect("write test runtime payload");
    } else {
        fs::write(runtime_output, []).expect("write empty runtime payload");
        println!(
            "cargo:warning=Runtime payload is unavailable; build BeatblockOnlineRuntime first"
        );
    }
    println!("cargo:rerun-if-env-changed=BBT_RUNTIME_EXE");
}

/// Compile the installer icon without adding a build dependency. Windows SDK's
/// resource compiler is present beside the MSVC linker on supported build hosts.
fn compile_windows_resource(manifest_dir: &Path, script: &Path) -> PathBuf {
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("installer-icon.res");
    let compiler = find_resource_compiler().unwrap_or_else(|| {
        panic!("Windows SDK resource compiler rc.exe was not found; install Windows Build Tools")
    });
    let status = Command::new(&compiler)
        .current_dir(manifest_dir)
        .args(["/nologo", "/fo"])
        .arg(&output)
        .arg(script)
        .status()
        .unwrap_or_else(|error| panic!("run {}: {error}", compiler.display()));
    assert!(status.success(), "rc.exe failed to compile installer.rc");
    output
}

fn find_resource_compiler() -> Option<PathBuf> {
    if let Some(explicit) = env::var_os("RC").map(PathBuf::from) {
        if explicit.is_file() {
            return Some(explicit);
        }
    }
    if let Some(sdk_bin) = env::var_os("WindowsSdkVerBinPath").map(PathBuf::from) {
        let candidate = sdk_bin.join("x64/rc.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let kits = env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)?
        .join("Windows Kits/10/bin");
    let mut versions = fs::read_dir(kits)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    versions.sort();
    versions
        .into_iter()
        .rev()
        .map(|path| path.join("x64/rc.exe"))
        .find(|path| path.is_file())
}

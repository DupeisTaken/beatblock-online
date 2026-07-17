use std::{env, fs, path::PathBuf};

fn main() {
    if cfg!(windows) && env::var_os("CARGO_FEATURE_INSTALLER_UI").is_some() {
        let manifest =
            PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("installer.manifest");
        // Explicit asInvoker metadata disables Windows' filename-based installer
        // elevation heuristic; protected writes request elevation only on demand.
        println!("cargo:rustc-link-arg-bin=BeatblockTogetherInstaller=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=BeatblockTogetherInstaller=/MANIFESTINPUT:{}",
            manifest.display()
        );
        println!("cargo:rerun-if-changed={}", manifest.display());
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("lovely-version.dll");
    let explicit = env::var_os("BBT_LOVELY_DLL").map(PathBuf::from);
    // Release and developer builds consume the generated, pinned Lovely
    // artifact. The release orchestrator creates it from the exact upstream
    // commit plus our reviewed patch; no binary input lives in Git.
    let repository_fixture = PathBuf::from("../artifacts/lovely/version.dll");
    let source = explicit
        .filter(|path| path.is_file())
        .or_else(|| repository_fixture.is_file().then_some(repository_fixture));
    if let Some(source) = source {
        fs::copy(&source, &output).expect("copy Lovely payload");
        println!("cargo:rerun-if-changed={}", source.display());
    } else {
        fs::write(&output, []).expect("write empty Lovely payload");
        println!("cargo:warning=Lovely DLL payload is unavailable; set BBT_LOVELY_DLL for release builds");
    }
    println!("cargo:rerun-if-env-changed=BBT_LOVELY_DLL");
    let obs_output =
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("beatblock-together-obs.dll");
    let obs = env::var_os("BBT_OBS_PLUGIN_DLL")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| {
            // Release builds use the generated OBS 32 artifact. Developer
            // builds can still override it with BBT_OBS_PLUGIN_DLL.
            let path = PathBuf::from("../artifacts/obs/beatblock-together-obs.dll");
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
            b"MZ\0Beatblock Together test-only runtime payload",
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

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
    // Release and developer builds use the reviewed, pinned Lovely artifact.
    // Never pull a DLL out of a mutable injected test game: doing so makes the
    // installer payload depend on whatever a previous trial happened to leave.
    let repository_fixture =
        PathBuf::from("../.reference/lovely-injector/target/release/version.dll");
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
            let path = PathBuf::from("../obs-plugin/build/Release/beatblock-together-obs.dll");
            path.is_file().then_some(path)
        });
    if let Some(source) = obs {
        fs::copy(source, obs_output).expect("copy OBS plugin");
    } else {
        fs::write(obs_output, []).expect("write empty OBS payload");
    }
    println!("cargo:rerun-if-env-changed=BBT_OBS_PLUGIN_DLL");

    // The public release is one installer download. The release packager builds
    // the lean runtime first and passes its exact artifact into this payload.
    let runtime_output =
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("BeatblockTogetherRuntime.exe");
    let runtime = env::var_os("BBT_RUNTIME_EXE")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| {
            let path = PathBuf::from("target/release/BeatblockTogetherRuntime.exe");
            path.is_file().then_some(path)
        });
    if let Some(source) = runtime {
        fs::copy(&source, &runtime_output).expect("copy runtime payload");
        println!("cargo:rerun-if-changed={}", source.display());
    } else {
        fs::write(runtime_output, []).expect("write empty runtime payload");
        println!(
            "cargo:warning=Runtime payload is unavailable; build BeatblockTogetherRuntime first"
        );
    }
    println!("cargo:rerun-if-env-changed=BBT_RUNTIME_EXE");
}
